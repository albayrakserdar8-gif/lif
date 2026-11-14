//! Events produced by [`crate::InputParser`].

use lif_types::{KeyEvent, MouseEvent, PixelSize, Size};

/// One thing the client's terminal sent.
#[derive(Clone, Debug, PartialEq, Eq)]
pub enum InputEvent {
    Key(KeyEvent),
    Mouse(MouseEvent),
    /// Bracketed paste started (`CSI 200 ~`). Followed by zero or more
    /// [`InputEvent::PasteData`] and then [`InputEvent::PasteEnd`].
    PasteStart,
    /// Raw pasted bytes, exactly as received (may contain ESC, may split a
    /// UTF-8 sequence between chunks). Each chunk is at most one `feed` call's
    /// worth of input, so memory stays bounded however big the paste is.
    PasteData(Vec<u8>),
    PasteEnd,
    /// Focus in (`true`) / out (`false`), mode 1004.
    Focus(bool),
    Response(TermResponse),
    /// A well-formed sequence this parser does not interpret, or a broken one
    /// flushed as best effort. Holds the raw bytes; for OSC / DCS / APC / PM /
    /// SOS strings the 7-bit form `ESC x ... ESC \` (truncated to the string
    /// cap if it was longer).
    Unknown(Vec<u8>),
}

/// DECRPM mode state (`CSI ? mode ; state $ y`).
#[derive(Clone, Copy, Debug, PartialEq, Eq, Hash)]
pub enum ModeState {
    NotRecognized,
    Set,
    Reset,
    PermanentlySet,
    PermanentlyReset,
}

impl ModeState {
    pub(crate) fn from_param(v: u32) -> ModeState {
        match v {
            1 => ModeState::Set,
            2 => ModeState::Reset,
            3 => ModeState::PermanentlySet,
            4 => ModeState::PermanentlyReset,
            _ => ModeState::NotRecognized,
        }
    }

    /// The terminal knows the mode (it can be queried and, unless permanently
    /// reset, used).
    pub fn is_supported(self) -> bool {
        matches!(self, ModeState::Set | ModeState::Reset | ModeState::PermanentlySet)
    }
}

/// Which color an OSC color report is about.
#[derive(Clone, Copy, Debug, PartialEq, Eq, Hash)]
pub enum ColorSlot {
    /// OSC 4: palette entry.
    Palette(u8),
    /// OSC 10.
    Foreground,
    /// OSC 11.
    Background,
    /// OSC 12.
    Cursor,
}

/// An 8-bit-per-channel color.
#[derive(Clone, Copy, Debug, Default, PartialEq, Eq, Hash)]
pub struct Rgb {
    pub r: u8,
    pub g: u8,
    pub b: u8,
}

impl Rgb {
    pub const fn new(r: u8, g: u8, b: u8) -> Self {
        Self { r, g, b }
    }

    /// Relative luminance below one half (sRGB weights, no gamma).
    pub fn is_dark(self) -> bool {
        let l = 299 * self.r as u32 + 587 * self.g as u32 + 114 * self.b as u32;
        l < 128 * 1000
    }
}

#[derive(Clone, Copy, Debug, PartialEq, Eq, Hash)]
pub enum ColorScheme {
    Dark,
    Light,
}

/// An answer to a query lif (or someone) sent to the client's terminal.
#[derive(Clone, Debug, PartialEq, Eq)]
pub enum TermResponse {
    /// DA1 `CSI ? Ps ; ... c`: conformance level first, then the attributes.
    /// lif sends DA1 last, so this doubles as "all answers are in".
    PrimaryDa(Vec<u32>),
    /// DA2 `CSI > Pp ; Pv ; Pc c`.
    SecondaryDa { model: u32, version: u32, rom: u32 },
    /// DA3 `DCS ! | hex ST`: the unit id, as sent.
    TertiaryDa(String),
    /// XTVERSION `DCS > | text ST`.
    XtVersion(String),
    /// Kitty keyboard query reply `CSI ? flags u`.
    KittyKeyboard(u32),
    /// DECRPM. `private` is true for DEC modes (`CSI ? ...`).
    ModeReport { mode: u32, private: bool, state: ModeState },
    /// CPR `CSI row ; col R` or DECXCPR `CSI ? row ; col R`, 1-based as
    /// reported.
    CursorPosition { row: u32, col: u32 },
    /// OSC 4 / 10 / 11 / 12 reply.
    Color { slot: ColorSlot, rgb: Rgb },
    /// OSC 52 reply, base64-decoded. `truncated` when the reply exceeded the
    /// string cap and `data` holds only a prefix.
    Clipboard { selection: String, data: Vec<u8>, truncated: bool },
    /// XTGETTCAP success `DCS 1 + r name=value ; ... ST`, hex-decoded.
    /// Boolean capabilities have no value.
    TermCap(Vec<(String, Option<String>)>),
    /// XTGETTCAP failure `DCS 0 + r [names] ST`.
    TermCapFailed(Vec<String>),
    /// `CSI ? 997 ; 1|2 n` (mode 2031 notification or reply to `CSI ? 996 n`).
    ColorScheme(ColorScheme),
    /// In-band resize `CSI 48 ; rows ; cols ; ypx ; xpx t` (mode 2048).
    InBandResize { cells: Size, pixels: PixelSize },
    /// `CSI 4 ; height ; width t`.
    TextAreaPixels(PixelSize),
    /// `CSI 8 ; rows ; cols t`.
    TextAreaCells(Size),
    /// `CSI 6 ; height ; width t`.
    CellSize(PixelSize),
}
