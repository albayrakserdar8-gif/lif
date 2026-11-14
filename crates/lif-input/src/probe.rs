//! Refining what the outer terminal can do.

use lif_types::{OuterCaps, PixelSize};

use crate::{ColorSlot, Rgb, TermResponse};

/// Builds [`OuterCaps`] for one client: a guess from its environment,
/// refined by each answer to [`crate::write_queries`] until the DA1 sentinel.
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct CapsProbe {
    caps: OuterCaps,
    complete: bool,
    kitty_flags: Option<u32>,
    in_band_resize: bool,
    color_scheme_reports: bool,
    version: Option<String>,
    foreground: Option<Rgb>,
    background: Option<Rgb>,
    cell: PixelSize,
    da1: Vec<u32>,
}

impl CapsProbe {
    /// Guess from the client's `$TERM`, `$COLORTERM` and `$TERM_PROGRAM`
    /// (pass "" for unset). These are the client's variables, not the
    /// server's. The kitty keyboard protocol is never guessed: pushing flags
    /// a terminal does not understand would be visible, so only the query
    /// reply turns it on.
    pub fn from_env(term: &str, colorterm: &str, term_program: &str) -> Self {
        let mut caps = OuterCaps::default();
        let ct = colorterm.to_ascii_lowercase();
        if ct == "truecolor" || ct == "24bit" || term.ends_with("-direct") {
            caps.truecolor = true;
        }
        apply_profile(&mut caps, term);
        apply_profile(&mut caps, term_program);
        Self {
            caps,
            complete: false,
            kitty_flags: None,
            in_band_resize: false,
            color_scheme_reports: false,
            version: None,
            foreground: None,
            background: None,
            cell: PixelSize::default(),
            da1: Vec::new(),
        }
    }

    /// [`CapsProbe::from_env`] with this process's environment; right only
    /// when called in the client process.
    pub fn from_process_env() -> Self {
        let v = |k| std::env::var(k).unwrap_or_default();
        Self::from_env(&v("TERM"), &v("COLORTERM"), &v("TERM_PROGRAM"))
    }

    /// Take one answer into account. Returns true once the DA1 sentinel has
    /// arrived (and from then on).
    pub fn apply(&mut self, r: &TermResponse) -> bool {
        match r {
            TermResponse::KittyKeyboard(f) => {
                self.kitty_flags = Some(*f);
                self.caps.kitty_keyboard = true;
            }
            TermResponse::ModeReport { mode, private: true, state } => {
                let on = state.is_supported();
                match mode {
                    2026 => self.caps.sync_output = on,
                    2027 => self.caps.grapheme_clusters = on,
                    1016 => self.caps.sgr_pixel_mouse = on,
                    1006 => self.caps.sgr_mouse = on,
                    1004 => self.caps.focus_events = on,
                    2004 => self.caps.bracketed_paste = on,
                    2031 => self.color_scheme_reports = on,
                    2048 => self.in_band_resize = on,
                    _ => {}
                }
            }
            TermResponse::XtVersion(v) => {
                apply_profile(&mut self.caps, v);
                self.version = Some(v.clone());
            }
            TermResponse::Color { slot: ColorSlot::Foreground, rgb } => self.foreground = Some(*rgb),
            TermResponse::Color { slot: ColorSlot::Background, rgb } => self.background = Some(*rgb),
            TermResponse::CellSize(px) => self.cell = *px,
            TermResponse::PrimaryDa(attrs) => {
                // Attribute 52: OSC 52 clipboard access (xterm, foot, ...).
                if attrs.iter().skip(1).any(|&a| a == 52) {
                    self.caps.osc52_write = true;
                }
                self.da1 = attrs.clone();
                self.complete = true;
            }
            _ => {}
        }
        self.complete
    }

    pub fn caps(&self) -> OuterCaps {
        self.caps
    }

    pub fn is_complete(&self) -> bool {
        self.complete
    }

    /// The flags in effect when the terminal answered the kitty query.
    pub fn kitty_keyboard_flags(&self) -> Option<u32> {
        self.kitty_flags
    }

    /// Mode 2048 is supported; enable with [`crate::write_in_band_resize`].
    pub fn in_band_resize(&self) -> bool {
        self.in_band_resize
    }

    /// Mode 2031 is supported; enable with [`crate::write_color_scheme_reports`].
    pub fn color_scheme_reports(&self) -> bool {
        self.color_scheme_reports
    }

    /// XTVERSION answer, e.g. "kitty(0.39.1)".
    pub fn version(&self) -> Option<&str> {
        self.version.as_deref()
    }

    pub fn foreground(&self) -> Option<Rgb> {
        self.foreground
    }

    pub fn background(&self) -> Option<Rgb> {
        self.background
    }

    /// Zero while unknown.
    pub fn cell_size(&self) -> PixelSize {
        self.cell
    }

    pub fn primary_da(&self) -> &[u32] {
        &self.da1
    }
}

/// Features of terminals recognizable by name (`$TERM`, `$TERM_PROGRAM` or
/// XTVERSION). DECRQM answers, which arrive later, override the modes.
fn apply_profile(c: &mut OuterCaps, name: &str) {
    let n = name.to_ascii_lowercase();
    let has = |s: &str| n.contains(s);
    let modern = |c: &mut OuterCaps| {
        c.truecolor = true;
        c.sync_output = true;
        c.styled_underline = true;
        c.underline_color = true;
        c.hyperlinks = true;
        c.osc52_write = true;
    };
    if has("kitty") {
        modern(c);
        c.overline = true;
        c.sgr_pixel_mouse = true;
    } else if has("ghostty") {
        modern(c);
        c.overline = true;
        c.sgr_pixel_mouse = true;
        c.grapheme_clusters = true;
    } else if has("wezterm") {
        modern(c);
        c.overline = true;
        c.sgr_pixel_mouse = true;
    } else if has("foot") {
        modern(c);
        c.osc52_read = true;
        c.sgr_pixel_mouse = true;
    } else if has("alacritty") || has("contour") {
        modern(c);
    } else if has("iterm") {
        c.truecolor = true;
        c.sync_output = true;
        c.hyperlinks = true;
        c.osc52_write = true;
    } else if has("vscode") || has("xterm.js") {
        c.truecolor = true;
        c.hyperlinks = true;
    } else if n.starts_with("xterm(") {
        // Real xterm (XTVERSION "XTerm(398)").
        c.truecolor = true;
        c.sgr_pixel_mouse = true;
    }
}
