//! Session groups (`new-session -t`): sessions that share one list of
//! windows, each with its own current window, options and clients. A
//! change to the window list of one member (a window made, moved, swapped,
//! killed or renumbered) is copied to the others, as tmux's
//! `session_group_synchronize_from` does. The group is named after the
//! session it was made from and keeps that name; it lasts as long as one of
//! its sessions does.

use crate::host::{Host, Redraw};
use crate::ids::SessionId;
use crate::model::Model;

impl Model {
    /// `sid`'s window list changed: sync its group when the command is done.
    pub(crate) fn sync_group_quiet(&mut self, sid: SessionId) {
        if self.sessions.get(&sid).is_some_and(|s| s.group.is_some()) && !self.group_dirty.contains(&sid) {
            self.group_dirty.push(sid);
        }
    }

    /// Sync the groups [`Model::sync_group_quiet`] marked.
    pub(crate) fn flush_group_sync(&mut self, host: &mut dyn Host) {
        while let Some(s) = self.group_dirty.pop() {
            self.sync_group(s, host);
        }
    }

    /// The sessions of `sid`'s group (`sid` too), oldest first; just
    /// `sid` when it is in none.
    pub(crate) fn group_members(&self, sid: SessionId) -> Vec<SessionId> {
        match self.sessions.get(&sid).and_then(|s| s.group.as_ref()) {
            Some(g) => self.sessions.values().filter(|s| s.group.as_ref() == Some(g)).map(|s| s.id).collect(),
            None => vec![sid],
        }
    }

    /// Give every other session of `sid`'s group `sid`'s windows at the
    /// same indexes. A member whose current window went keeps its place
    /// (the window now at the same index), else takes the first one.
    pub(crate) fn sync_group(&mut self, sid: SessionId, host: &mut dyn Host) {
        let members = self.group_members(sid);
        if members.len() < 2 {
            return;
        }
        let Some(want) = self.sessions.get(&sid).map(|s| s.windows.clone()) else { return };
        for m in members.into_iter().filter(|m| *m != sid) {
            let Some(s) = self.sessions.get_mut(&m) else { continue };
            if s.windows == want {
                continue;
            }
            let cur_idx = s.windows.iter().find(|(_, w)| **w == s.current).map(|(i, _)| *i);
            let old = std::mem::replace(&mut s.windows, want.clone());
            for (idx, wid) in &old {
                if want.get(idx) != Some(wid)
                    && let Some(w) = self.windows.get_mut(wid)
                {
                    w.unlink(m, *idx);
                }
            }
            for (idx, wid) in &want {
                if old.get(idx) != Some(wid)
                    && let Some(w) = self.windows.get_mut(wid)
                {
                    w.link(m, *idx);
                }
            }
            let s = self.sessions.get_mut(&m).expect("member");
            s.last.retain(|w| want.values().any(|x| x == w));
            if !want.values().any(|w| *w == s.current)
                && let Some(first) = cur_idx.and_then(|i| want.get(&i)).or_else(|| want.values().next())
            {
                s.current = *first;
                s.last.retain(|w| w != first);
            }
            host.redraw(Redraw::Session(m));
        }
    }
}
