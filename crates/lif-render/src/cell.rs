//! The renderer's cell: one cell of the client screen, either desired (just
//! composed from the scene) or as the outer terminal is believed to show it.

use lif_types::{Attrs, Cell, Color, Style, Underline};

pub(crate) const DEFAULT_STYLE: Style = Cell::BLANK.style;

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
#[repr(u8)]
pub(crate) enum W {
    Narrow,
    Wide,
    /// Second half of a wide character; compares equal to any other tail,
    /// since its content is decided by the head.
    Tail,
    /// The outer terminal's content here is not known (for example the
    /// orphaned half of a wide character that was partly overwritten).
    /// Never equal to anything, so it is always redrawn.
    Unknown,
}

/// A cell. Multi-codepoint grapheme clusters live in an arena owned by
/// whoever owns the cell (`off`/`len`, `len == 0` for none); `ch` is
/// always the first codepoint. `link` is a renderer-wide hyperlink id.
#[derive(Clone, Copy, Debug)]
pub(crate) struct RCell {
    pub ch: char,
    pub off: u32,
    pub len: u16,
    pub w: W,
    pub style: Style,
    pub link: u32,
}

impl RCell {
    pub const BLANK: RCell = RCell {
        ch: ' ',
        off: 0,
        len: 0,
        w: W::Narrow,
        style: DEFAULT_STYLE,
        link: 0,
    };
    pub const UNKNOWN: RCell = RCell {
        w: W::Unknown,
        ..RCell::BLANK
    };

    #[inline]
    pub fn text<'a>(&self, arena: &'a str) -> Option<&'a str> {
        if self.len == 0 {
            None
        } else {
            Some(&arena[self.off as usize..self.off as usize + self.len as usize])
        }
    }

    /// Exactly what an erase (EL/ECH/ED/scroll) with default SGR leaves.
    #[inline(always)]
    pub fn is_blank(&self) -> bool {
        self.w == W::Narrow
            && self.ch == ' '
            && self.len == 0
            && self.link == 0
            && is_default_style(&self.style)
    }

    /// Normalize a narrow space whose style cannot be seen to the blank
    /// style, so that erasing can stand in for printing it.
    #[inline]
    pub fn normalize_blank(&mut self) {
        if self.ch == ' '
            && self.len == 0
            && self.w == W::Narrow
            && !is_default_style(&self.style)
            && invisible_on_space(&self.style)
        {
            self.style = DEFAULT_STYLE;
        }
    }
}

#[inline(always)]
fn color_eq(a: Color, b: Color) -> bool {
    match (a, b) {
        (Color::Default, Color::Default) => true,
        (Color::Indexed(x), Color::Indexed(y)) => x == y,
        (Color::Rgb(r, g, b), Color::Rgb(s, h, c)) => r == s && g == h && b == c,
        _ => false,
    }
}

/// `Style == Style`, but always inlined: the derived comparison is an
/// out-of-line call in this crate and was the single hottest function of a
/// full repaint.
#[inline(always)]
pub(crate) fn style_eq(a: &Style, b: &Style) -> bool {
    a.attrs.0 == b.attrs.0
        && a.underline == b.underline
        && color_eq(a.fg, b.fg)
        && color_eq(a.bg, b.bg)
        && color_eq(a.underline_color, b.underline_color)
}

/// The default style.
#[inline(always)]
pub(crate) fn is_default_style(s: &Style) -> bool {
    s.attrs.0 == 0
        && s.underline == Underline::None
        && s.fg == Color::Default
        && s.bg == Color::Default
        && s.underline_color == Color::Default
}

/// Whether a space in this style looks exactly like a space in the default
/// style.
#[inline]
pub(crate) fn invisible_on_space(s: &Style) -> bool {
    s.bg == Color::Default
        && s.underline == Underline::None
        && !s.attrs.contains(Attrs::INVERSE)
        && !s.attrs.contains(Attrs::STRIKETHROUGH)
        && !s.attrs.contains(Attrs::OVERLINE)
}

/// Desired cell `d` (text in `da`) equals model cell `m` (text in `ma`).
#[inline(always)]
pub(crate) fn same(d: &RCell, da: &str, m: &RCell, ma: &str) -> bool {
    if d.w != m.w {
        return false;
    }
    if d.w == W::Tail {
        return true;
    }
    d.ch == m.ch
        && style_eq(&d.style, &m.style)
        && d.link == m.link
        && d.len == m.len
        && (d.len == 0 || d.text(da) == m.text(ma))
}

#[inline]
fn mix(h: u64, v: u64) -> u64 {
    (h.rotate_left(5) ^ v).wrapping_mul(0x517c_c1b7_2722_0a95)
}

#[inline]
fn color_bits(c: Color) -> u64 {
    match c {
        Color::Default => 0,
        Color::Indexed(n) => 0x100 | n as u64,
        Color::Rgb(r, g, b) => 0x100_0000 | (r as u64) << 16 | (g as u64) << 8 | b as u64,
    }
}

/// Hash of a run of cells, used to detect scrolled content.
pub(crate) fn hash_cells(cells: &[RCell], arena: &str) -> u64 {
    let mut h = 0u64;
    for c in cells {
        if c.w == W::Tail {
            h = mix(h, 0xfeed);
            continue;
        }
        let mut v = c.ch as u64
            | (c.w as u64) << 21
            | (c.style.attrs.0 as u64) << 24
            | (c.style.underline as u64) << 40;
        v ^= color_bits(c.style.fg).rotate_left(13) ^ color_bits(c.style.bg).rotate_left(39);
        if c.style.underline_color != Color::Default || c.link != 0 {
            v ^= color_bits(c.style.underline_color).rotate_left(7)
                ^ (c.link as u64).rotate_left(29);
        }
        h = mix(h, v);
        if let Some(t) = c.text(arena) {
            for b in t.bytes() {
                h = mix(h, b as u64);
            }
        }
    }
    h
}
