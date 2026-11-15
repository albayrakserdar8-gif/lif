//! The runtime state the model does not own, and lif-core's `Host` trait
//! on it.

use std::collections::{BTreeMap, HashMap};
use std::ffi::OsString;
use std::path::PathBuf;
use std::time::Instant;

use lif_core::{
    CaptureRange, ClientId, ConfigError, ConfigFile, CopyModeOptions, DetachReason, Host, LinePos, PaneId, PaneInfo,
    PaneInput, Prompt, Redraw, SessionId, SpawnCommand, SpawnSpec,
};
use lif_input::{CapsProbe, InputParser};
use lif_proto::{ExitReason, ServerMsg};
use lif_pty::{ChildReaper, Pty, SpawnOptions};
use lif_render::Renderer;
use lif_types::{PaneFrame, PixelSize, Pos, Size, Span, StyledLine};
use lif_vt::{Rgb, TextOptions, TextRange, VtEngine, VtOptions};
use mio::unix::SourceFd;
use mio::{Interest, Registry, Token};

use crate::client::{Attached, ClientRt, DisplayPanes, Message, TermOut, leave_bytes};
use crate::mode::{Engine, ModeAction, ModeCtx, new_copy_mode};
use crate::pane::PaneRt;
use crate::prompt::PromptState;

/// (key bindings generation, vi keys, word-separators, wrap-search,
/// mouse-drag-end): the options copy mode's settings come from.
pub(crate) type CopyKey = (u64, bool, String, bool, lif_copy::DragEnd);

/// Copy mode's settings for one combination of the options it reads.
pub(crate) struct CopyConfig {
    pub(crate) key: CopyKey,
    pub(crate) cfg: lif_copy::Config,
}

pub(crate) const PANE_TOKEN: usize = 1 << 32;
pub(crate) const CLIENT_TOKEN: usize = 2 << 32;
/// A client's terminal, opened by the server (`TermOut::Direct`).
pub(crate) const TTY_TOKEN: usize = 4 << 32;
/// The stdin of a `copy-pipe` command (`crate::pipe`).
pub(crate) const PIPE_TOKEN: usize = 8 << 32;

/// `TERM` inside panes. xterm-256color has a terminfo entry on every
/// system (unlike a `lif` entry would), and libghostty-vt implements the
/// xterm control set, so what terminfo promises is what the pane does.
pub(crate) const PANE_TERM: &str = "xterm-256color";

/// Variables a pane never inherits: they would make programs think they
/// run directly under another multiplexer.
const ENV_REMOVE: &[&str] = &["TMUX", "TMUX_PANE"];

pub(crate) struct Rt {
    pub(crate) registry: Registry,
    pub(crate) panes: HashMap<PaneId, PaneRt>,
    pub(crate) pids: HashMap<u32, PaneId>,
    /// Process groups that were hung up and have not been reaped, with
    /// when each gets SIGKILL (`PANE_KILL_GRACE`). At most one entry per
    /// killed pane whose process is still alive.
    pub(crate) dying: Vec<(u32, std::time::Instant)>,
    pub(crate) clients: BTreeMap<ClientId, ClientRt>,
    pub(crate) reaper: ChildReaper,
    pub(crate) socket: String,
    pub(crate) server_pid: u32,
    /// The outer terminal's default colors, as the latest probe found them.
    pub(crate) default_colors: (Option<Rgb>, Option<Rgb>),
    /// Environment for new panes, refreshed from the client on every attach.
    pub(crate) session_env: HashMap<SessionId, Vec<(String, String)>>,
    /// The terminal of a pane killed for `respawn-pane`, kept until the
    /// model call that killed it returns.
    pub(crate) respawn_stash: Option<(PaneId, Engine, PaneFrame)>,
    /// Client views must be recomputed from the model.
    pub(crate) views_stale: bool,
    pub(crate) shutdown: bool,
    pub(crate) prompt_history: Vec<String>,
    pub(crate) scratch: Vec<u8>,
    pub(crate) server_env_zellij: Vec<OsString>,
    /// Settings new copy modes get (the server keeps them current).
    pub(crate) copy_cfg: CopyConfig,
    /// What modes asked for during a model call, carried out after it
    /// returns (a copy needs the model's paste buffers): (client, pane, what).
    pub(crate) mode_actions: Vec<(Option<ClientId>, PaneId, ModeAction)>,
    /// Panes that entered or left a mode during a model call.
    pub(crate) mode_changed: Vec<PaneId>,
    /// Configuration problems found at start-up, for the first client.
    pub(crate) startup_message: Option<String>,
    /// The `escape-time` option (kept current by `Server::apply_options`).
    pub(crate) escape_time: std::time::Duration,
    /// Keys `send-keys` sent to a pane in a mode whose binding is commands
    /// (`ModeAction::Binding`): run after the model call, which they need.
    pub(crate) mode_bindings: Vec<(PaneId, lif_types::KeyEvent)>,
    /// Standard input the client sent with the command being run.
    pub(crate) cmd_stdin: Option<Vec<u8>>,
    /// `copy-pipe` commands still being fed, by id (`PIPE_TOKEN | id`).
    pub(crate) pipes: HashMap<u32, crate::pipe::PipeJob>,
    pub(crate) next_pipe: u32,
    /// The RLIMIT_NOFILE soft limit (0: unknown).
    pub(crate) fd_limit: usize,
}

pub(crate) fn pane_token(p: PaneId) -> Token {
    Token(PANE_TOKEN | p.0 as usize)
}

impl Rt {
    pub(crate) fn new(registry: Registry, reaper: ChildReaper, socket: String) -> Self {
        let server_env_zellij = std::env::vars_os().map(|(k, _)| k).filter(|k| k.to_string_lossy().starts_with("ZELLIJ")).collect();
        Rt {
            registry,
            panes: HashMap::new(),
            pids: HashMap::new(),
            dying: Vec::new(),
            clients: BTreeMap::new(),
            reaper,
            socket,
            server_pid: std::process::id(),
            default_colors: (None, None),
            session_env: HashMap::new(),
            respawn_stash: None,
            views_stale: true,
            shutdown: false,
            prompt_history: Vec::new(),
            scratch: Vec::new(),
            server_env_zellij,
            // Built from the model on first use (the key never matches).
            copy_cfg: CopyConfig {
                key: (u64::MAX, false, String::new(), true, lif_copy::DragEnd::default()),
                cfg: lif_copy::Config {
                    mode_keys: lif_copy::ModeKeys::Emacs,
                    table: std::rc::Rc::new(lif_copy::KeyTable::new()),
                    word_separators: String::new(),
                    wrap_search: true,
                    mouse: lif_copy::MouseConfig::default(),
                    search_step_rows: lif_copy::DEFAULT_SEARCH_STEP_ROWS,
                },
            },
            mode_actions: Vec::new(),
            mode_changed: Vec::new(),
            startup_message: None,
            escape_time: lif_input::DEFAULT_ESC_TIMEOUT,
            mode_bindings: Vec::new(),
            cmd_stdin: None,
            pipes: HashMap::new(),
            next_pipe: 0,
            fd_limit: 0,
        }
    }

    /// Hangs up the process group of a watched child and gives it
    /// `PANE_KILL_GRACE` to exit. The caller closes the PTY.
    pub(crate) fn hang_up(&mut self, pid: u32) {
        crate::sys::kill_group(pid, libc::SIGHUP);
        self.dying.push((pid, std::time::Instant::now() + crate::PANE_KILL_GRACE));
    }

    /// A watched child was reaped: its pid may be reused from now on.
    pub(crate) fn child_reaped(&mut self, pid: u32) {
        self.dying.retain(|d| d.0 != pid);
        crate::panic::pane_reaped(pid);
    }

    /// SIGKILL for hung-up process groups whose grace is over at `now`.
    /// Their pids are still ours: an unreaped child's pid is not reused.
    pub(crate) fn kill_overdue(&mut self, now: std::time::Instant) {
        self.dying.retain(|&(pid, at)| {
            if now < at {
                return true;
            }
            log!("pid {pid} outlived its hangup: SIGKILL");
            crate::sys::kill_group(pid, libc::SIGKILL);
            false
        });
    }

    /// `copy-pipe`: run `command` with `data` on its stdin, in `cwd`.
    pub(crate) fn copy_pipe(&mut self, command: &str, cwd: &str, data: Vec<u8>) -> Result<(), String> {
        if self.pipes.len() >= crate::pipe::MAX_PIPES {
            return Err(format!("copy-pipe: {} commands are still reading; not running {command}", self.pipes.len()));
        }
        let cwd = (!cwd.is_empty()).then(|| std::path::Path::new(cwd));
        let (pid, mut job) = crate::pipe::spawn(command, cwd, data).map_err(|e| format!("copy-pipe: {command}: {e}"))?;
        self.reaper.watch(pid);
        log!("copy-pipe: pid {pid}: {command}");
        // Otherwise all written (dropping the job closes the pipe), or the
        // command exited without reading all of it.
        if let Ok(false) = job.write() {
            self.next_pipe = self.next_pipe.wrapping_add(1);
            let id = self.next_pipe;
            if let Err(e) = self.registry.register(&mut job.tx, Token(PIPE_TOKEN | id as usize), Interest::WRITABLE) {
                log!("copy-pipe: pid {pid}: {e}");
                return Ok(());
            }
            self.pipes.insert(id, job);
        }
        Ok(())
    }

    /// A `copy-pipe` command can take more of its input.
    pub(crate) fn pipe_writable(&mut self, id: u32) {
        let Some(job) = self.pipes.get_mut(&id) else { return };
        if let Ok(false) = job.write() {
            return;
        }
        if let Some(mut job) = self.pipes.remove(&id) {
            let _ = self.registry.deregister(&mut job.tx);
        }
    }

    /// Queue bytes for a pane's program and try to write them now. Every
    /// byte for a pane goes through here (keys, pastes, `send-keys`,
    /// terminal replies, reports), so this is where the queue's one hard
    /// bound, [`crate::PANE_INPUT_MAX`], holds: bytes that would take the
    /// queue past it are not queued at all, and false says so; the caller
    /// decides what that means (replies and reports are dropped, as a real
    /// terminal drops them; commands fail; typed input is dropped with a
    /// message). False also when the pane has no program to write to.
    pub(crate) fn write_pane(&mut self, pane: PaneId, bytes: &[u8]) -> bool {
        self.queue_pane(pane, bytes, crate::PANE_INPUT_MAX - crate::PANE_INPUT_RESERVE)
    }

    /// [`Rt::write_pane`] for the few bytes that end a state the program
    /// was put in (the end of a bracketed paste): they may use the reserve
    /// kept below the hard bound, so a paste that was let in is closed.
    pub(crate) fn write_pane_closing(&mut self, pane: PaneId, bytes: &[u8]) -> bool {
        self.queue_pane(pane, bytes, crate::PANE_INPUT_MAX)
    }

    fn queue_pane(&mut self, pane: PaneId, bytes: &[u8], limit: usize) -> bool {
        if bytes.is_empty() {
            return true;
        }
        let Some(p) = self.panes.get(&pane) else { return false };
        if p.pty.is_none() {
            return false;
        }
        if p.wq.len() + bytes.len() > limit {
            // The program may have read since the last write.
            self.flush_pane(pane);
            let Some(p) = self.panes.get(&pane) else { return false };
            if p.wq.len() + bytes.len() > limit {
                return false;
            }
        }
        let Some(p) = self.panes.get_mut(&pane) else { return false };
        p.wq.push(bytes);
        self.flush_pane(pane);
        true
    }

    /// [`Rt::write_pane`] for input a client typed (keys, mouse, paste):
    /// the pane's next output is its echo (see `crate::ECHO_WINDOW`).
    pub(crate) fn write_input(&mut self, pane: PaneId, bytes: &[u8]) -> bool {
        if bytes.is_empty() {
            return true;
        }
        if let Some(p) = self.panes.get_mut(&pane) {
            p.note_input(Instant::now());
        }
        self.write_pane(pane, bytes)
    }

    /// Stop using `client`'s terminal for now and say so with `Suspended`
    /// (`ClientMsg::Suspend`, `suspend-client`): its modes are restored
    /// (the bytes for that go out before the message), no frame, other
    /// output or read touches it until `Rt::resume`.
    pub(crate) fn suspend(&mut self, id: ClientId) -> Result<(), String> {
        let Some(c) = self.clients.get_mut(&id) else { return Err("no such client".into()) };
        if !c.caps.contains(lif_proto::Caps::SUSPEND) {
            return Err("suspend-client: this client cannot be suspended (it is an older lif)".into());
        }
        let Some(a) = c.att.as_mut() else { return Err("client is not attached".into()) };
        if a.suspended.is_none() {
            let mut b = Vec::new();
            leave_bytes(a, &mut b);
            a.mouse = (false, false);
            c.send_output(&b);
            if let Some(a) = c.att.as_mut() {
                a.suspended = Some(crate::client::Suspension::Stopped);
            }
            if let Some(t) = c.tty_mut() {
                t.reading = false;
                t.read_more = false;
            }
            log!("client {id}: suspended");
        }
        c.send(&ServerMsg::Suspended);
        self.flush_client(id);
        self.end_paste(id);
        Ok(())
    }

    /// The client is running again in the foreground (`ClientMsg::Resume`):
    /// take the terminal back, set its modes, draw it all. Whatever it
    /// shows now (a shell, its prompt) is not ours. A client that is not
    /// in the foreground of the terminal the server reads stays suspended.
    pub(crate) fn resume(&mut self, id: ClientId) {
        let Some(c) = self.clients.get_mut(&id) else { return };
        if c.att.is_none() {
            return;
        }
        if let Some(t) = c.tty_mut() {
            if t.dead {
                return;
            }
            t.background = false;
            if t.usable() {
                t.reading = true;
                t.read_more = true;
            } else {
                log!("client {id}: resumed but not in the foreground");
                if let Some(a) = c.att.as_mut() {
                    a.suspended.get_or_insert(crate::client::Suspension::Background);
                }
                self.flush_client(id);
                return;
            }
        }
        let Some(a) = c.att.as_mut() else { return };
        let mut b = Vec::new();
        lif_input::write_enter(&a.enter_caps, &mut b);
        if a.kitty_pushed {
            lif_input::write_kitty_upgrade(&mut b);
        }
        a.suspended = None;
        // draw_client sends the mouse mode again.
        a.mouse = (false, false);
        a.renderer.invalidate();
        a.needs_frame = true;
        a.status_stale = true;
        a.last_frame = None;
        c.send_output(&b);
        c.arm_panic();
        log!("client {id}: resumed");
        self.flush_client(id);
    }

    /// A paste `client` was streaming into a pane is over before its end
    /// arrived (the client left, detached or the pane changed): end it for
    /// the program, which would otherwise take everything typed later as
    /// pasted text.
    pub(crate) fn end_paste(&mut self, client: ClientId) {
        let Some(a) = self.clients.get_mut(&client).and_then(|c| c.att.as_mut()) else { return };
        a.paste_dropping = false;
        let Some(pane) = a.paste.take() else { return };
        if let Some(p) = self.panes.get_mut(&pane)
            && p.paste_bracketed.take() == Some(true)
        {
            self.write_pane_closing(pane, b"\x1b[201~");
        }
    }

    /// Write a pane's queue and keep WRITABLE interest in step with it.
    pub(crate) fn flush_pane(&mut self, pane: PaneId) {
        let Some(p) = self.panes.get_mut(&pane) else { return };
        let Some(pty) = p.pty.as_mut() else { return };
        let want = match p.wq.flush(pty) {
            Ok(done) => !done,
            Err(_) => {
                // The program is gone; its exit is reported by the reaper.
                p.wq = Default::default();
                false
            }
        };
        if want != p.want_write {
            p.want_write = want;
            let interest = if want { Interest::READABLE | Interest::WRITABLE } else { Interest::READABLE };
            let _ = self.registry.reregister(&mut SourceFd(&p.raw_fd()), pane_token(pane), interest);
        }
    }

    /// Flush a client's output and keep WRITABLE interest in step with it.
    pub(crate) fn flush_client(&mut self, id: ClientId) {
        let Some(c) = self.clients.get_mut(&id) else { return };
        let result = c.flush();
        let mid = c.partly_sent();
        if mid != c.mid_frame {
            c.mid_frame = mid;
            crate::panic::set_mid_frame(std::os::fd::AsRawFd::as_raw_fd(&c.stream), mid);
        }
        match result {
            Ok((sock, tty)) => {
                // The socket waits behind the terminal: only the terminal's
                // writability matters until it drained.
                let want = sock && !tty;
                if want != c.want_write {
                    c.want_write = want;
                    let interest = if want { Interest::READABLE | Interest::WRITABLE } else { Interest::READABLE };
                    let _ = self.registry.reregister(&mut c.stream, c.token, interest);
                }
                // The terminal went to the background during the flush:
                // the client is stopped, and the terminal is its shell's.
                let background = c.tty().is_some_and(|t| t.background);
                if background && let Some(a) = c.att.as_mut() && a.suspended.is_none() {
                    log!("client {id}: not in the foreground of its terminal; leaving it alone");
                    a.suspended = Some(crate::client::Suspension::Background);
                }
                if let Some(t) = c.tty_mut() {
                    tty_interest(&self.registry, t);
                }
                if !sock && !tty && c.closing {
                    c.dead = true;
                }
            }
            Err(e) => {
                log!("client {id}: write failed: {e}");
                c.dead = true;
            }
        }
    }

    pub(crate) fn send_output(&mut self, id: ClientId, bytes: &[u8]) {
        if let Some(c) = self.clients.get_mut(&id) {
            c.send_output(bytes);
        }
        self.flush_client(id);
    }

    pub(crate) fn needs_frame(&mut self, id: ClientId) {
        if let Some(a) = self.clients.get_mut(&id).and_then(|c| c.att.as_mut()) {
            a.needs_frame = true;
        }
    }

    pub(crate) fn mark_all(&mut self, status: bool) {
        for c in self.clients.values_mut() {
            if let Some(a) = c.att.as_mut() {
                a.needs_frame = true;
                a.status_stale |= status;
            }
        }
    }

    /// `path` relative to the directory of the client that ran the command
    /// (as tmux resolves `load-buffer` and `save-buffer` paths).
    fn client_path(&self, client: Option<ClientId>, path: &str) -> PathBuf {
        let p = std::path::Path::new(path);
        if p.is_absolute() {
            return p.to_path_buf();
        }
        let cwd = client.and_then(|c| self.clients.get(&c)).and_then(|c| c.hello.as_ref()).map(|h| h.cwd.as_str()).unwrap_or("");
        if cwd.is_empty() { p.to_path_buf() } else { std::path::Path::new(cwd).join(p) }
    }

    fn pane_env(&self, spec: &SpawnSpec) -> Vec<(OsString, OsString)> {
        let mut env: Vec<(OsString, OsString)> = Vec::new();
        let from_client = spec.client.and_then(|c| self.clients.get(&c)).and_then(|c| c.hello.as_ref()).map(|h| &h.env);
        let session = self.session_env.get(&spec.session);
        let base: &[(String, String)] = match (session, from_client) {
            (Some(s), _) => s,
            (None, Some(c)) => c,
            (None, None) => &[],
        };
        for (k, v) in base {
            if k.is_empty() || k.contains('=') || k == "TERM" || k.starts_with("LIF") || k.starts_with("TMUX") || k.starts_with("ZELLIJ") {
                continue;
            }
            env.push((k.into(), v.into()));
        }
        let mut set = |k: &str, v: String| {
            env.retain(|(x, _)| x != k);
            env.push((k.into(), v.into()));
        };
        set("COLORTERM", "truecolor".into());
        set("TERM_PROGRAM", "lif".into());
        set("TERM_PROGRAM_VERSION", env!("CARGO_PKG_VERSION").into());
        set("LIF", format!("{},{},{}", self.socket, self.server_pid, spec.session.0));
        set("LIF_PANE", spec.pane.to_string());
        if spec.tmux_compat {
            // What tmux sets, pointing at lif's socket: programs that only
            // know tmux run `tmux` (lif, see `install-tmux-shim`) with it.
            set("TMUX", format!("{},{},{}", self.socket, self.server_pid, spec.session.0));
            set("TMUX_PANE", spec.pane.to_string());
        }
        if let SpawnCommand::LoginShell { shell } = &spec.command {
            set("SHELL", shell.clone());
        }
        env
    }

    fn attach_new(&mut self, client: ClientId, session: SessionId) {
        let Some(c) = self.clients.get_mut(&client) else { return };
        let hello = c.hello.clone().unwrap_or_default();
        let var = |k: &str| hello.env.iter().find(|(n, _)| n == k).map(|(_, v)| v.as_str()).unwrap_or("");
        let probe = CapsProbe::from_env(&hello.term, var("COLORTERM"), var("TERM_PROGRAM"));
        let caps = probe.caps();
        let mut att = Box::new(Attached {
            session,
            parser: InputParser::new(),
            probe,
            renderer: Renderer::new(caps),
            enter_caps: caps,
            kitty_pushed: false,
            events: Vec::new(),
            input_deadline: None,
            key_deadline: None,
            needs_frame: true,
            last_frame: None,
            frame_deadline: None,
            view: None,
            status: StyledLine::new(),
            status_stale: true,
            clock: false,
            clock_deadline: None,
            message: None,
            prompt: None,
            prompt_line: StyledLine::new(),
            display_panes: None,
            mouse: (false, false),
            mouse_opt: false,
            drag: None,
            focused: true,
            active_pane: None,
            paste: None,
            paste_dropping: false,
            suspended: None,
            clip_reads: Default::default(),
            theme: lif_render::Theme::default(),
            msg_style: crate::prompt::message_style(),
            mode_drag: None,
            mouse_press: None,
            clicks: None,
            labels: Vec::new(),
            labels_stale: true,
            title: None,
        });
        if let Some(text) = self.startup_message.take() {
            att.message = Some(Message { line: vec![Span::new(text, att.msg_style)], expires: None });
        }
        c.send(&ServerMsg::Attached);
        if c.caps.contains(lif_proto::Caps::TTY) {
            // Nothing reaches the terminal before the client put it in raw
            // mode and said which one it is (UseTty).
            c.term_out = TermOut::Hold(Vec::new());
        }
        let mut b = Vec::new();
        lif_input::write_queries(&mut b);
        lif_input::write_enter(&caps, &mut b);
        c.send_output(&b);
        att.needs_frame = true;
        c.att = Some(att);
        c.arm_panic();
        self.session_env.insert(session, hello.env.clone());
        self.flush_client(client);
    }
}

/// Registers a client terminal's fd for what it is wanted for now (see
/// `Tty::wanted_interest`), deregistering it when for nothing: a
/// terminal that belongs to a shell for now must not wake the loop.
pub(crate) fn tty_interest(registry: &Registry, t: &mut crate::client::Tty) {
    let want = t.wanted_interest();
    if want == t.interest {
        return;
    }
    let fd = std::os::fd::AsRawFd::as_raw_fd(&t.fd);
    let r = match (t.interest, want) {
        (None, Some(i)) => registry.register(&mut SourceFd(&fd), t.token, i),
        (Some(_), Some(i)) => registry.reregister(&mut SourceFd(&fd), t.token, i),
        (Some(_), None) => registry.deregister(&mut SourceFd(&fd)),
        (None, None) => Ok(()),
    };
    match r {
        Ok(()) => t.interest = want,
        Err(e) => log!("terminal registration: {e}"),
    }
}

impl PaneRt {
    pub(crate) fn raw_fd(&self) -> std::os::fd::RawFd {
        self.pty.as_ref().map(std::os::fd::AsRawFd::as_raw_fd).unwrap_or(-1)
    }
}

fn exit_message(reason: &DetachReason) -> (ExitReason, String) {
    match reason {
        DetachReason::Detached { session } => (ExitReason::Detached, format!("[detached (from session {session})]")),
        DetachReason::Kicked { session } => (ExitReason::Kicked, format!("[detached (from session {session})]")),
        DetachReason::SessionEnded => (ExitReason::SessionEnded, "[exited]".into()),
        DetachReason::ServerExit => (ExitReason::ServerExit, "[server exited]".into()),
    }
}

impl Host for Rt {
    fn suspend_client(&mut self, client: ClientId) -> Result<(), String> {
        self.suspend(client)
    }

    fn command_stdin(&mut self) -> Option<Vec<u8>> {
        self.cmd_stdin.take()
    }

    fn read_file(&mut self, client: Option<ClientId>, path: &str) -> Result<Vec<u8>, String> {
        use std::io::Read;
        let full = self.client_path(client, path);
        let f = std::fs::File::open(&full).map_err(|e| format!("{path}: {}", io_text(&e)))?;
        let mut data = Vec::new();
        // A buffer bigger than a protocol frame could not be shown or saved.
        f.take(lif_proto::MAX_PAYLOAD as u64 + 1).read_to_end(&mut data).map_err(|e| format!("{path}: {}", io_text(&e)))?;
        if data.len() > lif_proto::MAX_PAYLOAD {
            return Err(format!("{path}: file too large"));
        }
        Ok(data)
    }

    fn write_file(&mut self, client: Option<ClientId>, path: &str, data: &[u8], append: bool) -> Result<(), String> {
        use std::io::Write;
        let full = self.client_path(client, path);
        let mut o = std::fs::OpenOptions::new();
        o.create(true);
        if append {
            o.append(true);
        } else {
            o.write(true).truncate(true);
        }
        o.open(&full).and_then(|mut f| f.write_all(data)).map_err(|e| format!("{path}: {}", io_text(&e)))
    }

    fn set_clipboard(&mut self, data: &[u8]) {
        let mut seq = String::from("\x1b]52;c;");
        crate::input::base64(data, &mut seq);
        seq.push_str("\x1b\\");
        let attached: Vec<ClientId> = self.clients.values().filter(|c| c.att.is_some()).map(|c| c.id).collect();
        for c in attached {
            self.send_output(c, seq.as_bytes());
        }
    }

    fn spawn_pane(&mut self, spec: &SpawnSpec) -> Result<(), String> {
        // Panes must not take the last descriptors: clients still need
        // some to connect (and to run kill-pane).
        if self.fd_limit > 0 {
            let open = crate::sys::open_fd_count();
            if open + crate::FD_RESERVE >= self.fd_limit {
                return Err(format!("cannot create a pane: {open} of {} file descriptors in use", self.fd_limit));
            }
        }
        let argv: Vec<OsString> = match &spec.command {
            SpawnCommand::LoginShell { .. } => Vec::new(),
            SpawnCommand::Shell { shell, command } => vec![shell.into(), "-c".into(), command.into()],
            SpawnCommand::Argv(v) => v.iter().map(OsString::from).collect(),
        };
        let mut env_remove: Vec<OsString> = ENV_REMOVE.iter().map(OsString::from).collect();
        env_remove.extend(self.server_env_zellij.iter().cloned());
        let opts = SpawnOptions {
            argv,
            cwd: (!spec.cwd.is_empty()).then(|| PathBuf::from(&spec.cwd)),
            env: self.pane_env(spec),
            env_remove,
            term: PANE_TERM.into(),
            size: spec.size,
            pixels: PixelSize::default(),
        };
        let stash = match self.respawn_stash.take() {
            Some((p, e, f)) if spec.respawn && p == spec.pane => Some((e, f)),
            other => {
                self.respawn_stash = other;
                None
            }
        };
        let (mut engine, frame) = match stash {
            Some(x) => x,
            None => {
                let vo = VtOptions {
                    size: spec.size,
                    pixels: PixelSize::default(),
                    scrollback_bytes: crate::pane::scrollback_limits(spec.scrollback_bytes, spec.history_limit).0,
                    scrollback_lines: crate::pane::scrollback_limits(spec.scrollback_bytes, spec.history_limit).1,
                    default_fg: self.default_colors.0,
                    default_bg: self.default_colors.1,
                    default_cursor: None,
                };
                let e = Engine::new(&vo).map_err(|e| e.to_string())?;
                // The frame is filled the first time a client shows the pane.
                (e, PaneFrame::default())
            }
        };
        if engine.size() != spec.size {
            engine.resize(spec.size, PixelSize::default());
        }
        let pty = Pty::spawn(&opts).map_err(|e| e.to_string())?;
        let pid = pty.pid();
        self.reaper.watch(pid);
        let fd = std::os::fd::AsRawFd::as_raw_fd(&pty);
        if let Err(e) = self.registry.register(&mut SourceFd(&fd), pane_token(spec.pane), Interest::READABLE) {
            // Dropping the PTY hangs the child up; the reaper collects it.
            self.hang_up(pid);
            drop(pty);
            return Err(e.to_string());
        }
        log!("pane {} spawned pid {pid} size {}x{}", spec.pane, spec.size.cols, spec.size.rows);
        self.pids.insert(pid, spec.pane);
        crate::panic::pane_started(pid);
        let mut p = PaneRt::new(spec.pane, pty, engine, frame, spec.size);
        p.current_path = spec.cwd.clone();
        p.scrollback = crate::pane::scrollback_limits(spec.scrollback_bytes, spec.history_limit);
        if spec.respawn {
            // A respawned pane keeps its terminal: give it the current limits.
            p.engine.set_scrollback_limits(p.scrollback.0, p.scrollback.1);
        }
        self.panes.insert(spec.pane, p);
        // The child may have exited before it was watched.
        self.views_stale = true;
        Ok(())
    }

    fn kill_pane(&mut self, pane: PaneId) {
        let Some(mut p) = self.panes.remove(&pane) else { return };
        log!("pane {pane} killed");
        if let Some(pty) = p.pty.take() {
            let fd = std::os::fd::AsRawFd::as_raw_fd(&pty);
            let _ = self.registry.deregister(&mut SourceFd(&fd));
            if self.pids.remove(&p.pid).is_some() {
                self.hang_up(p.pid);
            }
            drop(pty);
        }
        self.pids.retain(|_, v| *v != pane);
        p.mode = None;
        self.respawn_stash = Some((pane, p.engine, p.frame));
        self.views_stale = true;
    }

    fn resize_pane(&mut self, pane: PaneId, size: Size) {
        let Some(p) = self.panes.get_mut(&pane) else { return };
        if size.is_empty() {
            return;
        }
        p.size = size;
        p.engine.resize(size, PixelSize::default());
        // The engine gives back what the resize left behind once the pane
        // is quiet (see VtEngine::compact).
        p.compact_at = Some(Instant::now() + crate::COMPACT_AFTER);
        if let Some(pty) = p.pty.as_ref() {
            let _ = pty.resize(size, PixelSize::default());
        }
        p.update_mode();
        self.views_stale = true;
    }

    fn send_to_pane(&mut self, pane: PaneId, input: PaneInput<'_>) -> Result<(), String> {
        let mut buf = std::mem::take(&mut self.scratch);
        buf.clear();
        if let Some(p) = self.panes.get_mut(&pane) {
            match input {
                PaneInput::Key(k) => {
                    // send-keys into a pane in a mode drives the mode through
                    // its key table, as in tmux.
                    match p.with_mode(|m, ctx| m.key(k, ctx)) {
                        Some(ModeAction::Binding) => self.mode_bindings.push((pane, k.clone())),
                        Some(a) => self.mode_actions.push((None, pane, a)),
                        None => p.engine.encode_key(k, &mut buf),
                    }
                }
                PaneInput::Text(t) => buf.extend_from_slice(t.as_bytes()),
                PaneInput::Bytes(b) => buf.extend_from_slice(b),
                PaneInput::Paste { data, bracketed } => {
                    if bracketed {
                        p.engine.encode_paste(data, &mut buf);
                    } else {
                        buf.extend_from_slice(data);
                    }
                }
                PaneInput::Reset => {
                    p.engine.feed(b"\x1bc");
                    let mut junk = Vec::new();
                    p.engine.drain_replies(&mut junk);
                }
            }
        }
        let queued = self.write_pane(pane, &buf);
        let len = buf.len();
        self.scratch = buf;
        if queued || len == 0 {
            Ok(())
        } else if self.panes.get(&pane).is_some_and(|p| p.pty.is_some()) {
            Err(format!("pane {pane}: input queue full ({len} bytes not sent; the program is not reading its input)"))
        } else {
            Err(format!("pane {pane}: the program has exited"))
        }
    }

    fn pane_info(&self, pane: PaneId) -> PaneInfo<'_> {
        let Some(p) = self.panes.get(&pane) else { return PaneInfo::default() };
        let path = if p.current_path.is_empty() { p.osc7_path.as_str() } else { p.current_path.as_str() };
        PaneInfo {
            pid: p.pid,
            title: p.engine.title(),
            current_command: &p.current_command,
            current_path: path,
            history_size: p.engine.scrollback_rows(),
            history_bytes: p.engine.memory().resident_bytes,
            in_mode: p.mode.is_some(),
        }
    }

    fn capture_pane(&self, pane: PaneId, range: CaptureRange) -> String {
        let Some(p) = self.panes.get(&pane) else { return String::new() };
        let hist = p.engine.scrollback_rows() as i64;
        let rows = p.size.rows as i64;
        let total = hist + rows;
        let pos = |l: LinePos| -> i64 {
            let v = match l {
                LinePos::HistoryStart => 0,
                LinePos::ScreenEnd => total - 1,
                LinePos::Line(n) => hist + n,
            };
            v.clamp(0, (total - 1).max(0))
        };
        let (mut a, mut b) = (pos(range.start), pos(range.end));
        if b < a {
            std::mem::swap(&mut a, &mut b);
        }
        let opts = TextOptions { join_wrapped: range.join_wrapped, keep_trailing: range.keep_trailing, styled: range.styled };
        let text = p.engine.capture(TextRange::Rows { start: a as usize, end: b as usize + 1 }, opts);
        let mut out = String::with_capacity(text.len() + 64);
        let mut lines = 0i64;
        if !text.is_empty() {
            for line in text.split('\n') {
                out.push_str(if range.keep_trailing { line } else { line.trim_end_matches(' ') });
                out.push('\n');
                lines += 1;
            }
        }
        if !range.join_wrapped {
            while lines < b - a + 1 {
                out.push('\n');
                lines += 1;
            }
        }
        out
    }

    fn attach_client(&mut self, client: ClientId, session: SessionId) {
        let attached = self.clients.get(&client).is_some_and(|c| c.att.is_some());
        if attached {
            let hello_env = self.clients.get(&client).and_then(|c| c.hello.as_ref()).map(|h| h.env.clone());
            if let Some(a) = self.clients.get_mut(&client).and_then(|c| c.att.as_mut()) {
                a.session = session;
                a.renderer.invalidate();
                a.needs_frame = true;
                a.status_stale = true;
                a.prompt = None;
                a.display_panes = None;
            }
            if let Some(env) = hello_env {
                self.session_env.insert(session, env);
            }
        } else {
            self.attach_new(client, session);
        }
        log!("client {client} attached to session {session}");
        self.views_stale = true;
    }

    fn detach_client(&mut self, client: ClientId, reason: DetachReason) {
        if !self.clients.contains_key(&client) {
            return;
        }
        let (why, mut message) = exit_message(&reason);
        // A session name is user input: keep the message to a line.
        if message.len() > 1024 {
            let mut end = 1024;
            while !message.is_char_boundary(end) {
                end -= 1;
            }
            message.truncate(end);
        }
        self.end_paste(client);
        let Some(c) = self.clients.get_mut(&client) else { return };
        if let Some(mut att) = c.att.take() {
            let mut b = Vec::new();
            leave_bytes(&mut att, &mut b);
            if c.tty_mut().is_some_and(|t| !t.dead && !t.usable()) {
                // The terminal is the shell's now (the client was stopped
                // without a word): the client writes the restore when it
                // is continued, before it reads the Exit.
                c.send_terminal_via_socket(&b);
            } else {
                c.send_output(&b);
            }
            c.disarm_panic();
        }
        c.send(&ServerMsg::Exit { reason: why, message });
        c.closing = true;
        log!("client {client} detached: {why:?}");
        self.flush_client(client);
        self.views_stale = true;
    }

    fn hangup_client_parent(&mut self, client: ClientId) {
        let Some(pid) = self.clients.get(&client).and_then(|c| c.hello.as_ref()).map(|h| h.pid) else { return };
        match crate::sys::parent_pid(pid) {
            // Never init, nor the server itself (a client the server started).
            Some(ppid) if ppid > 1 && ppid != self.server_pid => {
                log!("client {client}: SIGHUP to its parent {ppid}");
                // SAFETY: kill(2) with a pid we looked up; no memory is involved.
                unsafe { libc::kill(ppid as libc::pid_t, libc::SIGHUP) };
            }
            other => log!("client {client}: no parent to hang up ({other:?})"),
        }
    }

    fn enter_copy_mode(&mut self, client: Option<ClientId>, pane: PaneId, opts: CopyModeOptions) {
        let Some(p) = self.panes.get_mut(&pane) else { return };
        if opts.cancel {
            if p.mode.is_some() {
                p.leave_mode();
                self.mode_changed.push(pane);
            }
        } else if let Some(a) = p.with_mode(|m, ctx| m.reenter(opts, ctx)) {
            self.mode_actions.push((client, pane, a));
        } else {
            enter_copy(p, opts, &self.copy_cfg.cfg);
            if opts.page_down
                && let Some(a) = p.with_mode(|m, ctx| m.command("page-down", None, 1, ctx).unwrap_or(ModeAction::None))
            {
                self.mode_actions.push((client, pane, a));
            }
            self.mode_changed.push(pane);
        }
        self.views_stale = true;
        self.mark_all(true);
    }

    fn mode_command(
        &mut self,
        client: Option<ClientId>,
        pane: PaneId,
        command: &str,
        args: &[String],
        count: u32,
    ) -> Result<(), String> {
        let p = self.panes.get_mut(&pane).ok_or("not in a mode")?;
        // copy-pipe* [command [prefix]]: the copy itself takes no command.
        let pipe = command.starts_with("copy-pipe");
        let arg = if pipe { args.get(1) } else { args.first() }.map(|s| s.as_str());
        let mut a = p.with_mode(|m, ctx| m.command(command, arg, count, ctx)).ok_or("not in a mode")??;
        if pipe && let ModeAction::Copy { pipe, .. } = &mut a {
            *pipe = Some(args.first().cloned().unwrap_or_default());
        }
        self.mode_actions.push((client, pane, a));
        Ok(())
    }

    fn read_config(&mut self, path: &str) -> Result<ConfigFile, ConfigError> {
        read_config(path)
    }

    fn show_message(&mut self, client: ClientId, text: &str, duration_ms: u64) {
        let Some(a) = self.clients.get_mut(&client).and_then(|c| c.att.as_mut()) else { return };
        let line = vec![Span::new(text.replace(['\n', '\r'], " "), a.msg_style)];
        let expires = (duration_ms > 0).then(|| Instant::now() + std::time::Duration::from_millis(duration_ms));
        a.message = Some(Message { line, expires });
        a.needs_frame = true;
    }

    fn prompt(&mut self, client: ClientId, prompt: Prompt) {
        let Some(a) = self.clients.get_mut(&client).and_then(|c| c.att.as_mut()) else { return };
        a.prompt = Some(PromptState::new(prompt));
        a.needs_frame = true;
    }

    fn display_panes(&mut self, client: ClientId, duration_ms: u64, template: &str, keys: bool) {
        let Some(a) = self.clients.get_mut(&client).and_then(|c| c.att.as_mut()) else { return };
        let labels = a
            .view
            .as_ref()
            .map(|v| v.panes.iter().enumerate().map(|(i, (p, _))| (*p, i.to_string())).collect())
            .unwrap_or_default();
        a.display_panes = Some(DisplayPanes {
            expires: Instant::now() + std::time::Duration::from_millis(duration_ms.max(1)),
            template: template.to_string(),
            labels,
            keys,
        });
        a.needs_frame = true;
    }

    fn bell(&mut self, client: ClientId) {
        self.send_output(client, b"\x07");
    }

    fn redraw(&mut self, what: Redraw) {
        self.views_stale = true;
        match what {
            Redraw::Client(c) => {
                if let Some(a) = self.clients.get_mut(&c).and_then(|c| c.att.as_mut()) {
                    a.needs_frame = true;
                    a.status_stale = true;
                }
            }
            Redraw::Status(s) | Redraw::Session(s) => {
                for c in self.clients.values_mut() {
                    if let Some(a) = c.att.as_mut()
                        && a.session == s
                    {
                        a.needs_frame = true;
                        a.status_stale = true;
                    }
                }
            }
            Redraw::Window(_) => self.mark_all(true),
        }
    }

    fn shutdown(&mut self) {
        log!("model asked for shutdown");
        self.shutdown = true;
    }
}

/// Put `p` into copy mode, the cursor where the program's is.
pub(crate) fn enter_copy(p: &mut PaneRt, opts: CopyModeOptions, cfg: &lif_copy::Config) {
    // The frame is only refreshed at frame ticks; bring its cursor up to
    // date (the dirty flags this sets are drawn at the next tick).
    p.engine.refresh(&mut p.frame);
    let last = p.size.rows.saturating_sub(1);
    let cursor = p.frame.cursor.pos.unwrap_or(Pos { x: 0, y: last });
    // Copy mode keeps its positions on their text while the scrollback
    // limit evicts rows; counting them costs a little per feed, so only now.
    p.engine.track_trimmed(true);
    p.mode = Some(new_copy_mode(opts, cfg, cursor, &mut ModeCtx { engine: &mut p.engine }));
}

/// Expand a leading `~/` with `$HOME`, as tmux does for file names.
fn expand_home(path: &str) -> PathBuf {
    match (path.strip_prefix("~/"), std::env::var_os("HOME")) {
        (Some(rest), Some(home)) => PathBuf::from(home).join(rest),
        _ => PathBuf::from(path),
    }
}

/// `source-file`: lif's TOML configuration, translated by lif-config.
pub(crate) fn read_config(path: &str) -> Result<ConfigFile, ConfigError> {
    let file = expand_home(path);
    let hint = || {
        ConfigError::Invalid(format!(
            "{path}: this looks like a tmux configuration file, but lif reads TOML; \
             convert it with: lif migrate-tmux-conf {path} > ~/.config/lif/config.toml"
        ))
    };
    if lif_config::looks_like_tmux_conf(path) {
        return Err(hint());
    }
    // Read once, with lif-config's limits: the server is one thread, so a
    // FIFO, a device or a huge file here would stall or exhaust every session.
    let bytes = match lif_config::read_file_limited(&file) {
        Err(e) if e.kind() == std::io::ErrorKind::NotFound => {
            return Err(ConfigError::Missing(format!("{path}: No such file or directory")));
        }
        Err(e) => return Err(ConfigError::Invalid(format!("{path}: {e}"))),
        Ok(b) => b,
    };
    let cfg = lif_config::Config::from_bytes(&bytes);
    // Not TOML at all, but tmux commands: point at the translator rather
    // than at a TOML syntax error.
    if cfg.commands.is_empty()
        && cfg.has_errors()
        && let Ok(text) = std::str::from_utf8(&bytes)
        && lif_config::looks_like_tmux_commands(text)
    {
        return Err(hint());
    }
    let name = file.display().to_string();
    let commands = cfg.commands.iter().map(|e| (e.command.clone(), format!("{name}:{}:{}", e.line, e.column))).collect();
    let diagnostics = cfg.diagnostics.iter().map(|d| (d.severity == lif_config::Severity::Error, format!("{name}:{d}"))).collect();
    Ok(ConfigFile { commands, diagnostics })
}

/// An I/O error as tmux prints it: the OS's message without Rust's
/// "(os error N)".
fn io_text(e: &std::io::Error) -> String {
    let s = e.to_string();
    match s.find(" (os error") {
        Some(i) => s[..i].to_string(),
        None => s,
    }
}
