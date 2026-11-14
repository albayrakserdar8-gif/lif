//! Target resolution (`-t` / `-s`), following tmux's `cmd-find.c`.
//!
//! Forms: `session`, `session:window`, `session:window.pane`, `:window`,
//! `.pane`, `window.pane`; `$id`, `@id`, `%id`; sessions by exact name, then
//! unique prefix, then unique glob; windows by index, `@id`, exact name,
//! unique prefix, glob, `{start}`/`^`, `{end}`/`$`, `{last}`/`!`,
//! `{next}`/`+`, `{previous}`/`-`, `+N`/`-N`; panes by index, `%id`,
//! `{last}`/`!`, `{next}`/`+`, `{previous}`/`-`, `+N`/`-N`, `{top}`,
//! `{bottom}`, `{left}`, `{right}`, `{top-left}` ..., `{up-of}`,
//! `{down-of}`, `{left-of}`, `{right-of}`. A leading `=` on a session or
//! window name asks for an exact match.

use crate::cmd::CmdError;
use crate::format::glob;
use crate::ids::{PaneId, SessionId, WindowId};
use crate::layout::Direction;
use crate::model::{CmdContext, Model};

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub(crate) enum Kind {
    Session,
    Window,
    Pane,
}

#[derive(Clone, Copy, Debug, Default)]
pub(crate) struct Flags {
    /// The window part may name an index with no window (new-window -t).
    pub window_index: bool,
    /// With no target, prefer a session nobody is attached to.
    pub prefer_unattached: bool,
}

/// What a target resolved to.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub(crate) struct Found {
    pub s: SessionId,
    /// The window (the session's current one if the target named none),
    /// `None` only for a free index with `window_index`.
    pub w: Option<WindowId>,
    pub p: Option<PaneId>,
    /// The window index the target named explicitly, if any.
    pub idx: Option<u32>,
}

impl Model {
    pub(crate) fn current_state(&self, ctx: &CmdContext, prefer_unattached: bool) -> Result<Found, CmdError> {
        if let Some(p) = ctx.pane
            && let Some(f) = self.found_pane(p)
        {
            return Ok(f);
        }
        if let Some(c) = ctx.client.and_then(|c| self.clients.get(&c))
            && let Some(s) = c.session
        {
            return Ok(self.found_session(s));
        }
        self.best_session(prefer_unattached).map(|s| self.found_session(s)).ok_or_else(|| CmdError::new("no current session"))
    }

    pub(crate) fn best_session(&self, prefer_unattached: bool) -> Option<SessionId> {
        self.sessions
            .values()
            .max_by_key(|s| {
                let unattached = prefer_unattached && self.attached_count(s.id) == 0;
                (unattached, s.activity)
            })
            .map(|s| s.id)
    }

    pub(crate) fn found_session(&self, s: SessionId) -> Found {
        let w = self.sessions[&s].current;
        Found { s, w: Some(w), p: self.windows.get(&w).map(|w| w.active), idx: None }
    }

    fn found_window(&self, w: WindowId) -> Option<Found> {
        let win = self.windows.get(&w)?;
        Some(Found { s: win.session, w: Some(w), p: Some(win.active), idx: Some(win.index) })
    }

    fn found_pane(&self, p: PaneId) -> Option<Found> {
        let w = self.panes.get(&p)?.window;
        let win = self.windows.get(&w)?;
        Some(Found { s: win.session, w: Some(w), p: Some(p), idx: Some(win.index) })
    }

    /// Resolve `target` (`None` or empty: the current state).
    pub(crate) fn find_target(&self, ctx: &CmdContext, target: Option<&str>, kind: Kind, flags: Flags) -> Result<Found, CmdError> {
        let Some(target) = target.filter(|t| !t.is_empty()) else {
            return self.current_state(ctx, flags.prefer_unattached);
        };
        let (mut session, mut window, mut pane) = (None, None, None);
        let (mut window_only, mut pane_only) = (false, false);
        if let Some((s, rest)) = target.split_once(':') {
            session = Some(s);
            window_only = true;
            match rest.split_once('.') {
                Some((w, p)) => {
                    window = Some(w);
                    pane = Some(p);
                    pane_only = true;
                }
                None => window = Some(rest),
            }
        } else if let Some((w, p)) = target.split_once('.') {
            window = Some(w);
            pane = Some(p);
            pane_only = true;
        } else if target.starts_with('$') {
            session = Some(target);
        } else if target.starts_with('@') {
            window = Some(target);
        } else if target.starts_with('%') {
            pane = Some(target);
        } else {
            match kind {
                Kind::Session => session = Some(target),
                Kind::Window => window = Some(target),
                Kind::Pane => pane = Some(target),
            }
        }
        let mut exact_session = false;
        let mut exact_window = false;
        if let Some(s) = session.and_then(|s| s.strip_prefix('=')) {
            session = Some(s);
            exact_session = true;
        }
        if let Some(w) = window.and_then(|w| w.strip_prefix('=')) {
            window = Some(w);
            exact_window = true;
        }
        let session = session.filter(|s| !s.is_empty());
        let window = window.filter(|w| !w.is_empty()).map(map_window);
        let pane = pane.filter(|p| !p.is_empty()).map(map_pane);

        let no_session = |s: &str| CmdError(format!("can't find session: {s}"));
        let no_window = |w: &str| CmdError(format!("can't find window: {w}"));
        let no_pane = |p: &str| CmdError(format!("can't find pane: {p}"));

        if let Some(sname) = session {
            let sid = self.get_session(sname, exact_session).ok_or_else(|| no_session(sname))?;
            let mut f = self.found_session(sid);
            if let Some(w) = window {
                f = self.get_window_with_session(sid, w, exact_window, flags.window_index).ok_or_else(|| no_window(w))?;
            }
            if let Some(p) = pane {
                let wid = f.w.ok_or_else(|| no_window(window.unwrap_or("")))?;
                f.p = Some(self.get_pane_with_window(wid, p).ok_or_else(|| no_pane(p))?);
            }
            return Ok(f);
        }
        if let Some(w) = window {
            let mut f = self.get_window(ctx, w, window_only, exact_window, flags)?.ok_or_else(|| no_window(w))?;
            if let Some(p) = pane {
                let wid = f.w.ok_or_else(|| no_window(w))?;
                f.p = Some(self.get_pane_with_window(wid, p).ok_or_else(|| no_pane(p))?);
            }
            return Ok(f);
        }
        if let Some(p) = pane {
            if let Some(id) = PaneId::parse(p) {
                return self.found_pane(id).ok_or_else(|| no_pane(p));
            }
            let mut cur = self.current_state(ctx, false)?;
            if let Some(wid) = cur.w
                && let Some(found) = self.get_pane_with_window(wid, p)
            {
                cur.p = Some(found);
                return Ok(cur);
            }
            if !pane_only && let Some(f) = self.get_window(ctx, p, false, false, Flags::default())? {
                return Ok(f);
            }
            return Err(no_pane(p));
        }
        self.current_state(ctx, flags.prefer_unattached)
    }

    pub(crate) fn get_session(&self, name: &str, exact: bool) -> Option<SessionId> {
        if let Some(id) = SessionId::parse(name) {
            return self.sessions.contains_key(&id).then_some(id);
        }
        if let Some(id) = self.session_by_name(name) {
            return Some(id);
        }
        if exact {
            return None;
        }
        let unique = |pred: &dyn Fn(&str) -> bool| {
            let mut it = self.sessions.values().filter(|s| pred(&s.name));
            match (it.next(), it.next()) {
                (Some(s), None) => Some(s.id),
                _ => None,
            }
        };
        unique(&|n| n.starts_with(name)).or_else(|| unique(&|n| glob(name.as_bytes(), n.as_bytes())))
    }

    fn get_window(&self, ctx: &CmdContext, w: &str, only: bool, exact: bool, flags: Flags) -> Result<Option<Found>, CmdError> {
        if let Some(id) = WindowId::parse(w) {
            return Ok(self.found_window(id));
        }
        let cur = self.current_state(ctx, false)?;
        if let Some(f) = self.get_window_with_session(cur.s, w, exact, flags.window_index) {
            return Ok(Some(f));
        }
        if !only && let Some(sid) = self.get_session(w, false) {
            return Ok(Some(self.found_session(sid)));
        }
        Ok(None)
    }

    fn get_window_with_session(&self, sid: SessionId, w: &str, exact: bool, window_index: bool) -> Option<Found> {
        let sess = self.sessions.get(&sid)?;
        if let Some(id) = WindowId::parse(w) {
            return self.found_window(id).filter(|f| f.s == sid);
        }
        let cur_idx = self.windows.get(&sess.current)?.index;
        let by_index = |idx: u32| sess.windows.get(&idx).and_then(|wid| self.found_window(*wid));
        if !exact && (w.starts_with('+') || w.starts_with('-')) {
            let n: u32 = if w.len() > 1 { w[1..].parse().ok().filter(|n| *n >= 1)? } else { 1 };
            let plus = w.starts_with('+');
            if window_index {
                let idx = if plus { cur_idx.checked_add(n)? } else { cur_idx.checked_sub(n)? };
                return Some(by_index(idx).unwrap_or(Found { s: sid, w: None, p: None, idx: Some(idx) }));
            }
            let keys: Vec<u32> = sess.windows.keys().copied().collect();
            let pos = keys.iter().position(|k| *k == cur_idx)? as i64;
            let len = keys.len() as i64;
            let to = if plus { (pos + n as i64).rem_euclid(len) } else { (pos - n as i64).rem_euclid(len) };
            return by_index(keys[to as usize]);
        }
        if !exact {
            match w {
                "!" => return sess.last.first().and_then(|w| self.found_window(*w)),
                "^" => return sess.windows.values().next().and_then(|w| self.found_window(*w)),
                "$" => return sess.windows.values().next_back().and_then(|w| self.found_window(*w)),
                _ => {}
            }
        }
        if !w.is_empty()
            && w.bytes().all(|b| b.is_ascii_digit())
            && let Ok(idx) = w.parse::<u32>()
        {
            if let Some(f) = by_index(idx) {
                return Some(f);
            }
            if window_index {
                return Some(Found { s: sid, w: None, p: None, idx: Some(idx) });
            }
        }
        let unique = |pred: &dyn Fn(&str) -> bool| -> Option<Option<Found>> {
            let mut it = sess.windows.values().filter(|wid| self.windows.get(wid).is_some_and(|x| pred(&x.name)));
            match (it.next(), it.next()) {
                (Some(wid), None) => Some(self.found_window(*wid)),
                (Some(_), Some(_)) => Some(None),
                _ => None,
            }
        };
        if let Some(r) = unique(&|n| n == w) {
            return r;
        }
        if exact {
            return None;
        }
        if let Some(r) = unique(&|n| n.starts_with(w)) {
            return r;
        }
        unique(&|n| glob(w.as_bytes(), n.as_bytes())).flatten()
    }

    fn get_pane_with_window(&self, wid: WindowId, p: &str) -> Option<PaneId> {
        let win = self.windows.get(&wid)?;
        if let Some(id) = PaneId::parse(p) {
            return (self.panes.get(&id)?.window == wid).then_some(id);
        }
        let rec = |id: PaneId| self.panes.get(&id).map(|p| p.active_point).unwrap_or(0);
        let dir = match p {
            "!" => return win.last_panes.first().copied(),
            "{up-of}" => Some(Direction::Up),
            "{down-of}" => Some(Direction::Down),
            "{left-of}" => Some(Direction::Left),
            "{right-of}" => Some(Direction::Right),
            _ => None,
        };
        if let Some(d) = dir {
            return win.layout.neighbor(win.active, d, &win.panes, rec);
        }
        if p.starts_with('+') || p.starts_with('-') {
            let n: i64 = if p.len() > 1 { p[1..].parse().ok().filter(|n| *n >= 1)? } else { 1 };
            let pos = win.panes.iter().position(|x| *x == win.active)? as i64;
            let len = win.panes.len() as i64;
            let to = if p.starts_with('+') { (pos + n).rem_euclid(len) } else { (pos - n).rem_euclid(len) };
            return Some(win.panes[to as usize]);
        }
        if !p.is_empty() && p.bytes().all(|b| b.is_ascii_digit()) {
            let base = self.wopt(Some(wid), "pane-base-index").as_num() as usize;
            if let Ok(i) = p.parse::<usize>()
                && i >= base
                && let Some(id) = win.panes.get(i - base)
            {
                return Some(*id);
            }
        }
        win.layout.pane_at_position(p, &win.panes)
    }
}

fn map_window(w: &str) -> &str {
    match w {
        "{start}" => "^",
        "{last}" => "!",
        "{end}" => "$",
        "{next}" => "+",
        "{previous}" => "-",
        other => other,
    }
}

fn map_pane(p: &str) -> &str {
    match p {
        "{last}" => "!",
        "{next}" => "+",
        "{previous}" => "-",
        "{top}" => "top",
        "{bottom}" => "bottom",
        "{left}" => "left",
        "{right}" => "right",
        "{top-left}" => "top-left",
        "{top-right}" => "top-right",
        "{bottom-left}" => "bottom-left",
        "{bottom-right}" => "bottom-right",
        other => other,
    }
}
