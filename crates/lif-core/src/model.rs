//! The session / window / pane model and everything that changes it
//! outside of commands (events reported by the server).

use crate::bindings::{KeyBindings, KeyDecision, KeyState};
use crate::cmd::CmdError;
use crate::format::{self, Vars};
use crate::host::{DetachReason, Host, Redraw, SpawnCommand, SpawnSpec};
use crate::ids::{ClientId, PaneId, SessionId, WindowId};
use crate::keys::KeyCode;
use crate::layout::{Geometry, Layout, Preset};
use crate::options::{self, OptionSet, Scope, Value};
use crate::status::{self, StatusParts, WindowEntry};
use crate::time::WallClock;
use lif_types::{KeyEvent, Rect, Size, StyledLine};
use std::collections::{BTreeMap, HashMap};

/// What the server knows about a client connection (from its `Hello`).
#[derive(Clone, Debug, Default, PartialEq, Eq)]
pub struct ClientInfo {
    /// Name shown by `list-clients` and used by `-t target-client`: the
    /// client's tty path (`/dev/ttys003`).
    pub name: String,
    /// `$TERM` of the client.
    pub term: String,
    /// Terminal size.
    pub size: Size,
    /// Working directory of the client process.
    pub cwd: String,
    /// Process id of the client.
    pub pid: u32,
    /// stdin and stdout are terminals; only such clients can attach.
    pub terminal: bool,
}

/// A session.
#[derive(Clone, Debug)]
pub struct Session {
    pub(crate) id: SessionId,
    pub(crate) name: String,
    pub(crate) windows: BTreeMap<u32, WindowId>,
    pub(crate) current: WindowId,
    /// Previously current windows, most recent first.
    pub(crate) last: Vec<WindowId>,
    pub(crate) created: i64,
    pub(crate) activity: u64,
    pub(crate) cwd: String,
    pub(crate) options: OptionSet,
    /// Window size used while no client is attached.
    pub(crate) size: Size,
    pub(crate) hooks: crate::hooks::HookSet,
    pub(crate) environ: crate::environ::Environ,
}

impl Session {
    /// The id (`$N`).
    pub fn id(&self) -> SessionId {
        self.id
    }
    /// The name.
    pub fn name(&self) -> &str {
        &self.name
    }
    /// Windows by index, in index order.
    pub fn windows(&self) -> impl Iterator<Item = (u32, WindowId)> + '_ {
        self.windows.iter().map(|(i, w)| (*i, *w))
    }
    /// The current window.
    pub fn current_window(&self) -> WindowId {
        self.current
    }
    /// The last (previously current) window, if any.
    pub fn last_window(&self) -> Option<WindowId> {
        self.last.first().copied()
    }
    /// Creation time, seconds since the epoch.
    pub fn created(&self) -> i64 {
        self.created
    }
    /// Default working directory for new windows.
    pub fn cwd(&self) -> &str {
        &self.cwd
    }
}

/// A window.
#[derive(Clone, Debug)]
pub struct Window {
    pub(crate) id: WindowId,
    pub(crate) session: SessionId,
    pub(crate) index: u32,
    pub(crate) name: String,
    pub(crate) layout: Layout,
    /// Panes in window order (pane index order).
    pub(crate) panes: Vec<PaneId>,
    pub(crate) active: PaneId,
    /// Previously active panes, most recent first.
    pub(crate) last_panes: Vec<PaneId>,
    pub(crate) bell: bool,
    pub(crate) activity: bool,
    pub(crate) silence: bool,
    pub(crate) options: OptionSet,
    pub(crate) last_preset: Option<Preset>,
    pub(crate) old_layout: Option<Layout>,
    pub(crate) size: Size,
    pub(crate) hooks: crate::hooks::HookSet,
}

impl Window {
    /// The id (`@N`).
    pub fn id(&self) -> WindowId {
        self.id
    }
    /// The session the window belongs to.
    pub fn session(&self) -> SessionId {
        self.session
    }
    /// Its index in the session.
    pub fn index(&self) -> u32 {
        self.index
    }
    /// The name.
    pub fn name(&self) -> &str {
        &self.name
    }
    /// The layout.
    pub fn layout(&self) -> &Layout {
        &self.layout
    }
    /// Panes in index order.
    pub fn panes(&self) -> &[PaneId] {
        &self.panes
    }
    /// The active pane.
    pub fn active_pane(&self) -> PaneId {
        self.active
    }
    /// The last (previously active) pane, if any.
    pub fn last_pane(&self) -> Option<PaneId> {
        self.last_panes.first().copied()
    }
    /// The zoomed pane, if the window is zoomed.
    pub fn zoomed(&self) -> Option<PaneId> {
        self.layout.zoomed()
    }
    /// A bell rang in the window while it was not current.
    pub fn bell_flag(&self) -> bool {
        self.bell
    }
    /// There was output in the window while it was not current (with
    /// `monitor-activity`).
    pub fn activity_flag(&self) -> bool {
        self.activity
    }
    /// The window was quiet for `monitor-silence` seconds while it was
    /// not current.
    pub fn silence_flag(&self) -> bool {
        self.silence
    }
    /// The window size.
    pub fn size(&self) -> Size {
        self.size
    }
}

/// A pane.
#[derive(Clone, Debug)]
pub struct Pane {
    pub(crate) id: PaneId,
    pub(crate) window: WindowId,
    pub(crate) size: Size,
    pub(crate) command: SpawnCommand,
    pub(crate) cwd: String,
    pub(crate) dead: bool,
    pub(crate) exit_status: Option<i32>,
    pub(crate) active_point: u64,
    /// Window options set for this pane only (`set -p`).
    pub(crate) options: OptionSet,
    /// Title set by `select-pane -T`; a title the program sets later
    /// replaces it, as in tmux.
    pub(crate) title: Option<String>,
    pub(crate) hooks: crate::hooks::HookSet,
    /// `select-pane -d`: what clients type does not reach the program.
    pub(crate) input_off: bool,
}

impl Pane {
    /// The id (`%N`).
    pub fn id(&self) -> PaneId {
        self.id
    }
    /// The window it is in.
    pub fn window(&self) -> WindowId {
        self.window
    }
    /// The size of its terminal.
    pub fn size(&self) -> Size {
        self.size
    }
    /// The command it was started with.
    pub fn command(&self) -> &SpawnCommand {
        &self.command
    }
    /// The directory it was started in.
    pub fn start_path(&self) -> &str {
        &self.cwd
    }
    /// Its process exited and `remain-on-exit` kept it.
    pub fn is_dead(&self) -> bool {
        self.dead
    }
}

/// A connected client.
#[derive(Clone, Debug)]
pub struct Client {
    pub(crate) id: ClientId,
    pub(crate) info: ClientInfo,
    pub(crate) session: Option<SessionId>,
    pub(crate) last_session: Option<SessionId>,
    pub(crate) activity: u64,
    pub(crate) keys: KeyState,
}

impl Client {
    /// The id.
    pub fn id(&self) -> ClientId {
        self.id
    }
    /// What it told us about itself.
    pub fn info(&self) -> &ClientInfo {
        &self.info
    }
    /// The session it is attached to.
    pub fn session(&self) -> Option<SessionId> {
        self.session
    }
    /// Its key table state.
    pub fn key_state(&self) -> &KeyState {
        &self.keys
    }
}

/// A paste buffer.
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct PasteBuffer {
    /// Buffer name.
    pub name: String,
    /// Contents.
    pub data: Vec<u8>,
    /// Named automatically (`bufferNNNN`); only these count against
    /// `buffer-limit`.
    pub automatic: bool,
    /// When it was made (Unix seconds, as of the last command run).
    pub created: i64,
}

/// Who runs a command, for default targets.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub struct CmdContext {
    /// The client that sent the command or pressed the key, if any (none
    /// for commands from a configuration file).
    pub client: Option<ClientId>,
    /// The pane the command came from, when the client is not attached but
    /// runs inside a pane (the server takes it from the client's `LIF_PANE`
    /// environment variable). Ignored if the pane no longer exists.
    pub pane: Option<PaneId>,
    /// Wall-clock time, for `session_created` and time formats.
    pub now: WallClock,
}

impl CmdContext {
    /// A context for client `client` at time `now`.
    pub fn client(client: ClientId, now: WallClock) -> Self {
        Self { client: Some(client), pane: None, now }
    }

    /// A context with no client (configuration, internal use).
    pub fn none(now: WallClock) -> Self {
        Self { client: None, pane: None, now }
    }
}

/// Result of a command.
#[derive(Clone, Debug, Default, PartialEq, Eq)]
pub struct CmdOutput {
    /// Text for the client's stdout (for a key binding run by an attached
    /// client tmux shows it in a view; the server decides).
    pub stdout: String,
    /// The exit status for a client that ran the command from a shell: 0,
    /// or the status of a failed `run-shell` command (tmux exits with it).
    pub status: i32,
    /// The commands are not finished: one of them waits for a shell command
    /// or a `wait-for` channel. `stdout` is empty; the outcome arrives
    /// later through [`Model::take_finished`].
    pub pending: bool,
}

/// What an attached client should show.
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct ClientView {
    /// The session the client is attached to.
    pub session: SessionId,
    /// Its current window.
    pub window: WindowId,
    /// The window's active pane (where the cursor goes).
    pub active_pane: PaneId,
    /// The window is zoomed (`panes` holds only the zoomed pane).
    pub zoomed: bool,
    /// Visible panes with rectangles in client coordinates (offset by the
    /// status line when it is at the top). Rectangles may extend past the
    /// client's screen when the window is larger; clip them.
    pub panes: Vec<(PaneId, Rect)>,
    /// Pane borders in client coordinates.
    pub borders: Vec<Rect>,
    /// Row of the status line, if it is shown.
    pub status_row: Option<u16>,
    /// Size of the window (may differ from the client's area).
    pub window_size: Size,
    /// Where each pane's `pane-border-format` line goes (`pane-border-status`).
    pub border_status: BorderStatus,
}

/// `pane-border-status`: a line with each pane's `pane-border-format` on
/// the border above or below it. The window gives up one row for it.
#[derive(Clone, Copy, Debug, Default, PartialEq, Eq)]
pub enum BorderStatus {
    /// No border lines (the default).
    #[default]
    Off,
    /// On the border above each pane.
    Top,
    /// On the border below each pane.
    Bottom,
}

/// The whole server state that is not I/O.
#[derive(Debug)]
pub struct Model {
    pub(crate) hostname: String,
    pub(crate) sessions: BTreeMap<SessionId, Session>,
    pub(crate) windows: HashMap<WindowId, Window>,
    pub(crate) panes: HashMap<PaneId, Pane>,
    pub(crate) clients: BTreeMap<ClientId, Client>,
    pub(crate) next_session: u32,
    pub(crate) next_window: u32,
    pub(crate) next_pane: u32,
    pub(crate) server_opts: OptionSet,
    pub(crate) session_opts: OptionSet,
    pub(crate) window_opts: OptionSet,
    pub(crate) bindings: KeyBindings,
    /// Most recent first.
    pub(crate) buffers: Vec<PasteBuffer>,
    pub(crate) next_buffer: u32,
    pub(crate) counter: u64,
    /// `#{pid}` and `#{socket_path}`, as the server reported them.
    pub(crate) server_pid: u32,
    pub(crate) socket_path: String,
    /// The message log (`show-messages`), oldest first.
    pub(crate) messages: std::collections::VecDeque<String>,
    /// Commands that wait, and what they wait for (`crate::queue`).
    pub(crate) q: crate::queue::Queue,
    /// `#()` commands and their last output. Behind a `RefCell` because
    /// formats are expanded with `&self` (while rendering).
    pub(crate) fjobs: std::cell::RefCell<crate::queue::FormatJobs>,
    pub(crate) hooks: crate::hooks::Hooks,
    /// The global environment overrides (`set-environment -g`).
    pub(crate) environ: crate::environ::Environ,
    /// What the command being run last resolved a target to.
    pub(crate) last_found: std::cell::Cell<Option<crate::target::Found>>,
    /// The time the last command list ran at (Unix seconds): the model
    /// reads no clock, and paste buffers want a creation time.
    pub(crate) clock: i64,
    /// Where the mouse is, while a mouse key's commands run.
    pub(crate) mouse: Option<crate::host::MouseAt>,
    /// A client left a session, a session was made or an option was set
    /// since [`Model::check_unattached`] last looked.
    pub(crate) unattached_dirty: bool,
    /// The server is starting: `source-file` keeps the files' `[startup]`
    /// commands in `startup_commands` for it.
    pub(crate) collect_startup: bool,
    pub(crate) startup_commands: Vec<(String, String)>,
    /// tmux's marked pane (`select-pane -m`): the default source of
    /// `swap-pane`, `join-pane` and `swap-window`. Read through
    /// [`Model::marked`], which forgets a pane that is gone.
    pub(crate) marked_pane: Option<PaneId>,
}

/// The largest window index: `base-index`'s maximum, tmux's INT_MAX. Every
/// index a command computes (`-t`, `-a`/`-b` shifting, renumbering) is
/// checked against it, so index arithmetic has headroom below `u32::MAX`
/// and cannot wrap onto a live window.
pub(crate) const MAX_WINDOW_INDEX: u32 = i32::MAX as u32;

/// Messages kept for `show-messages` (tmux's `message-limit` default).
const MESSAGE_LIMIT: usize = 1000;

pub(crate) fn short_host(h: &str) -> &str {
    h.split('.').next().unwrap_or(h)
}

impl Model {
    /// A server with no sessions, tmux's default options and key bindings.
    /// `hostname` feeds `#{host}` and default pane titles. The server should
    /// set `default-shell` from `$SHELL` (tmux does) with
    /// [`Model::set_global_option`].
    pub fn new(hostname: impl Into<String>) -> Model {
        Model {
            hostname: hostname.into(),
            sessions: BTreeMap::new(),
            windows: HashMap::new(),
            panes: HashMap::new(),
            clients: BTreeMap::new(),
            next_session: 0,
            next_window: 0,
            next_pane: 0,
            server_opts: OptionSet::defaults(Scope::Server),
            session_opts: OptionSet::defaults(Scope::Session),
            window_opts: OptionSet::defaults(Scope::Window),
            bindings: KeyBindings::with_defaults(),
            buffers: Vec::new(),
            next_buffer: 0,
            counter: 0,
            server_pid: 0,
            socket_path: String::new(),
            messages: std::collections::VecDeque::new(),
            q: Default::default(),
            fjobs: Default::default(),
            hooks: Default::default(),
            environ: Default::default(),
            last_found: Default::default(),
            clock: 0,
            mouse: None,
            marked_pane: None,
            unattached_dirty: false,
            collect_startup: false,
            startup_commands: Vec::new(),
        }
    }

    /// The server's process id and socket, for `#{pid}` and
    /// `#{socket_path}`.
    pub fn set_server_info(&mut self, pid: u32, socket_path: impl Into<String>) {
        self.server_pid = pid;
        self.socket_path = socket_path.into();
    }

    /// Add a line to the message log `show-messages` prints (configuration
    /// problems go there). The oldest lines are dropped past 1000.
    pub fn log_message(&mut self, text: impl Into<String>) {
        if self.messages.len() >= MESSAGE_LIMIT {
            self.messages.pop_front();
        }
        self.messages.push_back(text.into());
    }

    /// The message log, oldest first.
    pub fn messages(&self) -> impl Iterator<Item = &str> {
        self.messages.iter().map(|s| s.as_str())
    }

    // ----- read access -------------------------------------------------

    /// All sessions, by id.
    pub fn sessions(&self) -> impl Iterator<Item = &Session> {
        self.sessions.values()
    }
    /// A session.
    pub fn session(&self, id: SessionId) -> Option<&Session> {
        self.sessions.get(&id)
    }
    /// A window.
    pub fn window(&self, id: WindowId) -> Option<&Window> {
        self.windows.get(&id)
    }
    /// A pane.
    pub fn pane(&self, id: PaneId) -> Option<&Pane> {
        self.panes.get(&id)
    }
    /// All panes.
    pub fn panes(&self) -> impl Iterator<Item = &Pane> {
        self.panes.values()
    }
    /// A connected client.
    pub fn client(&self, id: ClientId) -> Option<&Client> {
        self.clients.get(&id)
    }
    /// Connected clients.
    pub fn clients(&self) -> impl Iterator<Item = &Client> {
        self.clients.values()
    }
    /// Clients attached to `session`.
    pub fn clients_of(&self, session: SessionId) -> Vec<ClientId> {
        self.clients.values().filter(|c| c.session == Some(session)).map(|c| c.id).collect()
    }
    /// The key bindings.
    pub fn bindings(&self) -> &KeyBindings {
        &self.bindings
    }
    /// Paste buffers, most recent first.
    pub fn buffers(&self) -> &[PasteBuffer] {
        &self.buffers
    }
    /// The host name given to [`Model::new`].
    pub fn hostname(&self) -> &str {
        &self.hostname
    }

    /// The text value of an option for a session (session and server
    /// options) or window (window options), as `show-options -v` prints it.
    /// Unknown names give `None`.
    pub fn option_text(&self, name: &str, session: Option<SessionId>, window: Option<WindowId>) -> Option<String> {
        if name.starts_with('@') {
            return self.user_option(name, session, window).map(|s| s.to_string());
        }
        let def = options::DEFS.iter().find(|d| d.name == name)?;
        Some(options::value_text(def, self.opt_value(def.name, def.scope, session, window)))
    }

    fn user_option(&self, name: &str, session: Option<SessionId>, window: Option<WindowId>) -> Option<&str> {
        window
            .and_then(|w| self.windows.get(&w))
            .and_then(|w| w.options.user(name))
            .or_else(|| self.window_opts.user(name))
            .or_else(|| session.and_then(|s| self.sessions.get(&s)).and_then(|s| s.options.user(name)))
            .or_else(|| self.session_opts.user(name))
            .or_else(|| self.server_opts.user(name))
    }

    pub(crate) fn opt_value(&self, name: &str, scope: Scope, s: Option<SessionId>, w: Option<WindowId>) -> &Value {
        let local = match scope {
            Scope::Server => None,
            Scope::Session => s.and_then(|s| self.sessions.get(&s)).and_then(|s| s.options.get(name)),
            Scope::Window => w.and_then(|w| self.windows.get(&w)).and_then(|w| w.options.get(name)),
        };
        let global = match scope {
            Scope::Server => &self.server_opts,
            Scope::Session => &self.session_opts,
            Scope::Window => &self.window_opts,
        };
        local.or_else(|| global.get(name)).unwrap_or_else(|| panic!("option {name} not defined"))
    }

    /// A window option as it applies to one pane: the pane's own value
    /// (`set -p`) if it has one, else the window's.
    pub(crate) fn popt(&self, p: PaneId, name: &str) -> &Value {
        let Some(pane) = self.panes.get(&p) else { return self.wopt(None, name) };
        pane.options.get(name).unwrap_or_else(|| self.wopt(Some(pane.window), name))
    }

    pub(crate) fn sopt(&self, s: Option<SessionId>, name: &str) -> &Value {
        self.opt_value(name, Scope::Session, s, None)
    }

    pub(crate) fn wopt(&self, w: Option<WindowId>, name: &str) -> &Value {
        self.opt_value(name, Scope::Window, None, w)
    }

    pub(crate) fn gopt(&self, name: &str) -> &Value {
        self.opt_value(name, Scope::Server, None, None)
    }

    /// Set a global option as `set-option -g` (or `-s` for server options)
    /// would, e.g. `("default-shell", "/bin/zsh")` at server start.
    pub fn set_global_option(&mut self, name: &str, value: &str) -> Result<(), CmdError> {
        let def = options::find(name).map_err(CmdError)?;
        let set = match def.scope {
            Scope::Server => &mut self.server_opts,
            Scope::Session => &mut self.session_opts,
            Scope::Window => &mut self.window_opts,
        };
        let cur = set.get(def.name).cloned().unwrap_or(Value::Flag(false));
        let v = options::parse_value(def, &cur, Some(value), false).map_err(CmdError)?;
        set.set(def.name, v);
        Ok(())
    }

    // ----- ids and small helpers ----------------------------------------

    pub(crate) fn tick(&mut self) -> u64 {
        self.counter += 1;
        self.counter
    }

    // Ids are never reused (scripts and hooks hold `%N` across commands), so
    // a long-lived server can in principle run a counter out. Then the
    // command fails rather than wrapping onto a live id; `u32::MAX` itself
    // is never handed out because the model uses it as a placeholder.

    pub(crate) fn alloc_pane(&mut self) -> Result<PaneId, CmdError> {
        let id = next_id(&mut self.next_pane).ok_or_else(|| CmdError::new("no free pane id"))?;
        Ok(PaneId(id))
    }

    pub(crate) fn alloc_window(&mut self) -> Result<WindowId, CmdError> {
        let id = next_id(&mut self.next_window).ok_or_else(|| CmdError::new("no free window id"))?;
        Ok(WindowId(id))
    }

    pub(crate) fn alloc_session(&mut self) -> Result<SessionId, CmdError> {
        let id = next_id(&mut self.next_session).ok_or_else(|| CmdError::new("no free session id"))?;
        Ok(SessionId(id))
    }

    pub(crate) fn session_by_name(&self, name: &str) -> Option<SessionId> {
        self.sessions.values().find(|s| s.name == name).map(|s| s.id)
    }

    /// Sessions sorted by name, as tmux keeps them.
    pub(crate) fn sessions_by_name(&self) -> Vec<SessionId> {
        let mut v: Vec<&Session> = self.sessions.values().collect();
        v.sort_by(|a, b| a.name.cmp(&b.name));
        v.into_iter().map(|s| s.id).collect()
    }

    pub(crate) fn pane_index(&self, p: PaneId) -> Option<usize> {
        let w = self.windows.get(&self.panes.get(&p)?.window)?;
        let base = self.wopt(Some(w.id), "pane-base-index").as_num() as usize;
        w.panes.iter().position(|x| *x == p).map(|i| i + base)
    }

    pub(crate) fn attached_count(&self, s: SessionId) -> usize {
        self.clients.values().filter(|c| c.session == Some(s)).count()
    }

    pub(crate) fn status_lines(&self, s: SessionId) -> u16 {
        self.sopt(Some(s), "status").as_num() as u16
    }

    /// The lowest unused window index at or above `base-index`.
    pub(crate) fn first_free_index(&self, s: SessionId) -> Result<u32, CmdError> {
        let base = self.base_index(s);
        let sess = &self.sessions[&s];
        (base..=MAX_WINDOW_INDEX).find(|i| !sess.windows.contains_key(i)).ok_or_else(|| CmdError::new("no free window index"))
    }

    /// `base-index`, clamped so a value from anywhere but the option parser
    /// cannot put a window above [`MAX_WINDOW_INDEX`].
    pub(crate) fn base_index(&self, s: SessionId) -> u32 {
        self.sopt(Some(s), "base-index").as_num().clamp(0, MAX_WINDOW_INDEX as i64) as u32
    }

    // ----- sizes ----------------------------------------------------------

    /// The size `window-size` asks for, from the clients attached to the
    /// window's session; `None` when no client decides (keep the size).
    pub(crate) fn desired_size(&self, sid: SessionId, wid: Option<WindowId>) -> Option<Size> {
        let policy = self.wopt(wid, "window-size").as_num();
        if policy == 2 {
            return None;
        }
        // aggressive-resize: only clients showing the window count, and a
        // session's clients all show its current window.
        if let Some(w) = wid
            && self.sessions.get(&sid).is_some_and(|s| s.current != w)
            && self.wopt(wid, "aggressive-resize").as_flag()
        {
            return None;
        }
        let sl = self.status_lines(sid) + (self.border_status(wid) != BorderStatus::Off) as u16;
        let sizes: Vec<(u64, Size)> = self
            .clients
            .values()
            .filter(|c| c.session == Some(sid) && !c.info.size.is_empty())
            .map(|c| (c.activity, Size::new(c.info.size.cols.max(1), c.info.size.rows.saturating_sub(sl).max(1))))
            .collect();
        if sizes.is_empty() {
            return None;
        }
        Some(match policy {
            0 => Size::new(sizes.iter().map(|s| s.1.cols).max()?, sizes.iter().map(|s| s.1.rows).max()?),
            1 => Size::new(sizes.iter().map(|s| s.1.cols).min()?, sizes.iter().map(|s| s.1.rows).min()?),
            _ => sizes.iter().max_by_key(|s| s.0)?.1,
        })
    }

    /// Size for a new window in `sid`.
    pub(crate) fn new_window_size(&self, sid: SessionId) -> Size {
        self.desired_size(sid, None).unwrap_or(self.sessions[&sid].size)
    }

    /// Resize the windows of a session to what its clients want.
    pub(crate) fn recalc_session(&mut self, sid: SessionId, host: &mut dyn Host) {
        // Every way a client joins or leaves a session comes through here.
        self.unattached_dirty = true;
        let Some(sess) = self.sessions.get(&sid) else { return };
        let wids: Vec<WindowId> = sess.windows.values().copied().collect();
        for wid in wids {
            if let Some(size) = self.desired_size(sid, Some(wid))
                && self.windows[&wid].size != size
            {
                self.resize_window(wid, size, host);
            }
        }
        host.redraw(Redraw::Session(sid));
    }

    pub(crate) fn resize_window(&mut self, wid: WindowId, size: Size, host: &mut dyn Host) {
        let Some(w) = self.windows.get_mut(&wid) else { return };
        w.size = size;
        w.layout.resize(size);
        self.sync_window(wid, host);
    }

    /// Push pane sizes from the layout to the host and redraw.
    pub(crate) fn sync_window(&mut self, wid: WindowId, host: &mut dyn Host) {
        let Some(w) = self.windows.get(&wid) else { return };
        let mut g = w.layout.unzoomed_geometry();
        if let Some(z) = w.layout.zoomed() {
            let full = w.layout.geometry();
            for (p, r) in &mut g.panes {
                if *p == z {
                    *r = full.rect(z).unwrap_or(*r);
                }
            }
        }
        for (p, r) in g.panes {
            if let Some(pane) = self.panes.get_mut(&p)
                && pane.size != r.size()
            {
                pane.size = r.size();
                host.resize_pane(p, r.size());
            }
        }
        if self.any_hooks() {
            self.notify("window-layout-changed", None, None, Some(wid), None);
        }
        host.redraw(Redraw::Window(wid));
    }

    // ----- current window / pane -----------------------------------------

    /// tmux's `session_set_current`. Returns whether anything changed.
    pub(crate) fn select_window(&mut self, sid: SessionId, wid: WindowId, host: &mut dyn Host) -> bool {
        let Some(s) = self.sessions.get_mut(&sid) else { return false };
        if s.current == wid {
            return false;
        }
        s.last.retain(|w| *w != wid);
        let old = s.current;
        s.last.insert(0, old);
        s.current = wid;
        if let Some(w) = self.windows.get_mut(&wid) {
            w.bell = false;
            w.activity = false;
            w.silence = false;
        }
        // A window sized only for the clients looking at it has some now,
        // and the one left behind has none.
        if [old, wid].iter().any(|w| self.wopt(Some(*w), "aggressive-resize").as_flag()) {
            self.recalc_session(sid, host);
        }
        self.notify("session-window-changed", None, Some(sid), Some(wid), None);
        host.redraw(Redraw::Session(sid));
        true
    }

    /// tmux's `window_set_active_pane`. Returns whether anything changed.
    pub(crate) fn set_active_pane(&mut self, wid: WindowId, pid: PaneId, host: &mut dyn Host) -> bool {
        let point = self.tick();
        let Some(w) = self.windows.get_mut(&wid) else { return false };
        if w.active == pid || !w.panes.contains(&pid) {
            return false;
        }
        w.last_panes.retain(|p| *p != pid);
        let old = w.active;
        w.last_panes.insert(0, old);
        w.active = pid;
        let sid = w.session;
        if let Some(p) = self.panes.get_mut(&pid) {
            p.active_point = point;
        }
        self.update_window_name(wid, host);
        self.notify("window-pane-changed", None, Some(sid), Some(wid), Some(pid));
        host.redraw(Redraw::Window(wid));
        host.redraw(Redraw::Status(sid));
        true
    }

    // ----- removal -------------------------------------------------------

    /// Remove a pane (kill-pane, or its process exited). `kill`: tell the
    /// host to kill the process. Closes the window and session as needed.
    pub(crate) fn remove_pane(&mut self, pid: PaneId, kill: bool, host: &mut dyn Host) {
        let Some(pane) = self.panes.get(&pid) else { return };
        let wid = pane.window;
        if self.windows[&wid].panes.len() == 1 {
            self.remove_window(wid, if kill { None } else { Some(pid) }, host);
            return;
        }
        if kill {
            host.kill_pane(pid);
        }
        self.panes.remove(&pid);
        let w = self.windows.get_mut(&wid).expect("window of pane");
        w.layout.unzoom();
        w.layout.remove(pid);
        let pos = w.panes.iter().position(|p| *p == pid).expect("pane in window");
        w.last_panes.retain(|p| *p != pid);
        if w.active == pid {
            let next = w.last_panes.first().copied().or_else(|| if pos > 0 { Some(w.panes[pos - 1]) } else { w.panes.get(pos + 1).copied() });
            w.panes.remove(pos);
            if let Some(n) = next {
                w.active = n;
                w.last_panes.retain(|p| *p != n);
            }
        } else {
            w.panes.remove(pos);
        }
        let point = self.tick();
        let active = self.windows[&wid].active;
        if let Some(p) = self.panes.get_mut(&active) {
            p.active_point = point;
        }
        self.sync_window(wid, host);
        self.update_window_name(wid, host);
        host.redraw(Redraw::Status(self.windows[&wid].session));
    }

    /// Remove a window and all its panes (killing their processes except
    /// `exited`, which is already gone). Destroys the session if it was its
    /// last window.
    pub(crate) fn remove_window(&mut self, wid: WindowId, exited: Option<PaneId>, host: &mut dyn Host) {
        self.remove_window_inner(wid, exited, true, host);
    }

    /// [`Model::remove_window`]; `renumber` is false when the caller is
    /// about to reuse the index (`-k`), as tmux does not renumber then.
    pub(crate) fn remove_window_inner(&mut self, wid: WindowId, exited: Option<PaneId>, renumber: bool, host: &mut dyn Host) {
        self.notify("window-unlinked", None, None, Some(wid), None);
        let Some(w) = self.windows.remove(&wid) else { return };
        for p in &w.panes {
            if Some(*p) != exited {
                host.kill_pane(*p);
            }
            self.panes.remove(p);
        }
        let sid = w.session;
        let Some(s) = self.sessions.get_mut(&sid) else { return };
        s.windows.remove(&w.index);
        s.last.retain(|x| *x != wid);
        if s.windows.is_empty() {
            self.destroy_session(sid, host);
            return;
        }
        if s.current == wid {
            // tmux's session_detach: the last window, else the previous one
            // (wrapping to the end).
            let next = s.last.first().copied().unwrap_or_else(|| {
                s.windows.range(..w.index).next_back().or_else(|| s.windows.iter().next_back()).map(|(_, w)| *w).expect("not empty")
            });
            s.last.retain(|x| *x != next);
            s.current = next;
            if let Some(nw) = self.windows.get_mut(&next) {
                nw.bell = false;
                nw.activity = false;
                nw.silence = false;
            }
        }
        if renumber && self.sopt(Some(sid), "renumber-windows").as_flag() {
            self.renumber(sid);
        }
        host.redraw(Redraw::Session(sid));
    }

    pub(crate) fn renumber(&mut self, sid: SessionId) {
        let base = self.base_index(sid);
        let s = self.sessions.get_mut(&sid).expect("session");
        let wins: Vec<WindowId> = s.windows.values().copied().collect();
        // With base-index near the maximum the run would pass it; end the
        // run at the maximum instead (tmux would overflow its int there).
        let n = u32::try_from(wins.len()).unwrap_or(u32::MAX);
        // Every window already has an index at or below the maximum, so
        // there are at most MAX_WINDOW_INDEX + 1 of them and the zip below
        // drops none.
        let start = base.min(MAX_WINDOW_INDEX.saturating_sub(n.saturating_sub(1)));
        s.windows.clear();
        for (wid, idx) in wins.into_iter().zip(start..=MAX_WINDOW_INDEX) {
            s.windows.insert(idx, wid);
            if let Some(w) = self.windows.get_mut(&wid) {
                w.index = idx;
            }
        }
    }

    /// Destroy a session: move or detach its clients (`detach-on-destroy`),
    /// kill its panes, and shut down if it was the last one and
    /// `exit-empty` is on.
    pub(crate) fn destroy_session(&mut self, sid: SessionId, host: &mut dyn Host) {
        self.notify("session-closed", None, Some(sid), None, None);
        let Some(s) = self.sessions.remove(&sid) else { return };
        for wid in s.windows.values() {
            if let Some(w) = self.windows.remove(wid) {
                for p in &w.panes {
                    host.kill_pane(*p);
                    self.panes.remove(p);
                }
            }
        }
        let policy = self.sopt(Some(sid), "detach-on-destroy").as_num();
        let policy = s.options.get("detach-on-destroy").map(|v| v.as_num()).unwrap_or(policy);
        for cid in self.clients_of(sid) {
            let target = self.destroy_target(&s, policy);
            match target {
                Some(t) => self.switch_client(cid, t, host),
                None => {
                    let c = self.clients.get_mut(&cid).expect("client");
                    c.session = None;
                    c.keys = KeyState::default();
                    host.detach_client(cid, DetachReason::SessionEnded);
                }
            }
        }
        for c in self.clients.values_mut() {
            if c.last_session == Some(sid) {
                c.last_session = None;
            }
        }
        if self.sessions.is_empty() && self.gopt("exit-empty").as_flag() {
            self.shutdown(host);
        }
    }

    fn destroy_target(&self, gone: &Session, policy: i64) -> Option<SessionId> {
        let by_name = self.sessions_by_name();
        let after = by_name.iter().find(|s| self.sessions[*s].name > gone.name).or_else(|| by_name.first());
        let before = by_name.iter().rev().find(|s| self.sessions[*s].name < gone.name).or_else(|| by_name.last());
        match policy {
            0 => self.sessions.values().max_by_key(|s| s.activity).map(|s| s.id),
            2 => self.sessions.values().filter(|s| self.attached_count(s.id) == 0).max_by_key(|s| s.activity).map(|s| s.id),
            3 => before.copied(),
            4 => after.copied(),
            _ => None,
        }
    }

    /// Detach everybody, kill everything, ask the host to exit.
    pub(crate) fn shutdown(&mut self, host: &mut dyn Host) {
        let attached: Vec<ClientId> = self.clients.values().filter(|c| c.session.is_some()).map(|c| c.id).collect();
        for c in attached {
            if let Some(cl) = self.clients.get_mut(&c) {
                cl.session = None;
            }
            host.detach_client(c, DetachReason::ServerExit);
        }
        let mut panes: Vec<PaneId> = self.panes.keys().copied().collect();
        panes.sort();
        for p in panes {
            host.kill_pane(p);
        }
        self.panes.clear();
        self.windows.clear();
        self.sessions.clear();
        host.shutdown();
    }

    // ----- clients ---------------------------------------------------------

    /// Attach (or switch) a client to a session.
    pub(crate) fn switch_client(&mut self, cid: ClientId, sid: SessionId, host: &mut dyn Host) {
        let tick = self.tick();
        let Some(c) = self.clients.get_mut(&cid) else { return };
        let old = c.session;
        if old != Some(sid) {
            c.last_session = old;
        }
        c.session = Some(sid);
        c.activity = tick;
        if let Some(s) = self.sessions.get_mut(&sid) {
            s.activity = tick;
        }
        host.attach_client(cid, sid);
        match old {
            None => self.notify("client-attached", Some(cid), Some(sid), None, None),
            Some(o) if o != sid => self.notify("client-session-changed", Some(cid), Some(sid), None, None),
            Some(_) => {}
        }
        self.recalc_session(sid, host);
        if let Some(o) = old
            && o != sid
        {
            self.recalc_session(o, host);
        }
        host.redraw(Redraw::Client(cid));
    }

    pub(crate) fn detach(&mut self, cid: ClientId, reason: DetachReason, host: &mut dyn Host) {
        let Some(c) = self.clients.get_mut(&cid) else { return };
        let Some(sid) = c.session.take() else { return };
        c.keys = KeyState::default();
        self.notify("client-detached", Some(cid), Some(sid), None, None);
        host.detach_client(cid, reason);
        self.recalc_session(sid, host);
    }

    /// A client connected (before it runs any command).
    pub fn client_connect(&mut self, id: ClientId, mut info: ClientInfo) {
        info.size = clamp_size(info.size);
        let activity = self.tick();
        self.clients.insert(
            id,
            Client { id, info, session: None, last_session: None, activity, keys: KeyState::default() },
        );
    }

    /// A client's connection closed. Its attachment ends without a
    /// `detach_client` call (there is nobody to tell).
    pub fn client_disconnect(&mut self, id: ClientId, host: &mut dyn Host) {
        self.forget_waiter(id);
        if let Some(c) = self.clients.remove(&id)
            && let Some(sid) = c.session
        {
            self.recalc_session(sid, host);
        }
    }

    /// An attached client's terminal changed size.
    pub fn client_resized(&mut self, id: ClientId, size: Size, host: &mut dyn Host) {
        let size = clamp_size(size);
        let tick = self.tick();
        let Some(c) = self.clients.get_mut(&id) else { return };
        c.info.size = size;
        c.activity = tick;
        if let Some(sid) = c.session {
            self.recalc_session(sid, host);
            self.notify("client-resized", Some(id), Some(sid), None, None);
            host.redraw(Redraw::Client(id));
        }
    }

    /// A key press from an attached client: returns what to do with it
    /// (see [`KeyState::handle`]). Counts as client activity for
    /// `window-size latest`. `now_ms` is a monotonic clock in milliseconds.
    pub fn handle_key(&mut self, id: ClientId, key: &KeyEvent, now_ms: u64, host: &mut dyn Host) -> KeyDecision {
        let tick = self.tick();
        let Some(c) = self.clients.get(&id) else { return KeyDecision::SendToPane };
        let sid = c.session;
        let mut prefixes = [KeyCode::new(lif_types::Key::Escape, lif_types::Mods::NONE); 2];
        let mut n = 0;
        for name in ["prefix", "prefix2"] {
            if let Value::Key(Some(k)) = self.sopt(sid, name) {
                prefixes[n] = *k;
                n += 1;
            }
        }
        let prefixes = &prefixes[..n];
        let repeat = self.sopt(sid, "repeat-time").as_num().max(0) as u64;
        let was_latest = sid.is_none_or(|s| {
            self.clients.values().filter(|x| x.session == Some(s)).max_by_key(|x| x.activity).map(|x| x.id) == Some(id)
        });
        let c = self.clients.get_mut(&id).expect("client");
        let was_prefix = c.keys.in_prefix();
        c.activity = tick;
        let d = c.keys.handle(&self.bindings, prefixes, repeat, key, now_ms);
        let now_prefix = c.keys.in_prefix();
        if let Some(sid) = sid {
            if let Some(s) = self.sessions.get_mut(&sid) {
                s.activity = tick;
            }
            if was_prefix != now_prefix {
                host.redraw(Redraw::Status(sid));
            }
            if !was_latest {
                self.recalc_session(sid, host);
            }
        }
        d
    }

    /// Leave an expired repeat window (call when the timer for
    /// [`KeyState::repeat_deadline`] fires). Returns whether the client's
    /// state changed.
    pub fn expire_key_state(&mut self, id: ClientId, now_ms: u64) -> bool {
        self.clients.get_mut(&id).is_some_and(|c| c.keys.expire(now_ms))
    }

    // ----- pane events -------------------------------------------------------

    /// The process in `pane` exited with `status` (`None` if killed by a
    /// signal). The pane is removed, then its window if it was the last
    /// pane, then the session (clients detached or switched per
    /// `detach-on-destroy`), unless `remain-on-exit` keeps it as a dead pane.
    pub fn pane_exited(&mut self, pane: PaneId, status: Option<i32>, host: &mut dyn Host) {
        let Some(p) = self.panes.get(&pane) else { return };
        let wid = p.window;
        let remain = self.popt(pane, "remain-on-exit").as_num();
        if remain == 1 || (remain == 2 && status != Some(0)) {
            let p = self.panes.get_mut(&pane).expect("pane");
            p.dead = true;
            p.exit_status = status;
            self.notify("pane-died", None, None, Some(wid), Some(pane));
            self.update_window_name(wid, host);
            host.redraw(Redraw::Window(wid));
            return;
        }
        self.notify("pane-exited", None, None, Some(wid), Some(pane));
        self.remove_pane(pane, false, host);
    }

    /// A bell rang in `pane`. Sets the window's bell flag if it is not the
    /// current window and rings the clients `bell-action` selects.
    pub fn pane_bell(&mut self, pane: PaneId, host: &mut dyn Host) {
        let Some(p) = self.panes.get(&pane) else { return };
        let wid = p.window;
        if !self.wopt(Some(wid), "monitor-bell").as_flag() {
            return;
        }
        let sid = self.windows[&wid].session;
        let current = self.sessions[&sid].current == wid;
        if !current {
            let w = self.windows.get_mut(&wid).expect("window");
            if !w.bell {
                w.bell = true;
                self.notify("alert-bell", None, Some(sid), Some(wid), Some(pane));
                host.redraw(Redraw::Status(sid));
            }
        }
        self.alert(sid, wid, current, "bell-action", "visual-bell", "Bell", host);
    }

    /// Tell a session's clients about an alert in one of its windows, as
    /// the `*-action` option selects (none, any window, the current one,
    /// the others) and the `visual-*` option says how: a bell, a message
    /// (`Bell in window 2`), or both.
    #[allow(clippy::too_many_arguments)]
    fn alert(&mut self, sid: SessionId, wid: WindowId, current: bool, action: &str, visual: &str, what: &str, host: &mut dyn Host) {
        let tell = match self.sopt(Some(sid), action).as_num() {
            0 => false,
            2 => current,
            3 => !current,
            _ => true,
        };
        if !tell {
            return;
        }
        let visual = self.sopt(Some(sid), visual).as_num();
        let ms = self.sopt(Some(sid), "display-time").as_num() as u64;
        let text = if current { format!("{what} in current window") } else { format!("{what} in window {}", self.windows[&wid].index) };
        for c in self.clients_of(sid) {
            if visual != 1 {
                host.bell(c);
            }
            if visual != 0 {
                host.show_message(c, &text, ms);
            }
        }
    }

    /// How long `pane`'s window may be quiet before it is flagged
    /// (`monitor-silence`, seconds); 0: silence is not watched.
    pub fn silence_secs(&self, pane: PaneId) -> u64 {
        match self.panes.get(&pane) {
            Some(p) => self.wopt(Some(p.window), "monitor-silence").as_num().max(0) as u64,
            None => 0,
        }
    }

    /// `pane` printed nothing for its window's `monitor-silence` seconds:
    /// the window gets its silence flag unless it is the current one, and
    /// the clients `silence-action` selects are told.
    pub fn pane_silence(&mut self, pane: PaneId, host: &mut dyn Host) {
        let Some(p) = self.panes.get(&pane) else { return };
        let wid = p.window;
        if self.wopt(Some(wid), "monitor-silence").as_num() <= 0 {
            return;
        }
        let sid = self.windows[&wid].session;
        let current = self.sessions[&sid].current == wid;
        if current || self.windows[&wid].silence {
            return;
        }
        self.windows.get_mut(&wid).expect("window").silence = true;
        self.notify("alert-silence", None, Some(sid), Some(wid), Some(pane));
        host.redraw(Redraw::Status(sid));
        self.alert(sid, wid, current, "silence-action", "visual-silence", "Silence", host);
    }

    /// Destroy the sessions nobody is attached to that have
    /// `destroy-unattached` on. Cheap unless something that could have
    /// left one changed since the last call (a client left, a session was
    /// made, an option was set); the server calls it after every batch of
    /// commands.
    pub fn check_unattached(&mut self, host: &mut dyn Host) {
        if !std::mem::take(&mut self.unattached_dirty) {
            return;
        }
        let gone: Vec<SessionId> = self
            .sessions
            .keys()
            .copied()
            .filter(|s| self.sopt(Some(*s), "destroy-unattached").as_num() == 1 && self.clients.values().all(|c| c.session != Some(*s)))
            .collect();
        for sid in gone {
            self.destroy_session(sid, host);
        }
    }

    /// Output arrived in `pane`. Only matters with `monitor-activity`; cheap
    /// otherwise.
    pub fn pane_activity(&mut self, pane: PaneId, host: &mut dyn Host) {
        let Some(p) = self.panes.get(&pane) else { return };
        let wid = p.window;
        if !self.wopt(Some(wid), "monitor-activity").as_flag() {
            return;
        }
        let sid = self.windows[&wid].session;
        if self.sessions[&sid].current != wid {
            let w = self.windows.get_mut(&wid).expect("window");
            if !w.activity {
                w.activity = true;
                self.notify("alert-activity", None, Some(sid), Some(wid), Some(pane));
                host.redraw(Redraw::Status(sid));
                // Once per flag, as in tmux: not for every line of output.
                self.alert(sid, wid, false, "activity-action", "visual-activity", "Activity", host);
            }
        }
    }

    /// The title, foreground command or directory of `pane` changed:
    /// re-derive the automatic window name and redraw status lines.
    pub fn pane_changed(&mut self, pane: PaneId, host: &mut dyn Host) {
        let Some(p) = self.panes.get(&pane) else { return };
        let wid = p.window;
        self.update_window_name(wid, host);
        let sid = self.windows[&wid].session;
        host.redraw(Redraw::Status(sid));
    }

    pub(crate) fn update_window_name(&mut self, wid: WindowId, host: &dyn Host) {
        if !self.wopt(Some(wid), "automatic-rename").as_flag() {
            return;
        }
        let Some(w) = self.windows.get(&wid) else { return };
        let fmt = self.wopt(Some(wid), "automatic-rename-format").as_str().to_string();
        let name = {
            let ctx = self.fctx(host, Some(w.session), Some(wid), Some(w.active), None, WallClock::default());
            format::expand(&fmt, &ctx)
        };
        if name.is_empty() {
            return;
        }
        let w = self.windows.get_mut(&wid).expect("window");
        if w.name != name {
            w.name = name;
        }
    }

    // ----- rendering helpers -------------------------------------------------

    /// What `client` shows, or `None` if it is not attached.
    pub fn client_view(&self, client: ClientId) -> Option<ClientView> {
        let c = self.clients.get(&client)?;
        let sid = c.session?;
        let s = self.sessions.get(&sid)?;
        let w = self.windows.get(&s.current)?;
        let status = self.status_lines(sid) > 0;
        let top = status && self.sopt(Some(sid), "status-position").as_num() == 0;
        let border_status = self.border_status(Some(w.id));
        let dy = top as u16 + (border_status == BorderStatus::Top) as u16;
        let Geometry { panes, borders } = w.layout.geometry();
        let shift = |r: Rect| Rect::new(r.x, r.y + dy, r.cols, r.rows);
        let status_row = status.then(|| if top { 0 } else { c.info.size.rows.saturating_sub(1) });
        Some(ClientView {
            session: sid,
            window: w.id,
            active_pane: w.active,
            zoomed: w.layout.zoomed().is_some(),
            panes: panes.into_iter().map(|(p, r)| (p, shift(r))).collect(),
            borders: borders.into_iter().map(shift).collect(),
            status_row,
            window_size: w.size,
            border_status,
        })
    }

    /// `pane-border-status` of a window (the global value for `None`).
    pub(crate) fn border_status(&self, wid: Option<WindowId>) -> BorderStatus {
        match self.wopt(wid, "pane-border-status").as_num() {
            1 => BorderStatus::Top,
            2 => BorderStatus::Bottom,
            _ => BorderStatus::Off,
        }
    }

    /// The `pane-border-format` line of `pane`, `width` cells at most, in
    /// the pane's border style (the active one for the active pane).
    pub fn pane_border_line(&self, pane: PaneId, width: u16, host: &dyn Host, now: WallClock) -> StyledLine {
        let Some(p) = self.panes.get(&pane) else { return StyledLine::new() };
        let Some(w) = self.windows.get(&p.window) else { return StyledLine::new() };
        let active = w.active == pane;
        let base = status::parse_style(self.popt(pane, if active { "pane-active-border-style" } else { "pane-border-style" }).as_str());
        let ctx = self.fctx(host, Some(w.session), Some(w.id), Some(pane), None, now);
        let text = format::expand(self.popt(pane, "pane-border-format").as_str(), &ctx);
        status::label(&text, base, width as usize)
    }

    /// A window option (or user option) as it applies to `pane`, as text.
    pub fn pane_option_text(&self, pane: PaneId, name: &str) -> Option<String> {
        let p = self.panes.get(&pane)?;
        if name.starts_with('@') {
            return p.options.user(name).map(str::to_string).or_else(|| {
                let sid = self.windows.get(&p.window).map(|w| w.session);
                self.user_option(name, sid, Some(p.window)).map(str::to_string)
            });
        }
        let def = options::DEFS.iter().find(|d| d.name == name && d.scope == Scope::Window)?;
        Some(options::value_text(def, self.popt(pane, def.name)))
    }

    /// While `on`, the `[startup]` commands of configuration files that
    /// `source-file` reads are kept for [`Model::take_startup`]; the
    /// server turns this on only while it starts.
    pub fn collect_startup(&mut self, on: bool) {
        self.collect_startup = on;
    }

    /// The `[startup]` commands collected so far, each with where it came
    /// from (`path:line:column`), in the order they were read.
    pub fn take_startup(&mut self) -> Vec<(String, String)> {
        std::mem::take(&mut self.startup_commands)
    }

    /// The marked pane, if there is one and it still exists.
    pub fn marked(&self) -> Option<PaneId> {
        self.marked_pane.filter(|p| self.panes.contains_key(p))
    }

    /// Where the mouse is, for the commands of a mouse key about to run
    /// (`-t =`, the `mouse_*` formats); `None` when they are done.
    pub fn set_mouse(&mut self, at: Option<crate::host::MouseAt>) {
        self.mouse = at;
    }

    /// `select-pane -d` turned the pane's input off: what clients type
    /// or paste there is dropped.
    pub fn pane_input_off(&self, pane: PaneId) -> bool {
        self.panes.get(&pane).is_some_and(|p| p.input_off)
    }

    /// The panes that get what is typed into `pane` as well, appended to
    /// `out`: with `synchronize-panes` on for `pane`, the other live
    /// panes of its window that have it on too. Whether one of them is in
    /// a mode (and so takes no input) is for the caller to check.
    pub fn synchronized_panes(&self, pane: PaneId, out: &mut Vec<PaneId>) {
        let Some(p) = self.panes.get(&pane) else { return };
        if !self.popt(pane, "synchronize-panes").as_flag() {
            return;
        }
        let Some(w) = self.windows.get(&p.window) else { return };
        // A zoomed window shows one pane: the hidden ones are left alone,
        // as tmux leaves panes that are not visible.
        if w.layout.zoomed().is_some() {
            return;
        }
        for o in &w.panes {
            if *o != pane && self.panes.get(o).is_some_and(|x| !x.dead) && self.popt(*o, "synchronize-panes").as_flag() {
                out.push(*o);
            }
        }
    }

    /// The program in `pane` set its title: it replaces one set with
    /// `select-pane -T`, as in tmux.
    pub fn pane_title_reset(&mut self, pane: PaneId) {
        if let Some(p) = self.panes.get_mut(&pane) {
            p.title = None;
        }
    }

    /// The outer terminal's title for clients of `session` (`set-titles`,
    /// `set-titles-string`), or `None` when `set-titles` is off.
    pub fn titles_string(&self, session: SessionId, host: &dyn Host, now: WallClock) -> Option<String> {
        let sid = Some(session);
        if !self.sopt(sid, "set-titles").as_flag() {
            return None;
        }
        let s = self.sessions.get(&session)?;
        let w = self.windows.get(&s.current)?;
        let ctx = self.fctx(host, sid, Some(w.id), Some(w.active), None, now);
        let text = format::expand_time(self.sopt(sid, "set-titles-string").as_str(), &ctx);
        Some(text.chars().filter(|c| !c.is_control()).collect())
    }

    /// The status line of `session` for a client `width` cells wide, in
    /// tmux's default look unless the status options say otherwise.
    pub fn status_line(&self, session: SessionId, width: u16, now: WallClock, host: &dyn Host) -> StyledLine {
        self.status_parts(session, width, now, host).map(|p| status::render(&p)).unwrap_or_default()
    }

    /// Where the parts of [`Model::status_line`] are, and the window of
    /// each entry: what a mouse key on the status line is about.
    pub fn status_ranges(&self, session: SessionId, width: u16, now: WallClock, host: &dyn Host) -> (status::StatusRanges, Vec<WindowId>) {
        let mut r = status::StatusRanges::default();
        if let Some(p) = self.status_parts(session, width, now, host) {
            status::render_with(&p, Some(&mut r));
        }
        let ids = self.sessions.get(&session).map(|s| s.windows.values().copied().filter(|w| self.windows.contains_key(w)).collect()).unwrap_or_default();
        (r, ids)
    }

    fn status_parts(&self, session: SessionId, width: u16, now: WallClock, host: &dyn Host) -> Option<StatusParts> {
        let s = self.sessions.get(&session)?;
        let sid = Some(session);
        let cur = s.current;
        let active = self.windows.get(&cur).map(|w| w.active);
        let ctx = self.fctx(host, sid, Some(cur), active, None, now);
        let base = status::parse_style(self.sopt(sid, "status-style").as_str());
        let mut left_style = base;
        status::apply_style(self.sopt(sid, "status-left-style").as_str(), &mut left_style, base);
        let mut right_style = base;
        status::apply_style(self.sopt(sid, "status-right-style").as_str(), &mut right_style, base);
        let left = format::expand_time(self.sopt(sid, "status-left").as_str(), &ctx);
        let right = format::expand_time(self.sopt(sid, "status-right").as_str(), &ctx);
        let last = s.last.first().copied();
        let mut windows = Vec::with_capacity(s.windows.len());
        for wid in s.windows.values() {
            let Some(w) = self.windows.get(wid) else { continue };
            let wo = Some(*wid);
            let is_cur = *wid == cur;
            let fmt = if is_cur { "window-status-current-format" } else { "window-status-format" };
            let wctx = self.fctx(host, sid, wo, Some(w.active), None, now);
            let text = format::expand_time(self.wopt(wo, fmt).as_str(), &wctx);
            let mut style = base;
            status::apply_style(self.wopt(wo, "window-status-style").as_str(), &mut style, base);
            if is_cur {
                status::apply_style(self.wopt(wo, "window-status-current-style").as_str(), &mut style, base);
            }
            if Some(*wid) == last {
                status::apply_style(self.wopt(wo, "window-status-last-style").as_str(), &mut style, base);
            }
            if w.bell {
                status::apply_style(self.wopt(wo, "window-status-bell-style").as_str(), &mut style, base);
            } else if w.activity || w.silence {
                status::apply_style(self.wopt(wo, "window-status-activity-style").as_str(), &mut style, base);
            }
            windows.push(WindowEntry { text, style, current: is_cur });
        }
        Some(StatusParts {
            width: width as usize,
            base,
            left,
            left_style,
            left_length: self.sopt(sid, "status-left-length").as_num() as usize,
            right,
            right_style,
            right_length: self.sopt(sid, "status-right-length").as_num() as usize,
            windows,
            separator: self.wopt(Some(cur), "window-status-separator").as_str().to_string(),
            justify: self.sopt(sid, "status-justify").as_num() as usize,
        })
    }

    /// Expand a format with session / window / pane / client context
    /// (any may be `None`).
    #[allow(clippy::too_many_arguments)]
    pub fn expand_format(
        &self,
        fmt: &str,
        session: Option<SessionId>,
        window: Option<WindowId>,
        pane: Option<PaneId>,
        client: Option<ClientId>,
        now: WallClock,
        host: &dyn Host,
    ) -> String {
        format::expand(fmt, &self.fctx(host, session, window, pane, client, now))
    }

    pub(crate) fn fctx<'a>(
        &'a self,
        host: &'a dyn Host,
        s: Option<SessionId>,
        w: Option<WindowId>,
        p: Option<PaneId>,
        c: Option<ClientId>,
        now: WallClock,
    ) -> Fctx<'a> {
        Fctx { m: self, host, s, w, p, c, now, buffer: None }
    }

    // ----- spawning -----------------------------------------------------------

    pub(crate) fn spawn_command(&self, sid: SessionId, args: &[String]) -> SpawnCommand {
        let shell = self.sopt(Some(sid), "default-shell").as_str().to_string();
        match args {
            [] => {
                let dc = self.sopt(Some(sid), "default-command").as_str();
                if dc.is_empty() {
                    SpawnCommand::LoginShell { shell }
                } else {
                    SpawnCommand::Shell { shell, command: dc.to_string() }
                }
            }
            [one] => SpawnCommand::Shell { shell, command: one.clone() },
            many => SpawnCommand::Argv(many.to_vec()),
        }
    }

    #[allow(clippy::too_many_arguments)]
    pub(crate) fn spawn_spec(&self, pane: PaneId, window: WindowId, session: SessionId, command: SpawnCommand, cwd: String, size: Size, client: Option<ClientId>, respawn: bool) -> SpawnSpec {
        SpawnSpec {
            pane,
            window,
            session,
            command,
            cwd,
            size,
            history_limit: self.sopt(Some(session), "history-limit").as_num() as usize,
            scrollback_bytes: self.sopt(Some(session), "scrollback-bytes").as_num() as usize,
            client,
            respawn,
            tmux_compat: self.gopt("tmux-compat").as_flag(),
            env: self.environ_overrides(Some(session)),
        }
    }
}

/// Format variables for one context.
pub(crate) struct Fctx<'a> {
    pub m: &'a Model,
    pub host: &'a dyn Host,
    pub s: Option<SessionId>,
    pub w: Option<WindowId>,
    pub p: Option<PaneId>,
    pub c: Option<ClientId>,
    pub now: WallClock,
    pub buffer: Option<&'a PasteBuffer>,
}

fn flag(b: bool) -> Option<String> {
    Some(if b { "1" } else { "0" }.to_string())
}

impl Vars for Fctx<'_> {
    fn now(&self) -> WallClock {
        self.now
    }

    fn shell(&self, command: &str) -> Option<String> {
        let interval = self.m.sopt(self.s, "status-interval").as_num();
        Some(self.m.fjobs.borrow_mut().expand(command, self.s, self.now.unix_secs, interval))
    }

    fn var(&self, name: &str) -> Option<String> {
        let m = self.m;
        let s = self.s.and_then(|s| m.sessions.get(&s));
        let w = self.w.and_then(|w| m.windows.get(&w));
        let p = self.p.and_then(|p| m.panes.get(&p));
        let c = self.c.and_then(|c| m.clients.get(&c));
        if name.starts_with("hook")
            && let Some(h) = &m.hooks.current
            && let Some(v) = h.var(name)
        {
            return Some(v);
        }
        match name {
            "mouse_any_flag" => return flag(self.p.is_some_and(|p| self.host.pane_info(p).mouse_any)),
            "alternate_on" => return flag(self.p.is_some_and(|p| self.host.pane_info(p).alternate)),
            "mouse_x" | "mouse_y" | "mouse_word" | "mouse_line" => {
                let at = m.mouse?;
                let (x, y) = if at.pane.is_some() { at.cell } else { at.screen };
                return Some(match name {
                    "mouse_x" => x.to_string(),
                    "mouse_y" => y.to_string(),
                    _ => {
                        // The line under the mouse, and the word on it.
                        let pane = at.pane?;
                        let range = crate::host::CaptureRange {
                            start: crate::host::LinePos::Line(y as i64),
                            end: crate::host::LinePos::Line(y as i64),
                            join_wrapped: false,
                            keep_trailing: false,
                            styled: false,
                        };
                        let line = self.host.capture_pane(pane, range);
                        let line = line.trim_end_matches('\n');
                        if name == "mouse_line" {
                            line.to_string()
                        } else {
                            let seps = m.sopt(self.s, "word-separators").as_str();
                            let chars: Vec<char> = line.chars().collect();
                            let is_word = |c: char| !c.is_whitespace() && !seps.contains(c);
                            let i = (x as usize).min(chars.len().saturating_sub(1));
                            if chars.is_empty() || !is_word(chars[i]) {
                                String::new()
                            } else {
                                let a = (0..=i).rev().take_while(|j| is_word(chars[*j])).last().unwrap_or(i);
                                let b = (i..chars.len()).take_while(|j| is_word(chars[*j])).last().unwrap_or(i);
                                chars[a..=b].iter().collect()
                            }
                        }
                    }
                });
            }
            _ => {}
        }
        if let Some(rest) = name.strip_prefix("session_") {
            let s = s?;
            return match rest {
                "name" => Some(s.name.clone()),
                "id" => Some(s.id.to_string()),
                "windows" => Some(s.windows.len().to_string()),
                "attached" => Some(m.attached_count(s.id).to_string()),
                "many_attached" => flag(m.attached_count(s.id) > 1),
                "created" | "activity" => Some(s.created.to_string()),
                "path" => Some(s.cwd.clone()),
                "grouped" => flag(false),
                "group" => Some(String::new()),
                "format" => flag(true),
                "alerts" => {
                    let v: Vec<String> = s
                        .windows
                        .iter()
                        .filter_map(|(i, wid)| {
                            let w = m.windows.get(wid)?;
                            let f = if w.bell { "!" } else if w.activity { "#" } else if w.silence { "~" } else { return None };
                            Some(format!("{i}{f}"))
                        })
                        .collect();
                    Some(v.join(","))
                }
                _ => None,
            };
        }
        if let Some(rest) = name.strip_prefix("window_") {
            let w = w?;
            let sess = m.sessions.get(&w.session);
            let is_cur = sess.is_some_and(|s| s.current == w.id);
            let is_last = sess.is_some_and(|s| s.last.first() == Some(&w.id));
            return match rest {
                "index" => Some(w.index.to_string()),
                "name" => Some(w.name.clone()),
                "id" => Some(w.id.to_string()),
                "flags" | "raw_flags" => {
                    let mut f = String::new();
                    if w.activity {
                        f.push_str(if rest == "flags" { "##" } else { "#" });
                    }
                    if w.bell {
                        f.push('!');
                    }
                    if w.silence {
                        f.push('~');
                    }
                    if is_cur {
                        f.push('*');
                    }
                    if is_last {
                        f.push('-');
                    }
                    if m.marked().is_some_and(|p| m.panes[&p].window == w.id) {
                        f.push('M');
                    }
                    if w.layout.zoomed().is_some() {
                        f.push('Z');
                    }
                    Some(f)
                }
                "active" => flag(is_cur),
                "last_flag" => flag(is_last),
                "bell_flag" => flag(w.bell),
                "activity_flag" => flag(w.activity),
                "marked_flag" => flag(m.marked().is_some_and(|p| m.panes[&p].window == w.id)),
                "silence_flag" => flag(w.silence),
                "bigger" | "linked" => flag(false),
                "zoomed_flag" => flag(w.layout.zoomed().is_some()),
                "panes" => Some(w.panes.len().to_string()),
                "width" => Some(w.size.cols.to_string()),
                "height" => Some(w.size.rows.to_string()),
                "layout" | "visible_layout" => Some(w.layout.dump()),
                "offset_x" | "offset_y" => Some("0".into()),
                "start_flag" => flag(sess.and_then(|s| s.windows.keys().next()) == Some(&w.index)),
                "end_flag" => flag(sess.and_then(|s| s.windows.keys().next_back()) == Some(&w.index)),
                "format" => flag(true),
                _ => None,
            };
        }
        if let Some(rest) = name.strip_prefix("pane_") {
            let p = p?;
            let pw = m.windows.get(&p.window);
            let info = self.host.pane_info(p.id);
            let rect = pw.and_then(|w| w.layout.unzoomed_geometry().rect(p.id));
            let wsize = pw.map(|w| w.layout.size()).unwrap_or_default();
            return match rest {
                "index" => m.pane_index(p.id).map(|i| i.to_string()),
                "id" => Some(p.id.to_string()),
                "title" => Some(match &p.title {
                    Some(t) => t.clone(),
                    None if info.title.is_empty() => m.hostname.clone(),
                    None => info.title.to_string(),
                }),
                "current_command" => Some(info.current_command.to_string()),
                "current_path" => Some(info.current_path.to_string()),
                "width" => Some(p.size.cols.to_string()),
                "height" => Some(p.size.rows.to_string()),
                "active" => flag(pw.is_some_and(|w| w.active == p.id)),
                "last" => flag(pw.is_some_and(|w| w.last_panes.first() == Some(&p.id))),
                "pid" => Some(info.pid.to_string()),
                "dead" => flag(p.dead),
                "dead_status" => Some(p.exit_status.map(|s| s.to_string()).unwrap_or_default()),
                "in_mode" => flag(info.in_mode),
                "mode" => Some(if info.in_mode { "copy-mode" } else { "" }.to_string()),
                "synchronized" => flag(m.popt(p.id, "synchronize-panes").as_flag()),
                "start_command" => Some(p.command.display()),
                "start_path" => Some(p.cwd.clone()),
                "left" => rect.map(|r| r.x.to_string()),
                "top" => rect.map(|r| r.y.to_string()),
                "right" => rect.map(|r| (r.right().saturating_sub(1)).to_string()),
                "bottom" => rect.map(|r| (r.bottom().saturating_sub(1)).to_string()),
                "at_left" => flag(rect.is_some_and(|r| r.x == 0)),
                "at_top" => flag(rect.is_some_and(|r| r.y == 0)),
                "at_right" => flag(rect.is_some_and(|r| r.right() == wsize.cols)),
                "at_bottom" => flag(rect.is_some_and(|r| r.bottom() == wsize.rows)),
                "format" => flag(true),
                "input_off" => flag(p.input_off),
                "pipe" => flag(info.pipe),
                "marked" => flag(m.marked() == Some(p.id)),
                "marked_set" => flag(m.marked().is_some()),
                _ => None,
            };
        }
        if let Some(rest) = name.strip_prefix("history_") {
            let p = p?;
            let info = self.host.pane_info(p.id);
            let sid = m.windows.get(&p.window).map(|w| w.session);
            return match rest {
                "size" => Some(info.history_size.to_string()),
                "bytes" => Some(info.history_bytes.to_string()),
                "limit" => Some(m.sopt(sid, "history-limit").as_num().to_string()),
                _ => None,
            };
        }
        if let Some(rest) = name.strip_prefix("client_") {
            let c = c?;
            return match rest {
                "name" | "tty" => Some(c.info.name.clone()),
                "termname" => Some(c.info.term.clone()),
                "width" => Some(c.info.size.cols.to_string()),
                "height" => Some(c.info.size.rows.to_string()),
                "pid" => Some(c.info.pid.to_string()),
                "session" => c.session.and_then(|s| m.sessions.get(&s)).map(|s| s.name.clone()),
                "last_session" => c.last_session.and_then(|s| m.sessions.get(&s)).map(|s| s.name.clone()),
                "prefix" => flag(c.keys.in_prefix()),
                "key_table" => Some(c.keys.table().to_string()),
                "flags" => Some(if c.session.is_some() { "attached,focused,UTF-8" } else { "UTF-8" }.to_string()),
                "cwd" => Some(c.info.cwd.clone()),
                _ => None,
            };
        }
        if let Some(rest) = name.strip_prefix("buffer_") {
            let b = self.buffer?;
            return match rest {
                "name" => Some(b.name.clone()),
                "size" => Some(b.data.len().to_string()),
                "sample" => Some(buffer_sample(&b.data)),
                _ => None,
            };
        }
        match name {
            "host" => return Some(m.hostname.clone()),
            "host_short" => return Some(short_host(&m.hostname).to_string()),
            "version" => return Some(env!("CARGO_PKG_VERSION").to_string()),
            "pid" => return Some(m.server_pid.to_string()),
            "socket_path" => return Some(m.socket_path.clone()),
            _ => {}
        }
        // Like tmux, fall back to options.
        let sid = self.s.or_else(|| w.map(|w| w.session));
        m.option_text(name, sid, self.w)
    }
}

/// Take the next id from `counter`; `None` once only `u32::MAX` is left.
fn next_id(counter: &mut u32) -> Option<u32> {
    let id = *counter;
    if id == u32::MAX {
        return None;
    }
    *counter = id + 1;
    Some(id)
}

/// tmux's buffer sample: the first 50 characters, made printable.
pub(crate) fn buffer_sample(data: &[u8]) -> String {
    let text = String::from_utf8_lossy(data);
    let mut out = String::new();
    for (i, c) in text.chars().enumerate() {
        if i == 50 {
            out.push_str("...");
            break;
        }
        match c {
            '\n' => out.push_str("\\n"),
            '\t' => out.push_str("\\t"),
            '\r' => out.push_str("\\r"),
            '\\' => out.push_str("\\\\"),
            c if c.is_control() => out.push_str(&format!("\\{:03o}", c as u32)),
            c => out.push(c),
        }
    }
    out
}

/// A terminal size from a client, within [`crate::MAX_WINDOW_DIM`] and
/// [`crate::MAX_WINDOW_CELLS`] (rows give way): a window as large as a
/// u16 allows would be billions of cells, and every cell of a pane's
/// screen is allocated.
pub fn clamp_size(s: Size) -> Size {
    let cols = s.cols.min(crate::MAX_WINDOW_DIM);
    let rows = s.rows.min(crate::MAX_WINDOW_DIM);
    let max_rows = (crate::MAX_WINDOW_CELLS / u32::from(cols.max(1))).min(u32::from(u16::MAX)) as u16;
    Size::new(cols, rows.min(max_rows.max(1)))
}

/// Whether a window of `cols` x `rows` is within the limits.
pub(crate) fn size_allowed(cols: u16, rows: u16) -> bool {
    cols <= crate::MAX_WINDOW_DIM && rows <= crate::MAX_WINDOW_DIM && u32::from(cols) * u32::from(rows) <= crate::MAX_WINDOW_CELLS
}
