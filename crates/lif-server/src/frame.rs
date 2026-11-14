//! Views, deadlines and frames.
//!
//! Per frame tick, exactly as the `Renderer` docs prescribe: every pane
//! visible to a client that gets a frame now is refreshed once into its
//! single `PaneFrame`; every attached client showing a refreshed pane then
//! either renders (it is due) or `skip`s (paced, its socket is full, or a
//! pane it shows is inside a synchronized update); then the dirty flags of
//! the refreshed frames are cleared.

use std::collections::HashMap;
use std::time::{Duration, Instant};

use lif_core::{ClientId, PaneId};
use lif_render::{Message, Scene, ScenePane, StatusBar, StatusPosition, Theme, pane_cursor};
use lif_types::{Cursor, Pos, Rect, Size, Style};
use lif_vt::VtEngine;

use crate::client::ClientRt;
use crate::pane::PaneRt;
use crate::server::Server;
use crate::{MIN_FRAME_INTERVAL, SYNC_TIMEOUT};

/// Automatic window names are re-derived at most this often per window.
const NAME_CHECK_INTERVAL: Duration = Duration::from_secs(1);

/// Reuses the allocation of a scratch vector for a borrow with a shorter
/// lifetime (the vector is empty, so no value changes type). `filter_map`
/// because it may change the element type and still collects in place.
#[allow(clippy::unnecessary_filter_map)]
fn recycle<'a>(v: &mut Vec<ScenePane<'static>>) -> Vec<ScenePane<'a>> {
    let mut v = std::mem::take(v);
    v.clear();
    v.into_iter().filter_map(|_| None).collect()
}

#[allow(clippy::unnecessary_filter_map)]
fn unrecycle(v: Vec<ScenePane<'_>>) -> Vec<ScenePane<'static>> {
    let mut v = v;
    v.clear();
    v.into_iter().filter_map(|_| None).collect()
}

fn clip(r: Rect, size: Size) -> Option<Rect> {
    if r.x >= size.cols || r.y >= size.rows {
        return None;
    }
    Some(Rect::new(r.x, r.y, r.cols.min(size.cols - r.x), r.rows.min(size.rows - r.y)))
}

impl Server {
    /// Recompute client views from the model after it changed.
    pub(crate) fn sync_views(&mut self) {
        if !self.rt.views_stale {
            return;
        }
        self.rt.views_stale = false;
        let mut ids = std::mem::take(&mut self.scratch_views);
        ids.clear();
        ids.extend(self.rt.clients.values().filter(|c| c.att.is_some()).map(|c| c.id));
        for &id in &ids {
            let Some(view) = self.model.client_view(id) else { continue };
            let mouse = self.model.option_text("mouse", Some(view.session), None).as_deref() == Some("on");
            let (theme, msg_style) = self.theme(view.session, view.active_pane);
            let Some(a) = self.rt.clients.get_mut(&id).and_then(|c| c.att.as_mut()) else { continue };
            let old = a.active_pane;
            let new = view.active_pane;
            let focused = a.focused;
            a.mouse_opt = mouse;
            if a.theme != theme || a.msg_style != msg_style {
                a.theme = theme;
                a.msg_style = msg_style;
                a.status_stale = true;
            }
            a.view = Some(view);
            a.active_pane = Some(new);
            a.needs_frame = true;
            if old != Some(new) && focused {
                if let Some(o) = old {
                    self.focus_pane(o, false);
                }
                self.focus_pane(new, true);
            }
        }
        self.scratch_views = ids;
    }

    /// The styles lif draws its own decorations in, from the options of
    /// the session and of the window holding `pane` (tmux's
    /// `pane-border-style`, `pane-active-border-style`, `mode-style`,
    /// `copy-mode-match-style`, `copy-mode-current-match-style`,
    /// `message-style`). Only computed when views are, never per frame.
    fn theme(&self, session: lif_core::SessionId, pane: PaneId) -> (Theme, Style) {
        let w = self.model.pane(pane).map(|p| p.window());
        let style = |name: &str| lif_core::parse_style(&self.model.option_text(name, Some(session), w).unwrap_or_default());
        let mode = style("mode-style");
        let theme = Theme {
            border: style("pane-border-style"),
            active_border: style("pane-active-border-style"),
            selection: mode,
            indicator: mode,
            search_match: style("copy-mode-match-style"),
            current_match: style("copy-mode-current-match-style"),
        };
        (theme, style("message-style"))
    }

    pub(crate) fn timers(&mut self, now: Instant) {
        let now_ms = self.now_ms(now);
        let mut ids = std::mem::take(&mut self.scratch_clients);
        ids.clear();
        ids.extend(self.rt.clients.values().filter(|c| c.att.is_some()).map(|c| c.id));
        for &id in &ids {
            let Some(a) = self.rt.clients.get_mut(&id).and_then(|c| c.att.as_mut()) else { continue };
            if a.input_deadline.is_some_and(|d| d <= now) {
                self.flush_input(id, now);
            }
            let Some(a) = self.rt.clients.get_mut(&id).and_then(|c| c.att.as_mut()) else { continue };
            if a.key_deadline.is_some_and(|d| d <= now) {
                a.key_deadline = None;
                if self.model.expire_key_state(id, now_ms) {
                    a.status_stale = true;
                    a.needs_frame = true;
                }
            }
            if a.message.as_ref().and_then(|m| m.expires).is_some_and(|d| d <= now) {
                a.message = None;
                a.needs_frame = true;
            }
            if a.display_panes.as_ref().is_some_and(|d| d.expires <= now) {
                a.display_panes = None;
                a.needs_frame = true;
            }
            if a.clock && a.clock_deadline.is_some_and(|d| d <= now) {
                a.clock_deadline = None;
                a.status_stale = true;
                a.needs_frame = true;
            }
        }
        self.scratch_clients = ids;
        let mut scroll = Vec::new();
        for p in self.rt.panes.values() {
            if p.autoscroll_at.is_some_and(|d| d <= now) {
                scroll.push(p.id);
            }
        }
        for id in scroll {
            if let Some(p) = self.rt.panes.get_mut(&id) {
                p.autoscroll_at = None;
            }
            let act = self.with_mode(id, |m, ctx| m.autoscroll_tick(ctx));
            self.mode_action(None, id, act);
        }
        let mut renamed = Vec::new();
        for p in self.rt.panes.values_mut() {
            if p.compact_at.is_some_and(|d| d <= now) {
                p.compact_at = None;
                p.engine.compact();
            }
            if p.name_recheck.is_some_and(|d| d <= now) {
                p.name_recheck = None;
                if check_name(p, now) {
                    renamed.push(p.id);
                }
            }
        }
        for p in renamed {
            self.model.pane_changed(p, &mut self.rt);
        }
    }

    pub(crate) fn frame_tick(&mut self, now: Instant) {
        let mut due = std::mem::take(&mut self.due);
        due.clear();
        {
            let panes = &self.rt.panes;
            for (id, c) in self.rt.clients.iter_mut() {
                let pending = c.out_pending() || c.closing;
                let Some(a) = c.att.as_mut() else { continue };
                a.frame_deadline = None;
                let Some(view) = a.view.as_ref() else { continue };
                let mut dirty = a.needs_frame;
                let mut hold: Option<Instant> = None;
                for (pid, _) in &view.panes {
                    if let Some(p) = panes.get(pid) {
                        dirty |= p.engine.is_dirty();
                        if let Some(t) = p.sync_since {
                            let until = t + SYNC_TIMEOUT;
                            if now < until {
                                hold = Some(hold.map_or(until, |h| h.min(until)));
                            }
                        }
                    }
                }
                if !dirty {
                    continue;
                }
                a.needs_frame = true;
                if pending {
                    // The socket is full: rendered when it drains.
                    continue;
                }
                if let Some(h) = hold {
                    a.frame_deadline = Some(h);
                    continue;
                }
                if let Some(last) = a.last_frame
                    && now < last + MIN_FRAME_INTERVAL
                {
                    a.frame_deadline = Some(last + MIN_FRAME_INTERVAL);
                    continue;
                }
                due.push(*id);
            }
        }
        if due.is_empty() {
            self.due = due;
            return;
        }
        for &id in &due {
            self.prepare_chrome(id, now);
        }
        let mut refreshed = std::mem::take(&mut self.refreshed);
        refreshed.clear();
        for &id in &due {
            let Some(view) = self.rt.clients.get(&id).and_then(|c| c.att.as_ref()).and_then(|a| a.view.as_ref()) else { continue };
            for (pid, _) in &view.panes {
                if let Some(p) = self.rt.panes.get_mut(pid)
                    && !p.refreshed
                {
                    p.engine.refresh(&mut p.frame);
                    p.refreshed = true;
                    refreshed.push(*pid);
                }
            }
        }
        let mut scratch = std::mem::take(&mut self.scene_scratch);
        let mut ids = std::mem::take(&mut self.scratch_clients);
        ids.clear();
        ids.extend(self.rt.clients.keys().copied());
        for &id in &ids {
            let is_due = due.contains(&id);
            let Some(c) = self.rt.clients.get_mut(&id) else { continue };
            let Some(a) = c.att.as_ref() else { continue };
            let Some(view) = a.view.as_ref() else { continue };
            if !is_due && !view.panes.iter().any(|(p, _)| refreshed.contains(p)) {
                continue;
            }
            draw_client(&self.rt.panes, c, is_due, &mut scratch);
            if is_due && let Some(a) = c.att.as_mut() {
                a.needs_frame = false;
                a.last_frame = Some(now);
            }
            if is_due {
                self.rt.flush_client(id);
            }
        }
        self.scene_scratch = scratch;
        self.scratch_clients = ids;
        for pid in &refreshed {
            if let Some(p) = self.rt.panes.get_mut(pid) {
                p.frame.clear_dirty();
                if let Some(m) = p.mode.as_mut() {
                    m.clear_frame_dirty();
                }
                p.refreshed = false;
            }
        }
        self.refreshed = refreshed;
        self.due = due;
    }

    /// Status line (and automatic window names) for a client about to get
    /// a frame.
    fn prepare_chrome(&mut self, id: ClientId, now: Instant) {
        let Some(c) = self.rt.clients.get(&id) else { return };
        let Some(a) = c.att.as_ref() else { return };
        let Some(view) = a.view.as_ref() else { return };
        if view.status_row.is_none() {
            return;
        }
        let sid = view.session;
        let width = c.hello.as_ref().map(|h| h.size.cols).unwrap_or(80);
        let mut changed = Vec::new();
        if let Some(s) = self.model.session(sid) {
            for (_, wid) in s.windows() {
                let Some(w) = self.model.window(wid) else { continue };
                let ap = w.active_pane();
                let Some(p) = self.rt.panes.get_mut(&ap) else { continue };
                if p.pty.is_none() {
                    continue;
                }
                // A frame drawn only for the clock schedules nothing.
                if p.output_seen {
                    p.name_recheck = Some(now + NAME_CHECK_INTERVAL);
                }
                if p.name_checked.is_some_and(|t| now < t + NAME_CHECK_INTERVAL) {
                    continue;
                }
                if check_name(p, now) {
                    changed.push(ap);
                }
            }
        }
        for p in changed {
            self.model.pane_changed(p, &mut self.rt);
        }
        let stale = self.rt.clients.get(&id).and_then(|c| c.att.as_ref()).is_some_and(|a| a.status_stale);
        if !stale {
            return;
        }
        let wall = crate::sys::wall_clock();
        let line = self.model.status_line(sid, width, wall, &self.rt);
        let clock = ["status-left", "status-right"]
            .iter()
            .any(|o| self.model.option_text(o, Some(sid), None).is_some_and(|v| v.contains('%')));
        let Some(a) = self.rt.clients.get_mut(&id).and_then(|c| c.att.as_mut()) else { return };
        a.status = line;
        a.status_stale = false;
        a.clock = clock;
        a.clock_deadline = clock.then(|| {
            let into = wall.unix_secs.rem_euclid(60) as u64;
            now + Duration::from_secs(60 - into)
        });
    }
}

/// Re-reads the pane's foreground process name; true if it changed.
fn check_name(p: &mut PaneRt, now: Instant) -> bool {
    p.name_checked = Some(now);
    p.output_seen = false;
    match p.pty.as_ref().and_then(|t| t.foreground_process_name()) {
        Some(name) if name != p.current_command => {
            p.current_command = name;
            true
        }
        _ => false,
    }
}

/// Builds the client's scene and renders it (or records a skipped frame).
fn draw_client(panes: &HashMap<PaneId, PaneRt>, c: &mut ClientRt, render: bool, scratch: &mut Vec<ScenePane<'static>>) {
    let size = c.hello.as_ref().map(|h| h.size).unwrap_or_default();
    // Mouse reporting on the outer terminal only while someone needs it.
    if render && let Some(a) = c.att.as_ref() {
        let active = a.view.as_ref().and_then(|v| panes.get(&v.active_pane));
        let m = active.map(|p| p.engine.modes()).unwrap_or_default();
        let want = (a.mouse_opt || m.mouse_tracking, m.mouse_tracking && m.mouse_motion);
        let want = if want.0 { want } else { (false, false) };
        if want != a.mouse {
            let caps = a.renderer.caps();
            let mut b = Vec::new();
            lif_input::write_mouse(want.0, want.1, &caps, &mut b);
            c.send_output(&b);
            if let Some(a) = c.att.as_mut() {
                a.mouse = want;
            }
        }
    }
    let start = if render { Some(c.begin_frame()) } else { None };
    let ClientRt { att, out, .. } = c;
    let Some(a) = att.as_mut() else { return };
    let mut sp = recycle(scratch);
    let mut prompt_col = 0;
    if let Some(p) = a.prompt.as_ref() {
        prompt_col = p.render(&mut a.prompt_line, a.msg_style);
    }
    {
        let Some(view) = a.view.as_ref() else {
            *scratch = unrecycle(sp);
            return;
        };
        let mut active_cursor = Cursor::default();
        for (i, (pid, rect)) in view.panes.iter().enumerate() {
            let Some(p) = panes.get(pid) else { continue };
            let Some(rect) = clip(*rect, size) else { continue };
            let label = a.display_panes.as_ref().and_then(|d| d.labels.get(i)).map(|(_, l)| l.as_str());
            let mode = p.mode.as_deref();
            let ov = mode.map(|m| m.overlays());
            let indicator = label.or_else(|| ov.map(|o| o.indicator.as_str()).filter(|s| !s.is_empty()));
            let active = *pid == view.active_pane;
            if active {
                active_cursor = match ov {
                    Some(o) if o.cursor.x < rect.cols && o.cursor.y < rect.rows => Cursor {
                        pos: Some(Pos { x: rect.x + o.cursor.x, y: rect.y + o.cursor.y }),
                        visible: true,
                        ..Default::default()
                    },
                    Some(_) => Cursor::default(),
                    None => pane_cursor(rect, &p.frame),
                };
            }
            sp.push(ScenePane {
                id: pid.0 as u64,
                rect,
                frame: mode.and_then(|m| m.frame()).unwrap_or(&p.frame),
                active,
                selection: ov.and_then(|o| o.selection),
                matches: ov.map_or(&[], |o| o.matches.as_slice()),
                indicator,
            });
        }
        let status = view.status_row.filter(|r| *r < size.rows).map(|row| StatusBar {
            line: &a.status,
            fill: a.status.first().map(|s| s.style).unwrap_or_default(),
            position: if row == 0 && size.rows > 1 && view.panes.iter().all(|(_, r)| r.y >= 1) {
                StatusPosition::Top
            } else {
                StatusPosition::Bottom
            },
        });
        let message = if a.prompt.is_some() {
            Some(Message { line: &a.prompt_line, fill: a.msg_style })
        } else {
            a.message.as_ref().map(|m| Message { line: &m.line, fill: a.msg_style })
        };
        let mut scene = Scene::new(size, &sp);
        scene.theme = a.theme;
        scene.status = status;
        scene.message = message;
        scene.cursor = active_cursor;
        if a.prompt.is_some() {
            let row = scene.chrome_row().unwrap_or(size.rows.saturating_sub(1));
            scene.cursor = Cursor { pos: Some(Pos { x: prompt_col.min(size.cols.saturating_sub(1)), y: row }), visible: true, ..Default::default() };
        }
        let sp_ref: &[ScenePane<'_>] = &sp;
        let lookup = |i: usize, link: u32| -> Option<&str> {
            let id = sp_ref.get(i)?.id;
            panes.get(&PaneId(id as u32))?.engine.hyperlink(link)
        };
        scene.links = Some(&lookup);
        if render {
            a.renderer.render(&scene, out);
        } else {
            a.renderer.skip(&scene);
        }
    }
    *scratch = unrecycle(sp);
    if let Some(s) = start {
        c.end_frame(s);
    }
}
