//! Control-mode clients (`lif -C`, tmux's control mode): attached
//! without a terminal. Each line they send is a command list, answered
//! with `%begin` ... `%end` (or `%error`); panes they can see send their
//! output as `%output %N data`; changes to sessions and windows come as
//! notifications (`%window-add`, `%layout-change`, ...). An empty line
//! detaches (`%exit`). The text is tmux 3.8's.
//!
//! Nothing is queued without a bound: a client that reads its socket too
//! slowly stops getting a pane's output (`%pause %N`) until it has caught
//! up (`%continue %N`), and so the pane is never held up.

use std::collections::{HashMap, VecDeque};
use std::time::Instant;

use lif_core::{ClientId, PaneId, SessionId};

use crate::server::Server;

/// Longest command line a control client may send, in bytes; a longer one
/// is answered with an error and dropped.
pub(crate) const CONTROL_LINE_MAX: usize = 1024 * 1024;

/// Protocol text that may wait for a control client, in bytes. Past it,
/// pane output is paused for that client; answers and notifications are
/// small and still go out (up to twice this, then the client is too slow
/// and is dropped).
pub(crate) const CONTROL_BACKLOG_MAX: usize = 1024 * 1024;

#[derive(Default)]
pub(crate) struct Control {
    line: Vec<u8>,
    /// tmux numbers the commands of a client.
    next_cmd: u64,
    /// Commands still running (`run-shell` and the like): number and time,
    /// answered in order as they finish.
    pub(crate) pending: VecDeque<(u64, i64)>,
    /// Panes whose output is paused for this client.
    pub(crate) paused: Vec<PaneId>,
    /// The last layout told for each window (`%layout-change` when it
    /// differs).
    layouts: HashMap<lif_core::WindowId, String>,
}

/// `%output` data: bytes below space and backslash as `\ooo`, the rest
/// as they are (tmux's control_append_data).
pub(crate) fn escape(data: &[u8], out: &mut Vec<u8>) {
    for &b in data {
        if b < b' ' || b == b'\\' {
            out.extend_from_slice(format!("\\{b:03o}").as_bytes());
        } else {
            out.push(b);
        }
    }
}

fn unix_now() -> i64 {
    crate::sys::wall_clock().unix_secs
}

impl Server {
    /// What a control client sent: complete lines are run as commands.
    pub(crate) fn control_input(&mut self, id: ClientId, bytes: &[u8]) {
        let mut lines = Vec::new();
        {
            let Some(ctl) = self.rt.clients.get_mut(&id).and_then(|c| c.control.as_mut()) else { return };
            for &b in bytes {
                if b == b'\n' {
                    let mut l = std::mem::take(&mut ctl.line);
                    if l.last() == Some(&b'\r') {
                        l.pop();
                    }
                    lines.push(Some(l));
                } else if ctl.line.len() < CONTROL_LINE_MAX {
                    ctl.line.push(b);
                } else if !ctl.line.is_empty() {
                    ctl.line.clear();
                    lines.push(None);
                }
            }
        }
        for l in lines {
            let Some(c) = self.rt.clients.get(&id) else { return };
            if c.closing || c.dead {
                return;
            }
            match l {
                None => self.control_answer(id, Err("command too long".into())),
                // An empty line ends control mode, as in tmux.
                Some(l) if l.is_empty() => {
                    if let Ok(list) = lif_core::parse_string("detach-client") {
                        let _ = self.run_commands(Some(id), &list);
                    }
                    return;
                }
                Some(l) => {
                    let text = String::from_utf8_lossy(&l).into_owned();
                    // What the command does is told after its answer, as
                    // in tmux.
                    self.rt.control_hold = true;
                    let r = match lif_core::parse_string(&text) {
                        Ok(list) => self.run_commands(Some(id), &list).map_err(|e| e.to_string()),
                        Err(e) => Err(format!("parse error: {e}")),
                    };
                    self.rt.control_hold = false;
                    match r {
                        Ok(o) if o.pending => {
                            let n = self.control_begin(id);
                            if let Some(ctl) = self.rt.clients.get_mut(&id).and_then(|c| c.control.as_mut()) {
                                ctl.pending.push_back(n);
                            }
                        }
                        Ok(o) => self.control_answer(id, Ok(o.stdout)),
                        Err(e) => self.control_answer(id, Err(e)),
                    }
                    self.control_notify(Instant::now());
                }
            }
        }
        self.rt.flush_client(id);
    }

    /// `%begin` for the next command of `id`: its number and time.
    fn control_begin(&mut self, id: ClientId) -> (u64, i64) {
        let Some(c) = self.rt.clients.get_mut(&id) else { return (0, 0) };
        let Some(ctl) = c.control.as_mut() else { return (0, 0) };
        let n = (ctl.next_cmd, unix_now());
        ctl.next_cmd += 1;
        c.send_control(format!("%begin {} {} 1\n", n.1, n.0).as_bytes(), true);
        n
    }

    /// A command's answer, from `%begin` to `%end` or `%error`.
    pub(crate) fn control_answer(&mut self, id: ClientId, r: Result<String, String>) {
        let n = self.control_begin(id);
        self.control_end(id, n, r);
    }

    pub(crate) fn control_end(&mut self, id: ClientId, (n, t): (u64, i64), r: Result<String, String>) {
        let Some(c) = self.rt.clients.get_mut(&id) else { return };
        let (text, ok) = match r {
            Ok(s) => (s, true),
            Err(s) => (s, false),
        };
        let mut out = text.into_bytes();
        if !out.is_empty() && out.last() != Some(&b'\n') {
            out.push(b'\n');
        }
        out.extend_from_slice(format!("%{} {t} {n} 1\n", if ok { "end" } else { "error" }).as_bytes());
        c.send_control(&out, true);
        self.rt.flush_client(id);
    }

    /// The first lines a control client gets once attached: the answer to
    /// the command that attached it, and its session.
    pub(crate) fn control_attached(&mut self, id: ClientId, session: SessionId) {
        let name = self.model.session(session).map(|s| s.name().to_string()).unwrap_or_default();
        let Some(c) = self.rt.clients.get_mut(&id) else { return };
        let Some(ctl) = c.control.as_mut() else { return };
        let (n, t) = (ctl.next_cmd, unix_now());
        ctl.next_cmd += 1;
        c.send_control(format!("%begin {t} {n} 0\n%end {t} {n} 0\n%session-changed {session} {name}\n").as_bytes(), true);
    }

    /// Output of `pane` for the control clients that see it.
    pub(crate) fn control_output(&mut self, pane: PaneId, data: &[u8]) {
        let Some(w) = self.model.pane(pane).and_then(|p| self.model.window(p.window())) else { return };
        let sessions: Vec<SessionId> = w.links().iter().map(|l| l.0).collect();
        let mut line = Vec::with_capacity(data.len() + 16);
        line.extend_from_slice(format!("%output {pane} ").as_bytes());
        escape(data, &mut line);
        line.push(b'\n');
        let ids: Vec<ClientId> = self.rt.clients.values().filter(|c| c.control.is_some() && c.att.as_ref().is_some_and(|a| sessions.contains(&a.session))).map(|c| c.id).collect();
        for id in ids {
            let Some(c) = self.rt.clients.get_mut(&id) else { continue };
            let backlog = c.control_backlog();
            let Some(ctl) = c.control.as_mut() else { continue };
            if ctl.paused.contains(&pane) {
                continue;
            }
            if backlog + line.len() > CONTROL_BACKLOG_MAX {
                ctl.paused.push(pane);
                c.send_control(format!("%pause {pane}\n").as_bytes(), true);
            } else {
                c.send_control(&line, false);
            }
            self.rt.flush_client(id);
        }
    }

    /// Paused output resumes for clients that have caught up (half the
    /// bound); called when a client's socket took data.
    pub(crate) fn control_drained(&mut self, id: ClientId) {
        let Some(c) = self.rt.clients.get_mut(&id) else { return };
        if c.control_backlog() > CONTROL_BACKLOG_MAX / 2 {
            return;
        }
        let Some(ctl) = c.control.as_mut() else { return };
        if ctl.paused.is_empty() {
            return;
        }
        let mut out = Vec::new();
        for p in std::mem::take(&mut ctl.paused) {
            out.extend_from_slice(format!("%continue {p}\n").as_bytes());
        }
        c.send_control(&out, true);
    }

    /// Notifications for what happened since the last call.
    pub(crate) fn control_notify(&mut self, _now: Instant) {
        if self.rt.control_hold {
            return;
        }
        let mut events = self.model.take_control_events();
        // tmux tells a new window after the session's current one changed.
        let (linked, mut rest): (Vec<_>, Vec<_>) = events.drain(..).partition(|e| e.name == "window-linked");
        rest.extend(linked);
        let events = rest;
        let ids: Vec<(ClientId, SessionId)> =
            self.rt.clients.values().filter(|c| c.control.is_some()).filter_map(|c| Some((c.id, c.att.as_ref()?.session))).collect();
        if ids.is_empty() {
            return;
        }
        let wall = crate::sys::wall_clock();
        for ev in events {
            let window_name = |m: &lif_core::Model, w| m.window(w).map(|w| w.name().to_string());
            for &(id, own) in &ids {
                let mine = ev.session == Some(own) || ev.window.and_then(|w| self.model.window(w)).is_some_and(|w| w.index_in(own).is_some());
                let line = match ev.name {
                    "window-linked" => ev.window.map(|w| if mine { format!("%window-add {w}") } else { format!("%unlinked-window-add {w}") }),
                    "window-unlinked" => ev.window.map(|w| if ev.session.is_none_or(|s| s == own) { format!("%window-close {w}") } else { format!("%unlinked-window-close {w}") }),
                    "window-renamed" => ev.window.and_then(|w| Some(format!("%window-renamed {w} {}", window_name(&self.model, w)?))),
                    "session-renamed" => ev.session.and_then(|s| Some(format!("%session-renamed {s} {}", self.model.session(s)?.name()))),
                    "session-created" | "session-closed" => Some("%sessions-changed".to_string()),
                    "client-session-changed" | "client-attached" if ev.client == Some(id) && ev.name == "client-session-changed" => {
                        ev.session.and_then(|s| Some(format!("%session-changed {s} {}", self.model.session(s)?.name())))
                    }
                    "session-window-changed" => ev.session.zip(ev.window).map(|(s, w)| format!("%session-window-changed {s} {w}")),
                    "window-pane-changed" if mine => ev.window.zip(ev.pane).map(|(w, p)| format!("%window-pane-changed {w} {p}")),
                    _ => None,
                };
                if let Some(mut l) = line
                    && let Some(c) = self.rt.clients.get_mut(&id)
                {
                    l.push('\n');
                    c.send_control(l.as_bytes(), true);
                }
            }
        }
        // Layouts: any window of the client's session whose layout is not
        // the one told last (splits, kills, resizes, select-layout).
        for &(id, own) in &ids {
            let wins: Vec<lif_core::WindowId> = self.model.session(own).map(|s| s.windows().map(|(_, w)| w).collect()).unwrap_or_default();
            let mut lines = Vec::new();
            for w in wins {
                let layout = self.model.expand_format("#{window_layout} #{window_visible_layout}", Some(own), Some(w), None, Some(id), wall, &self.rt);
                let Some(ctl) = self.rt.clients.get_mut(&id).and_then(|c| c.control.as_mut()) else { break };
                match ctl.layouts.get(&w) {
                    Some(old) if *old == layout => {}
                    old => {
                        // A window just made is told by %window-add.
                        if old.is_some() {
                            let flags = self.model.expand_format("#{window_raw_flags}", Some(own), Some(w), None, Some(id), wall, &self.rt);
                            lines.push(format!("%layout-change {w} {layout} {flags}\n"));
                        }
                        if let Some(ctl) = self.rt.clients.get_mut(&id).and_then(|c| c.control.as_mut()) {
                            ctl.layouts.insert(w, layout);
                        }
                    }
                }
            }
            if let Some(c) = self.rt.clients.get_mut(&id) {
                for l in lines {
                    c.send_control(l.as_bytes(), true);
                }
            }
        }
        for (id, _) in ids {
            self.rt.flush_client(id);
        }
    }
}
