//! A client connection and, while it is attached, everything that belongs
//! to its terminal (input parser, capability probe, renderer, prompt...).
//!
//! # Direct terminal
//!
//! When the client announced `Caps::TTY` the server opens the client's
//! terminal itself (by path, so the open file description and its
//! `O_NONBLOCK` are the server's own, never the shell's) and reads keys
//! from it and writes frames to it directly ([`TermOut::Direct`]); the
//! client then only forwards resizes and signals. That takes the client
//! out of the echo path of every keystroke: one process wake-up less on the
//! way in and one on the way out. Until the client says, in raw mode, which
//! terminal it is (`UseTty`), terminal bytes are held back
//! ([`TermOut::Hold`]). Socket messages (`Exit`) are only written once the
//! terminal took every byte before them, so they keep their order.
//!
//! # Output bound
//!
//! The terminal queue (`out` when relaying, the tty's queue when direct)
//! holds at most one frame plus small control output: a frame is only
//! rendered when `out` is empty (otherwise the renderer `skip`s and the
//! client is drawn again once the socket drains), and non-frame bytes (bell,
//! OSC 52, notifications) are dropped once [`MAX_EXTRA`] are pending. So a
//! client that stops reading costs one frame of memory, however much output
//! the panes produce.

use std::collections::VecDeque;
use std::io::{self, Write};
use std::os::fd::{AsRawFd, OwnedFd};
use std::time::Instant;

use lif_core::{ClientId, ClientView, PaneId, SessionId};
use lif_input::{CapsProbe, InputEvent, InputParser};
use lif_proto::{BYTES_HEADER_LEN, Caps, ClientMsg, Decoder, ExitReason, Hello, MAX_BYTES_CHUNK, ServerMsg, encode_output, output_header};
use lif_render::Renderer;
use lif_types::{OuterCaps, StyledLine};
use mio::Token;

use crate::prompt::PromptState;

/// Non-frame output allowed to pile up behind a frame the client has not
/// read yet. Beyond it bells and notifications are dropped.
pub(crate) const MAX_EXTRA: usize = 64 * 1024;

/// Bytes read from one client per loop iteration.
pub(crate) const CLIENT_READ_BUDGET: usize = 256 * 1024;

pub(crate) struct Message {
    pub(crate) line: StyledLine,
    /// `None`: until the next key press.
    pub(crate) expires: Option<Instant>,
}

pub(crate) struct DisplayPanes {
    pub(crate) expires: Instant,
    pub(crate) template: String,
    /// (pane, label) in pane index order; the label is the digit to press.
    pub(crate) labels: Vec<(PaneId, String)>,
    /// Keys choose a pane or close the numbers (not with `-N`).
    pub(crate) keys: bool,
}

/// A border being dragged with the mouse.
#[derive(Clone, Copy)]
pub(crate) struct Drag {
    pub(crate) pane: PaneId,
    /// A vertical border (resize left/right) or a horizontal one.
    pub(crate) vertical: bool,
    pub(crate) last: u16,
}

pub(crate) struct Attached {
    pub(crate) session: SessionId,
    pub(crate) parser: InputParser,
    pub(crate) probe: CapsProbe,
    pub(crate) renderer: Renderer,
    /// What `write_enter` turned on (plus the kitty upgrade): what
    /// `write_leave` must undo.
    pub(crate) enter_caps: OuterCaps,
    pub(crate) kitty_pushed: bool,
    pub(crate) events: Vec<InputEvent>,
    pub(crate) input_deadline: Option<Instant>,
    pub(crate) key_deadline: Option<Instant>,
    pub(crate) needs_frame: bool,
    pub(crate) last_frame: Option<Instant>,
    pub(crate) frame_deadline: Option<Instant>,
    pub(crate) view: Option<ClientView>,
    pub(crate) status: StyledLine,
    pub(crate) status_stale: bool,
    /// The status line shows the time: redraw it every minute.
    pub(crate) clock: bool,
    pub(crate) clock_deadline: Option<Instant>,
    pub(crate) message: Option<Message>,
    pub(crate) prompt: Option<PromptState>,
    pub(crate) prompt_line: StyledLine,
    pub(crate) display_panes: Option<DisplayPanes>,
    /// Outer mouse reporting state: (on, all-motion).
    pub(crate) mouse: (bool, bool),
    /// The session's `mouse` option.
    pub(crate) mouse_opt: bool,
    pub(crate) drag: Option<Drag>,
    pub(crate) focused: bool,
    pub(crate) active_pane: Option<PaneId>,
    /// The pane receiving the current bracketed paste.
    pub(crate) paste: Option<PaneId>,
    /// Part of the current paste did not fit the pane's input queue: the
    /// rest of it is dropped too (a paste with a hole is worse than a
    /// paste cut short).
    pub(crate) paste_dropping: bool,
    /// The terminal is not the server's to use (see [`Suspension`]): no
    /// frames, no other output, no reading.
    pub(crate) suspended: Option<Suspension>,
    /// OSC 52 reads relayed to this terminal, in order.
    pub(crate) clip_reads: VecDeque<(PaneId, u8)>,
    /// Styles of borders and copy-mode overlays (from the options).
    pub(crate) theme: lif_render::Theme,
    /// `message-style`: messages and prompts.
    pub(crate) msg_style: lif_types::Style,
    /// A pane in a mode holds the mouse while a button is down over it:
    /// drags past its edges still go to it (and scroll it).
    pub(crate) mode_drag: Option<(PaneId, lif_types::Rect)>,
    /// A left button press on a pane not in a mode: a drag from it starts
    /// copy mode with a selection (tmux's MouseDrag1Pane: copy-mode -M).
    pub(crate) mouse_press: Option<(PaneId, lif_types::Rect, lif_types::MouseEvent)>,
    /// The last left click on a pane not in a mode: (pane, cell, time in
    /// ms, clicks so far), for double and triple clicks.
    pub(crate) clicks: Option<(PaneId, lif_types::Pos, u64, u8)>,
}

/// Why an attached client's terminal is left alone.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub(crate) enum Suspension {
    /// The client stops itself (SIGTSTP, `suspend-client`): the server
    /// restored the terminal and said `Suspended`.
    Stopped,
    /// The server found the client out of the foreground of its terminal
    /// (a shell took it back after stopping the client): it stopped using
    /// the terminal without a word, the shell owns it now.
    Background,
}

/// Where the bytes for a client's terminal go.
pub(crate) enum TermOut {
    /// Framed as `Output` messages in the socket queue; the client writes them.
    Relay,
    /// Attached with `Caps::TTY`, waiting for `UseTty`: kept here
    /// (bounded like any terminal queue) and sent once the way is known.
    Hold(Vec<u8>),
    /// Written to the terminal by the server.
    Direct(Tty),
}

/// The client's terminal, opened by the server.
pub(crate) struct Tty {
    pub(crate) fd: OwnedFd,
    pub(crate) token: Token,
    /// Bytes for the terminal; `out[pos..]` is unwritten.
    pub(crate) out: Vec<u8>,
    pos: usize,
    /// What the fd is registered for (`None`: not registered).
    pub(crate) interest: Option<mio::Interest>,
    /// The server reads keys from it (not while the client is suspended).
    pub(crate) reading: bool,
    /// Input may be left (budget, or reading was paused): read again.
    pub(crate) read_more: bool,
    /// Hung up or failed: never read or written again.
    pub(crate) dead: bool,
    /// The client process, to check before every read and write that the
    /// terminal is still the client's (see `sys::TtyOwner`).
    pub(crate) owner: crate::sys::TtyOwner,
    /// The check failed: the terminal is someone else's until the client
    /// says it is back (`Resume`).
    pub(crate) background: bool,
}

impl Tty {
    pub(crate) fn new(fd: OwnedFd, token: Token, owner: crate::sys::TtyOwner) -> Tty {
        Tty {
            fd,
            token,
            out: Vec::new(),
            pos: 0,
            interest: None,
            reading: true,
            read_more: true,
            dead: false,
            owner,
            background: false,
        }
    }

    /// The terminal may be used: the client is in its foreground. Once it
    /// is not, nothing queued for it is written (it would land on the
    /// shell's screen) and nothing is read from it until `Resume`.
    pub(crate) fn usable(&mut self) -> bool {
        if self.dead || self.background {
            return false;
        }
        if !self.owner.foreground() {
            self.background = true;
            self.read_more = false;
            self.out.clear();
            self.pos = 0;
            return false;
        }
        true
    }

    /// The interest the fd should be registered with now.
    pub(crate) fn wanted_interest(&self) -> Option<mio::Interest> {
        if self.dead || self.background {
            return None;
        }
        let read = self.reading.then_some(mio::Interest::READABLE);
        let write = (self.pending() > 0).then_some(mio::Interest::WRITABLE);
        match (read, write) {
            (Some(r), Some(w)) => Some(r | w),
            (r, w) => r.or(w),
        }
    }

    pub(crate) fn pending(&self) -> usize {
        self.out.len() - self.pos
    }

    fn compact(&mut self) {
        if self.pos == self.out.len() {
            self.out.clear();
            self.pos = 0;
        }
    }

    /// Writes what the terminal takes; a failure makes it dead and drops
    /// what was queued (nobody can see it any more).
    fn flush(&mut self) {
        if self.pos < self.out.len() && !self.usable() {
            return;
        }
        while !self.dead && self.pos < self.out.len() {
            // SAFETY: the slice is valid for its length; the fd is ours.
            let n = unsafe { libc::write(self.fd.as_raw_fd(), self.out[self.pos..].as_ptr().cast(), self.out.len() - self.pos) };
            if n > 0 {
                self.pos += n as usize;
                continue;
            }
            let e = io::Error::last_os_error();
            match e.kind() {
                io::ErrorKind::WouldBlock => break,
                io::ErrorKind::Interrupted => {}
                _ => self.dead = true,
            }
        }
        if self.dead || self.pos == self.out.len() {
            self.out.clear();
            self.pos = 0;
            if self.out.capacity() > 4 * 1024 * 1024 {
                self.out.shrink_to(64 * 1024);
            }
        }
    }
}

pub(crate) struct ClientRt {
    pub(crate) id: ClientId,
    pub(crate) stream: mio::net::UnixStream,
    pub(crate) token: Token,
    pub(crate) dec: Decoder<ClientMsg>,
    pub(crate) hello: Option<Hello>,
    /// Protocol bytes for the socket; `out[out_pos..]` is unsent.
    pub(crate) out: Vec<u8>,
    out_pos: usize,
    /// WRITABLE interest is registered.
    pub(crate) want_write: bool,
    /// The socket holds part of a frame (told to the panic hook).
    pub(crate) mid_frame: bool,
    /// Close once `out` is flushed (Exit was sent).
    pub(crate) closing: bool,
    /// Remove at the end of the iteration.
    pub(crate) dead: bool,
    /// The process at the other end (checked on accept).
    pub(crate) peer: crate::sys::PeerCred,
    /// Data may be left in the socket (budget): read again.
    pub(crate) read_more: bool,
    pub(crate) att: Option<Box<Attached>>,
    /// Capabilities both sides announced.
    pub(crate) caps: Caps,
    pub(crate) term_out: TermOut,
}

impl ClientRt {
    pub(crate) fn new(id: ClientId, stream: mio::net::UnixStream, token: Token, peer: crate::sys::PeerCred) -> Self {
        ClientRt {
            id,
            stream,
            token,
            dec: Decoder::new(),
            hello: None,
            out: Vec::new(),
            out_pos: 0,
            want_write: false,
            mid_frame: false,
            closing: false,
            dead: false,
            peer,
            read_more: false,
            att: None,
            caps: Caps::NONE,
            term_out: TermOut::Relay,
        }
    }

    /// Keys may be waiting in the terminal the server reads itself.
    pub(crate) fn tty_read_more(&self) -> bool {
        !self.dead && self.tty().is_some_and(|t| t.read_more && t.reading && !t.dead && !t.background)
    }

    /// The terminal is left alone for now (see [`Suspension`]).
    pub(crate) fn suspended(&self) -> bool {
        self.att.as_ref().is_some_and(|a| a.suspended.is_some())
    }

    /// The terminal the server writes itself, if it does.
    pub(crate) fn tty(&self) -> Option<&Tty> {
        match &self.term_out {
            TermOut::Direct(t) => Some(t),
            _ => None,
        }
    }

    pub(crate) fn tty_mut(&mut self) -> Option<&mut Tty> {
        match &mut self.term_out {
            TermOut::Direct(t) => Some(t),
            _ => None,
        }
    }

    /// Terminal bytes not yet handed on.
    fn term_pending(&self) -> usize {
        match &self.term_out {
            TermOut::Relay => self.pending_len(),
            TermOut::Hold(h) => h.len(),
            TermOut::Direct(t) => t.pending(),
        }
    }

    /// Some but not all of the queued bytes went out: the socket may hold
    /// part of a frame.
    pub(crate) fn partly_sent(&self) -> bool {
        self.out_pos > 0 && self.out_pos < self.out.len()
    }

    /// Something for the socket or the terminal is not written yet (or,
    /// while holding, cannot be): no new frame is rendered until it is.
    pub(crate) fn out_pending(&self) -> bool {
        self.out_pos < self.out.len()
            || match &self.term_out {
                TermOut::Relay => false,
                TermOut::Hold(_) => true,
                TermOut::Direct(t) => t.pending() > 0,
            }
    }

    pub(crate) fn pending_len(&self) -> usize {
        self.out.len() - self.out_pos
    }

    fn compact(&mut self) {
        if self.out_pos == self.out.len() {
            self.out.clear();
            self.out_pos = 0;
        }
    }

    /// Queues `msg` for the socket. False if it cannot be encoded (too
    /// large): nothing was queued. Never logs the payload, which can be
    /// megabytes (a Debug dump of it froze the loop for minutes).
    pub(crate) fn send(&mut self, msg: &ServerMsg) -> bool {
        self.compact();
        match msg.encode(&mut self.out) {
            Ok(()) => true,
            Err(e) => {
                log!("client {}: cannot encode {}: {e}", self.id, msg_summary(msg));
                false
            }
        }
    }

    /// A command's result. Output that does not fit one frame goes in
    /// `CommandOutput` parts to a client that takes them; one that does not
    /// gets an error instead of nothing (it would wait forever).
    pub(crate) fn send_command_done(&mut self, status: i32, stdout: Vec<u8>, stderr: Vec<u8>) {
        let total = stdout.len() + stderr.len();
        if total <= COMMAND_CHUNK {
            if self.send(&ServerMsg::CommandDone { status, stdout, stderr }) {
                return;
            }
        } else if total <= MAX_COMMAND_OUTPUT && self.caps.contains(Caps::OUTPUT_STREAM) {
            let mut sent = 0;
            while stdout.len() - sent > COMMAND_CHUNK {
                let part = ServerMsg::CommandOutput { stdout: stdout[sent..sent + COMMAND_CHUNK].to_vec(), stderr: Vec::new() };
                self.send(&part);
                sent += COMMAND_CHUNK;
            }
            let mut err_sent = 0;
            while stderr.len() - err_sent > COMMAND_CHUNK {
                let part = ServerMsg::CommandOutput { stdout: Vec::new(), stderr: stderr[err_sent..err_sent + COMMAND_CHUNK].to_vec() };
                self.send(&part);
                err_sent += COMMAND_CHUNK;
            }
            let last = ServerMsg::CommandDone { status, stdout: stdout[sent..].to_vec(), stderr: stderr[err_sent..].to_vec() };
            if self.send(&last) {
                return;
            }
        }
        let why = if self.caps.contains(Caps::OUTPUT_STREAM) || total > MAX_COMMAND_OUTPUT {
            format!("command output too large ({total} bytes, limit {MAX_COMMAND_OUTPUT})\n")
        } else {
            format!("command output too large for this client ({total} bytes, limit {COMMAND_CHUNK}); use a newer lif client\n")
        };
        self.send(&ServerMsg::CommandDone { status: 1, stdout: Vec::new(), stderr: why.into_bytes() });
    }

    /// Terminal bytes outside a frame (bell, OSC 52 / 9, mode switches).
    /// Dropped when a stalled client already has [`MAX_EXTRA`] pending.
    pub(crate) fn send_output(&mut self, bytes: &[u8]) {
        if bytes.is_empty() || self.closing || self.suspended() {
            return;
        }
        self.compact();
        let pending = self.term_pending();
        if pending > MAX_EXTRA && pending + bytes.len() > MAX_EXTRA * 2 {
            return;
        }
        match &mut self.term_out {
            TermOut::Relay => encode_output(bytes, &mut self.out),
            TermOut::Hold(h) => h.extend_from_slice(bytes),
            TermOut::Direct(t) => {
                t.compact();
                t.out.extend_from_slice(bytes);
            }
        }
    }

    /// Terminal bytes as `Output` messages on the socket, whoever writes
    /// the terminal otherwise: the client writes every `Output` it gets.
    pub(crate) fn send_terminal_via_socket(&mut self, bytes: &[u8]) {
        self.compact();
        encode_output(bytes, &mut self.out);
    }

    /// The attachment and the buffer a frame is rendered into: the socket
    /// queue (between [`ClientRt::begin_frame`] and
    /// [`ClientRt::end_frame`]) or the terminal's own.
    pub(crate) fn frame_parts(&mut self) -> (&mut Option<Box<Attached>>, &mut Vec<u8>) {
        let ClientRt { att, out, term_out, .. } = self;
        let buf = match term_out {
            TermOut::Relay => out,
            TermOut::Hold(h) => h,
            TermOut::Direct(t) => &mut t.out,
        };
        (att, buf)
    }

    /// Reserves an `Output` header and returns the buffer to render into
    /// plus where the frame starts. Finish with [`ClientRt::end_frame`].
    pub(crate) fn begin_frame(&mut self) -> usize {
        self.compact();
        if let TermOut::Direct(t) = &mut self.term_out {
            t.compact();
            return t.out.len();
        }
        let start = self.out.len();
        self.out.extend_from_slice(&[0; BYTES_HEADER_LEN]);
        start
    }

    pub(crate) fn end_frame(&mut self, start: usize) {
        if !matches!(self.term_out, TermOut::Relay) {
            return;
        }
        let len = self.out.len() - start - BYTES_HEADER_LEN;
        if len == 0 {
            self.out.truncate(start);
        } else if len <= MAX_BYTES_CHUNK {
            self.out[start..start + BYTES_HEADER_LEN].copy_from_slice(&output_header(len));
        } else {
            let data = self.out.split_off(start + BYTES_HEADER_LEN);
            self.out.truncate(start);
            encode_output(&data, &mut self.out);
        }
    }

    /// Writes what the terminal and the socket take. Returns whether bytes
    /// remain for (the socket, the terminal). The socket waits while the
    /// terminal has bytes queued before it.
    pub(crate) fn flush(&mut self) -> io::Result<(bool, bool)> {
        if let TermOut::Direct(t) = &mut self.term_out {
            t.flush();
            if t.pending() > 0 {
                return Ok((self.out_pos < self.out.len(), true));
            }
        }
        self.flush_socket().map(|s| (s, false))
    }

    fn flush_socket(&mut self) -> io::Result<bool> {
        while self.out_pos < self.out.len() {
            match self.stream.write(&self.out[self.out_pos..]) {
                Ok(0) => return Err(io::ErrorKind::WriteZero.into()),
                Ok(n) => self.out_pos += n,
                Err(e) if e.kind() == io::ErrorKind::WouldBlock => break,
                Err(e) if e.kind() == io::ErrorKind::Interrupted => {}
                Err(e) => return Err(e),
            }
        }
        if self.out_pos == self.out.len() {
            self.out.clear();
            self.out_pos = 0;
            if self.out.capacity() > 4 * 1024 * 1024 {
                self.out.shrink_to(64 * 1024);
            }
            return Ok(false);
        }
        Ok(true)
    }

    /// Shutdown: push out what is pending, waiting a little.
    pub(crate) fn flush_blocking(&mut self, timeout_ms: i32) {
        if let TermOut::Direct(t) = &mut self.term_out
            && !t.dead
            && t.pending() > 0
        {
            crate::sys::write_all_timeout(t.fd.as_raw_fd(), &t.out[t.pos..], timeout_ms);
            t.out.clear();
            t.pos = 0;
        }
        if self.out_pos < self.out.len() {
            crate::sys::write_all_timeout(self.stream.as_raw_fd(), &self.out[self.out_pos..], timeout_ms);
            self.out.clear();
            self.out_pos = 0;
        }
    }
}

/// One command's output in one message, at most: far below `MAX_PAYLOAD`
/// so a client that does not take output in parts still gets most outputs.
pub(crate) const COMMAND_CHUNK: usize = 4 * 1024 * 1024;

/// Most output one command may produce (`capture-pane -S -` of a huge
/// history): the whole output sits in the socket queue until the client
/// reads it, so it is bounded like any queue.
pub(crate) const MAX_COMMAND_OUTPUT: usize = 256 * 1024 * 1024;

/// A message for the log: its kind and size, never its payload.
pub(crate) fn msg_summary(msg: &ServerMsg) -> String {
    match msg {
        ServerMsg::CommandDone { status, stdout, stderr } => format!("CommandDone(status {status}, {} + {} bytes)", stdout.len(), stderr.len()),
        ServerMsg::CommandOutput { stdout, stderr } => format!("CommandOutput({} + {} bytes)", stdout.len(), stderr.len()),
        ServerMsg::Output(b) => format!("Output({} bytes)", b.len()),
        ServerMsg::Exit { reason, message } => format!("Exit({reason:?}, {} bytes)", message.len()),
        ServerMsg::Welcome { .. } => "Welcome".into(),
        ServerMsg::Attached => "Attached".into(),
        ServerMsg::Tty { direct } => format!("Tty({direct})"),
        ServerMsg::Suspended => "Suspended".into(),
        ServerMsg::Unknown { type_id } => format!("Unknown({type_id:#x})"),
    }
}

/// What restores a client's terminal, followed by `Exit`: the renderer's
/// `leave` and `write_leave` for the caps in effect.
pub(crate) fn leave_bytes(att: &mut Attached, out: &mut Vec<u8>) {
    att.renderer.leave(out);
    let mut caps = att.enter_caps;
    caps.kitty_keyboard = att.kitty_pushed;
    caps.sync_output = att.renderer.caps().sync_output;
    lif_input::write_leave(&caps, out);
}

impl ClientRt {
    /// Gives the panic hook what restores this client's terminal (a copy
    /// of [`leave_bytes`]) and the `Exit` after it: one socket write when
    /// relaying; when direct, the bytes go to the terminal, CAN first to
    /// abort a sequence a frame may have been cut off in, and `Exit` alone
    /// to the socket.
    pub(crate) fn arm_panic(&self) {
        let Some(att) = self.att.as_ref() else { return };
        let mut term = b"\x1b[m\x1b]8;;\x1b\\\x1b[r\x1b[?7h\x1b[0 q\x1b[?25h".to_vec();
        let mut caps = att.enter_caps;
        caps.kitty_keyboard = att.kitty_pushed;
        caps.sync_output = att.renderer.caps().sync_output;
        lif_input::write_leave(&caps, &mut term);
        let mut exit = Vec::new();
        let _ = ServerMsg::Exit { reason: ExitReason::Error, message: "[lost server]".into() }.encode(&mut exit);
        let sock = self.stream.as_raw_fd();
        match &self.term_out {
            TermOut::Direct(t) => {
                term.insert(0, 0x18);
                crate::panic::set(t.fd.as_raw_fd(), term);
                crate::panic::set(sock, exit);
            }
            _ => {
                let mut out = Vec::new();
                encode_output(&term, &mut out);
                out.extend_from_slice(&exit);
                crate::panic::set(sock, out);
            }
        }
    }

    /// Forget the panic hook's entries for this client.
    pub(crate) fn disarm_panic(&self) {
        crate::panic::remove(self.stream.as_raw_fd());
        if let Some(t) = self.tty() {
            crate::panic::remove(t.fd.as_raw_fd());
        }
    }
}
