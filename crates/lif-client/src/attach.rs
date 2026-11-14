//! The attached relay loop.
//!
//! One thread, one poll(2) over four fds: the signal self-pipe, the socket,
//! the terminal input and (only while it is full) the terminal output.
//!
//! Bounds: bytes to the server are queued only up to [`SOCK_HIGH`] plus one
//! read. Bytes to the terminal are never copied out of the decoder: `Output`
//! data is written straight from the decoder buffer as it arrives, what the
//! terminal did not take stays there, and the socket is not read again until
//! the terminal takes it. So the client holds at most one read
//! (`lif_proto::READ_CHUNK`) of output, and a slow terminal pushes back on
//! the server through the socket buffer.

use std::io;
use std::os::fd::{AsRawFd, RawFd};

use lif_proto::{encode_input, type_id, ClientMsg, ExitReason, ServerMsg};
use lif_types::{PixelSize, Size};

use crate::conn::Connection;
use crate::signals::SignalPipe;
use crate::sys;
use crate::term::RawGuard;

/// Size of the terminal read buffer.
const READ_BUF: usize = 64 * 1024;

/// Stop reading the terminal while this much is waiting for the socket.
const SOCK_HIGH: usize = 64 * 1024;

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
    /// SIGHUP or SIGTERM; the client sent `Detach`.
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
}

/// [`attach_loop_with`] on stdin / stdout.
pub fn attach_loop(conn: Connection) -> io::Result<ExitInfo> {
    attach_loop_with(conn, TermFds::stdio())
}

/// Relays between the terminal and the server until the server sends
/// `Exit`, a signal or terminal hangup detaches, or an error occurs. The
/// terminal is in raw mode for the duration and restored on every path
/// (including panics); the `Exit` message is printed after restoring.
pub fn attach_loop_with(mut conn: Connection, fds: TermFds) -> io::Result<ExitInfo> {
    let signals = SignalPipe::install()?;
    let guard = RawGuard::enter(fds.tty, fds.input, fds.output)?;
    conn.stream.set_nonblocking(true)?;
    let result = Relay::new().run(&mut conn, &fds, &signals);
    drop(guard);
    drop(signals);
    let exit = result?;
    if !exit.message.is_empty() {
        let mut line = exit.message.clone().into_bytes();
        line.push(b'\n');
        // The session is over either way; a terminal that cannot take the
        // message must not turn a clean exit into an error.
        let _ = sys::write_all(fds.output, &line);
    }
    Ok(exit)
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
        }
    }

    fn sock_pending(&self) -> usize {
        self.sock_out.len() - self.sock_pos
    }

    fn run(&mut self, conn: &mut Connection, fds: &TermFds, signals: &SignalPipe) -> io::Result<ExitInfo> {
        // The terminal may have changed size between Hello and now.
        self.last_size = (conn.hello_size, conn.hello_pixels);
        self.resize_pending = true;
        let sock = conn.stream.as_raw_fd();
        loop {
            if let Some(exit) = self.write_frames(conn, fds)? {
                return Ok(exit);
            }
            if self.resize_pending && self.sock_pending() < SOCK_HIGH {
                self.resize_pending = false;
                if let Some((size, pixels)) = self.size_changed(fds.tty) {
                    ClientMsg::Resize { size, pixels }.encode(&mut self.sock_out)?;
                }
            }
            self.flush_sock(sock)?;

            let want_input = self.input_open && self.sock_pending() < SOCK_HIGH;
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
            // SAFETY: pfds is a valid array of pollfd of the given length.
            let r = unsafe { libc::poll(pfds.as_mut_ptr(), pfds.len() as libc::nfds_t, -1) };
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
                signals.drain(|sig| match sig {
                    libc::SIGWINCH => self.resize_pending = true,
                    sig => leave = Some(sig),
                });
                if let Some(sig) = leave {
                    return Ok(self.detach(sock, ExitCause::Signal(sig)));
                }
            }
            if ready(3) {
                // Writable, or an error the next write will report.
                self.out_blocked = false;
            }
            if ready(2) && !self.read_input(fds.input)? {
                self.input_open = false;
                return Ok(self.detach(sock, ExitCause::InputClosed));
            }
            if ready(1) && pfds[1].revents & (libc::POLLIN | libc::POLLHUP | libc::POLLERR) != 0 && !self.out_blocked {
                match conn.dec.read_from(&mut conn.stream) {
                    Ok(0) => return Err(io::Error::new(io::ErrorKind::UnexpectedEof, "server closed the connection")),
                    Ok(_) => {}
                    Err(e) if matches!(e.kind(), io::ErrorKind::WouldBlock | io::ErrorKind::Interrupted) => {}
                    Err(e) => return Err(e),
                }
            }
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
                    Err(e) => return Err(e),
                }
                continue;
            }
            let Some(frame) = conn.dec.peek_frame()? else { return Ok(None) };
            let msg = frame.decode::<ServerMsg>();
            conn.dec.consume_frame();
            if let ServerMsg::Exit { reason, message } = msg? {
                return Ok(Some(ExitInfo { cause: ExitCause::Server, reason, message }));
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
        while self.sock_pending() > 0 {
            match sys::write(sock, &self.sock_out[self.sock_pos..]) {
                Ok(n) => self.sock_pos += n,
                Err(e) if e.kind() == io::ErrorKind::Interrupted => {}
                Err(e) if e.kind() == io::ErrorKind::WouldBlock => break,
                Err(e) => return Err(e),
            }
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

    /// Sends `Detach` as best it can without blocking: we are leaving
    /// anyway, and the server also notices the closed socket.
    fn detach(&mut self, sock: RawFd, cause: ExitCause) -> ExitInfo {
        let _ = ClientMsg::Detach.encode(&mut self.sock_out);
        let _ = self.flush_sock(sock);
        ExitInfo { cause, reason: ExitReason::Detached, message: String::new() }
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
