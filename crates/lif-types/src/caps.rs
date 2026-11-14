use crate::Style;

/// What the client's (outer) terminal can do. Starts from a conservative
/// guess based on `$TERM` / `$COLORTERM` and is refined by the answers to the
/// queries lif sends when a client attaches.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub struct OuterCaps {
    /// 24-bit SGR colors. Without it colors are mapped to the 256 palette.
    pub truecolor: bool,
    /// Synchronized output (DEC private mode 2026).
    pub sync_output: bool,
    /// Kitty keyboard protocol (progressive enhancement).
    pub kitty_keyboard: bool,
    /// Curly / dotted / dashed / double underlines (SGR 4:x).
    pub styled_underline: bool,
    /// Underline color (SGR 58).
    pub underline_color: bool,
    /// OSC 8 hyperlinks.
    pub hyperlinks: bool,
    /// OSC 52 clipboard write / read.
    pub osc52_write: bool,
    pub osc52_read: bool,
    /// Focus in/out reporting (mode 1004).
    pub focus_events: bool,
    /// SGR mouse (1006) and SGR-pixel mouse (1016).
    pub sgr_mouse: bool,
    pub sgr_pixel_mouse: bool,
    pub bracketed_paste: bool,
    /// Grapheme-cluster width mode (2027).
    pub grapheme_clusters: bool,
    pub overline: bool,
    /// Left and right margins (DECLRMM mode 69 / DECSLRM), which let a pane
    /// narrower than the screen scroll in place.
    pub lr_margins: bool,
    /// In-band resize reports (mode 2048).
    pub in_band_resize: bool,
    /// Color-scheme change reports (mode 2031).
    pub color_scheme_reports: bool,
}

impl Default for OuterCaps {
    /// What any xterm-256color-like terminal can be assumed to do.
    fn default() -> Self {
        Self {
            truecolor: false,
            sync_output: false,
            kitty_keyboard: false,
            styled_underline: false,
            underline_color: false,
            hyperlinks: false,
            osc52_write: false,
            osc52_read: false,
            focus_events: true,
            sgr_mouse: true,
            sgr_pixel_mouse: false,
            bracketed_paste: true,
            grapheme_clusters: false,
            overline: false,
            lr_margins: false,
            in_band_resize: false,
            color_scheme_reports: false,
        }
    }
}

/// A run of text in one style. Used for lif's own UI (status line, messages).
#[derive(Clone, Debug, Default, PartialEq, Eq)]
pub struct Span {
    pub text: String,
    pub style: Style,
}

impl Span {
    pub fn new(text: impl Into<String>, style: Style) -> Self {
        Self { text: text.into(), style }
    }

    pub fn plain(text: impl Into<String>) -> Self {
        Self { text: text.into(), style: Style::default() }
    }
}

/// One line of lif's own UI.
pub type StyledLine = Vec<Span>;
