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
use crate::host::{CLIENT_TOKEN, PANE_TOKEN, Rt, TTY_TOKEN};
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
const SERVER_CAPS: Caps = Caps::TTY;

/// About how long one step of a copy-mode search may take before the loop
/// looks at its descriptors again.
const SEARCH_STEP_TIME: Duration = Duration::from_millis(2);

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
    /// Reused id lists, so a loop iteration allocates nothing.
    pub(crate) scratch_panes: Vec<PaneId>,
    pub(crate) scratch_clients: Vec<ClientId>,
    pub(crate) scratch_views: Vec<ClientId>,
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
        let rt = Rt::new(poll.registry().try_clone()?, reaper, opts.socket.to_string_lossy().into_owned());
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
            scratch_panes: Vec::new(),
            scratch_clients: Vec::new(),
            scratch_views: Vec::new(),
        };
        log!("server {} started on {}", std::process::id(), s.opts.socket.display());
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
        for c in self.rt.clients.values_mut() {
            // A client waiting for a pane that is gone waits no more.
            if let Some(p) = c.blocked_on
                && !self.rt.panes.get(&p).is_some_and(|p| p.pty.is_some())
            {
                c.unblock();
            }
            if c.read_more && c.blocked_on.is_none() {
                clients.push(c.id);
            }
        }
        for &c in &clients {
            self.read_client(c, now);
        }
        clients.clear();
        clients.extend(self.rt.clients.values().filter(|c| c.tty_read_more()).map(|c| c.id));
        for &c in &clients {
            self.read_tty(c, now);
        }
        self.scratch_clients = clients;
        self.timers(now);
        self.step_searches();
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
            min(p.name_recheck);
            min(p.autoscroll_at);
        }
        for c in self.rt.clients.values() {
            if (c.read_more && c.blocked_on.is_none()) || c.tty_read_more() {
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
        d
    }

    fn accept(&mut self) {
        loop {
            match self.listener.accept() {
                Ok((mut stream, _)) => {
                    let id = ClientId(self.next_client);
                    self.next_client += 1;
                    let token = Token(CLIENT_TOKEN | id.0 as usize);
                    if let Err(e) = self.poll.registry().register(&mut stream, token, Interest::READABLE) {
                        log!("register client: {e}");
                        continue;
                    }
                    self.first_client_deadline = None;
                    self.rt.clients.insert(id, ClientRt::new(id, stream, token));
                    // Data may already be waiting: the edge was before registration.
                    if let Some(c) = self.rt.clients.get_mut(&id) {
                        c.read_more = true;
                    }
                }
                Err(e) if e.kind() == io::ErrorKind::WouldBlock => return,
                Err(e) if e.kind() == io::ErrorKind::Interrupted => {}
                Err(e) => {
                    log!("accept: {e}");
                    return;
                }
            }
        }
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
        let mut vt = 0;
        for p in self.rt.panes.values() {
            vt += p.engine.memory().resident_bytes;
        }
        log!(
            "stats: iterations={} panes={} clients={} rss={} vt_bytes={}",
            self.iterations,
            self.rt.panes.len(),
            self.rt.clients.len(),
            sys::rss_bytes(),
            vt
        );
    }

    // ----- panes ---------------------------------------------------------

    fn pane_writable(&mut self, id: PaneId) {
        self.rt.flush_pane(id);
        let Some(p) = self.rt.panes.get(&id) else { return };
        if p.wq.len() <= crate::PANE_WRITE_HIGH / 2 {
            for c in self.rt.clients.values_mut() {
                if c.blocked_on == Some(id) {
                    c.unblock();
                }
            }
        }
    }

    pub(crate) fn read_pane(&mut self, id: PaneId, now: Instant, budget: usize) {
        let Some(p) = self.rt.panes.get_mut(&id) else { return };
        let r = p.read(&mut self.read_buf, budget);
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
            let Some(id) = self.rt.pids.remove(&pid) else { continue };
            log!("pane {id} pid {pid} exited: {status:?}");
            // Output the program wrote just before exiting is still in the PTY.
            self.read_pane(id, now, usize::MAX);
            self.pane_gone(id, status.code());
        }
    }

    /// The pane's process is gone: tell the model, then drop what the model
    /// no longer knows (or just the PTY of a pane kept by remain-on-exit).
    fn pane_gone(&mut self, id: PaneId, code: Option<i32>) {
        self.model.pane_exited(id, code, &mut self.rt);
        self.after_model();
        if self.model.pane(id).is_none() {
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

    // ----- clients -------------------------------------------------------

    fn read_client(&mut self, id: ClientId, now: Instant) {
        let mut total = 0;
        loop {
            let Some(c) = self.rt.clients.get_mut(&id) else { return };
            c.read_more = false;
            if c.dead || c.blocked_on.is_some() {
                c.read_more = c.blocked_on.is_some();
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
            if c.dead || c.blocked_on.is_some() {
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
            match msg {
                ClientMsg::Hello(h) => self.hello(id, h),
                ClientMsg::Command { argv } => self.command(id, argv),
                ClientMsg::Input(bytes) => self.client_input(id, &bytes, now),
                ClientMsg::Resize { size, pixels: _ } => {
                    if let Some(c) = self.rt.clients.get_mut(&id) {
                        if let Some(h) = c.hello.as_mut() {
                            h.size = size;
                        }
                        if let Some(a) = c.att.as_mut() {
                            a.renderer.invalidate();
                            a.needs_frame = true;
                            a.status_stale = true;
                        }
                    }
                    self.model.client_resized(id, size, &mut self.rt);
                    self.after_model();
                }
                ClientMsg::Detach => {
                    self.disconnect(id);
                    return;
                }
                ClientMsg::UseTty { path, dev } => self.use_tty(id, &path, dev),
                ClientMsg::Unknown { .. } => {}
            }
        }
    }

    /// The attached client's terminal is in raw mode and is `path`: open
    /// it and use it directly (see `client::TermOut`), or relay if that
    /// fails. Either way the terminal bytes held back so far go out now.
    fn use_tty(&mut self, id: ClientId, path: &str, dev: u64) {
        let Some(c) = self.rt.clients.get_mut(&id) else { return };
        if !matches!(c.term_out, TermOut::Hold(_)) {
            return;
        }
        let token = Token(TTY_TOKEN | id.0 as usize);
        let mut tty = None;
        if !path.is_empty() {
            match sys::open_tty(path, dev) {
                Ok(fd) => match self.rt.registry.register(&mut SourceFd(&fd.as_raw_fd()), token, Interest::READABLE) {
                    Ok(()) => tty = Some(Tty::new(fd, token)),
                    Err(e) => log!("client {id}: cannot poll {path}: {e}"),
                },
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
        if let Some(c) = self.rt.clients.get_mut(&id) {
            let paused = c.dead || c.blocked_on.is_some();
            if let Some(t) = c.tty_mut()
                && !t.dead
                && !paused
            {
                t.read_more = false;
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
                    let _ = self.rt.registry.deregister(&mut SourceFd(&fd));
                }
            }
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

    fn hello(&mut self, id: ClientId, h: lif_proto::Hello) {
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
        let result = lif_core::parse_list(&argv).and_then(|list| self.run_commands(Some(id), &list));
        let Some(c) = self.rt.clients.get_mut(&id) else { return };
        let attached_now = c.att.is_some() && !was_attached;
        match result {
            Ok(out) if attached_now => {
                if !out.stdout.is_empty() {
                    log!("client {id}: output of an attaching command dropped");
                }
            }
            Ok(out) => c.send(&ServerMsg::CommandDone { status: 0, stdout: out.stdout.into_bytes(), stderr: Vec::new() }),
            Err(e) if attached_now => {
                let text = e.to_string();
                lif_core::Host::show_message(&mut self.rt, id, &text, 750);
            }
            Err(e) => c.send(&ServerMsg::CommandDone { status: 1, stdout: Vec::new(), stderr: format!("{e}\n").into_bytes() }),
        }
        self.rt.flush_client(id);
    }

    /// Runs commands for `client` (or with no client), keeping the pane
    /// facts formats read fresh and the views in step afterwards.
    pub(crate) fn run_commands(&mut self, client: Option<ClientId>, list: &CommandList) -> Result<lif_core::CmdOutput, lif_core::CmdError> {
        self.refresh_pane_facts();
        let pane = client
            .and_then(|c| self.rt.clients.get(&c))
            .filter(|c| c.att.is_none())
            .and_then(|c| c.hello.as_ref())
            .and_then(|h| h.env.iter().find(|(k, _)| k == "LIF_PANE"))
            .and_then(|(_, v)| PaneId::parse(v));
        let ctx = CmdContext { client, pane, now: sys::wall_clock() };
        // A copy mode the commands open gets the settings of the pane they
        // most likely target. (Only then: compiling the copy-mode key table
        // parses its defaults, which a server that never enters copy mode
        // does without.)
        let target = pane.or_else(|| client.and_then(|c| self.rt.clients.get(&c)?.att.as_ref()?.active_pane));
        if let Some(p) = target
            && list.commands().iter().any(|c| c.name() == "copy-mode")
        {
            self.sync_mode_config(p);
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
    }

    /// After any model call: drop a respawn stash nobody took, forget
    /// environments of sessions that are gone, bring views up to date.
    pub(crate) fn after_model(&mut self) {
        self.rt.respawn_stash = None;
        if !self.rt.mode_actions.is_empty() {
            for (c, p, a) in std::mem::take(&mut self.rt.mode_actions) {
                self.mode_action(c, p, a);
            }
        }
        if !self.rt.mode_changed.is_empty() {
            let key = self.rt.copy_cfg.key.clone();
            for p in std::mem::take(&mut self.rt.mode_changed) {
                if let Some(pr) = self.rt.panes.get_mut(&p)
                    && pr.mode.is_some()
                {
                    pr.mode_cfg = Some(key.clone());
                }
                self.model.pane_changed(p, &mut self.rt);
            }
        }
        if self.rt.session_env.len() > self.model.sessions().count() {
            let model = &self.model;
            self.rt.session_env.retain(|s, _| model.session(*s).is_some());
        }
        self.sync_views();
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
        c.dead = true;
        log!("client {id} disconnected");
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
        self.rt.respawn_stash = None;
        let _ = std::fs::remove_file(&self.opts.socket);
        if let Some(l) = &self.opts.lock {
            let _ = std::fs::remove_file(l);
        }
        self.log_stats();
    }
}
