//! A pane's runtime half: its PTY, terminal engine, frame and input queue.

use std::io;
use std::time::Instant;

use lif_core::PaneId;
use lif_pty::{Pty, ReadOutcome};
use lif_types::{PaneFrame, Size};
use lif_vt::VtEngine;

use crate::mode::{Engine, ModeCtx, PaneMode};

/// Bytes waiting for a pane's program. A PTY takes about 1 KiB before
/// EAGAIN, so writes are always partial-write aware.
#[derive(Default)]
pub(crate) struct WriteQueue {
    buf: Vec<u8>,
    pos: usize,
}

impl WriteQueue {
    pub(crate) fn len(&self) -> usize {
        self.buf.len() - self.pos
    }

    pub(crate) fn push(&mut self, bytes: &[u8]) {
        if self.pos == self.buf.len() {
            self.buf.clear();
            self.pos = 0;
        } else if self.pos >= 16 * 1024 && self.pos * 2 >= self.buf.len() {
            self.buf.drain(..self.pos);
            self.pos = 0;
        }
        self.buf.extend_from_slice(bytes);
    }

    /// Writes what the PTY takes. Ok(true) when the queue is empty.
    pub(crate) fn flush(&mut self, pty: &mut Pty) -> io::Result<bool> {
        while self.pos < self.buf.len() {
            match pty.write(&self.buf[self.pos..]) {
                Ok(0) => break,
                Ok(n) => self.pos += n,
                Err(e) if e.kind() == io::ErrorKind::WouldBlock => break,
                Err(e) if e.kind() == io::ErrorKind::Interrupted => continue,
                Err(e) => return Err(e),
            }
        }
        if self.pos == self.buf.len() {
            self.buf.clear();
            self.pos = 0;
            if self.buf.capacity() > 4 * crate::PANE_WRITE_HIGH {
                self.buf.shrink_to(crate::PANE_WRITE_HIGH);
            }
            return Ok(true);
        }
        Ok(false)
    }
}

pub(crate) struct PaneRt {
    pub(crate) id: PaneId,
    /// `None` once the process exited (a dead pane kept by remain-on-exit).
    pub(crate) pty: Option<Pty>,
    pub(crate) pid: u32,
    pub(crate) engine: Engine,
    pub(crate) frame: PaneFrame,
    pub(crate) size: Size,
    pub(crate) wq: WriteQueue,
    /// WRITABLE interest is registered (the queue is not empty).
    pub(crate) want_write: bool,
    /// The read budget ran out before EAGAIN: read again next iteration.
    pub(crate) read_more: bool,
    pub(crate) eof: bool,
    pub(crate) compact_at: Option<Instant>,
    /// When the program's open synchronized update was first seen.
    pub(crate) sync_since: Option<Instant>,
    pub(crate) current_command: String,
    pub(crate) current_path: String,
    pub(crate) name_checked: Option<Instant>,
    /// One more name check after the last frame that showed this pane's
    /// window in a status line: catches a program that started right after.
    pub(crate) name_recheck: Option<Instant>,
    /// Output arrived since the last name check (a program may have started).
    pub(crate) output_seen: bool,
    pub(crate) mode: Option<Box<dyn PaneMode>>,
    /// A streamed paste is open: whether it was bracketed.
    pub(crate) paste_bracketed: Option<bool>,
    /// Refreshed during the current frame tick.
    pub(crate) refreshed: bool,
}

/// What one round of reading did.
pub(crate) struct ReadResult {
    pub(crate) bytes: usize,
    pub(crate) eof: bool,
}

impl PaneRt {
    pub(crate) fn new(id: PaneId, pty: Pty, engine: Engine, frame: PaneFrame, size: Size) -> Self {
        PaneRt {
            id,
            pid: pty.pid(),
            pty: Some(pty),
            engine,
            frame,
            size,
            wq: WriteQueue::default(),
            want_write: false,
            read_more: false,
            eof: false,
            compact_at: None,
            sync_since: None,
            current_command: String::new(),
            current_path: String::new(),
            name_checked: None,
            name_recheck: None,
            output_seen: false,
            mode: None,
            paste_bracketed: None,
            refreshed: false,
        }
    }

    /// Reads and feeds until EAGAIN, EOF or `budget` bytes. Many small
    /// reads (1 KiB on macOS) are batched into `buf` so the engine is fed
    /// in large slices.
    pub(crate) fn read(&mut self, buf: &mut [u8], budget: usize) -> ReadResult {
        let mut res = ReadResult { bytes: 0, eof: false };
        self.read_more = false;
        let Some(pty) = self.pty.as_mut() else {
            res.eof = true;
            return res;
        };
        if self.eof {
            res.eof = true;
            return res;
        }
        loop {
            let mut filled = 0;
            let mut stop = false;
            while filled < buf.len() {
                match pty.read(&mut buf[filled..]) {
                    Ok(ReadOutcome::Data(n)) => filled += n,
                    Ok(ReadOutcome::WouldBlock) => {
                        stop = true;
                        break;
                    }
                    Ok(ReadOutcome::Eof) => {
                        stop = true;
                        res.eof = true;
                        break;
                    }
                    Err(e) if e.kind() == io::ErrorKind::Interrupted => {}
                    Err(_) => {
                        stop = true;
                        res.eof = true;
                        break;
                    }
                }
            }
            if filled > 0 {
                self.engine.feed(&buf[..filled]);
                res.bytes += filled;
            }
            if stop {
                break;
            }
            if res.bytes >= budget {
                self.read_more = true;
                break;
            }
        }
        if res.eof {
            self.eof = true;
        }
        res
    }

    /// Lets the mode see new output / a new size.
    pub(crate) fn update_mode(&mut self) {
        if let Some(m) = self.mode.as_mut() {
            m.update(&mut ModeCtx { engine: &mut self.engine, rows: self.size.rows });
        }
    }

    pub(crate) fn leave_mode(&mut self) {
        if let Some(mut m) = self.mode.take() {
            m.leave(&mut ModeCtx { engine: &mut self.engine, rows: self.size.rows });
        }
    }
}
