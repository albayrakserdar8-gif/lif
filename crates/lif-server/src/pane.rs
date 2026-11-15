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
            if self.buf.capacity() > 256 * 1024 {
                self.buf.shrink_to(64 * 1024);
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
    /// The directory the program reported with OSC 7, as a path.
    pub(crate) osc7_path: String,
    pub(crate) name_checked: Option<Instant>,
    /// One more name check after the last frame that showed this pane's
    /// window in a status line: catches a program that started right after.
    pub(crate) name_recheck: Option<Instant>,
    /// Output arrived since the last name check (a program may have started).
    pub(crate) output_seen: bool,
    pub(crate) mode: Option<Box<dyn PaneMode>>,
    /// The mode's search runs: step it once per loop turn.
    pub(crate) searching: bool,
    /// Who started the mode's current search (its messages go there).
    pub(crate) search_client: Option<lif_core::ClientId>,
    /// Rows per search step, adapted so a step takes about
    /// `SEARCH_STEP_TIME` however costly rows are to read.
    pub(crate) search_rows: usize,
    /// A copy-mode drag rests on an edge: scroll again at this time.
    pub(crate) autoscroll_at: Option<Instant>,
    /// The copy-mode settings the mode has (see `Server::sync_mode_config`).
    pub(crate) mode_cfg: Option<crate::host::CopyKey>,
    /// A streamed paste is open: whether it was bracketed.
    pub(crate) paste_bracketed: Option<bool>,
    /// Refreshed during the current frame tick.
    pub(crate) refreshed: bool,
    /// The scrollback limits in effect: (bytes, lines).
    pub(crate) scrollback: (usize, Option<usize>),
    /// Some attached client shows the pane (see `Server::sync_views`).
    pub(crate) shown: bool,
    /// When this pane's frame was last refreshed.
    pub(crate) last_refresh: Option<Instant>,
    /// `frame`'s dirty flags were set outside `refresh` (leaving a mode).
    pub(crate) frame_marked: bool,
    /// Input from a client reached the pane since its last refresh: its
    /// next output is an echo and is drawn at once (see `ECHO_WINDOW`).
    pub(crate) echo_credit: bool,
    /// When a client's input last reached the pane.
    pub(crate) input_at: Option<Instant>,
    /// Output read since then.
    pub(crate) echo_bytes: usize,
    /// tmux passthrough unwrapping, while `allow-passthrough` or
    /// `tmux-compat` is on for the pane (see `Server::sync_views`).
    pub(crate) passthrough: Option<Box<crate::passthrough::Unwrap>>,
    /// Window-style colors for the pane's default colors (`window-style`,
    /// `window-active-style`), from its options.
    pub(crate) base_style: lif_types::Style,
}

/// The engine limits for the `scrollback-bytes` and `history-limit`
/// options: both hold (history-limit 0 means no scrollback at all).
pub(crate) fn scrollback_limits(bytes: usize, lines: usize) -> (usize, Option<usize>) {
    if lines == 0 { (0, Some(0)) } else { (bytes, Some(lines)) }
}

/// Whether a dirty pane may be drawn now for the echo of input.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub(crate) enum Echo {
    /// Draw it now, without pacing.
    Now,
    /// Inside the echo window but too soon after the last frame.
    At(Instant),
    /// Not interactive: paced.
    No,
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
            osc7_path: String::new(),
            name_checked: None,
            name_recheck: None,
            output_seen: false,
            mode: None,
            searching: false,
            search_client: None,
            search_rows: lif_copy::DEFAULT_SEARCH_STEP_ROWS,
            autoscroll_at: None,
            mode_cfg: None,
            paste_bracketed: None,
            refreshed: false,
            last_refresh: None,
            scrollback: (0, None),
            shown: false,
            frame_marked: false,
            echo_credit: false,
            input_at: None,
            echo_bytes: 0,
            passthrough: None,
            base_style: lif_types::Style::default(),
        }
    }

    /// The pane's own content changed since its frame was refreshed. (A
    /// mode's changes reach clients as `needs_frame` instead.)
    pub(crate) fn changed(&self) -> bool {
        self.engine.is_dirty() || self.frame_marked
    }

    /// Inside a synchronized update the program opened less than
    /// `SYNC_TIMEOUT` ago: not drawn until then (returned).
    pub(crate) fn held(&self, now: Instant) -> Option<Instant> {
        let until = self.sync_since? + crate::SYNC_TIMEOUT;
        (now < until).then_some(until)
    }

    pub(crate) fn echo(&self, now: Instant) -> Echo {
        if self.echo_credit {
            return Echo::Now;
        }
        match self.input_at {
            Some(t) if now < t + crate::ECHO_WINDOW && self.echo_bytes < crate::ECHO_WINDOW_BYTES => match self.last_refresh {
                Some(r) if now < r + crate::ECHO_INTERVAL => Echo::At(r + crate::ECHO_INTERVAL),
                _ => Echo::Now,
            },
            _ => Echo::No,
        }
    }

    /// Input from a client was written to the pane.
    pub(crate) fn note_input(&mut self, now: Instant) {
        self.echo_credit = true;
        self.input_at = Some(now);
        self.echo_bytes = 0;
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
                match self.passthrough.as_mut() {
                    Some(u) => {
                        let engine = &mut self.engine;
                        u.feed(&buf[..filled], &mut |b| engine.feed(b));
                    }
                    None => self.engine.feed(&buf[..filled]),
                }
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
            m.update(&mut ModeCtx { engine: &mut self.engine });
        }
    }

    pub(crate) fn leave_mode(&mut self) {
        if let Some(mut m) = self.mode.take() {
            let view = m.is_view();
            m.leave(&mut ModeCtx { engine: &mut self.engine });
            self.engine.track_trimmed(false);
            if view {
                // The terminal's frame comes back in place of the mode's.
                self.frame.mark_all_dirty();
                self.frame_marked = true;
            }
        }
        self.searching = false;
        self.search_client = None;
        self.autoscroll_at = None;
        self.mode_cfg = None;
    }

    /// Run `f` on the pane's mode, if it has one.
    pub(crate) fn with_mode<R>(&mut self, f: impl FnOnce(&mut dyn PaneMode, &mut ModeCtx<'_>) -> R) -> Option<R> {
        let m = self.mode.as_deref_mut()?;
        Some(f(m, &mut ModeCtx { engine: &mut self.engine }))
    }
}

/// The path of an OSC 7 `file://host/path` URL, percent-decoded; text that
/// is not such a URL is taken as a path already.
pub(crate) fn file_url_path(url: &str) -> String {
    let Some(rest) = url.strip_prefix("file://") else { return url.to_string() };
    let path = match rest.find('/') {
        Some(i) => &rest[i..],
        None => return String::new(),
    };
    let b = path.as_bytes();
    let mut out = Vec::with_capacity(b.len());
    let mut i = 0;
    while i < b.len() {
        let hex = |c: u8| (c as char).to_digit(16).map(|d| d as u8);
        if b[i] == b'%'
            && i + 2 < b.len()
            && let (Some(h), Some(l)) = (hex(b[i + 1]), hex(b[i + 2]))
        {
            out.push(h << 4 | l);
            i += 3;
            continue;
        }
        out.push(b[i]);
        i += 1;
    }
    String::from_utf8_lossy(&out).into_owned()
}

#[cfg(test)]
mod url_tests {
    use super::file_url_path;

    #[test]
    fn osc7_urls_become_paths() {
        assert_eq!(file_url_path("file://host/tmp"), "/tmp");
        assert_eq!(file_url_path("file:///Users/a/My%20Dir/%C3%A7"), "/Users/a/My Dir/ç");
        assert_eq!(file_url_path("file://host/a%2"), "/a%2");
        assert_eq!(file_url_path("/already/a/path"), "/already/a/path");
        assert_eq!(file_url_path("file://hostonly"), "");
    }
}
