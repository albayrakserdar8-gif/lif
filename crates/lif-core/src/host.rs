//! The interface between the model and the server that owns the real
//! things: PTYs, child processes, terminal engines and client connections.
//!
//! The model calls [`Host`] methods while it executes a command or handles
//! an event; the server implements them. Calls are synchronous and are made
//! in the order the effects must happen. The model never calls back into
//! itself from a host method, and a host method must not call into the model
//! (it holds `&mut Model` already); queue anything that needs the model and
//! do it after the model call returns.

use crate::ids::{ClientId, PaneId, SessionId, WindowId};
use lif_types::{KeyEvent, Size};

/// What to run in a new pane.
#[derive(Clone, Debug, PartialEq, Eq)]
pub enum SpawnCommand {
    /// The user's shell as a login shell (`argv[0]` prefixed with `-`, like
    /// tmux), because neither the command nor `default-command` said
    /// otherwise.
    LoginShell {
        /// Value of `default-shell` (an absolute path).
        shell: String,
    },
    /// A command string run as `shell -c command` (one argument given, or
    /// `default-command`).
    Shell {
        /// Value of `default-shell`.
        shell: String,
        /// The command string.
        command: String,
    },
    /// Exec these arguments directly, no shell (several arguments given).
    Argv(Vec<String>),
}

impl SpawnCommand {
    /// The command as tmux shows it in `#{pane_start_command}`.
    pub fn display(&self) -> String {
        match self {
            SpawnCommand::LoginShell { .. } => String::new(),
            SpawnCommand::Shell { command, .. } => command.clone(),
            SpawnCommand::Argv(v) => v.join(" "),
        }
    }

    /// The default window name for this command: the basename of the
    /// program (tmux's `default_window_name`).
    pub fn program_name(&self) -> String {
        let first = match self {
            SpawnCommand::LoginShell { shell } => shell.as_str(),
            SpawnCommand::Shell { command, .. } => command.split_whitespace().next().unwrap_or(""),
            SpawnCommand::Argv(v) => v.first().map(|s| s.as_str()).unwrap_or(""),
        };
        let base = first.rsplit('/').next().unwrap_or(first);
        base.trim_start_matches('-').to_string()
    }
}

/// Everything needed to start the process of a pane.
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct SpawnSpec {
    /// The pane being started.
    pub pane: PaneId,
    /// Its window.
    pub window: WindowId,
    /// Its session.
    pub session: SessionId,
    /// What to run.
    pub command: SpawnCommand,
    /// Absolute working directory. Empty means the user's home directory.
    pub cwd: String,
    /// Initial size of the pane's terminal.
    pub size: Size,
    /// `history-limit` of the session, in lines (tmux compatibility).
    pub history_limit: usize,
    /// `scrollback-bytes` of the session: the scrollback budget in bytes.
    pub scrollback_bytes: usize,
    /// The client whose command created the pane, if any; the server may
    /// use its environment (`Hello::env`) for the child.
    pub client: Option<ClientId>,
    /// The pane already exists and this replaces its process
    /// (`respawn-pane`); keep the terminal and its scrollback.
    pub respawn: bool,
    /// The `tmux-compat` option: give the pane `TMUX` and `TMUX_PANE`.
    pub tmux_compat: bool,
    /// Variables set (`Some`) or removed (`None`) on top of the pane's
    /// environment, in order (`set-environment`, then `-e`).
    pub env: Vec<(String, Option<String>)>,
}

/// A shell command the server runs for the model (`run-shell`,
/// `if-shell`, `#()` in formats): `/bin/sh -c command`, standard input
/// from `/dev/null`, standard output captured.
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct JobSpec {
    /// The model's name for the job, handed back to
    /// [`Model::job_done`](crate::Model::job_done).
    pub id: u64,
    /// The command string.
    pub command: String,
    /// Working directory. Empty, or not a directory: the user's home.
    pub cwd: String,
    /// The session the command belongs to, if any: the job gets its
    /// environment (and `LIF` naming it).
    pub session: Option<SessionId>,
    /// The target pane, if any (`LIF_PANE`).
    pub pane: Option<PaneId>,
    /// Capture standard error with standard output (`run-shell -E`);
    /// otherwise it is discarded.
    pub stderr: bool,
    /// Variables set (`Some`) or removed (`None`) on top of the job's
    /// environment, in order (`set-environment`).
    pub env: Vec<(String, Option<String>)>,
    /// The `tmux-compat` option: also set `TMUX` (and `TMUX_PANE`).
    pub tmux_compat: bool,
}

/// How a job ended.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum JobStatus {
    /// It exited with this status.
    Exited(i32),
    /// It was killed by this signal.
    Signaled(i32),
}

/// Input for the program in a pane.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum PaneInput<'a> {
    /// A key, to be encoded by the pane's terminal engine
    /// (`VtEngine::encode_key`) according to the program's keyboard modes.
    Key(&'a KeyEvent),
    /// Text to write to the PTY as UTF-8, unchanged (`send-keys -l`).
    Text(&'a str),
    /// Raw bytes to write to the PTY unchanged (`send-keys -H`).
    Bytes(&'a [u8]),
    /// A paste (`paste-buffer`). Line endings are already converted. When
    /// `bracketed` is set, wrap it in bracketed-paste markers if the program
    /// enabled them (`VtEngine::encode_paste`); otherwise write it as is.
    Paste {
        /// The bytes to paste.
        data: &'a [u8],
        /// `paste-buffer -p`.
        bracketed: bool,
    },
    /// Reset the pane's terminal state (`send-keys -R`): clear the screen and
    /// modes as a hard terminal reset would. Nothing is sent to the program.
    Reset,
}

/// Live facts about a pane that only the server knows. Strings may borrow
/// from the host.
#[derive(Clone, Copy, Debug, Default, PartialEq, Eq)]
pub struct PaneInfo<'a> {
    /// Process id of the pane's child (0 if none).
    pub pid: u32,
    /// Title set by the program (OSC 0/2); empty if none (the model then
    /// uses the host name, like tmux).
    pub title: &'a str,
    /// Name of the foreground process (e.g. `vim`), for automatic window
    /// names and `#{pane_current_command}`. Empty if unknown.
    pub current_command: &'a str,
    /// Working directory of the foreground process (or OSC 7), for
    /// `#{pane_current_path}` and the default directory of new splits.
    /// Empty if unknown.
    pub current_path: &'a str,
    /// Lines of scrollback above the screen.
    pub history_size: usize,
    /// Bytes used by the scrollback.
    pub history_bytes: usize,
    /// The pane shows copy mode (or another mode) instead of its program.
    pub in_mode: bool,
    /// `pipe-pane` is piping the pane's output to a command.
    pub pipe: bool,
}

/// A line position for `capture-pane -S/-E`.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum LinePos {
    /// `-` as the start: the oldest line of the scrollback.
    HistoryStart,
    /// `-` as the end: the last line of the screen.
    ScreenEnd,
    /// A line number: 0 is the first line of the visible screen, negative
    /// numbers count back into the scrollback (-1 is the newest history
    /// line). Values outside the available lines are clamped.
    Line(i64),
}

/// What `capture-pane` wants.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub struct CaptureRange {
    /// First line.
    pub start: LinePos,
    /// Inclusive.
    pub end: LinePos,
    /// Join soft-wrapped lines (`-J`).
    pub join_wrapped: bool,
    /// Keep trailing spaces (`-N` / `-J`).
    pub keep_trailing: bool,
    /// Include escape sequences for colors and attributes (`-e`).
    pub styled: bool,
}

/// A configuration file translated into commands, for `source-file`.
#[derive(Clone, Debug, Default, PartialEq, Eq)]
pub struct ConfigFile {
    /// Commands in the order they run, each with where it came from
    /// (`path:line:column`), which prefixes its error if it fails.
    pub commands: Vec<(String, String)>,
    /// Problems found while reading it, already formatted
    /// (`path:line:column: error: ...`), in file order, each marked as an
    /// error (`true`) or a warning. The commands that could be read still
    /// run.
    pub diagnostics: Vec<(bool, String)>,
}

/// Why a configuration file could not be read at all.
#[derive(Clone, Debug, PartialEq, Eq)]
pub enum ConfigError {
    /// It does not exist (`source-file -q` stays quiet about this).
    Missing(String),
    /// Anything else, with the message to show.
    Invalid(String),
}

/// Why a client is being detached. The server maps these to its exit
/// message (tmux prints `[detached (from session x)]`, `[exited]`,
/// `[server exited]`, ...).
#[derive(Clone, Debug, PartialEq, Eq)]
pub enum DetachReason {
    /// `detach-client` or the prefix-d binding; carries the session name.
    Detached {
        /// Name of the session the client was attached to.
        session: String,
    },
    /// Another client's `attach -d` / `detach -a`; carries the session name.
    Kicked {
        /// Name of the session the client was attached to.
        session: String,
    },
    /// The session the client was attached to no longer exists.
    SessionEnded,
    /// `kill-server` or the last session ended.
    ServerExit,
}

/// Options of `copy-mode`.
#[derive(Clone, Copy, Debug, Default, PartialEq, Eq)]
pub struct CopyModeOptions {
    /// `-u`: start one page up.
    pub page_up: bool,
    /// `-d`: one page down (with `-e`, leaving copy mode at the bottom).
    pub page_down: bool,
    /// `-e`: leave copy mode when scrolling reaches the bottom.
    pub exit_at_bottom: bool,
    /// `-H`: hide the position indicator.
    pub hide_position: bool,
    /// `-M`: begin a mouse drag (only from a mouse binding).
    pub mouse_drag: bool,
    /// `-q`: cancel copy mode (and other modes) instead of entering it.
    pub cancel: bool,
}

/// A prompt shown on a client's status line.
#[derive(Clone, Debug, PartialEq, Eq)]
pub enum Prompt {
    /// `command-prompt`. The user answers each of `prompts` in turn (each
    /// starting with the matching `inputs` text). When all are answered the
    /// server builds the command with
    /// [`expand_prompt_template`](crate::expand_prompt_template), parses it
    /// with [`parse_string`](crate::parse_string) and executes it with the
    /// client as context. Escape cancels without running anything.
    Command {
        /// One prompt text per answer (`-p`, or `:` / `(command) `).
        prompts: Vec<String>,
        /// Initial text per answer (`-I`), already format-expanded.
        inputs: Vec<String>,
        /// The command template the answers are substituted into.
        template: String,
        /// `-1`: accept a single key press as the whole answer.
        single_key: bool,
        /// `-k`: like `-1` but the answer is the key's name (`C-a`).
        key_name: bool,
        /// `-N`: accept only digits.
        numeric: bool,
    },
    /// `confirm-before`: show `prompt`; on `y` (or Enter when
    /// `default_yes`) parse and run `command` with the client as context;
    /// anything else cancels.
    Confirm {
        /// The question, already expanded.
        prompt: String,
        /// Command string to run on yes.
        command: String,
        /// `-y`: Enter means yes.
        default_yes: bool,
    },
}

/// What needs redrawing after a model change.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum Redraw {
    /// Every client attached to this session (the window shown, layout or
    /// current window changed).
    Session(SessionId),
    /// Clients currently showing this window (panes, borders, zoom).
    Window(WindowId),
    /// Only the status line of clients attached to this session.
    Status(SessionId),
    /// One client, everything.
    Client(ClientId),
}

/// The server side of the model. See the module docs for the call rules.
pub trait Host {
    /// Start the process for a new pane (or a new process for an existing
    /// pane when `spec.respawn` is set) with a PTY of `spec.size`, and create
    /// its terminal engine. On error return a short message (e.g.
    /// `"fork failed: Resource temporarily unavailable"`); the model then
    /// undoes everything it did for the pane and the host must not keep any
    /// state for `spec.pane`.
    fn spawn_pane(&mut self, spec: &SpawnSpec) -> Result<(), String>;

    /// The model removed this pane (or is about to respawn it): hang up its
    /// process group, close the PTY and drop its terminal, unless
    /// `spec.respawn` follows. Never report [`Model::pane_exited`](crate::Model::pane_exited)
    /// for a pane after this call.
    fn kill_pane(&mut self, pane: PaneId);

    /// The pane's size changed: resize the terminal engine and the PTY
    /// (TIOCSWINSZ). Only called when the size actually differs.
    fn resize_pane(&mut self, pane: PaneId, size: Size);

    /// `suspend-client`: restore the client's terminal, stop using it and
    /// have the client stop itself (as with job control's SIGTSTP); it
    /// carries on when it is continued. An error if the client cannot be
    /// suspended (an older client).
    fn suspend_client(&mut self, client: ClientId) -> Result<(), String> {
        let _ = client;
        Err("suspend-client: not supported".into())
    }

    /// Start a shell command. When it has exited and its output is read to
    /// the end, the server calls [`Model::job_done`](crate::Model::job_done)
    /// with `job.id`, the status and the output (the server bounds how much
    /// it keeps), from the event loop, never from inside this call. On
    /// error nothing was started and `job_done` is not called.
    fn start_job(&mut self, job: &JobSpec) -> Result<(), String> {
        Err(format!("{}: shell commands are not supported here", job.command))
    }

    /// Call [`Model::job_done`](crate::Model::job_done) with `id` (status
    /// 0, no output) in `delay_ms` milliseconds (`run-shell -d`).
    fn start_timer(&mut self, id: u64, delay_ms: u64) -> Result<(), String> {
        let _ = (id, delay_ms);
        Err("timers are not supported here".into())
    }

    /// `choose-tree`, `choose-buffer`, `choose-client`, `find-window`:
    /// put `pane` into the chooser mode `spec` describes, for `client`
    /// (whose commands the choices run as). The host builds the list with
    /// [`Model::choose_items`](crate::Model::choose_items) after this call
    /// returns, and calls
    /// [`Model::chooser_closed`](crate::Model::chooser_closed) when the
    /// mode ends.
    fn choose(&mut self, client: ClientId, pane: PaneId, spec: crate::choose::ChooseSpec) -> Result<(), String> {
        let _ = (client, pane, spec);
        Err("chooser modes are not supported here".into())
    }

    /// `display-popup`: open a popup on `client`'s screen and run its
    /// program. When it closes, report it with
    /// [`Model::job_done`](crate::Model::job_done) (`spec.id`).
    fn popup(&mut self, client: ClientId, spec: crate::overlay::PopupSpec) -> Result<(), String> {
        let _ = (client, spec);
        Err("popups are not supported here".into())
    }

    /// `display-menu`: show a menu on `client`'s screen. Its keys and
    /// clicks go to [`MenuState`](crate::MenuState); the command it gives
    /// runs for the client, with `menu.pane` as the current pane.
    fn menu(&mut self, client: ClientId, menu: crate::overlay::MenuState) -> Result<(), String> {
        let _ = (client, menu);
        Err("menus are not supported here".into())
    }

    /// Whether `client` has a popup or menu up.
    fn has_overlay(&self, client: ClientId) -> bool {
        let _ = client;
        false
    }

    /// `display-popup -C`: close `client`'s popup or menu, if any.
    fn close_overlay(&mut self, client: ClientId) {
        let _ = client;
    }

    /// `clock-mode`: show a large clock in the pane until a key is
    /// pressed there.
    fn clock_mode(&mut self, pane: PaneId) {
        let _ = pane;
    }

    /// `pipe-pane`: from now on, also write what the program in `pane`
    /// prints to the standard input of `command` (`/bin/sh -c`, started
    /// in `cwd`); `None` stops a pipe. A pane has at most one: a new
    /// command replaces the old.
    fn pipe_pane(&mut self, pane: PaneId, command: Option<&str>, cwd: &str) -> Result<(), String> {
        let _ = (pane, cwd);
        match command {
            Some(c) => Err(format!("{c}: shell commands are not supported here")),
            None => Ok(()),
        }
    }

    /// `show-prompt-history` / `clear-prompt-history`: what was typed at
    /// the command prompt, oldest first; `clear` forgets it.
    fn prompt_history(&mut self, clear: bool) -> Vec<String> {
        let _ = clear;
        Vec::new()
    }

    /// `clear-history`: drop the pane's scrollback (and leave any mode it
    /// is in, which shows that history).
    fn clear_history(&mut self, pane: PaneId) {
        let _ = pane;
    }

    /// Deliver input to the program in a pane. An error means the input
    /// was not queued (the pane's input queue is full: the program is not
    /// reading); the command reports it.
    fn send_to_pane(&mut self, pane: PaneId, input: PaneInput<'_>) -> Result<(), String>;

    /// Current facts about a pane. Must be cheap: formats call it often. For
    /// an unknown pane return `PaneInfo::default()`.
    fn pane_info(&self, pane: PaneId) -> PaneInfo<'_>;

    /// Plain text of part of a pane's screen and scrollback for
    /// `capture-pane`: one line per row, each ending in `\n`, trailing spaces
    /// trimmed unless `range.keep_trailing`.
    fn capture_pane(&self, pane: PaneId, range: CaptureRange) -> String;

    /// Show `session` on `client`. If the client is not attached yet, put it
    /// into attached mode (send `Attached`, enter raw mode on its side) and
    /// start rendering; if it is attached to another session, switch it. The
    /// model has already recorded the attachment and resized the session's
    /// windows.
    fn attach_client(&mut self, client: ClientId, session: SessionId);

    /// End `client`'s attachment: restore its terminal and send `Exit` with
    /// a message for `reason`. The model has already forgotten the
    /// attachment.
    fn detach_client(&mut self, client: ClientId, reason: DetachReason);

    /// `detach-client -P`, `attach-session -x`: send SIGHUP to the parent
    /// process of `client` (usually its shell, which then exits), after
    /// [`Host::detach_client`].
    fn hangup_client_parent(&mut self, client: ClientId);

    /// Put `pane` into copy mode (or, with `opts.cancel`, take it out of any
    /// mode). `client` is the client that asked, if any. Copy mode itself is
    /// the server's.
    fn enter_copy_mode(&mut self, client: Option<ClientId>, pane: PaneId, opts: CopyModeOptions);

    /// `send-keys -X`: run the copy-mode command named `command` (tmux's
    /// name) with `args`, `count` times, in `pane`'s mode. `client` is the
    /// client that asked, if any. The error is the message to report; tmux's
    /// for a pane in no mode is `not in a mode`.
    fn mode_command(
        &mut self,
        client: Option<ClientId>,
        pane: PaneId,
        command: &str,
        args: &[String],
        count: u32,
    ) -> Result<(), String> {
        let _ = (client, pane, command, args, count);
        Err("not in a mode".to_string())
    }

    /// `source-file`: read and translate a configuration file. The model
    /// runs the commands and reports what failed.
    fn read_config(&mut self, path: &str) -> Result<ConfigFile, ConfigError> {
        Err(ConfigError::Invalid(format!("{path}: configuration files are not supported here")))
    }

    /// The standard input the client sent with the command being run
    /// (`load-buffer -`), taken once; `None` if it sent none.
    fn command_stdin(&mut self) -> Option<Vec<u8>> {
        None
    }

    /// `load-buffer`: the contents of `path`, relative to the directory of
    /// `client` (or the server's) when not absolute.
    fn read_file(&mut self, client: Option<ClientId>, path: &str) -> Result<Vec<u8>, String> {
        let _ = client;
        Err(format!("{path}: files are not supported here"))
    }

    /// `save-buffer`: write (or append) `data` to `path`, resolved as in
    /// [`Host::read_file`].
    fn write_file(&mut self, client: Option<ClientId>, path: &str, data: &[u8], append: bool) -> Result<(), String> {
        let _ = (client, data, append);
        Err(format!("{path}: files are not supported here"))
    }

    /// Set the clipboard of every attached terminal to `data` (OSC 52), as
    /// copying does (`load-buffer -w`).
    fn set_clipboard(&mut self, data: &[u8]) {
        let _ = data;
    }

    /// Show `text` on `client`'s status line for `duration_ms` (0: until the
    /// next key press).
    fn show_message(&mut self, client: ClientId, text: &str, duration_ms: u64);

    /// Show an interactive prompt on `client`'s status line. See [`Prompt`].
    fn prompt(&mut self, client: ClientId, prompt: Prompt);

    /// Show pane numbers over `client`'s panes for `duration_ms`; when the
    /// user presses a digit, run `template` with `%%` replaced by the pane id
    /// (default `select-pane -t %%`). Without `keys` (`-N`) keys are not
    /// looked at: the numbers stay up until the time is over.
    fn display_panes(&mut self, client: ClientId, duration_ms: u64, template: &str, keys: bool);

    /// Ring the bell on `client`'s terminal (write BEL).
    fn bell(&mut self, client: ClientId);

    /// Schedule a redraw. The host coalesces these; nothing needs to happen
    /// before the model call returns.
    fn redraw(&mut self, what: Redraw);

    /// The server should exit: the model has already detached every client
    /// and killed every pane.
    fn shutdown(&mut self);
}
