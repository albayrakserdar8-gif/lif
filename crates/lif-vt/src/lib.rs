//! The terminal emulation engine behind every lif pane.
//!
//! [`VtEngine`] is the only interface the rest of lif uses, so the engine can
//! be swapped. The production engine is [`GhosttyVt`] (libghostty-vt).
//!
//! An engine is single-threaded and owned by its pane. It never does I/O: the
//! server feeds it the bytes it read from the PTY and writes back whatever
//! [`VtEngine::drain_replies`] hands out.

use lif_types::{FrameRow, KeyEvent, MouseEvent, PaneFrame, PixelSize, Size};

#[cfg(feature = "ghostty")]
mod ghostty;
#[cfg(feature = "ghostty")]
pub use ghostty::GhosttyVt;

#[derive(Debug)]
pub enum VtError {
    OutOfMemory,
    InvalidSize,
    Engine(String),
}

impl std::fmt::Display for VtError {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        match self {
            VtError::OutOfMemory => f.write_str("out of memory"),
            VtError::InvalidSize => f.write_str("invalid terminal size"),
            VtError::Engine(e) => write!(f, "terminal engine error: {e}"),
        }
    }
}

impl std::error::Error for VtError {}

/// An 8-bit-per-channel RGB color.
pub type Rgb = (u8, u8, u8);

/// Default foreground used when none is set, so that OSC 10 queries are
/// always answered (light gray, like a VGA console).
pub const FALLBACK_FOREGROUND: Rgb = (0xc0, 0xc0, 0xc0);
/// Default background used when none is set, so that OSC 11 queries are
/// always answered.
pub const FALLBACK_BACKGROUND: Rgb = (0x00, 0x00, 0x00);

#[derive(Clone, Debug)]
pub struct VtOptions {
    pub size: Size,
    /// Pixel size of the whole pane; zero if unknown.
    pub pixels: PixelSize,
    /// Scrollback budget in bytes. 0 disables scrollback.
    pub scrollback_bytes: usize,
    /// Scrollback limit in lines (`None`: only the byte budget). Both
    /// limits hold; the engine trims whole storage pages, so slightly more
    /// lines than this may stay.
    pub scrollback_lines: Option<usize>,
    /// The default colors the pane reports to programs that ask (OSC 10 /
    /// 11 / 12 queries, which vim and neovim use to pick a dark or light
    /// theme). The server passes the outer terminal's colors as the
    /// client's `CapsProbe` learned them. `None` reports
    /// [`FALLBACK_FOREGROUND`] / [`FALLBACK_BACKGROUND`], and the cursor
    /// color falls back to the foreground. They do not affect frames:
    /// default-colored cells stay `Color::Default`.
    pub default_fg: Option<Rgb>,
    pub default_bg: Option<Rgb>,
    pub default_cursor: Option<Rgb>,
}

impl Default for VtOptions {
    fn default() -> Self {
        Self {
            size: Size::new(80, 24),
            pixels: PixelSize::default(),
            scrollback_bytes: 10 * 1024 * 1024,
            scrollback_lines: None,
            default_fg: None,
            default_bg: None,
            default_cursor: None,
        }
    }
}

/// What the program inside the pane has asked of the terminal. The server
/// uses this to decide what to request from the outer terminal.
#[derive(Clone, Copy, Debug, Default, PartialEq, Eq)]
pub struct PaneModes {
    pub alt_screen: bool,
    pub app_cursor_keys: bool,
    pub app_keypad: bool,
    pub bracketed_paste: bool,
    pub focus_events: bool,
    /// Any mouse tracking mode is on.
    pub mouse_tracking: bool,
    /// The program asked for motion events (button-event or any-event mode).
    pub mouse_motion: bool,
    /// Kitty keyboard protocol flags currently in effect (0 = legacy).
    pub kitty_keyboard: u8,
    /// A synchronized update (mode 2026) is open; the frame should not be
    /// shown until it closes.
    pub synchronized_output: bool,
}

/// Things a pane wants its owner to do. Collected by the engine while
/// feeding, handed out by [`VtEngine::drain_events`].
#[derive(Clone, Debug, PartialEq, Eq)]
pub enum VtEvent {
    Bell,
    TitleChanged,
    PwdChanged,
    /// OSC 52 write: set the clipboard `selection` (e.g. `b'c'`) to `data`.
    ClipboardWrite { selection: u8, data: Vec<u8> },
    /// OSC 52 read: the program wants the contents of `selection`. The owner
    /// answers by feeding the reply through `VtEngine::clipboard_reply`.
    ClipboardRead { selection: u8 },
    /// A desktop notification (OSC 9 / OSC 777 / OSC 99).
    Notification { title: String, body: String },
    /// OSC 133 shell integration: a command started / finished.
    CommandStarted,
    CommandFinished { exit_code: Option<i32> },
}

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum Scroll {
    /// Positive scrolls toward the newest output.
    Lines(i32),
    Top,
    /// Back to following live output.
    Bottom,
}

#[derive(Clone, Copy, Debug, Default, PartialEq, Eq)]
pub struct VtMemory {
    /// Resident bytes held by the grid and scrollback.
    pub resident_bytes: usize,
    /// Of which compressed.
    pub compressed_bytes: usize,
    /// Scrollback storage pages held compressed.
    pub compressed_pages: usize,
}

/// How [`VtEngine::capture`] formats text.
#[derive(Clone, Copy, Debug, Default, PartialEq, Eq)]
pub struct TextOptions {
    /// Join soft-wrapped rows into one line.
    pub join_wrapped: bool,
    /// Keep spaces at the end of lines (`capture-pane -N`).
    pub keep_trailing: bool,
    /// Include SGR sequences for colors and attributes (`capture-pane -e`).
    pub styled: bool,
}

/// Which lines [`VtEngine::text`] returns.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum TextRange {
    /// The visible screen.
    Screen,
    /// Scrollback and screen.
    All,
    /// Rows counted from the top of the scrollback, end exclusive.
    Rows { start: usize, end: usize },
}

pub trait VtEngine: Sized {
    fn new(opts: &VtOptions) -> Result<Self, VtError>;

    /// Feed bytes the program wrote to the PTY. Never blocks, never fails:
    /// malformed input is the engine's problem.
    fn feed(&mut self, bytes: &[u8]);

    /// Append the bytes the terminal must send back to the program (answers
    /// to DA, DSR, DECRQM, XTVERSION, OSC color queries, ...) to `out`.
    fn drain_replies(&mut self, out: &mut Vec<u8>);

    /// Append the events collected since the last call to `out`.
    fn drain_events(&mut self, out: &mut Vec<VtEvent>);

    fn resize(&mut self, size: Size, pixels: PixelSize);

    fn size(&self) -> Size;

    /// Bring `frame` up to date with the visible viewport.
    ///
    /// Rewrites only rows that changed since the previous call and sets their
    /// `frame.dirty` flag (it never clears flags). If `frame.size` differs
    /// from the engine's size the frame is reset and fully rewritten. Returns
    /// whether anything (rows or cursor) changed.
    ///
    /// The engine tracks changes for exactly one frame: always pass the same
    /// one. Several clients share it; see `PaneFrame` for who clears the
    /// flags.
    fn refresh(&mut self, frame: &mut PaneFrame) -> bool;

    /// Cheap check used by the event loop to decide whether to schedule a
    /// frame at all.
    fn is_dirty(&self) -> bool;

    fn modes(&self) -> PaneModes;

    /// Encode a key for the program, honoring its current keyboard modes
    /// (legacy / application cursor keys / kitty flags). Appends to `out`;
    /// appends nothing if the key produces no bytes in the current mode.
    fn encode_key(&mut self, key: &KeyEvent, out: &mut Vec<u8>);

    /// Encode a mouse event (pane-relative cell coordinates) for the program
    /// in its current mouse mode and format. Appends nothing if the program
    /// did not ask for this kind of event.
    fn encode_mouse(&mut self, event: &MouseEvent, out: &mut Vec<u8>);

    /// Encode pasted text, wrapping it in bracketed-paste markers when the
    /// program enabled them and making it safe otherwise.
    fn encode_paste(&mut self, data: &[u8], out: &mut Vec<u8>);

    /// Encode one piece of a paste that arrives in pieces (a bracketed
    /// paste from the outer terminal, streamed through with bounded
    /// memory): what [`VtEngine::encode_paste`] does to the text, but
    /// without the bracketed-paste markers. The caller writes `ESC [ 200 ~`
    /// once before the first piece and `ESC [ 201 ~` after the last when
    /// [`PaneModes::bracketed_paste`] was on at the start.
    fn encode_paste_chunk(&mut self, data: &[u8], out: &mut Vec<u8>) {
        const START: &[u8] = b"\x1b[200~";
        const END: &[u8] = b"\x1b[201~";
        let at = out.len();
        self.encode_paste(data, out);
        // encode_paste never lets ESC through from the data (that is what
        // makes a paste safe), so markers can only be its own framing.
        if out[at..].ends_with(END) {
            out.truncate(out.len() - END.len());
        }
        if out[at..].starts_with(START) {
            out.drain(at..at + START.len());
        }
    }

    /// Encode a focus change; appends nothing unless focus events are on.
    fn encode_focus(&mut self, focused: bool, out: &mut Vec<u8>);

    /// Answer a [`VtEvent::ClipboardRead`]; appends the OSC 52 reply to `out`.
    fn clipboard_reply(&mut self, selection: u8, data: &[u8], out: &mut Vec<u8>);

    /// Change the default colors (see [`VtOptions::default_fg`]), e.g. when
    /// a client with a different outer terminal attaches. Colors a program
    /// set itself (OSC 10 / 11 / 12) keep taking precedence until it resets
    /// them.
    fn set_default_colors(&mut self, fg: Option<Rgb>, bg: Option<Rgb>, cursor: Option<Rgb>);

    /// Move the viewport through the scrollback (copy mode).
    fn scroll(&mut self, to: Scroll);

    /// Rows of scrollback above the screen.
    fn scrollback_rows(&self) -> usize;

    /// How many rows the viewport is scrolled up from the bottom (0 = live).
    fn viewport_offset(&self) -> usize;

    /// Plain text of `range`, one `\n` per hard line break. Soft-wrapped rows
    /// are joined when `join_wrapped` is set; trailing blanks are trimmed.
    fn text(&self, range: TextRange, join_wrapped: bool) -> String;

    /// [`VtEngine::text`] with more choices: trailing spaces kept, and SGR
    /// sequences for styles. One `\n` per line.
    fn capture(&self, range: TextRange, opts: TextOptions) -> String;

    /// Write absolute row `abs_row` into `out`: exactly `size().cols`
    /// cells with their text, widths, grapheme clusters and styles, and
    /// whether the row soft-wraps. Row 0 is the oldest row of the scrollback
    /// still held, row `scrollback_rows()` the top of the screen; rows past
    /// the end come out blank. Hyperlink ids are not filled in (0).
    ///
    /// Cost: touches only the storage page holding the row (a compressed
    /// page is restored, no other one is) and allocates nothing per cell.
    /// `out`'s buffers are reused.
    fn row(&self, abs_row: usize, out: &mut FrameRow);

    /// [`VtEngine::row`] for reading many rows in order (a copy-mode
    /// search): the viewport is moved so that the rows around `abs_row` are
    /// cheap to reach, which makes a scan cost about the same per row
    /// however deep in the history it is. The caller puts the viewport back
    /// with [`VtEngine::set_viewport_top`] when done ([`VtEngine::viewport_top`]
    /// says where it was).
    fn row_scan(&mut self, abs_row: usize, out: &mut FrameRow);

    /// The absolute row at the top of the viewport (`scrollback_rows()`
    /// when it shows the live screen).
    fn viewport_top(&self) -> usize {
        self.scrollback_rows().saturating_sub(self.viewport_offset())
    }

    /// The nearest row at or after `from` (`forward`) or at or before it
    /// that starts a shell prompt (OSC 133;A), as an absolute row.
    fn find_prompt(&self, from: usize, forward: bool) -> Option<usize>;

    /// Rows evicted from the top of the scrollback (by the scrollback limit
    /// or by clearing the scrollback) while counting is on (see
    /// [`VtEngine::track_trimmed`]), a monotonic count. An absolute row `r`
    /// read when the count was `t0` is row `r - (t1 - t0)` when it is `t1`.
    /// Exact, except that a single `feed` evicting the whole scrollback
    /// counts only the rows that were there before it (all of which are
    /// gone, so no position survives).
    fn trimmed_rows(&self) -> u64;

    /// Count evicted rows from now on (`true`) or stop. Counting costs a
    /// little on every `feed` (about 7% of the throughput of a 100 MB
    /// `cat`), so the server turns it on only while a pane is in copy mode,
    /// which is the only reader. Off by default.
    fn track_trimmed(&mut self, on: bool);

    /// Show the rows starting at absolute row `top` (exactly). `top` at or
    /// past `scrollback_rows()` shows the live screen and follows output, as
    /// `Scroll::Bottom` does; any other value stays on the same rows while
    /// output arrives.
    fn set_viewport_top(&mut self, top: usize);

    /// Window title set by the program (OSC 0/2); empty if none.
    fn title(&self) -> &str;

    /// Working directory reported by the program (OSC 7); empty if none.
    fn pwd(&self) -> &str;

    /// The URI of a hyperlink id found in a frame cell.
    fn hyperlink(&self, id: u32) -> Option<&str>;

    fn memory(&self) -> VtMemory;

    /// Opportunistically give memory back: compress cold scrollback, and
    /// after a resize rebuild storage that the resize left larger than the
    /// contents. The server calls this when a pane has been quiet for a
    /// while (and after a resize). Each call does a bounded amount of work
    /// (about a millisecond); returns true when more remains, and the caller
    /// calls again soon (while the pane stays quiet). Must be cheap when
    /// there is nothing to do.
    fn compact(&mut self) -> bool;

    /// Change the scrollback limits (see [`VtOptions::scrollback_bytes`]
    /// and [`VtOptions::scrollback_lines`]). Lowering them drops history at
    /// once.
    fn set_scrollback_limits(&mut self, bytes: usize, lines: Option<usize>);

    /// The pane is not shown to anyone: drop what the next `refresh` can
    /// rebuild (render caches), which then rewrites the whole frame.
    fn release_caches(&mut self) {}
}
