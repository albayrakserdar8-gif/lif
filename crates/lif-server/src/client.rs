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
    /// WRITABLE interest is registered.
    pub(crate) want_write: bool,
    /// Input may be left (budget, or reading was paused): read again.
    pub(crate) read_more: bool,
    /// Hung up or failed: never read or written again.
    pub(crate) dead: bool,
}

impl Tty {
    pub(crate) fn new(fd: OwnedFd, token: Token) -> Tty {
        Tty { fd, token, out: Vec::new(), pos: 0, want_write: false, read_more: true, dead: false }
    }

    fn pending(&self) -> usize {
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
    /// Reading is paused because this pane's input queue is full.
    pub(crate) blocked_on: Option<PaneId>,
    /// Data may be left in the socket (budget, or unblocked): read again.
    pub(crate) read_more: bool,
    pub(crate) att: Option<Box<Attached>>,
    /// Capabilities both sides announced.
    pub(crate) caps: Caps,
    pub(crate) term_out: TermOut,
}

impl ClientRt {
    pub(crate) fn new(id: ClientId, stream: mio::net::UnixStream, token: Token) -> Self {
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
            blocked_on: None,
            read_more: false,
            att: None,
            caps: Caps::NONE,
            term_out: TermOut::Relay,
        }
    }

    /// Reading may go on: the pane this client was blocked on drained.
    pub(crate) fn unblock(&mut self) {
        self.blocked_on = None;
        self.read_more = true;
        if let Some(t) = self.tty_mut() {
            t.read_more = true;
        }
    }

    /// Keys may be waiting in the terminal the server reads itself.
    pub(crate) fn tty_read_more(&self) -> bool {
        self.blocked_on.is_none() && !self.dead && self.tty().is_some_and(|t| t.read_more && !t.dead)
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

    pub(crate) fn send(&mut self, msg: &ServerMsg) {
        self.compact();
        if let Err(e) = msg.encode(&mut self.out) {
            log!("client {}: cannot encode {msg:?}: {e}", self.id);
        }
    }

    /// Terminal bytes outside a frame (bell, OSC 52 / 9, mode switches).
    /// Dropped when a stalled client already has [`MAX_EXTRA`] pending.
    pub(crate) fn send_output(&mut self, bytes: &[u8]) {
        if bytes.is_empty() || self.closing {
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
