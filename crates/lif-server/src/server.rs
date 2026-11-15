//! The event loop.

use std::io;
use std::os::fd::AsRawFd;
use std::os::unix::net::UnixListener as StdListener;
use std::time::{Duration, Instant};

use lif_core::{ClientId, ClientInfo, CmdContext, CommandList, Model, PaneId};
use lif_proto::{Caps, ClientMsg, PROTOCOL_VERSION, ServerMsg};
use lif_pty::ChildReaper;
use lif_render::ScenePane;
use lif_vt::{VtEngine, VtEvent};
use mio::net::UnixListener;
use mio::unix::SourceFd;
use mio::{Events, Interest, Poll, Token};

use crate::ServerOptions;
use crate::client::{CLIENT_READ_BUDGET, ClientRt, TermOut, Tty};
use crate::host::{CLIENT_TOKEN, JOB_TOKEN, PANE_PIPE_TOKEN, PANE_TOKEN, PIPE_TOKEN, Rt, TTY_TOKEN};
use crate::sys::{self, SignalPipe};

const LISTENER: Token = Token(0);
const REAPER: Token = Token(1);
const SIGNALS: Token = Token(2);

/// How long a freshly started server waits for its first client before
/// giving up (the client that started it died).
const FIRST_CLIENT_TIMEOUT: Duration = Duration::from_secs(10);

/// Size of the PTY read buffer (one engine `feed`).
const READ_BUF: usize = 64 * 1024;

/// Bytes read from a client's terminal per loop iteration (one parser feed).
const TTY_READ_BUF: usize = 16 * 1024;

/// Optional protocol features this server implements.
const SERVER_CAPS: Caps = Caps(Caps::TTY.0 | Caps::SUSPEND.0 | Caps::OUTPUT_STREAM.0);

/// About how long one step of a copy-mode search may take before the loop
/// looks at its descriptors again.
const SEARCH_STEP_TIME: Duration = Duration::from_millis(2);

/// How deep copy-mode bindings run from `send-keys` may nest (a binding
/// that sends keys to copy mode runs those keys' bindings, and so on).
const MAX_BINDING_DEPTH: u32 = 8;

pub(crate) struct Server {
    poll: Poll,
    events: Events,
    listener: UnixListener,
    signals: SignalPipe,
    pub(crate) model: Model,
    pub(crate) rt: Rt,
    opts: ServerOptions,
    next_client: u32,
    /// Loop iterations, for the idle-wakeup test and the stats log.
    pub(crate) iterations: u64,
    read_buf: Vec<u8>,
    tty_buf: Vec<u8>,
    pub(crate) start: Instant,
    first_client_deadline: Option<Instant>,
    ready: Vec<(Token, bool, bool)>,
    sigs: Vec<libc::c_int>,
    pub(crate) vt_events: Vec<VtEvent>,
    pub(crate) replies: Vec<u8>,
    /// Clients drawn this tick: (client, full frame or echo only).
    pub(crate) due: Vec<(ClientId, bool)>,
    pub(crate) refreshed: Vec<PaneId>,
    pub(crate) scene_scratch: Vec<ScenePane<'static>>,
    pub(crate) label_scratch: Vec<lif_render::BorderLabel<'static>>,
    /// Reused id lists, so a loop iteration allocates nothing.
    pub(crate) scratch_panes: Vec<PaneId>,
    pub(crate) scratch_clients: Vec<ClientId>,
    pub(crate) scratch_views: Vec<ClientId>,
    /// Copy-mode bindings run from `send-keys` that are running now, so a
    /// binding that sends its own key cannot recurse without end.
    binding_depth: u32,
    /// A descriptor kept open to be closed when `accept` fails for lack of
    /// descriptors: it makes room to accept the waiting connection, tell
    /// it why and close it, so the listener's queue is drained and its
    /// readiness edge is not lost (see `Server::accept`).
    spare_fd: Option<std::os::fd::OwnedFd>,
    /// (device, inode) of the socket this server bound: on exit it removes
    /// the path only if it is still that socket.
    socket_id: Option<(u64, u64)>,
    /// The server's user, for the peer check of every connection.
    uid: u32,
}

impl Server {
    pub(crate) fn new(listener: StdListener, opts: ServerOptions) -> io::Result<Server> {
        let poll = Poll::new()?;
        listener.set_nonblocking(true)?;
        let mut listener = UnixListener::from_std(listener);
        poll.registry().register(&mut listener, LISTENER, Interest::READABLE)?;
        let reaper = ChildReaper::new()?;
        poll.registry().register(&mut SourceFd(&reaper.as_raw_fd()), REAPER, Interest::READABLE)?;
        let signals = SignalPipe::install()?;
        poll.registry().register(&mut SourceFd(&signals.fd()), SIGNALS, Interest::READABLE)?;
        let mut model = Model::new(sys::hostname());
        if let Ok(shell) = std::env::var("SHELL")
            && shell.starts_with('/')
        {
            let _ = model.set_global_option("default-shell", &shell);
        }
        model.set_server_info(std::process::id(), opts.socket.to_string_lossy());
        // tmux picks mode-keys from the VISUAL / EDITOR of the client that
        // starts the server. lif's clients do not send those variables, but
        // the server inherits the starting client's environment, which is
        // the same thing.
        let visual = std::env::var("VISUAL").ok();
        let editor = std::env::var("EDITOR").ok();
        let keys = lif_copy::ModeKeys::from_editor(visual.as_deref(), editor.as_deref());
        let _ = model.set_global_option("mode-keys", if keys == lif_copy::ModeKeys::Vi { "vi" } else { "emacs" });
        let mut rt = Rt::new(poll.registry().try_clone()?, reaper, opts.socket.to_string_lossy().into_owned());
        let (old_limit, limit) = sys::raise_fd_limit();
        rt.fd_limit = limit as usize;
        lif_pty::set_child_fd_limit(old_limit);
        let socket_id = std::fs::symlink_metadata(&opts.socket).ok().map(|m| {
            use std::os::unix::fs::MetadataExt;
            (m.dev(), m.ino())
        });
        let now = Instant::now();
        let mut s = Server {
            poll,
            events: Events::with_capacity(256),
            listener,
            signals,
            model,
            rt,
            opts,
            next_client: 1,
            iterations: 0,
            read_buf: vec![0; READ_BUF],
            tty_buf: Vec::new(),
            start: now,
            first_client_deadline: Some(now + FIRST_CLIENT_TIMEOUT),
            ready: Vec::new(),
            sigs: Vec::new(),
            vt_events: Vec::new(),
            replies: Vec::new(),
            due: Vec::new(),
            refreshed: Vec::new(),
            scene_scratch: Vec::new(),
            label_scratch: Vec::new(),
            scratch_panes: Vec::new(),
            scratch_clients: Vec::new(),
            scratch_views: Vec::new(),
            binding_depth: 0,
            spare_fd: sys::spare_fd(),
            socket_id,
            // SAFETY: getuid cannot fail.
            uid: unsafe { libc::getuid() },
        };
        log!("descriptor limit {limit} (was {old_limit:?})");
        log!("server {} started on {}", std::process::id(), s.opts.socket.display());
        s.model.collect_startup(true);
        for line in crate::load_startup_commands(&s.opts) {
            match lif_core::parse_string(&line) {
                Ok(list) => {
                    if let Err(e) = s.run_commands(None, &list) {
                        log!("startup command {line:?} failed");
                        for l in e.0.lines() {
                            log!("  {l}");
                        }
                    }
                }
                Err(e) => log!("startup command {line:?}: {e}"),
            }
        }
        s.model.collect_startup(false);
        // The configuration's [startup] commands, once the settings are in
        // place. Each is a command list of its own: one that waits (for a
        // shell command, say) does not hold up the next.
        for (cmd, at) in s.model.take_startup() {
            log!("startup: {cmd}");
            if let Err(e) = lif_core::parse_string(&cmd).and_then(|list| s.run_commands(None, &list)) {
                s.model.log_message(format!("{at}: {e}"));
            }
        }
        // Every problem of the configuration (warnings too) is in the
        // message log; the first client to attach sees the first one.
        let problems: Vec<&str> = s.model.messages().collect();
        if let Some(first) = problems.first() {
            for p in &problems {
                log!("config: {p}");
            }
            let more = match problems.len() {
                1 => String::new(),
                n => format!(" (and {} more, see show-messages)", n - 1),
            };
            s.rt.startup_message = Some(format!("{first}{more}"));
        }
        Ok(s)
    }

    pub(crate) fn now_ms(&self, now: Instant) -> u64 {
        now.duration_since(self.start).as_millis() as u64
    }

    pub(crate) fn run(&mut self) -> i32 {
        loop {
            let now = Instant::now();
            let timeout = self.next_deadline(now).map(|d| d.saturating_duration_since(now));
            if let Err(e) = self.poll.poll(&mut self.events, timeout) {
                if e.kind() == io::ErrorKind::Interrupted {
                    continue;
                }
                log!("poll failed: {e}");
                self.finish();
                return 1;
            }
            self.iterations += 1;
            if crate::log::enabled() {
                let tokens: Vec<usize> = self.events.iter().map(|e| e.token().0).collect();
                log!("wake {}: timeout {:?}, ready {:x?}", self.iterations, timeout, tokens);
            }
            self.ready.clear();
            for ev in self.events.iter() {
                let readable = ev.is_readable() || ev.is_read_closed() || ev.is_error();
                let writable = ev.is_writable() || ev.is_write_closed();
                self.ready.push((ev.token(), readable, writable));
            }
            let now = Instant::now();
            let ready = std::mem::take(&mut self.ready);
            for &(token, readable, writable) in &ready {
                self.dispatch(token, readable, writable, now);
            }
            self.ready = ready;
            self.service(now);
            if self.rt.shutdown {
                self.finish();
                return 0;
            }
            if self.empty_exit(now) {
                log!("no sessions and no clients left");
                self.finish();
                return 0;
            }
        }
    }

    fn empty_exit(&mut self, now: Instant) -> bool {
        if self.model.sessions().next().is_some() || !self.rt.clients.is_empty() {
            return false;
        }
        if self.model.option_text("exit-empty", None, None).as_deref() == Some("off") {
            return false;
        }
        match self.first_client_deadline {
            Some(d) => now >= d,
            None => true,
        }
    }

    fn dispatch(&mut self, token: Token, readable: bool, writable: bool, now: Instant) {
        match token {
            LISTENER => self.accept(),
            REAPER => self.reap(now),
            SIGNALS => self.handle_signals(),
            Token(t) if t & PANE_PIPE_TOKEN != 0 => self.pane_pipe_writable(PaneId((t & 0xffff_ffff) as u32)),
            Token(t) if t & JOB_TOKEN != 0 => self.read_job((t & 0xffff_ffff) as u32),
            Token(t) if t & PIPE_TOKEN != 0 => self.rt.pipe_writable((t & 0xffff_ffff) as u32),
            Token(t) if t & TTY_TOKEN != 0 => {
                let id = ClientId((t & 0xffff_ffff) as u32);
                if writable {
                    self.rt.flush_client(id);
                }
                if readable {
                    self.read_tty(id, now);
                }
            }
            Token(t) if t & CLIENT_TOKEN != 0 && t & PANE_TOKEN == 0 => {
                let id = ClientId((t & 0xffff_ffff) as u32);
                if writable {
                    self.rt.flush_client(id);
                }
                if readable {
                    self.read_client(id, now);
                }
            }
            Token(t) if t & PANE_TOKEN != 0 => {
                let id = PaneId((t & 0xffff_ffff) as u32);
                if writable {
                    self.pane_writable(id);
                }
                if readable {
                    self.read_pane(id, now, crate::PANE_READ_BUDGET);
                }
            }
            _ => {}
        }
    }

    /// Everything that happens after the ready events of an iteration:
    /// leftover reads, expired deadlines, view updates, frames, clean-up.
    fn service(&mut self, now: Instant) {
        let mut panes = std::mem::take(&mut self.scratch_panes);
        panes.clear();
        panes.extend(self.rt.panes.values().filter(|p| p.read_more).map(|p| p.id));
        for &p in &panes {
            self.read_pane(p, now, crate::PANE_READ_BUDGET);
        }
        self.scratch_panes = panes;
        let mut clients = std::mem::take(&mut self.scratch_clients);
        clients.clear();
        clients.extend(self.rt.clients.values().filter(|c| c.read_more).map(|c| c.id));
        for &c in &clients {
            self.read_client(c, now);
        }
        clients.clear();
        clients.extend(self.rt.clients.values().filter(|c| c.tty_read_more()).map(|c| c.id));
        for &c in &clients {
            self.read_tty(c, now);
        }
        self.scratch_clients = clients;
        if self.rt.jobs.values().any(|j| j.read_more) {
            let more: Vec<u32> = self.rt.jobs.iter().filter(|(_, j)| j.read_more).map(|(t, _)| *t).collect();
            for t in more {
                self.read_job(t);
            }
        }
        self.sweep_popups();
        self.job_timers(now);
        self.rt.kill_overdue(now);
        self.timers(now);
        self.step_searches();
        self.refresh_clocks(now);
        self.refresh_choosers();
        self.sync_views();
        self.frame_tick(now);
        self.remove_dead_clients();
    }

    /// One bounded step of every copy-mode search under way, once per turn
    /// of the loop (which polls with a zero timeout while any runs), so
    /// input and output of every pane and client keep flowing during a
    /// search through a huge history. Each pane's step size adapts so a
    /// step takes about [`SEARCH_STEP_TIME`] however costly its rows are.
    fn step_searches(&mut self) {
        let mut ids = std::mem::take(&mut self.scratch_panes);
        ids.clear();
        ids.extend(self.rt.panes.values().filter(|p| p.searching).map(|p| p.id));
        for &id in &ids {
            let Some(p) = self.rt.panes.get(&id) else { continue };
            let (rows, client) = (p.search_rows, p.search_client);
            let t = Instant::now();
            let act = self.with_mode(id, |m, ctx| m.search_step(rows, ctx));
            let took = t.elapsed().as_secs_f64();
            if let Some(p) = self.rt.panes.get_mut(&id)
                && took > 0.0
            {
                let want = rows as f64 * SEARCH_STEP_TIME.as_secs_f64() / took;
                p.search_rows = (want as usize).clamp(64, 200_000);
            }
            self.mode_action(client, id, act);
        }
        self.scratch_panes = ids;
    }

    /// The nearest deadline, or `None` (sleep until I/O) when nothing is pending.
    fn next_deadline(&self, now: Instant) -> Option<Instant> {
        let mut d: Option<Instant> = self.first_client_deadline;
        let mut min = |t: Option<Instant>| {
            if let Some(t) = t {
                d = Some(d.map_or(t, |x| x.min(t)));
            }
        };
        for p in self.rt.panes.values() {
            if p.read_more || p.searching {
                return Some(now);
            }
            min(p.compact_at);
            min(p.silence_at);
            min(p.name_recheck);
            min(p.autoscroll_at);
        }
        for c in self.rt.clients.values() {
            if c.read_more || c.tty_read_more() {
                return Some(now);
            }
            if let Some(a) = c.att.as_ref() {
                min(a.input_deadline);
                min(a.key_deadline);
                min(a.frame_deadline);
                min(a.message.as_ref().and_then(|m| m.expires));
                min(a.display_panes.as_ref().map(|d| d.expires));
                if a.clock {
                    min(a.clock_deadline);
                }
            }
        }
        min(self.rt.dying.iter().map(|d| d.1).min());
        if self.rt.jobs.values().any(|j| j.read_more) {
            return Some(now);
        }
        min(self.rt.job_timers.iter().map(|t| t.0).min());
        min(self.rt.clock_deadline);
        d
    }

    /// Accepts every waiting connection. The listener is edge-triggered:
    /// one that is left waiting would never be reported again, so this
    /// always drains the queue. Out of descriptors, the spare one is
    /// closed to accept the connection, tell the client and close it.
    fn accept(&mut self) {
        loop {
            // On macOS an accept that fails for lack of a descriptor has
            // already taken the connection off the queue and drops it: so
            // look first, and when there is no room take it with the spare.
            if !self.fd_available() {
                match self.refuse_one() {
                    Some(true) => continue,
                    Some(false) => return,
                    None => {
                        log!("accept: out of descriptors and no spare one");
                        return;
                    }
                }
            }
            match self.listener.accept() {
                Ok((mut stream, _)) => {
                    let fd = stream.as_raw_fd();
                    let peer = match sys::peer_cred(fd) {
                        Ok(p) if sys::peer_allowed(p.uid, self.uid) => p,
                        Ok(p) => {
                            log!("connection from uid {} refused", p.uid);
                            continue;
                        }
                        Err(e) => {
                            log!("connection refused: no peer credentials: {e}");
                            continue;
                        }
                    };
                    let Some(next) = self.next_client.checked_add(1) else {
                        log!("client ids exhausted");
                        continue;
                    };
                    let id = ClientId(self.next_client);
                    self.next_client = next;
                    let token = Token(CLIENT_TOKEN | id.0 as usize);
                    if let Err(e) = self.poll.registry().register(&mut stream, token, Interest::READABLE) {
                        log!("register client: {e}");
                        continue;
                    }
                    self.first_client_deadline = None;
                    self.rt.clients.insert(id, ClientRt::new(id, stream, token, peer));
                    // Data may already be waiting: the edge was before registration.
                    if let Some(c) = self.rt.clients.get_mut(&id) {
                        c.read_more = true;
                    }
                }
                Err(e) if e.kind() == io::ErrorKind::WouldBlock => return,
                Err(e) if e.kind() == io::ErrorKind::Interrupted => {}
                Err(e) if matches!(e.raw_os_error(), Some(libc::EMFILE | libc::ENFILE)) => {
                    // (ENFILE: the whole system is out.) Refuse the rest.
                    if self.refuse_one().is_none() {
                        log!("accept: {e}; no spare descriptor");
                        return;
                    }
                }
                Err(e) => {
                    log!("accept: {e}");
                    return;
                }
            }
        }
    }

    /// A descriptor is free for an accepted connection (checked by
    /// duplicating the spare: only when a connection arrives).
    fn fd_available(&mut self) -> bool {
        let Some(spare) = self.spare_fd.as_ref() else {
            self.spare_fd = sys::spare_fd();
            return self.spare_fd.is_some();
        };
        // SAFETY: F_DUPFD_CLOEXEC on an fd we own; the copy is closed at once.
        let fd = unsafe { libc::fcntl(spare.as_raw_fd(), libc::F_DUPFD_CLOEXEC, 0) };
        if fd < 0 {
            return false;
        }
        // SAFETY: fd is the copy just made.
        unsafe { libc::close(fd) };
        true
    }

    /// Out of descriptors: close the spare, accept one waiting connection
    /// into it, answer it with an error a lif client prints, close it and
    /// take the spare back. Some(false) when nothing was waiting, None
    /// when there was no spare.
    fn refuse_one(&mut self) -> Option<bool> {
        let Some(spare) = self.spare_fd.take() else {
            self.spare_fd = sys::spare_fd();
            return None;
        };
        drop(spare);
        let waiting = self.listener.accept();
        let handled = waiting.is_ok();
        if let Ok((stream, _)) = waiting {
            let mut out = Vec::new();
            let _ = ServerMsg::Welcome {
                protocol: PROTOCOL_VERSION,
                caps: Caps::NONE,
                server_version: env!("CARGO_PKG_VERSION").into(),
                server_pid: std::process::id(),
            }
            .encode(&mut out);
            let msg = format!("server out of file descriptors (limit {}): close some panes\n", self.rt.fd_limit);
            let _ = ServerMsg::CommandDone { status: 1, stdout: Vec::new(), stderr: msg.into_bytes() }.encode(&mut out);
            let _ = ServerMsg::Exit { reason: lif_proto::ExitReason::Error, message: "[server out of file descriptors]".into() }.encode(&mut out);
            // A fresh socket buffer takes these few bytes at once.
            sys::write_all_timeout(stream.as_raw_fd(), &out, 0);
            log!("out of descriptors: refused a connection");
        }
        self.spare_fd = sys::spare_fd();
        Some(handled)
    }

    fn handle_signals(&mut self) {
        let mut sigs = std::mem::take(&mut self.sigs);
        sigs.clear();
        self.signals.drain(&mut sigs);
        for &s in &sigs {
            match s {
                libc::SIGTERM | libc::SIGINT => {
                    log!("signal {s}: shutting down");
                    if let Ok(list) = lif_core::parse_list(&["kill-server".to_string()]) {
                        let _ = self.run_commands(None, &list);
                    }
                    self.rt.shutdown = true;
                }
                libc::SIGUSR1 => self.log_stats(),
                _ => {}
            }
        }
        self.sigs = sigs;
    }

    pub(crate) fn log_stats(&self) {
        let (mut vt, mut queued) = (0, 0);
        for p in self.rt.panes.values() {
            vt += p.engine.memory().resident_bytes;
            queued += p.wq.len();
        }
        log!(
            "stats: iterations={} panes={} clients={} rss={} vt_bytes={} input_queued={}",
            self.iterations,
            self.rt.panes.len(),
            self.rt.clients.len(),
            sys::rss_bytes(),
            vt,
            queued
        );
    }

    // ----- panes ---------------------------------------------------------

    fn pane_writable(&mut self, id: PaneId) {
        self.rt.flush_pane(id);
    }

    pub(crate) fn read_pane(&mut self, id: PaneId, now: Instant, budget: usize) {
        let Some(p) = self.rt.panes.get_mut(&id) else { return };
        let r = p.read(&mut self.read_buf, budget);
        if p.pipe.as_ref().is_some_and(|x| x.dead) {
            self.close_dead_pipe(id);
        }
        let Some(p) = self.rt.panes.get_mut(&id) else { return };
        if r.bytes > 0 {
            p.echo_bytes = p.echo_bytes.saturating_add(r.bytes);
            p.compact_at = Some(now + crate::COMPACT_AFTER);
            p.output_seen = true;
            if p.engine.modes().synchronized_output {
                p.sync_since.get_or_insert(now);
            } else {
                p.sync_since = None;
            }
            p.update_mode();
            self.after_output(id);
            self.model.pane_activity(id, &mut self.rt);
            // monitor-silence counts from the last output.
            let quiet = self.model.silence_secs(id);
            if let Some(p) = self.rt.panes.get_mut(&id) {
                p.silence_at = (quiet > 0).then(|| now + Duration::from_secs(quiet));
            }
        }
        if r.eof && let Some(p) = self.rt.panes.get_mut(&id) {
            // Stop polling a hung-up PTY; the exit comes from the reaper.
            let fd = p.raw_fd();
            if fd >= 0 {
                let _ = self.rt.registry.deregister(&mut SourceFd(&fd));
            }
            p.read_more = false;
            if !self.rt.pids.contains_key(&p.pid) {
                self.pane_gone(id, None);
            }
        }
    }

    /// Replies and events a feed produced.
    fn after_output(&mut self, id: PaneId) {
        let Some(p) = self.rt.panes.get_mut(&id) else { return };
        self.replies.clear();
        p.engine.drain_replies(&mut self.replies);
        let mut events = std::mem::take(&mut self.vt_events);
        events.clear();
        p.engine.drain_events(&mut events);
        if !self.replies.is_empty() {
            let r = std::mem::take(&mut self.replies);
            // A program that asks and never reads the answers: they are
            // dropped once its input queue is full.
            self.rt.write_pane(id, &r);
            self.replies = r;
        }
        for ev in events.drain(..) {
            self.pane_event(id, ev);
        }
        self.vt_events = events;
    }

    fn reap(&mut self, now: Instant) {
        for (pid, status) in self.rt.reaper.reap() {
            self.rt.child_reaped(pid);
            let Some(&id) = self.rt.pids.get(&pid) else {
                self.job_exited(pid, status);
                continue;
            };
            log!("pane {id} pid {pid} exited: {status:?}");
            // Output the program wrote just before exiting is still in the
            // PTY. Bounded: a background process that ignores SIGHUP may
            // keep writing to it, and only closing the PTY stops it.
            // (The pid stays known during the drain: the end of the
            // PTY's output must not report the pane gone without its
            // exit status.)
            self.read_pane(id, now, crate::EXIT_DRAIN);
            self.rt.pids.remove(&pid);
            self.pane_gone(id, status.code());
        }
    }

    /// The pane's process is gone: tell the model, then drop what the model
    /// no longer knows (or just the PTY of a pane kept by remain-on-exit).
    fn pane_gone(&mut self, id: PaneId, code: Option<i32>) {
        // A popup's program: the popup closes (-E), or stays with what
        // the program left until a key.
        let mut kept = false;
        if let Some(&(_, client, _)) = self.rt.popups.iter().find(|p| p.0 == id) {
            let status = code.unwrap_or(1);
            let close = match self.rt.clients.get_mut(&client).and_then(|c| c.att.as_mut()).and_then(|a| a.overlay.as_mut()) {
                Some(crate::overlay::Overlay::Popup(p)) if p.pane == id => {
                    p.exited = true;
                    match p.close {
                        lif_core::PopupClose::OnExit => true,
                        lif_core::PopupClose::OnSuccess => status == 0,
                        lif_core::PopupClose::Never => false,
                    }
                }
                _ => true,
            };
            if close {
                self.rt.end_overlay(client, status);
                // (Not a popup any more if its client had lost it.)
                self.rt.popups.retain(|p| p.0 != id);
            } else {
                kept = true;
            }
        }
        self.model.pane_exited(id, code, &mut self.rt);
        self.after_model();
        if self.model.pane(id).is_none() && !kept {
            if let Some(mut p) = self.rt.panes.remove(&id)
                && let Some(pty) = p.pty.take()
            {
                let fd = pty.as_raw_fd();
                let _ = self.rt.registry.deregister(&mut SourceFd(&fd));
            }
        } else if let Some(p) = self.rt.panes.get_mut(&id)
            && let Some(pty) = p.pty.take()
        {
            let fd = pty.as_raw_fd();
            let _ = self.rt.registry.deregister(&mut SourceFd(&fd));
            p.read_more = false;
        }
        self.rt.views_stale = true;
    }

    /// A `pipe-pane` command can take more: write what waits for it.
    fn pane_pipe_writable(&mut self, pane: PaneId) {
        if let Some(pipe) = self.rt.panes.get_mut(&pane).and_then(|p| p.pipe.as_mut()) {
            pipe.flush();
        }
        self.close_dead_pipe(pane);
    }

    /// Close `pane`'s pipe if its command stopped reading.
    fn close_dead_pipe(&mut self, pane: PaneId) {
        let Some(p) = self.rt.panes.get_mut(&pane) else { return };
        if p.pipe.as_ref().is_some_and(|x| x.dead)
            && let Some(mut old) = p.pipe.take()
        {
            let _ = self.rt.registry.deregister(&mut old.tx);
            log!("pipe-pane: pane {pane}: pid {} stopped reading", old.pid);
        }
    }

    /// Draw the clocks whose minute or size changed, and keep the
    /// deadline for the next minute while any pane shows one.
    fn refresh_clocks(&mut self, now: Instant) {
        if self.rt.clock_panes.is_empty() {
            return;
        }
        let wall = sys::wall_clock();
        let mut left = Vec::with_capacity(self.rt.clock_panes.len());
        for pane in std::mem::take(&mut self.rt.clock_panes) {
            let twelve = self.model.pane_option_text(pane, "clock-mode-style").as_deref() == Some("12");
            let colour = self.model.pane_option_text(pane, "clock-mode-colour").unwrap_or_default();
            let Some(p) = self.rt.panes.get_mut(&pane) else { continue };
            let size = p.engine.size();
            let Some(c) = p.mode.as_deref_mut().and_then(|m| m.clock()) else { continue };
            left.push(pane);
            let text = crate::clock::clock_text(wall, twelve);
            if c.needs_draw(&text, size) {
                c.draw(&text, size, lif_core::parse_style(&format!("fg={colour}")));
                self.rt.mark_all(false);
            }
        }
        self.rt.clock_panes = left;
        self.rt.clock_deadline = (!self.rt.clock_panes.is_empty()).then(|| now + Duration::from_secs(60 - wall.unix_secs.rem_euclid(60) as u64));
    }

    /// End the popups whose client is gone or no longer shows them, or
    /// whose screen became too small for them. Nothing to do, nearly
    /// always: the list is empty unless a popup is up.
    fn sweep_popups(&mut self) {
        if self.rt.popups.is_empty() {
            return;
        }
        let mut ended = false;
        for (pane, client, job) in self.rt.popups.clone() {
            let att = self.rt.clients.get(&client).filter(|c| !c.dead).and_then(|c| c.att.as_ref());
            let size = self.rt.clients.get(&client).and_then(|c| c.hello.as_ref()).map(|h| h.size);
            let shown = match att.and_then(|a| a.overlay.as_ref()) {
                Some(crate::overlay::Overlay::Popup(p)) if p.pane == pane => {
                    size.is_none_or(|s| p.rect.right() <= s.cols && p.rect.bottom() <= s.rows)
                }
                _ => false,
            };
            if shown {
                continue;
            }
            ended = true;
            if att.is_some_and(|a| matches!(&a.overlay, Some(crate::overlay::Overlay::Popup(p)) if p.pane == pane)) {
                self.rt.end_overlay(client, 0);
            } else {
                self.rt.popups.retain(|p| p.0 != pane);
                self.rt.popups_done.push((job, 0));
                lif_core::Host::kill_pane(&mut self.rt, pane);
            }
        }
        if ended {
            self.after_model();
        }
    }

    // ----- shell commands ------------------------------------------------

    fn read_job(&mut self, token: u32) {
        let Some(j) = self.rt.jobs.get_mut(&token) else { return };
        if j.read(&mut self.read_buf)
            && let Some(mut rx) = j.rx.take()
        {
            let _ = self.rt.registry.deregister(&mut rx);
        }
        self.job_check(token);
    }

    fn job_exited(&mut self, pid: u32, status: std::process::ExitStatus) {
        // A pipe-pane command that exited takes no more: close its pipe
        // now, not at the pane's next output.
        if let Some(pane) = self.rt.panes.values_mut().find(|p| p.pipe.as_ref().is_some_and(|x| x.pid == pid)) {
            let id = pane.id;
            if let Some(pipe) = pane.pipe.as_mut() {
                pipe.dead = true;
            }
            self.close_dead_pipe(id);
            return;
        }
        let Some((&token, j)) = self.rt.jobs.iter_mut().find(|(_, j)| j.pid == pid && j.status.is_none()) else { return };
        j.status = Some(crate::job::status_of(status));
        self.job_check(token);
    }

    /// Hands a job that is over to the model.
    fn job_check(&mut self, token: u32) {
        if !self.rt.jobs.get(&token).is_some_and(|j| j.done()) {
            return;
        }
        let Some(j) = self.rt.jobs.remove(&token) else { return };
        let status = j.status.unwrap_or(lif_core::JobStatus::Exited(1));
        log!("job {} pid {} done: {status:?}, {} bytes", j.id, j.pid, j.out.len());
        self.job_done(j.id, status, &j.out);
    }

    /// Reports a job or timer to the model; the commands that waited for
    /// it run now.
    fn job_done(&mut self, id: u64, status: lif_core::JobStatus, out: &[u8]) {
        self.refresh_pane_facts();
        self.model.job_done(id, status, out, sys::wall_clock(), &mut self.rt);
        // They may have been anything, set-option included.
        self.rt.views_stale = true;
        self.apply_options();
        self.after_model();
    }

    fn job_timers(&mut self, now: Instant) {
        if !self.rt.job_timers.iter().any(|t| t.0 <= now) {
            return;
        }
        let mut due: Vec<(Instant, u64)> = Vec::new();
        self.rt.job_timers.retain(|t| {
            if t.0 <= now {
                due.push(*t);
            }
            t.0 > now
        });
        due.sort();
        for (_, id) in due {
            self.job_done(id, lif_core::JobStatus::Exited(0), &[]);
        }
    }

    /// The outcome of commands that had to wait: the answer a client's
    /// command still owes, a message for an attached client, or a line in
    /// the message log when nobody is there to see it.
    fn deliver(&mut self, f: lif_core::Finished) {
        log!("finished: client {:?} waited {} status {} error {:?}", f.client, f.waited, f.status, f.error);
        let client = f.client.filter(|id| self.rt.clients.get(id).is_some_and(|c| !c.dead));
        let Some(id) = client else {
            if let Some(e) = f.error {
                self.model.log_message(e.to_string());
            }
            return;
        };
        let attached = self.rt.clients.get(&id).is_some_and(|c| c.att.is_some());
        if attached {
            match f.error {
                Some(e) => {
                    let d = self.display_time();
                    lif_core::Host::show_message(&mut self.rt, id, &e.to_string(), d);
                }
                None => self.show_output(id, &f.stdout),
            }
        } else if f.waited {
            let Some(c) = self.rt.clients.get_mut(&id) else { return };
            match f.error {
                Some(e) => c.send_command_done(1, f.stdout.into_bytes(), format!("{e}\n").into_bytes()),
                None => c.send_command_done(f.status, f.stdout.into_bytes(), Vec::new()),
            }
            self.rt.flush_client(id);
        }
    }

    // ----- clients -------------------------------------------------------

    fn read_client(&mut self, id: ClientId, now: Instant) {
        let mut total = 0;
        loop {
            let Some(c) = self.rt.clients.get_mut(&id) else { return };
            c.read_more = false;
            if c.dead {
                return;
            }
            if total >= CLIENT_READ_BUDGET {
                c.read_more = true;
                return;
            }
            match c.dec.read_from(&mut c.stream) {
                Ok(0) => {
                    self.disconnect(id);
                    return;
                }
                Ok(n) => total += n,
                Err(e) if e.kind() == io::ErrorKind::WouldBlock => {
                    self.process_messages(id, now);
                    return;
                }
                Err(e) if e.kind() == io::ErrorKind::Interrupted => continue,
                Err(e) => {
                    log!("client {id}: read failed: {e}");
                    self.disconnect(id);
                    return;
                }
            }
            self.process_messages(id, now);
        }
    }

    fn process_messages(&mut self, id: ClientId, now: Instant) {
        loop {
            let Some(c) = self.rt.clients.get_mut(&id) else { return };
            if c.dead {
                return;
            }
            let msg = match c.dec.next() {
                Ok(Some(m)) => m,
                Ok(None) => return,
                Err(e) => {
                    log!("client {id}: protocol error: {e}");
                    self.disconnect(id);
                    return;
                }
            };
            // What a message means depends on the connection's state:
            // Hello once, first; Command only before attaching; terminal
            // messages only while attached (after a detach the server
            // started, the client's messages in flight are ignored). A
            // message out of place is a client bug or a hostile one:
            // disconnect rather than guess (a second Hello used to
            // re-register the client and leave a zombie attachment).
            let (hello, attached, closing, tty_held) =
                (c.hello.is_some(), c.att.is_some(), c.closing, matches!(c.term_out, TermOut::Hold(_)));
            let misplaced = match &msg {
                ClientMsg::Unknown { .. } => false,
                _ if closing => {
                    continue;
                }
                ClientMsg::Hello(_) => hello,
                _ if !hello => true,
                ClientMsg::Command { .. } | ClientMsg::Stdin(_) => attached,
                ClientMsg::Resize { .. } | ClientMsg::Detach => false,
                ClientMsg::Input(_) | ClientMsg::Suspend | ClientMsg::Resume { .. } => !attached,
                ClientMsg::UseTty { .. } => !attached || !tty_held,
            };
            if misplaced {
                log!("client {id}: protocol error: {} out of place", client_msg_name(&msg));
                self.disconnect(id);
                return;
            }
            match msg {
                ClientMsg::Hello(h) => self.hello(id, h),
                ClientMsg::Command { argv } => self.command(id, argv),
                ClientMsg::Stdin(data) => {
                    if let Some(c) = self.rt.clients.get_mut(&id) {
                        c.stdin = Some(data);
                    }
                }
                ClientMsg::Input(bytes) => self.client_input(id, &bytes, now),
                ClientMsg::Resize { size, pixels: _ } => self.client_size(id, size),
                ClientMsg::Detach => {
                    self.disconnect(id);
                    return;
                }
                ClientMsg::UseTty { path, dev } => self.use_tty(id, &path, dev),
                ClientMsg::Suspend => {
                    if let Err(e) = self.rt.suspend(id) {
                        log!("client {id}: protocol error: {e}");
                        self.disconnect(id);
                        return;
                    }
                }
                ClientMsg::Resume { size, pixels: _ } => {
                    self.client_size(id, size);
                    self.rt.resume(id);
                }
                ClientMsg::Unknown { .. } => {}
            }
        }
    }

    /// The client's terminal changed size (`Resize`, `Resume`).
    fn client_size(&mut self, id: ClientId, size: lif_types::Size) {
        let size = lif_core::clamp_size(size);
        let Some(c) = self.rt.clients.get_mut(&id) else { return };
        if let Some(h) = c.hello.as_mut() {
            h.size = size;
        }
        let Some(a) = c.att.as_mut() else { return };
        a.renderer.invalidate();
        a.needs_frame = true;
        a.status_stale = true;
        self.model.client_resized(id, size, &mut self.rt);
        self.after_model();
    }

    /// The attached client's terminal is in raw mode and is `path`: open
    /// it and use it directly (see `client::TermOut`), or relay if that
    /// fails. Either way the terminal bytes held back so far go out now.
    ///
    /// Direct use needs the server to know, before every read and write,
    /// that the terminal is still the client's (`sys::TtyOwner`): so the
    /// terminal must be the client process's controlling terminal with the
    /// client in its foreground, and no other client may be using it.
    /// Otherwise the client relays, which is always safe: a stopped client
    /// reads nothing.
    fn use_tty(&mut self, id: ClientId, path: &str, dev: u64) {
        let taken = self.rt.clients.values().any(|o| o.id != id && o.tty().is_some_and(|t| !t.dead && t.owner.rdev() == dev));
        let Some(c) = self.rt.clients.get_mut(&id) else { return };
        let token = Token(TTY_TOKEN | id.0 as usize);
        let mut tty = None;
        if !path.is_empty() {
            let owner = match (taken, c.peer.pid) {
                (true, _) => Err(io::Error::other("another client uses it")),
                (false, Some(pid)) => sys::TtyOwner::new(pid, dev),
                (false, None) => Err(io::Error::other("the client process is unknown")),
            };
            match owner.and_then(|o| sys::open_tty(path, dev).map(|fd| (fd, o))) {
                Ok((fd, owner)) => {
                    let mut t = Tty::new(fd, token, owner);
                    crate::host::tty_interest(&self.rt.registry, &mut t);
                    if t.interest.is_some() {
                        tty = Some(t);
                    } else {
                        log!("client {id}: cannot poll {path}");
                    }
                }
                Err(e) => log!("client {id}: cannot use {path}: {e}"),
            }
        }
        let direct = tty.is_some();
        let held = match std::mem::replace(&mut c.term_out, tty.map_or(TermOut::Relay, TermOut::Direct)) {
            TermOut::Hold(h) => h,
            _ => Vec::new(),
        };
        c.send(&ServerMsg::Tty { direct });
        c.send_output(&held);
        c.arm_panic();
        if let Some(a) = c.att.as_mut() {
            a.needs_frame = true;
        }
        log!("client {id}: terminal {}", if direct { path } else { "relayed" });
        self.rt.flush_client(id);
    }

    /// Keys from a client's terminal the server reads itself: everything
    /// it has, up to one buffer, as one parser feed.
    fn read_tty(&mut self, id: ClientId, now: Instant) {
        let mut buf = std::mem::take(&mut self.tty_buf);
        buf.resize(TTY_READ_BUF, 0);
        let mut n = 0;
        let mut gone = false;
        let mut background = false;
        if let Some(c) = self.rt.clients.get_mut(&id) {
            let paused = c.dead || c.suspended();
            if let Some(t) = c.tty_mut()
                && !t.dead
                && t.reading
                && !paused
            {
                t.read_more = false;
                // Never read a terminal the client is not the foreground
                // of: what is typed there now is for the shell.
                if !t.usable() {
                    background = true;
                } else {
                    let fd = t.fd.as_raw_fd();
                    loop {
                        if n == buf.len() {
                            t.read_more = true;
                            break;
                        }
                        // SAFETY: the slice is valid for its length; the fd is ours.
                        let r = unsafe { libc::read(fd, buf[n..].as_mut_ptr().cast(), buf.len() - n) };
                        if r > 0 {
                            n += r as usize;
                            continue;
                        }
                        if r == 0 {
                            gone = true;
                            break;
                        }
                        match io::Error::last_os_error().kind() {
                            io::ErrorKind::WouldBlock => break,
                            io::ErrorKind::Interrupted => {}
                            _ => {
                                gone = true;
                                break;
                            }
                        }
                    }
                    if gone {
                        t.dead = true;
                        t.read_more = false;
                    }
                }
                crate::host::tty_interest(&self.rt.registry, t);
            }
        }
        if background {
            log!("client {id}: not in the foreground of its terminal; leaving it alone");
            if let Some(a) = self.rt.clients.get_mut(&id).and_then(|c| c.att.as_mut()) {
                a.suspended.get_or_insert(crate::client::Suspension::Background);
            }
            self.rt.end_paste(id);
        }
        if n > 0 {
            self.client_input(id, &buf[..n], now);
        }
        self.tty_buf = buf;
        if gone {
            log!("client {id}: terminal hung up");
            self.disconnect(id);
        }
    }

    fn hello(&mut self, id: ClientId, mut h: lif_proto::Hello) {
        // Only what a lif client may send reaches session environments
        // (and so every later pane): never LD_PRELOAD, PATH, SHELL...
        h.size = lif_core::clamp_size(h.size);
        h.env.retain(|(k, v)| {
            (lif_proto::ENV_ALLOW_LIST.contains(&k.as_str()) || k == "LIF_PANE") && !v.contains('\0') && v.len() <= 4096
        });
        let Some(c) = self.rt.clients.get_mut(&id) else { return };
        c.caps = SERVER_CAPS.intersect(h.caps);
        c.send(&ServerMsg::Welcome {
            protocol: PROTOCOL_VERSION,
            caps: c.caps,
            server_version: env!("CARGO_PKG_VERSION").into(),
            server_pid: std::process::id(),
        });
        if h.protocol != PROTOCOL_VERSION {
            log!("client {id}: protocol {} rejected", h.protocol);
            c.closing = true;
            self.rt.flush_client(id);
            return;
        }
        let info = ClientInfo {
            name: format!("client-{}", h.pid),
            term: h.term.clone(),
            size: h.size,
            cwd: h.cwd.clone(),
            pid: h.pid,
            terminal: h.is_tty,
        };
        c.hello = Some(h);
        self.model.client_connect(id, info);
        self.rt.flush_client(id);
    }

    fn command(&mut self, id: ClientId, argv: Vec<String>) {
        let was_attached = self.rt.clients.get(&id).is_some_and(|c| c.att.is_some());
        self.rt.cmd_stdin = self.rt.clients.get_mut(&id).and_then(|c| c.stdin.take());
        let result = lif_core::parse_list(&argv).and_then(|list| self.run_commands(Some(id), &list));
        self.rt.cmd_stdin = None;
        let Some(c) = self.rt.clients.get_mut(&id) else { return };
        let attached_now = c.att.is_some() && !was_attached;
        match result {
            Ok(out) if attached_now => {
                if !out.stdout.is_empty() {
                    log!("client {id}: output of an attaching command dropped");
                }
            }
            // The answer comes when the commands have finished waiting
            // (`Server::deliver`).
            Ok(out) if out.pending => {}
            Ok(out) => c.send_command_done(out.status, out.stdout.into_bytes(), Vec::new()),
            Err(e) if attached_now => {
                let text = e.to_string();
                lif_core::Host::show_message(&mut self.rt, id, &text, 750);
            }
            Err(e) => c.send_command_done(1, Vec::new(), format!("{e}\n").into_bytes()),
        }
        self.rt.flush_client(id);
    }

    /// Runs commands for `client` (or with no client), keeping the pane
    /// facts formats read fresh and the views in step afterwards.
    pub(crate) fn run_commands(&mut self, client: Option<ClientId>, list: &CommandList) -> Result<lif_core::CmdOutput, lif_core::CmdError> {
        let pane = client
            .and_then(|c| self.rt.clients.get(&c))
            .filter(|c| c.att.is_none())
            .and_then(|c| c.hello.as_ref())
            .and_then(|h| h.env.iter().find(|(k, _)| k == "LIF_PANE"))
            .and_then(|(_, v)| PaneId::parse(v));
        self.run_commands_in(client, pane, list)
    }

    /// [`Server::run_commands`] with `pane` as the current pane.
    pub(crate) fn run_commands_in(
        &mut self,
        client: Option<ClientId>,
        pane: Option<PaneId>,
        list: &CommandList,
    ) -> Result<lif_core::CmdOutput, lif_core::CmdError> {
        self.refresh_pane_facts();
        let ctx = CmdContext { client, pane, now: sys::wall_clock() };
        // A copy mode the commands open gets the settings of the pane they
        // most likely target. (Only then: compiling the copy-mode key table
        // parses its defaults, which a server that never enters copy mode
        // does without.)
        // (Without a client or a pane to go by, any pane: the settings are
        // nearly always global, and after_model corrects the rest.)
        let has = |name: &str| list.commands().iter().any(|c| c.name() == name);
        if has("copy-mode") {
            let target = pane
                .or_else(|| client.and_then(|c| self.rt.clients.get(&c)?.att.as_ref()?.active_pane))
                .or_else(|| self.model.panes().next().map(|p| p.id()));
            if let Some(p) = target {
                self.sync_mode_config(p);
            }
        }
        // Keys sent into a mode go through its key table, which options
        // changed since the mode was entered (mode-keys, bindings) affect.
        if has("send-keys") {
            let in_mode: Vec<PaneId> = self.rt.panes.values().filter(|p| p.mode.is_some()).map(|p| p.id).collect();
            for p in in_mode {
                self.sync_mode_config(p);
            }
        }
        let r = self.model.execute_list(list, &ctx, &mut self.rt);
        // Options may have changed the styles views are drawn with, and
        // the scrollback limits of existing panes.
        if list.commands().iter().any(|c| matches!(c.name(), "set-option" | "set-window-option" | "source-file")) {
            self.rt.views_stale = true;
            self.apply_options();
        }
        self.after_model();
        r
    }

    /// Options the loop keeps in its own form: `escape-time`, and
    /// `scrollback-bytes` / `history-limit` of each pane's session, applied
    /// to panes whose limits differ (tmux applies `history-limit` only to
    /// new panes; lif's engine can change them in place).
    fn apply_options(&mut self) {
        if let Some(ms) = self.model.option_text("escape-time", None, None).and_then(|v| v.parse::<u64>().ok()) {
            self.rt.escape_time = Duration::from_millis(ms);
        }
        for p in self.rt.panes.values_mut() {
            let Some(sid) = self.model.pane(p.id).and_then(|m| self.model.window(m.window())).map(|w| w.session()) else { continue };
            let num = |name: &str| self.model.option_text(name, Some(sid), None).and_then(|v| v.parse::<usize>().ok());
            let (Some(bytes), Some(lines)) = (num("scrollback-bytes"), num("history-limit")) else { continue };
            let want = crate::pane::scrollback_limits(bytes, lines);
            if want != p.scrollback {
                p.scrollback = want;
                p.engine.set_scrollback_limits(want.0, want.1);
            }
        }
        // monitor-silence turned on counts from now; turned off, it stops.
        let now = Instant::now();
        for p in self.rt.panes.values_mut() {
            match self.model.silence_secs(p.id) {
                0 => p.silence_at = None,
                secs if p.silence_at.is_none() => p.silence_at = Some(now + Duration::from_secs(secs)),
                _ => {}
            }
        }
    }

    /// After any model call: drop a respawn stash nobody took, forget
    /// environments of sessions that are gone, bring views up to date.
    pub(crate) fn after_model(&mut self) {
        self.rt.respawn_stash = None;
        // destroy-unattached: sessions the last client just left.
        self.model.check_unattached(&mut self.rt);
        // Popups that closed: the commands that waited for them go on.
        while !self.rt.popups_done.is_empty() {
            for (job, status) in std::mem::take(&mut self.rt.popups_done) {
                self.model.job_done(job, lif_core::JobStatus::Exited(status), &[], sys::wall_clock(), &mut self.rt);
            }
            self.rt.views_stale = true;
            self.apply_options();
        }
        if self.model.has_finished() {
            for f in self.model.take_finished() {
                self.deliver(f);
            }
        }
        if self.model.has_hooks() {
            // Hooks of events (a pane exited, a client resized); their
            // commands may have been anything.
            self.model.run_hooks(sys::wall_clock(), &mut self.rt);
            self.rt.views_stale = true;
            self.apply_options();
            for f in self.model.take_finished() {
                self.deliver(f);
            }
        }
        self.model.start_format_jobs(sys::wall_clock(), &mut self.rt);
        self.sync_choosers();
        if !self.rt.mode_actions.is_empty() {
            for (c, p, a) in std::mem::take(&mut self.rt.mode_actions) {
                self.mode_action(c, p, a);
            }
        }
        if !self.rt.mode_changed.is_empty() {
            for p in std::mem::take(&mut self.rt.mode_changed) {
                // A mode entered during the call got the settings at hand,
                // which may be another window's: give it its own.
                if self.rt.panes.get(&p).is_some_and(|pr| pr.mode.is_some()) {
                    self.sync_mode_config(p);
                }
                self.model.pane_changed(p, &mut self.rt);
            }
        }
        if self.rt.session_env.len() > self.model.sessions().count() {
            let model = &self.model;
            self.rt.session_env.retain(|s, _| model.session(*s).is_some());
        }
        self.sync_views();
        if !self.rt.mode_bindings.is_empty() {
            let keys = std::mem::take(&mut self.rt.mode_bindings);
            if self.binding_depth >= MAX_BINDING_DEPTH {
                log!("copy-mode bindings nested too deep: {} keys dropped", keys.len());
                return;
            }
            self.binding_depth += 1;
            for (pane, key) in keys {
                self.run_mode_binding(None, pane, &key);
            }
            self.binding_depth -= 1;
        }
    }

    /// The foreground process name and directory of every pane, which
    /// formats (`#{pane_current_command}`, `-c '#{pane_current_path}'`)
    /// read. Only done before commands and for status-line frames: never
    /// on a timer.
    pub(crate) fn refresh_pane_facts(&mut self) {
        let mut changed = Vec::new();
        for p in self.rt.panes.values_mut() {
            let Some(pty) = p.pty.as_ref() else { continue };
            if let Some(name) = pty.foreground_process_name()
                && name != p.current_command
            {
                p.current_command = name;
                changed.push(p.id);
            }
            if let Some(cwd) = pty.foreground_cwd() {
                let cwd = cwd.to_string_lossy();
                if cwd != p.current_path.as_str() {
                    p.current_path = cwd.into_owned();
                }
            }
        }
        for p in changed {
            self.model.pane_changed(p, &mut self.rt);
        }
    }

    pub(crate) fn disconnect(&mut self, id: ClientId) {
        let Some(c) = self.rt.clients.get_mut(&id) else { return };
        if c.dead {
            return;
        }
        log!("client {id} disconnected");
        self.rt.end_paste(id);
        let Some(c) = self.rt.clients.get_mut(&id) else { return };
        c.dead = true;
        // What it showed may now be hidden (see Server::release_hidden_panes).
        self.rt.views_stale = true;
        self.model.client_disconnect(id, &mut self.rt);
        self.after_model();
    }

    fn remove_dead_clients(&mut self) {
        if !self.rt.clients.values().any(|c| c.dead) {
            return;
        }
        let dead: Vec<ClientId> = self.rt.clients.values().filter(|c| c.dead).map(|c| c.id).collect();
        for id in dead {
            if let Some(mut c) = self.rt.clients.remove(&id) {
                c.disarm_panic();
                let _ = self.rt.registry.deregister(&mut c.stream);
                if let Some(t) = c.tty()
                    && !t.dead
                {
                    let _ = self.rt.registry.deregister(&mut SourceFd(&t.fd.as_raw_fd()));
                }
                if self.model.client(id).is_some() {
                    self.model.client_disconnect(id, &mut self.rt);
                }
            }
        }
        self.rt.views_stale = true;
        self.sync_views();
    }

    /// Waits for every hung-up process to exit, at most until the last of
    /// them is due its SIGKILL, and sends that to those still there: once
    /// the server is gone nothing else would. Programs that exit on the
    /// hangup do so within milliseconds, which is all this then takes.
    fn await_hung_up(&mut self) {
        loop {
            for (pid, _) in self.rt.reaper.reap() {
                self.rt.child_reaped(pid);
            }
            let now = Instant::now();
            self.rt.kill_overdue(now);
            let Some(next) = self.rt.dying.iter().map(|d| d.1).min() else { return };
            let ms = next.saturating_duration_since(now).as_millis() as i32;
            sys::wait_readable(self.rt.reaper.as_raw_fd(), ms.max(1));
        }
    }

    /// Orderly exit: deliver what is pending to clients, hang up every
    /// pane, remove the socket and lock file.
    fn finish(&mut self) {
        for c in self.rt.clients.values_mut() {
            c.flush_blocking(500);
        }
        let panes: Vec<PaneId> = self.rt.panes.keys().copied().collect();
        for p in panes {
            lif_core::Host::kill_pane(&mut self.rt, p);
        }
        // Shell commands still running go the way of the panes.
        let jobs: Vec<u32> = self.rt.jobs.values().filter(|j| j.status.is_none()).map(|j| j.pid).collect();
        for pid in jobs {
            self.rt.hang_up(pid);
        }
        self.await_hung_up();
        self.rt.respawn_stash = None;
        // Only our own socket: if another server was started on this path
        // (it could only after this one stopped holding the lock, but a
        // user may also have moved things around), its socket stays.
        let ours = std::fs::symlink_metadata(&self.opts.socket).ok().map(|m| {
            use std::os::unix::fs::MetadataExt;
            (m.dev(), m.ino())
        });
        if ours.is_some() && ours == self.socket_id {
            let _ = std::fs::remove_file(&self.opts.socket);
        }
        // The lock file stays: removing it would let a client that already
        // opened it and one that creates a new one both get "the" lock.
        // The lock itself goes with this process.
        self.log_stats();
    }
}

/// A client message's name for the log (never its payload).
fn client_msg_name(m: &ClientMsg) -> &'static str {
    match m {
        ClientMsg::Hello(_) => "Hello",
        ClientMsg::Command { .. } => "Command",
        ClientMsg::Stdin(_) => "Stdin",
        ClientMsg::Input(_) => "Input",
        ClientMsg::Resize { .. } => "Resize",
        ClientMsg::Detach => "Detach",
        ClientMsg::UseTty { .. } => "UseTty",
        ClientMsg::Suspend => "Suspend",
        ClientMsg::Resume { .. } => "Resume",
        ClientMsg::Unknown { .. } => "Unknown",
    }
}
