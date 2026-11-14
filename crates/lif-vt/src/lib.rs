//! The terminal emulation engine behind every lif pane.
//!
//! [`VtEngine`] is the only interface the rest of lif uses, so the engine can
//! be swapped. The production engine is [`GhosttyVt`] (libghostty-vt).
//!
//! An engine is single-threaded and owned by its pane. It never does I/O: the
//! server feeds it the bytes it read from the PTY and writes back whatever
//! [`VtEngine::drain_replies`] hands out.

use lif_types::{KeyEvent, MouseEvent, PaneFrame, PixelSize, Size};

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

#[derive(Clone, Debug)]
pub struct VtOptions {
    pub size: Size,
    /// Pixel size of the whole pane; zero if unknown.
    pub pixels: PixelSize,
    /// Scrollback budget in bytes (not lines). 0 disables scrollback.
    pub scrollback_bytes: usize,
}

impl Default for VtOptions {
    fn default() -> Self {
        Self { size: Size::new(80, 24), pixels: PixelSize::default(), scrollback_bytes: 10 * 1024 * 1024 }
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
    /// one. To render the pane for a second consumer, copy the frame.
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

    /// Encode a focus change; appends nothing unless focus events are on.
    fn encode_focus(&mut self, focused: bool, out: &mut Vec<u8>);

    /// Answer a [`VtEvent::ClipboardRead`]; appends the OSC 52 reply to `out`.
    fn clipboard_reply(&mut self, selection: u8, data: &[u8], out: &mut Vec<u8>);

    /// Move the viewport through the scrollback (copy mode).
    fn scroll(&mut self, to: Scroll);

    /// Rows of scrollback above the screen.
    fn scrollback_rows(&self) -> usize;

    /// How many rows the viewport is scrolled up from the bottom (0 = live).
    fn viewport_offset(&self) -> usize;

    /// Plain text of `range`, one `\n` per hard line break. Soft-wrapped rows
    /// are joined when `join_wrapped` is set; trailing blanks are trimmed.
    fn text(&self, range: TextRange, join_wrapped: bool) -> String;

    /// Window title set by the program (OSC 0/2); empty if none.
    fn title(&self) -> &str;

    /// Working directory reported by the program (OSC 7); empty if none.
    fn pwd(&self) -> &str;

    /// The URI of a hyperlink id found in a frame cell.
    fn hyperlink(&self, id: u32) -> Option<&str>;

    fn memory(&self) -> VtMemory;

    /// Opportunistically compress cold scrollback. The server calls this
    /// when a pane has been quiet for a while. Must be cheap when there is
    /// nothing to do.
    fn compact(&mut self);
}
