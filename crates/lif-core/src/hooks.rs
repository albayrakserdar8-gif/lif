//! Hooks: commands run when something happens (`set-hook`, `show-hooks`).
//!
//! Names and scopes are tmux 3.8's: session hooks (the `after-*` hooks of
//! commands, client, session and alert events) are set globally or on a
//! session, window hooks (pane and window events) globally, on a window or
//! on a pane; the most specific setting that exists is the one that runs.
//! tmux keeps hooks as array options, which is why unknown names are an
//! `invalid option` and entries print as `name[index] command`.
//!
//! Something that happens while commands run is noted ([`Model::notify`])
//! and its hook runs when the commands are done; its output joins theirs.
//! The server runs the hooks of events outside commands
//! ([`Model::run_hooks`]). Commands run by a hook never fire hooks.

use crate::cmd::{Args, CmdError, CommandList, commands_from_args};
use crate::host::Host;
use crate::ids::{ClientId, PaneId, SessionId, WindowId};
use crate::model::{CmdContext, CmdOutput, Model};
use crate::queue::Finished;
use crate::target::{Flags, Kind};
use crate::time::WallClock;
use std::collections::BTreeMap;

/// tmux 3.8's session hooks, in the order `show-hooks -g` lists them.
pub(crate) const SESSION_HOOKS: &[&str] = &[
    "after-bind-key",
    "after-capture-pane",
    "after-copy-mode",
    "after-display-message",
    "after-display-panes",
    "after-kill-pane",
    "after-list-buffers",
    "after-list-clients",
    "after-list-keys",
    "after-list-panes",
    "after-list-sessions",
    "after-list-windows",
    "after-load-buffer",
    "after-lock-server",
    "after-new-session",
    "after-new-window",
    "after-paste-buffer",
    "after-pipe-pane",
    "after-refresh-client",
    "after-rename-session",
    "after-rename-window",
    "after-resize-pane",
    "after-resize-window",
    "after-save-buffer",
    "after-select-layout",
    "after-select-pane",
    "after-select-window",
    "after-send-keys",
    "after-set-buffer",
    "after-set-environment",
    "after-set-hook",
    "after-set-option",
    "after-show-environment",
    "after-show-messages",
    "after-show-options",
    "after-split-window",
    "after-swap-window",
    "after-unbind-key",
    "alert-activity",
    "alert-bell",
    "alert-silence",
    "client-active",
    "client-attached",
    "client-created",
    "client-closed",
    "client-detached",
    "client-focus-in",
    "client-focus-out",
    "client-resized",
    "client-session-changed",
    "client-light-theme",
    "client-dark-theme",
    "command-error",
    "marked-pane-changed",
    "session-closed",
    "session-created",
    "session-added-to-group",
    "session-renamed",
    "session-removed-from-group",
    "session-window-changed",
    "window-linked",
    "window-unlinked",
];

/// tmux 3.8's window hooks, in the order `show-hooks -gw` lists them.
pub(crate) const WINDOW_HOOKS: &[&str] = &[
    "pane-activity",
    "pane-bell",
    "pane-command-finished",
    "pane-command-started",
    "pane-created",
    "pane-died",
    "pane-exited",
    "pane-focus-in",
    "pane-focus-out",
    "pane-mode-changed",
    "pane-mode-entered",
    "pane-mode-exited",
    "pane-moved",
    "pane-prompt-closed",
    "pane-prompt-opened",
    "pane-resized",
    "pane-set-clipboard",
    "pane-shell-prompt",
    "pane-title-changed",
    "window-created",
    "window-closed",
    "window-layout-changed",
    "window-pane-changed",
    "window-renamed",
    "window-resized",
    "window-unzoomed",
    "window-zoomed",
];

/// Every hook name, session hooks first.
pub fn hook_names() -> impl Iterator<Item = &'static str> {
    SESSION_HOOKS.iter().chain(WINDOW_HOOKS).copied()
}

/// Most commands one hook holds in one place.
const MAX_HOOK_ENTRIES: usize = 64;

/// Most hooks waiting to run; events past it run no hook. Commands run
/// hooks when they finish, so this is only reached by an event storm.
const MAX_PENDING_HOOKS: usize = 256;

/// The hooks set in one place (globally, or on a session, window or pane).
#[derive(Clone, Debug, Default)]
pub(crate) struct HookSet(BTreeMap<&'static str, BTreeMap<u32, CommandList>>);

impl HookSet {
    fn get(&self, name: &str) -> Option<&BTreeMap<u32, CommandList>> {
        self.0.get(name).filter(|m| !m.is_empty())
    }

    /// `name[i]` and the command of every hook set here, in name order.
    fn lines(&self) -> Vec<(String, String)> {
        self.0.iter().flat_map(|(name, m)| m.iter().map(move |(i, list)| (format!("{name}[{i}]"), list.print(false)))).collect()
    }

    fn is_empty(&self) -> bool {
        self.0.values().all(|m| m.is_empty())
    }
}

/// The `hook*` format variables while a hook runs (tmux's names). Taken
/// when the event happens: what it is about may be gone when the hook runs.
#[derive(Clone, Debug, Default)]
pub(crate) struct HookVars {
    pub hook: &'static str,
    pub client: String,
    pub session: String,
    pub session_name: String,
    pub window: String,
    pub window_name: String,
    pub pane: String,
}

impl HookVars {
    pub(crate) fn var(&self, name: &str) -> Option<String> {
        Some(
            match name {
                "hook" => self.hook,
                "hook_client" => &self.client,
                "hook_session" => &self.session,
                "hook_session_name" => &self.session_name,
                "hook_window" => &self.window,
                "hook_window_name" => &self.window_name,
                "hook_pane" => &self.pane,
                _ => return None,
            }
            .to_string(),
        )
    }
}

#[derive(Debug)]
struct Pending {
    vars: HookVars,
    client: Option<ClientId>,
    pane: Option<PaneId>,
    commands: Vec<CommandList>,
}

/// Hook state of the server.
#[derive(Debug, Default)]
pub(crate) struct Hooks {
    session: HookSet,
    window: HookSet,
    /// A hook is set somewhere: events are worth looking up.
    any: bool,
    pending: Vec<Pending>,
    /// Commands of a hook are running: nothing they do fires a hook.
    pub(crate) muted: bool,
    pub(crate) current: Option<HookVars>,
}

/// Most events kept for control-mode clients between two drains (the
/// server drains after every batch of commands and event).
pub(crate) const MAX_CONTROL_EVENTS: usize = 4096;

/// Something that happened, for control-mode clients' notifications: the
/// name of the hook it would run (`window-linked`, `session-renamed`...)
/// and what it happened to.
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct ControlEvent {
    /// The hook name of what happened.
    pub name: &'static str,
    /// The client it is about (`client-session-changed`), if any.
    pub client: Option<ClientId>,
    /// The session it happened in, if known.
    pub session: Option<SessionId>,
    /// The window it happened to, if any.
    pub window: Option<WindowId>,
    /// The pane it happened to, if any.
    pub pane: Option<PaneId>,
}

impl Model {
    /// Keep events for control-mode clients (only while there is one).
    pub fn set_control_events(&mut self, on: bool) {
        match (on, self.control_events.is_some()) {
            (true, false) => self.control_events = Some(Vec::new()),
            (false, true) => self.control_events = None,
            _ => {}
        }
    }

    /// The events since the last call.
    pub fn take_control_events(&mut self) -> Vec<ControlEvent> {
        self.control_events.as_mut().map(std::mem::take).unwrap_or_default()
    }
}

impl Hooks {
    /// Every global hook that is set (the window ones with `window`):
    /// `name[i]` and its command, for `customize-mode`.
    pub(crate) fn global_lines(&self, window: bool) -> Vec<(String, String)> {
        if window { self.window.lines() } else { self.session.lines() }
    }
}

/// `name` or `name[index]`.
fn split_index(raw: &str) -> Result<(&str, Option<u32>), CmdError> {
    let Some((name, rest)) = raw.split_once('[') else { return Ok((raw, None)) };
    match rest.strip_suffix(']').and_then(|i| i.parse::<u32>().ok()) {
        Some(i) => Ok((name, Some(i))),
        None => Err(CmdError(format!("invalid option: {raw}"))),
    }
}

/// The hook's name as the tables have it, and whether it is a window hook.
fn lookup(name: &str) -> Result<(&'static str, bool), CmdError> {
    if let Some(n) = SESSION_HOOKS.iter().find(|n| **n == name) {
        return Ok((n, false));
    }
    if let Some(n) = WINDOW_HOOKS.iter().find(|n| **n == name) {
        return Ok((n, true));
    }
    Err(CmdError(format!("invalid option: {name}")))
}

type R = Result<CmdOutput, CmdError>;

impl Model {
    /// `name` happened to this client / session / window / pane: queue its
    /// hook, if one is set, with the commands as they are now.
    pub(crate) fn notify(&mut self, name: &'static str, client: Option<ClientId>, s: Option<SessionId>, w: Option<WindowId>, p: Option<PaneId>) {
        // Control-mode clients hear about it too (hooks or not).
        if self.control_events.is_some() {
            let w2 = w.or_else(|| p.and_then(|p| self.panes.get(&p)).map(|p| p.window));
            let s2 = s.or_else(|| w2.and_then(|w| self.windows.get(&w)).map(|w| w.session()));
            if let Some(q) = self.control_events.as_mut()
                && q.len() < MAX_CONTROL_EVENTS
            {
                q.push(ControlEvent { name, client, session: s2, window: w2, pane: p });
            }
        }
        if !self.hooks.any || self.hooks.muted || self.hooks.pending.len() >= MAX_PENDING_HOOKS {
            return;
        }
        let w = w.or_else(|| p.and_then(|p| self.panes.get(&p)).map(|p| p.window));
        let s = s.or_else(|| w.and_then(|w| self.windows.get(&w)).map(|w| w.session()));
        let set = if WINDOW_HOOKS.contains(&name) {
            p.and_then(|p| self.panes.get(&p))
                .and_then(|p| p.hooks.get(name))
                .or_else(|| w.and_then(|w| self.windows.get(&w)).and_then(|w| w.hooks.get(name)))
                .or_else(|| self.hooks.window.get(name))
        } else {
            s.and_then(|s| self.sessions.get(&s)).and_then(|s| s.hooks.get(name)).or_else(|| self.hooks.session.get(name))
        };
        let Some(set) = set else { return };
        let commands: Vec<CommandList> = set.values().cloned().collect();
        let session = s.and_then(|s| self.sessions.get(&s));
        let window = w.and_then(|w| self.windows.get(&w));
        let vars = HookVars {
            hook: name,
            client: client.and_then(|c| self.clients.get(&c)).map(|c| c.info.name.clone()).unwrap_or_default(),
            session: session.map(|s| s.id.to_string()).unwrap_or_default(),
            session_name: session.map(|s| s.name.clone()).unwrap_or_default(),
            window: window.map(|w| w.id.to_string()).unwrap_or_default(),
            window_name: window.map(|w| w.name.clone()).unwrap_or_default(),
            pane: p.map(|p| p.to_string()).unwrap_or_default(),
        };
        // The commands' current pane: the one it happened to, else the
        // active one of the window or session it happened to.
        let pane = p
            .or_else(|| window.map(|w| w.active))
            .or_else(|| session.and_then(|s| self.windows.get(&s.current)).map(|w| w.active));
        self.hooks.pending.push(Pending { vars, client, pane, commands });
    }

    /// Whether [`Model::run_hooks`] has anything to run.
    pub fn has_hooks(&self) -> bool {
        !self.hooks.pending.is_empty()
    }

    /// Run the hooks of what happened since the last call (events the
    /// server reported; commands run their own hooks). What they print
    /// comes out of [`Model::take_finished`], for the client the event
    /// came from, if any.
    pub fn run_hooks(&mut self, now: WallClock, host: &mut dyn Host) {
        self.run_pending_hooks(now, None, host);
    }

    /// Runs queued hooks; their output is appended to `out`, or reported
    /// as finished lists when there is none.
    pub(crate) fn run_pending_hooks(&mut self, now: WallClock, mut out: Option<&mut String>, host: &mut dyn Host) {
        if self.hooks.pending.is_empty() || self.hooks.muted {
            return;
        }
        let pending = std::mem::take(&mut self.hooks.pending);
        for h in pending {
            let ctx = CmdContext { client: h.client, pane: h.pane.filter(|p| self.panes.contains_key(p)), now };
            self.hooks.current = Some(h.vars);
            for list in &h.commands {
                let (stdout, error) = self.run_hook_list(list, &ctx, host);
                if let Some(e) = &error {
                    self.log_message(e.to_string());
                }
                match out.as_deref_mut() {
                    Some(o) => o.push_str(&stdout),
                    None if !stdout.is_empty() || error.is_some() => {
                        self.push_finished(Finished { client: h.client, waited: false, stdout, status: 0, error });
                    }
                    None => {}
                }
            }
            self.hooks.current = None;
        }
    }

    pub(crate) fn any_hooks(&self) -> bool {
        self.hooks.any
    }

    fn refresh_any_hooks(&mut self) {
        self.hooks.any = !self.hooks.session.is_empty()
            || !self.hooks.window.is_empty()
            || self.sessions.values().any(|s| !s.hooks.is_empty())
            || self.windows.values().any(|w| !w.hooks.is_empty())
            || self.panes.values().any(|p| !p.hooks.is_empty());
    }

    /// The set `set-hook` / `show-hooks` work on, by their flags.
    fn hook_set(&mut self, a: &Args, ctx: &CmdContext, window_hook: bool) -> Result<&mut HookSet, CmdError> {
        if a.has('g') {
            return Ok(if window_hook { &mut self.hooks.window } else { &mut self.hooks.session });
        }
        let f = self.find_target(ctx, a.get('t'), Kind::Pane, Flags::default())?;
        if !window_hook {
            return Ok(&mut self.sessions.get_mut(&f.s).expect("session").hooks);
        }
        if a.has('p') {
            let p = f.p.ok_or_else(|| CmdError::new("no current pane"))?;
            return Ok(&mut self.panes.get_mut(&p).expect("pane").hooks);
        }
        let w = f.w.ok_or_else(|| CmdError::new("no current window"))?;
        Ok(&mut self.windows.get_mut(&w).expect("window").hooks)
    }

    pub(crate) fn cmd_set_hook(&mut self, a: &Args, ctx: &CmdContext) -> R {
        let (name, index) = split_index(a.value(0).unwrap_or(""))?;
        let (name, window_hook) = lookup(name)?;
        if a.has('R') {
            // Run it now, for the target.
            let f = self.find_target(ctx, a.get('t'), Kind::Pane, Flags::default())?;
            self.notify(name, ctx.client, Some(f.s), f.w, f.p);
            return Ok(CmdOutput::default());
        }
        if a.has('u') {
            let set = self.hook_set(a, ctx, window_hook)?;
            match index {
                Some(i) => {
                    if let Some(m) = set.0.get_mut(name) {
                        m.remove(&i);
                    }
                }
                None => {
                    set.0.remove(name);
                }
            }
            self.refresh_any_hooks();
            return Ok(CmdOutput::default());
        }
        if a.values().len() < 2 {
            return Err(CmdError::new("empty value"));
        }
        let list = commands_from_args(&a.values()[1..])?;
        let append = a.has('a');
        let set = self.hook_set(a, ctx, window_hook)?;
        let entries = set.0.entry(name).or_default();
        match index {
            Some(i) => {
                if !entries.contains_key(&i) && entries.len() >= MAX_HOOK_ENTRIES {
                    return Err(CmdError(format!("too many commands for {name} ({MAX_HOOK_ENTRIES})")));
                }
                entries.insert(i, list);
            }
            None if append => {
                if entries.len() >= MAX_HOOK_ENTRIES {
                    return Err(CmdError(format!("too many commands for {name} ({MAX_HOOK_ENTRIES})")));
                }
                let next = entries.keys().next_back().map_or(0, |i| i.saturating_add(1));
                entries.insert(next, list);
            }
            None => {
                entries.clear();
                entries.insert(0, list);
            }
        }
        self.hooks.any = true;
        Ok(CmdOutput::default())
    }

    pub(crate) fn cmd_show_hooks(&mut self, a: &Args, ctx: &CmdContext) -> R {
        let window_scope = a.has('w') || a.has('p');
        let only = match a.value(0) {
            Some(raw) => {
                let (name, window_hook) = lookup(split_index(raw)?.0)?;
                Some((name, window_hook))
            }
            None => None,
        };
        let window_hook = only.map_or(window_scope, |o| o.1);
        let global = a.has('g');
        let set = self.hook_set(a, ctx, window_hook)?.clone();
        let names = if window_hook { WINDOW_HOOKS } else { SESSION_HOOKS };
        let mut out = String::new();
        for name in names.iter().filter(|n| only.is_none_or(|o| o.0 == **n)) {
            match set.get(name) {
                Some(entries) => {
                    for (i, list) in entries {
                        out.push_str(&format!("{name}[{i}] {}\n", list.print(false)));
                    }
                }
                // Only the global tables list hooks that are not set.
                None if global => out.push_str(&format!("{name}\n")),
                None => {}
            }
        }
        Ok(CmdOutput { stdout: out, ..CmdOutput::default() })
    }
}
