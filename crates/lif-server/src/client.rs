//! A client connection and, while it is attached, everything that belongs
//! to its terminal (input parser, capability probe, renderer, prompt...).
//!
//! # Output bound
//!
//! `out` holds at most one frame plus small control output: a frame is only
//! rendered when `out` is empty (otherwise the renderer `skip`s and the
//! client is drawn again once the socket drains), and non-frame bytes (bell,
//! OSC 52, notifications) are dropped once [`MAX_EXTRA`] are pending. So a
//! client that stops reading costs one frame of memory, however much output
//! the panes produce.

use std::collections::VecDeque;
use std::io::{self, Write};
use std::os::fd::AsRawFd;
use std::time::Instant;

use lif_core::{ClientId, ClientView, PaneId, SessionId};
use lif_input::{CapsProbe, InputEvent, InputParser};
use lif_proto::{BYTES_HEADER_LEN, ClientMsg, Decoder, ExitReason, Hello, MAX_BYTES_CHUNK, ServerMsg, encode_output, output_header};
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
    /// Close once `out` is flushed (Exit was sent).
    pub(crate) closing: bool,
    /// Remove at the end of the iteration.
    pub(crate) dead: bool,
    /// Reading is paused because this pane's input queue is full.
    pub(crate) blocked_on: Option<PaneId>,
    /// Data may be left in the socket (budget, or unblocked): read again.
    pub(crate) read_more: bool,
    pub(crate) att: Option<Box<Attached>>,
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
            closing: false,
            dead: false,
            blocked_on: None,
            read_more: false,
            att: None,
        }
    }

    pub(crate) fn out_pending(&self) -> bool {
        self.out_pos < self.out.len()
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
        if self.pending_len() > MAX_EXTRA && self.pending_len() + bytes.len() > MAX_EXTRA * 2 {
            return;
        }
        encode_output(bytes, &mut self.out);
    }

    /// Reserves an `Output` header and returns the buffer to render into
    /// plus where the frame starts. Finish with [`ClientRt::end_frame`].
    pub(crate) fn begin_frame(&mut self) -> usize {
        self.compact();
        let start = self.out.len();
        self.out.extend_from_slice(&[0; BYTES_HEADER_LEN]);
        start
    }

    pub(crate) fn end_frame(&mut self, start: usize) {
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

    /// Writes what the socket takes. Returns whether bytes remain.
    pub(crate) fn flush(&mut self) -> io::Result<bool> {
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
        if self.out_pending() {
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

/// The panic hook's copy of [`leave_bytes`] + `Exit`, as protocol frames.
pub(crate) fn emergency_bytes(att: &Attached) -> Vec<u8> {
    let mut term = b"\x1b[m\x1b]8;;\x1b\\\x1b[r\x1b[?7h\x1b[0 q\x1b[?25h".to_vec();
    let mut caps = att.enter_caps;
    caps.kitty_keyboard = att.kitty_pushed;
    caps.sync_output = att.renderer.caps().sync_output;
    lif_input::write_leave(&caps, &mut term);
    let mut out = Vec::new();
    encode_output(&term, &mut out);
    let _ = ServerMsg::Exit { reason: ExitReason::Error, message: "[lost server]".into() }.encode(&mut out);
    out
}
