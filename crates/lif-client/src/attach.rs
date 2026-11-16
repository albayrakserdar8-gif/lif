//! The attached relay loop.
//!
//! One thread, one poll(2) over four fds: the signal self-pipe, the socket,
//! the terminal input and (only while it is full) the terminal output.
//!
//! Direct terminal. When both sides announced `Caps::TTY`, the client
//! names its terminal in `UseTty` once it is in raw mode, and if the server
//! answers `Tty { direct: true }` the server reads and writes the terminal
//! itself: the client then reads nothing from it and only forwards
//! resizes, signals and the server's `Exit` (and still writes any
//! `Output`). Until the answer the terminal is not read at all, so no key
//! can go both ways.
//!
//! Bounds: bytes to the server are queued only up to [`SOCK_HIGH`] plus one
//! read. Bytes to the terminal are never copied out of the decoder: `Output`
//! data is written straight from the decoder buffer as it arrives, what the
//! terminal did not take stays there, and the socket is not read again until
//! the terminal takes it. So the client holds at most one read
//! (`lif_proto::READ_CHUNK`) of output, and a slow terminal pushes back on
//! the server through the socket buffer.
//!
//! Leaving. When the server sends `Exit` it has already sent the bytes that
//! restore the outer terminal. When the client leaves on its own the server
//! still owns that knowledge, so on SIGTERM / SIGINT / SIGQUIT the client
//! sends `Detach` and keeps relaying until `Exit` (or the socket closes),
//! for at most [`SERVER_WAIT`]. Only then, if the server did not restore
//! the terminal, it writes [`FALLBACK_RESTORE`] itself. That phase is the
//! only one with poll timeouts: the steady state still has no timers. After
//! SIGHUP or end of input the terminal is gone: nothing is written to it and
//! `Detach` gets [`HANGUP_FLUSH`] at most.

use std::io;
use std::os::fd::{AsRawFd, RawFd};
use std::time::{Duration, Instant};

use lif_proto::{encode_input, type_id, Caps, ClientMsg, ExitReason, ServerMsg};
use lif_types::{PixelSize, Size};

use crate::conn::Connection;
use crate::signals::{self, SignalPipe};
use crate::sys;
use crate::term::{RawGuard, FALLBACK_RESTORE};

/// Size of the terminal read buffer.
const READ_BUF: usize = 64 * 1024;

/// Stop reading the terminal while this much is waiting for the socket.
const SOCK_HIGH: usize = 64 * 1024;

/// How long a client leaving on a signal waits for the server's `Exit`
/// (relaying the restore bytes before it). A healthy local server answers
/// in well under a millisecond and its restore sequence is a few hundred
/// bytes, so this only expires when the server is hung or a large frame is
/// stuck behind a terminal that does not read; it is short enough that
/// `kill` still feels immediate and far below the SIGTERM-to-SIGKILL grace
/// of service managers and shells closing.
const SERVER_WAIT: Duration = Duration::from_millis(400);

/// Upper bound for the whole shutdown on a signal: [`SERVER_WAIT`], then
/// the rest for writing [`FALLBACK_RESTORE`] (about 150 bytes, which a
/// live terminal takes at once).
const SHUTDOWN: Duration = Duration::from_millis(500);

/// After a hangup only `Detach` is left to send; it normally goes out at
/// once and this bounds the case of a socket buffer full of input.
const HANGUP_FLUSH: Duration = Duration::from_millis(100);

/// How long a client being suspended waits for the server to give the
/// terminal back (`Suspended`) before it stops anyway.
const SUSPEND_WAIT: Duration = Duration::from_millis(500);

/// The terminal the loop drives. Separate fds so tests can use a PTY.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub struct TermFds {
    /// Keyboard bytes are read from here.
    pub input: RawFd,
    /// Server output is written here.
    pub output: RawFd,
    /// The terminal whose modes are switched to raw and whose size is
    /// reported on SIGWINCH.
    pub tty: RawFd,
}

impl TermFds {
    /// stdin / stdout, with stdin as the terminal.
    pub const fn stdio() -> TermFds {
        TermFds { input: 0, output: 1, tty: 0 }
    }
}

/// Who ended the attachment.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum ExitCause {
    /// The server sent `Exit`.
    Server,
    /// A terminating signal (SIGTERM, SIGINT, SIGQUIT, SIGHUP); the client
    /// sent `Detach`. Except after SIGHUP it waited for the server to
    /// restore the terminal, and restored it itself if the server did not
    /// answer; `reason` and `message` come from the server's `Exit` if
    /// it answered.
    Signal(i32),
    /// The terminal input reached end of file or failed (the terminal went
    /// away); the client sent `Detach`.
    InputClosed,
}

#[derive(Clone, Debug, PartialEq, Eq)]
pub struct ExitInfo {
    pub cause: ExitCause,
    /// From the server's `Exit`; `Detached` when the client left on its own.
    pub reason: ExitReason,
    /// From the server's `Exit`; already printed to the terminal.
    pub message: String,
    /// A terminating signal received while attached, including one that
    /// arrived during cleanup after the server's `Exit`. [`attach_loop`]
    /// re-raises it before returning; callers of [`attach_loop_with`]
    /// decide themselves.
    pub signal: Option<i32>,
}

impl ExitInfo {
    fn detached(cause: ExitCause) -> ExitInfo {
        ExitInfo { cause, reason: ExitReason::Detached, message: String::new(), signal: None }
    }
}

/// [`attach_loop_with`] on stdin / stdout, for the lif binary.
///
/// If a terminating signal ended (or arrived during the end of) the
/// attachment, it is re-raised once the terminal is restored, under the
/// disposition the process had before attaching. With the default one the
/// process dies by that signal, which is what its parent (a shell, a
/// service manager) expects of a process it sent SIGTERM; if the
/// disposition was "ignore" this returns normally. A normal detach returns.
pub fn attach_loop(conn: Connection) -> io::Result<ExitInfo> {
    let (result, signal) = attach(conn, TermFds::stdio());
    if let Some(sig) = signal {
        signals::reraise(sig);
    }
    result
}

/// Relays between the terminal and the server until the server sends
/// `Exit`, a signal or terminal hangup detaches, or an error occurs. The
/// terminal is in raw mode for the duration and restored on every path
/// (including panics); the `Exit` message is printed after restoring.
///
/// The outer terminal's modes (alternate screen, mouse, keyboard
/// protocol, ...) are restored by the server's bytes when it answers, and
/// with [`FALLBACK_RESTORE`] when it does not (it closed the connection,
/// broke the protocol, or did not answer `Detach` within the shutdown
/// timeout), unless the terminal itself is gone.
pub fn attach_loop_with(conn: Connection, fds: TermFds) -> io::Result<ExitInfo> {
    attach(conn, fds).0
}

/// The attachment and its cleanup; also returns the terminating signal to
/// re-raise, if any (on the error path too).
fn attach(mut conn: Connection, fds: TermFds) -> (io::Result<ExitInfo>, Option<i32>) {
    let signals = match SignalPipe::install() {
        Ok(s) => s,
        Err(e) => return (Err(e), None),
    };
    let guard = match RawGuard::enter(fds.tty, fds.input, fds.output) {
        Ok(g) => g,
        Err(e) => return (Err(e), signals.finish()),
    };
    let mut guard = guard;
    let mut relay = Relay::new();
    let result = match conn.stream.set_nonblocking(true) {
        Ok(()) => relay.attach(&mut conn, &fds, &signals, &mut guard),
        Err(e) => Err(e),
    };
    drop(guard);
    // A signal that arrived after the loop stopped looking (e.g. right
    // behind the server's Exit) is still in the pipe.
    let late = signals.finish();
    let signal = relay.signal.or(late);
    let exit = match result {
        Ok(exit) => exit,
        Err(e) => return (Err(e), signal),
    };
    if !exit.message.is_empty() {
        let mut line = exit.message.clone().into_bytes();
        line.push(b'\n');
        // The session is over either way; a terminal that cannot take the
        // message must not turn a clean exit into an error.
        let _ = sys::write_all(fds.output, &line);
    }
    (Ok(ExitInfo { signal, ..exit }), signal)
}

/// How the steady-state loop ended.
enum End {
    /// The server sent `Exit`.
    Exit(ExitInfo),
    /// SIGTERM / SIGINT / SIGQUIT: the terminal is still there.
    Leave(i32),
    /// The terminal is gone (SIGHUP, input EOF or error).
    Hangup(ExitCause),
}

struct Relay {
    inbuf: Vec<u8>,
    /// Frames for the server; `sock_out[sock_pos..]` is unsent.
    sock_out: Vec<u8>,
    sock_pos: usize,
    /// The terminal returned EAGAIN; wait for POLLOUT, do not read the socket.
    out_blocked: bool,
    input_open: bool,
    resize_pending: bool,
    last_size: (Size, PixelSize),
    /// The terminal failed or hung up: never write to it again.
    term_dead: bool,
    /// The terminating signal that ended the loop.
    signal: Option<i32>,
    tty: TtyUse,
    suspend: Suspend,
    /// The server sent `Suspended`: stop.
    got_suspended: bool,
    /// The SIGCONT that continued our own stop is still in the pipe.
    skip_cont: bool,
    /// Writing to the socket failed with EPIPE / ECONNRESET: the server
    /// closed it, usually right after an `Exit` we have not read yet.
    sock_shut: bool,
}

/// Job control (see `Relay::stop`).
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
enum Suspend {
    Running,
    /// SIGTSTP: `Suspend` sent, waiting for `Suspended` until then.
    Asked(Instant),
    /// Stopped, or continued in the background (`bg`) and about to stop
    /// again: the terminal is the shell's.
    Background,
}

/// Who reads the terminal.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
enum TtyUse {
    /// The client, relaying keys as `Input`.
    Relay,
    /// `UseTty` sent, no answer yet: nobody.
    Asked,
    /// The server.
    Server,
}

impl Relay {
    fn new() -> Relay {
        Relay {
            inbuf: vec![0; READ_BUF],
            sock_out: Vec::with_capacity(SOCK_HIGH + READ_BUF + 64),
            sock_pos: 0,
            out_blocked: false,
            input_open: true,
            resize_pending: false,
            last_size: Default::default(),
            term_dead: false,
            signal: None,
            tty: TtyUse::Relay,
            suspend: Suspend::Running,
            got_suspended: false,
            skip_cont: false,
            sock_shut: false,
        }
    }

    fn sock_pending(&self) -> usize {
        self.sock_out.len() - self.sock_pos
    }

    /// The loop and the way out of it. Writes [`FALLBACK_RESTORE`] on every
    /// path where the terminal is alive and the server did not restore it.
    fn attach(&mut self, conn: &mut Connection, fds: &TermFds, signals: &SignalPipe, guard: &mut RawGuard) -> io::Result<ExitInfo> {
        let sock = conn.stream.as_raw_fd();
        match self.run(conn, fds, signals, guard) {
            Ok(End::Exit(exit)) => Ok(exit),
            Ok(End::Leave(sig)) => {
                self.signal = Some(sig);
                let start = Instant::now();
                let cause = ExitCause::Signal(sig);
                match self.shutdown(conn, fds, signals, start + SERVER_WAIT) {
                    Ok(Some(exit)) => Ok(ExitInfo { cause, ..exit }),
                    // Timed out, or the server is gone: we leave anyway.
                    Ok(None) | Err(_) => {
                        self.write_fallback(fds.output, start + SHUTDOWN);
                        Ok(ExitInfo::detached(cause))
                    }
                }
            }
            Ok(End::Hangup(cause)) => {
                if let ExitCause::Signal(sig) = cause {
                    self.signal = Some(sig);
                }
                // Best effort: the server also notices the closed socket.
                let _ = ClientMsg::Detach.encode(&mut self.sock_out);
                let _ = self.flush_sock_until(sock, Instant::now() + HANGUP_FLUSH);
                Ok(ExitInfo::detached(cause))
            }
            Err(e) => {
                self.write_fallback(fds.output, Instant::now() + SHUTDOWN);
                Err(e)
            }
        }
    }

    fn run(&mut self, conn: &mut Connection, fds: &TermFds, signals: &SignalPipe, guard: &mut RawGuard) -> io::Result<End> {
        // The terminal may have changed size between Hello and now.
        self.last_size = (conn.hello_size, conn.hello_pixels);
        self.resize_pending = true;
        let sock = conn.stream.as_raw_fd();
        if conn.caps().contains(Caps::TTY) {
            let (path, dev) = sys::tty_identity(fds.tty, fds.input, fds.output).unwrap_or_default();
            ClientMsg::UseTty { path, dev }.encode(&mut self.sock_out)?;
            self.tty = TtyUse::Asked;
        }
        loop {
            if let Some(exit) = self.write_frames(conn, fds)? {
                return Ok(End::Exit(exit));
            }
            let asked_too_long = matches!(self.suspend, Suspend::Asked(t) if Instant::now() >= t);
            if std::mem::take(&mut self.got_suspended) || asked_too_long {
                self.stop(conn, fds, guard)?;
                continue;
            }
            if self.resize_pending && self.sock_pending() < SOCK_HIGH && self.suspend == Suspend::Running {
                self.resize_pending = false;
                if let Some((size, pixels)) = self.size_changed(fds.tty) {
                    ClientMsg::Resize { size, pixels }.encode(&mut self.sock_out)?;
                }
            }
            self.flush_sock(sock)?;

            let want_input =
                self.input_open && self.sock_pending() < SOCK_HIGH && self.tty == TtyUse::Relay && self.suspend == Suspend::Running;
            let mut sock_events = 0;
            if !self.out_blocked {
                sock_events |= libc::POLLIN;
            }
            if self.sock_pending() > 0 {
                sock_events |= libc::POLLOUT;
            }
            // fd -1 makes poll skip an entry; a hung-up fd we are not
            // reading would otherwise report POLLHUP forever (a busy loop).
            let entry = |fd: RawFd, events: libc::c_short| libc::pollfd { fd: if events == 0 { -1 } else { fd }, events, revents: 0 };
            let mut pfds = [
                entry(signals.fd(), libc::POLLIN),
                entry(sock, sock_events),
                entry(fds.input, if want_input { libc::POLLIN } else { 0 }),
                entry(fds.output, if self.out_blocked { libc::POLLOUT } else { 0 }),
            ];
            // Only a suspension being asked for has a timeout.
            let timeout = match self.suspend {
                Suspend::Asked(t) => sys::poll_ms(t).unwrap_or(0),
                _ => -1,
            };
            // SAFETY: pfds is a valid array of pollfd of the given length.
            let r = unsafe { libc::poll(pfds.as_mut_ptr(), pfds.len() as libc::nfds_t, timeout) };
            if r < 0 {
                let e = io::Error::last_os_error();
                if e.kind() == io::ErrorKind::Interrupted {
                    continue;
                }
                return Err(e);
            }
            let ready = |i: usize| pfds[i].revents != 0;

            if ready(0) {
                let mut leave = None;
                let (mut tstp, mut cont) = (false, false);
                signals.drain(|sig| match sig {
                    libc::SIGWINCH => self.resize_pending = true,
                    libc::SIGTSTP => tstp = true,
                    libc::SIGCONT => cont = true,
                    // A hangup wins: the terminal is gone whatever else came.
                    libc::SIGHUP => leave = Some(sig),
                    sig => {
                        leave.get_or_insert(sig);
                    }
                });
                if cont && leave.is_none() {
                    self.continued(conn, fds, guard)?;
                }
                if tstp && leave.is_none() && self.suspend == Suspend::Running {
                    if conn.caps().contains(Caps::SUSPEND) {
                        ClientMsg::Suspend.encode(&mut self.sock_out)?;
                        self.suspend = Suspend::Asked(Instant::now() + SUSPEND_WAIT);
                    } else {
                        self.stop(conn, fds, guard)?;
                    }
                }
                match leave {
                    Some(libc::SIGHUP) => {
                        self.term_dead = true;
                        return Ok(End::Hangup(ExitCause::Signal(libc::SIGHUP)));
                    }
                    Some(sig) => return Ok(End::Leave(sig)),
                    None => {}
                }
            }
            if ready(3) {
                // Writable, or an error the next write will report.
                self.out_blocked = false;
            }
            if ready(2) && !self.read_input(fds.input)? {
                self.input_open = false;
                self.term_dead = true;
                return Ok(End::Hangup(ExitCause::InputClosed));
            }
            if ready(1) && pfds[1].revents & (libc::POLLIN | libc::POLLHUP | libc::POLLERR) != 0 && !self.out_blocked {
                match conn.dec.read_from(&mut conn.stream) {
                    Ok(0) => return Err(server_closed()),
                    Ok(_) => {}
                    Err(e) if matches!(e.kind(), io::ErrorKind::WouldBlock | io::ErrorKind::Interrupted) => {}
                    Err(e) => return Err(e),
                }
            }
        }
    }

    /// Job control: give the terminal back to the shell as it was before
    /// attaching, stop (as SIGTSTP's default action would), and when
    /// continued take it back. The server has stopped using the terminal
    /// (`Suspended`), or an older server does not know how to and it is
    /// stopped regardless.
    fn stop(&mut self, conn: &mut Connection, fds: &TermFds, guard: &mut RawGuard) -> io::Result<()> {
        let _ = self.flush_sock_until(conn.stream.as_raw_fd(), Instant::now() + HANGUP_FLUSH);
        guard.leave_raw();
        signals::stop_self();
        // Our SIGCONT handler has written to the pipe: it is handled here.
        self.skip_cont = true;
        self.suspend = Suspend::Background;
        self.resume(conn, fds, guard)
    }

    /// SIGCONT. After our own stop it was handled already. Otherwise the
    /// client was stopped without a word (SIGSTOP), or continued in the
    /// background: the shell may have changed the terminal's modes and
    /// drawn on it, so take it back and have the server repaint it all.
    fn continued(&mut self, conn: &mut Connection, fds: &TermFds, guard: &mut RawGuard) -> io::Result<()> {
        if std::mem::take(&mut self.skip_cont) && self.suspend == Suspend::Running {
            return Ok(());
        }
        if matches!(self.suspend, Suspend::Asked(_)) {
            return Ok(());
        }
        self.resume(conn, fds, guard)
    }

    /// Raw mode again and `Resume`, once the terminal is ours. Continued
    /// in the background (`bg`), the client stops again at once, as any
    /// program that uses its terminal does there (SIGTTIN), so the shell
    /// reports it stopped and `fg` continues it in the foreground (a
    /// shell sends no signal to a running job it brings to the foreground).
    fn resume(&mut self, conn: &mut Connection, fds: &TermFds, guard: &mut RawGuard) -> io::Result<()> {
        while !sys::is_foreground(fds.tty) {
            guard.leave_raw();
            self.suspend = Suspend::Background;
            signals::stop_self();
            self.skip_cont = true;
        }
        guard.enter_raw()?;
        self.suspend = Suspend::Running;
        let now = sys::winsize(fds.tty)
            .map(|ws| (Size::new(ws.ws_col, ws.ws_row), PixelSize { width: ws.ws_xpixel, height: ws.ws_ypixel }))
            .unwrap_or(self.last_size);
        self.last_size = now;
        let (size, pixels) = now;
        if conn.caps().contains(Caps::SUSPEND) {
            ClientMsg::Resume { size, pixels }.encode(&mut self.sock_out)?;
        } else {
            // An older server repaints everything after a resize.
            ClientMsg::Resize { size, pixels }.encode(&mut self.sock_out)?;
        }
        Ok(())
    }

    /// Sends `Detach` and relays server output until `Exit` (`Some`) or
    /// `deadline` (`None`). The terminal input is no longer read: keys
    /// typed now stay queued for the shell.
    fn shutdown(&mut self, conn: &mut Connection, fds: &TermFds, signals: &SignalPipe, deadline: Instant) -> io::Result<Option<ExitInfo>> {
        let sock = conn.stream.as_raw_fd();
        ClientMsg::Detach.encode(&mut self.sock_out)?;
        loop {
            if let Some(exit) = self.write_frames(conn, fds)? {
                return Ok(Some(exit));
            }
            self.flush_sock(sock)?;
            let Some(timeout) = sys::poll_ms(deadline) else { return Ok(None) };
            let mut sock_events = 0;
            if !self.out_blocked {
                sock_events |= libc::POLLIN;
            }
            if self.sock_pending() > 0 {
                sock_events |= libc::POLLOUT;
            }
            let entry = |fd: RawFd, events: libc::c_short| libc::pollfd { fd: if events == 0 { -1 } else { fd }, events, revents: 0 };
            let mut pfds = [
                entry(signals.fd(), libc::POLLIN),
                entry(sock, sock_events),
                entry(fds.output, if self.out_blocked { libc::POLLOUT } else { 0 }),
            ];
            // SAFETY: pfds is a valid array of pollfd of the given length.
            let r = unsafe { libc::poll(pfds.as_mut_ptr(), pfds.len() as libc::nfds_t, timeout) };
            if r < 0 {
                let e = io::Error::last_os_error();
                if e.kind() == io::ErrorKind::Interrupted {
                    continue;
                }
                return Err(e);
            }
            if pfds[0].revents != 0 {
                let mut hangup = false;
                // Further SIGTERMs change nothing: we are already leaving.
                signals.drain(|sig| hangup |= sig == libc::SIGHUP);
                if hangup {
                    self.term_dead = true;
                    return Ok(None);
                }
            }
            if pfds[2].revents != 0 {
                self.out_blocked = false;
            }
            if pfds[1].revents & (libc::POLLIN | libc::POLLHUP | libc::POLLERR) != 0 && !self.out_blocked {
                match conn.dec.read_from(&mut conn.stream) {
                    Ok(0) => return Err(server_closed()),
                    Ok(_) => {}
                    Err(e) if matches!(e.kind(), io::ErrorKind::WouldBlock | io::ErrorKind::Interrupted) => {}
                    Err(e) => return Err(e),
                }
            }
        }
    }

    /// The server did not restore the terminal: do it ourselves, unless the
    /// terminal is gone. Bounded by `deadline` for a terminal that does not
    /// read.
    fn write_fallback(&self, output: RawFd, deadline: Instant) {
        if !self.term_dead {
            let _ = sys::write_all_until(output, FALLBACK_RESTORE, deadline);
        }
    }

    /// Writes buffered server frames to the terminal until the decoder is
    /// empty, the terminal is full, or `Exit` arrives.
    fn write_frames(&mut self, conn: &mut Connection, fds: &TermFds) -> io::Result<Option<ExitInfo>> {
        while !self.out_blocked {
            // Output data is written as it arrives, before its frame is
            // complete, so a large frame never sits whole in memory.
            if let Some(data) = conn.dec.peek_data(type_id::OUTPUT) {
                match sys::write(fds.output, data) {
                    Ok(n) => conn.dec.consume_data(n),
                    Err(e) if e.kind() == io::ErrorKind::Interrupted => {}
                    Err(e) if e.kind() == io::ErrorKind::WouldBlock => self.out_blocked = true,
                    Err(e) => {
                        self.term_dead = true;
                        return Err(e);
                    }
                }
                continue;
            }
            let Some(frame) = conn.dec.peek_frame()? else { return Ok(None) };
            let msg = frame.decode::<ServerMsg>();
            conn.dec.consume_frame();
            match msg? {
                ServerMsg::Exit { reason, message } => {
                    return Ok(Some(ExitInfo { cause: ExitCause::Server, reason, message, signal: None }));
                }
                ServerMsg::Tty { direct } if self.tty == TtyUse::Asked => {
                    self.tty = if direct { TtyUse::Server } else { TtyUse::Relay };
                }
                // The terminal is restored (the bytes for that came before):
                // stop, whether we asked (SIGTSTP) or not (suspend-client).
                ServerMsg::Suspended => {
                    self.got_suspended = true;
                    return Ok(None);
                }
                _ => {}
            }
            // Anything else is meaningless while attached (or unknown, rule 2).
        }
        Ok(None)
    }

    /// Reads everything the terminal has (up to one buffer) into one
    /// `Input` frame. Returns false when the input is gone.
    fn read_input(&mut self, input: RawFd) -> io::Result<bool> {
        let mut n = 0;
        // A PTY hands out at most ~1 KiB per read; gathering until EAGAIN
        // turns a paste into one frame instead of hundreds.
        while n < self.inbuf.len() {
            match sys::read(input, &mut self.inbuf[n..]) {
                Ok(0) => {
                    if n == 0 {
                        return Ok(false);
                    }
                    break;
                }
                Ok(k) => n += k,
                Err(e) if e.kind() == io::ErrorKind::Interrupted => {}
                Err(e) if e.kind() == io::ErrorKind::WouldBlock => break,
                // EIO: the terminal hung up.
                Err(_) if n == 0 => return Ok(false),
                Err(_) => break,
            }
        }
        if self.sock_pos > 0 && self.sock_pos == self.sock_out.len() {
            self.sock_out.clear();
            self.sock_pos = 0;
        }
        encode_input(&self.inbuf[..n], &mut self.sock_out);
        Ok(true)
    }

    fn flush_sock(&mut self, sock: RawFd) -> io::Result<()> {
        while self.sock_pending() > 0 && !self.sock_shut {
            match sys::write(sock, &self.sock_out[self.sock_pos..]) {
                Ok(n) => self.sock_pos += n,
                Err(e) if e.kind() == io::ErrorKind::Interrupted => {}
                Err(e) if e.kind() == io::ErrorKind::WouldBlock => break,
                // The server may end the attachment on its own (a key it
                // read from the terminal directly, kill-server) while we
                // still write, e.g. the `Resume` after SIGCONT. What it sent
                // before closing is still readable: keep reading for its
                // `Exit`; a close without one still ends in server_closed.
                Err(e) if matches!(e.kind(), io::ErrorKind::BrokenPipe | io::ErrorKind::ConnectionReset) => self.sock_shut = true,
                Err(e) => return Err(e),
            }
        }
        if self.sock_shut {
            self.sock_pos = self.sock_out.len();
        }
        if self.sock_pos == self.sock_out.len() {
            self.sock_out.clear();
            self.sock_pos = 0;
        } else if self.sock_pos > SOCK_HIGH {
            self.sock_out.drain(..self.sock_pos);
            self.sock_pos = 0;
        }
        Ok(())
    }

    /// [`Relay::flush_sock`], waiting for the socket up to `deadline`.
    fn flush_sock_until(&mut self, sock: RawFd, deadline: Instant) -> io::Result<()> {
        loop {
            self.flush_sock(sock)?;
            if self.sock_pending() == 0 {
                return Ok(());
            }
            let ms = sys::poll_ms(deadline).ok_or(io::ErrorKind::TimedOut)?;
            let mut p = libc::pollfd { fd: sock, events: libc::POLLOUT, revents: 0 };
            // SAFETY: one valid pollfd.
            unsafe { libc::poll(&mut p, 1, ms) };
        }
    }

    fn size_changed(&mut self, tty: RawFd) -> Option<(Size, PixelSize)> {
        let ws = sys::winsize(tty).ok()?;
        let now = (Size::new(ws.ws_col, ws.ws_row), PixelSize { width: ws.ws_xpixel, height: ws.ws_ypixel });
        if now == self.last_size {
            return None;
        }
        self.last_size = now;
        Some(now)
    }
}

fn server_closed() -> io::Error {
    io::Error::new(io::ErrorKind::UnexpectedEof, "server closed the connection")
}
