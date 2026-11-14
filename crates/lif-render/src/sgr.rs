//! SGR transitions and color downgrade.

use crate::cell::DEFAULT_STYLE;
use crate::esc::num;
use lif_types::{Attrs, Color, OuterCaps, Style, Underline};

/// SGR parameters being appended to the output; `start` is where the
/// parameter list began, to know whether a separator is needed.
struct P<'o> {
    out: &'o mut Vec<u8>,
    start: usize,
}

fn param(p: &mut P<'_>, n: u32) {
    if p.out.len() > p.start {
        p.out.push(b';');
    }
    num(p.out, n);
}

fn raw(p: &mut P<'_>, s: &[u8]) {
    if p.out.len() > p.start {
        p.out.push(b';');
    }
    p.out.extend_from_slice(s);
}

/// `base` is 30 for the foreground, 40 for the background.
fn color(p: &mut P<'_>, base: u32, c: Color) {
    match c {
        Color::Default => param(p, base + 9),
        Color::Indexed(n) if n < 8 => param(p, base + n as u32),
        Color::Indexed(n) if n < 16 => param(p, base + 60 + n as u32 - 8),
        Color::Indexed(n) => {
            param(p, base + 8);
            param(p, 5);
            param(p, n as u32);
        }
        Color::Rgb(r, g, b) => {
            param(p, base + 8);
            param(p, 2);
            param(p, r as u32);
            param(p, g as u32);
            param(p, b as u32);
        }
    }
}

fn underline_color(p: &mut P<'_>, c: Color) {
    match c {
        Color::Default => param(p, 59),
        Color::Indexed(n) => {
            param(p, 58);
            param(p, 5);
            param(p, n as u32);
        }
        Color::Rgb(r, g, b) => {
            param(p, 58);
            param(p, 2);
            param(p, r as u32);
            param(p, g as u32);
            param(p, b as u32);
        }
    }
}

fn underline(p: &mut P<'_>, u: Underline) {
    match u {
        Underline::None => param(p, 24),
        Underline::Single => param(p, 4),
        Underline::Double => raw(p, b"4:2"),
        Underline::Curly => raw(p, b"4:3"),
        Underline::Dotted => raw(p, b"4:4"),
        Underline::Dashed => raw(p, b"4:5"),
    }
}

const TOGGLES: [(Attrs, u32, u32); 6] = [
    (Attrs::ITALIC, 3, 23),
    (Attrs::BLINK, 5, 25),
    (Attrs::INVERSE, 7, 27),
    (Attrs::INVISIBLE, 8, 28),
    (Attrs::STRIKETHROUGH, 9, 29),
    (Attrs::OVERLINE, 53, 55),
];

fn incremental(from: &Style, to: &Style, p: &mut P<'_>) {
    let (fa, ta) = (from.attrs, to.attrs);
    let bold_off = fa.contains(Attrs::BOLD) && !ta.contains(Attrs::BOLD);
    let faint_off = fa.contains(Attrs::FAINT) && !ta.contains(Attrs::FAINT);
    if bold_off || faint_off {
        // 22 turns off both bold and faint.
        param(p, 22);
        if ta.contains(Attrs::BOLD) {
            param(p, 1);
        }
        if ta.contains(Attrs::FAINT) {
            param(p, 2);
        }
    } else {
        if ta.contains(Attrs::BOLD) && !fa.contains(Attrs::BOLD) {
            param(p, 1);
        }
        if ta.contains(Attrs::FAINT) && !fa.contains(Attrs::FAINT) {
            param(p, 2);
        }
    }
    for (bit, on, off) in TOGGLES {
        let (f, t) = (fa.contains(bit), ta.contains(bit));
        if f != t {
            param(p, if t { on } else { off });
        }
    }
    if from.underline != to.underline {
        underline(p, to.underline);
    }
    if from.fg != to.fg {
        color(p, 30, to.fg);
    }
    if from.bg != to.bg {
        color(p, 40, to.bg);
    }
    if from.underline_color != to.underline_color {
        underline_color(p, to.underline_color);
    }
}

/// Append the shortest SGR sequence that changes the terminal's current
/// style from `from` to `to`: either the incremental changes, or a reset
/// followed by everything `to` sets, whichever is shorter.
pub(crate) fn transition(from: &Style, to: &Style, out: &mut Vec<u8>) {
    if from == to {
        return;
    }
    out.extend_from_slice(b"\x1b[");
    if *to == DEFAULT_STYLE {
        out.push(b'm');
        return;
    }
    // Write both candidates one after the other and keep the shorter. When
    // nothing has to be turned off the incremental one is a subset of the
    // reset one, so there is no need to try.
    let p0 = out.len();
    incremental(from, to, &mut P { out, start: p0 });
    let inc = out.len() - p0;
    let turns_off = !Attrs(from.attrs.0 & !to.attrs.0).is_empty()
        || (from.fg != Color::Default && to.fg == Color::Default)
        || (from.bg != Color::Default && to.bg == Color::Default)
        || (from.underline != Underline::None && to.underline == Underline::None)
        || (from.underline_color != Color::Default && to.underline_color == Color::Default);
    if turns_off {
        let r0 = out.len();
        out.extend_from_slice(b"0");
        incremental(&DEFAULT_STYLE, to, &mut P { out, start: r0 });
        let rst = out.len() - r0;
        if rst < inc {
            out.copy_within(r0.., p0);
            out.truncate(p0 + rst);
        } else {
            out.truncate(r0);
        }
    }
    out.push(b'm');
}

fn linear(c: u8) -> f32 {
    let c = c as f32 / 255.0;
    if c <= 0.04045 {
        c / 12.92
    } else {
        ((c + 0.055) / 1.055).powf(2.4)
    }
}

/// OKLab, a perceptually uniform color space: euclidean distance there
/// tracks how different two colors look, so hue is kept (a dark blue does
/// not become a gray just because the gray is numerically close).
fn oklab(r: u8, g: u8, b: u8) -> [f32; 3] {
    let (r, g, b) = (linear(r), linear(g), linear(b));
    let l = (0.412_221_47 * r + 0.536_332_55 * g + 0.051_445_99 * b).cbrt();
    let m = (0.211_903_5 * r + 0.680_699_5 * g + 0.107_396_96 * b).cbrt();
    let s = (0.088_302_46 * r + 0.281_718_85 * g + 0.629_978_7 * b).cbrt();
    [
        0.210_454_26 * l + 0.793_617_8 * m - 0.004_072_047 * s,
        1.977_998_5 * l - 2.428_592_2 * m + 0.450_593_7 * s,
        0.025_904_037 * l + 0.782_771_77 * m - 0.808_675_77 * s,
    ]
}

/// RGB of palette entries 16..=255 (the 6x6x6 cube and the gray ramp).
pub(crate) fn palette_rgb(i: u8) -> (u8, u8, u8) {
    const LV: [u8; 6] = [0, 95, 135, 175, 215, 255];
    if i >= 232 {
        let l = 8 + 10 * (i - 232);
        (l, l, l)
    } else {
        let i = (i - 16) as usize;
        (LV[i / 36], LV[(i / 6) % 6], LV[i % 6])
    }
}

fn palette_lab() -> &'static [[f32; 3]; 240] {
    static LAB: std::sync::OnceLock<[[f32; 3]; 240]> = std::sync::OnceLock::new();
    LAB.get_or_init(|| {
        let mut t = [[0.0; 3]; 240];
        for (i, e) in t.iter_mut().enumerate() {
            let (r, g, b) = palette_rgb(16 + i as u8);
            *e = oklab(r, g, b);
        }
        t
    })
}

/// The 256-color palette entry that looks closest to an RGB color: the
/// nearest of the 6x6x6 cube and the gray ramp in OKLab. The 16 system
/// colors are left out because their values depend on the terminal's theme.
pub fn rgb_to_256(r: u8, g: u8, b: u8) -> u8 {
    let p = oklab(r, g, b);
    let mut best = (f32::MAX, 16u8);
    for (i, q) in palette_lab().iter().enumerate() {
        // Chroma counts double: the palette is coarse in lightness, and a
        // wrong hue is more noticeable than a slightly wrong lightness.
        let d = (p[0] - q[0]).powi(2) + 4.0 * ((p[1] - q[1]).powi(2) + (p[2] - q[2]).powi(2));
        if d < best.0 {
            best = (d, 16 + i as u8);
        }
    }
    best.1
}

/// How cells are adjusted to what the outer terminal can show.
#[derive(Clone, Debug)]
pub(crate) struct Norm {
    caps: OuterCaps,
    /// The outer terminal can show every style: nothing to adjust.
    full: bool,
    /// Direct-mapped cache of downgraded colors (key: rgb | 1 << 24).
    cache: [(u32, u8); 64],
}

impl Norm {
    pub(crate) fn new(caps: OuterCaps) -> Self {
        let full = caps.truecolor && caps.styled_underline && caps.underline_color && caps.overline;
        Norm {
            caps,
            full,
            cache: [(0, 0); 64],
        }
    }

    pub(crate) fn is_full(&self) -> bool {
        self.full
    }

    pub(crate) fn hyperlinks(&self) -> bool {
        self.caps.hyperlinks
    }

    fn downgrade(&mut self, r: u8, g: u8, b: u8) -> u8 {
        let key = (r as u32) << 16 | (g as u32) << 8 | b as u32 | 1 << 24;
        let slot = ((key.wrapping_mul(0x9E37_79B9)) >> 26) as usize;
        if self.cache[slot].0 == key {
            return self.cache[slot].1;
        }
        let v = rgb_to_256(r, g, b);
        self.cache[slot] = (key, v);
        v
    }

    #[inline]
    pub(crate) fn style(&mut self, s: &mut Style) {
        if self.full {
            return;
        }
        if !self.caps.truecolor {
            for c in [&mut s.fg, &mut s.bg, &mut s.underline_color] {
                if let Color::Rgb(r, g, b) = *c {
                    *c = Color::Indexed(self.downgrade(r, g, b));
                }
            }
        }
        if !self.caps.styled_underline && s.underline != Underline::None {
            s.underline = Underline::Single;
        }
        if !self.caps.underline_color {
            s.underline_color = Color::Default;
        }
        if !self.caps.overline {
            s.attrs.remove(Attrs::OVERLINE);
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn tr(from: Style, to: Style) -> String {
        let mut out = Vec::new();
        transition(&from, &to, &mut out);
        String::from_utf8(out).unwrap()
    }

    fn st(fg: Color, bg: Color, attrs: Attrs) -> Style {
        Style {
            fg,
            bg,
            attrs,
            ..Style::default()
        }
    }

    #[test]
    fn transitions() {
        let d = Style::default();
        assert_eq!(tr(d, d), "");
        assert_eq!(
            tr(d, st(Color::Indexed(1), Color::Default, Attrs::NONE)),
            "\x1b[31m"
        );
        assert_eq!(
            tr(d, st(Color::Indexed(9), Color::Indexed(12), Attrs::NONE)),
            "\x1b[91;104m"
        );
        assert_eq!(
            tr(d, st(Color::Indexed(200), Color::Default, Attrs::NONE)),
            "\x1b[38;5;200m"
        );
        assert_eq!(
            tr(d, st(Color::Rgb(1, 2, 3), Color::Default, Attrs::NONE)),
            "\x1b[38;2;1;2;3m"
        );
        // Back to default: a bare reset is the shortest.
        assert_eq!(
            tr(st(Color::Indexed(1), Color::Indexed(2), Attrs::BOLD), d),
            "\x1b[m"
        );
        // One attribute off: incremental beats reset + respecify.
        let a = st(
            Color::Rgb(10, 20, 30),
            Color::Rgb(40, 50, 60),
            Attrs::BOLD.union(Attrs::ITALIC),
        );
        let b = st(Color::Rgb(10, 20, 30), Color::Rgb(40, 50, 60), Attrs::BOLD);
        assert_eq!(tr(a, b), "\x1b[23m");
        // Faint off while keeping bold needs 22 then 1.
        let c = st(
            Color::Indexed(100),
            Color::Default,
            Attrs::BOLD.union(Attrs::FAINT),
        );
        let e = st(Color::Indexed(100), Color::Default, Attrs::BOLD);
        assert_eq!(tr(c, e), "\x1b[22;1m");
        // Many things off, one on: reset wins.
        let f = st(
            Color::Indexed(1),
            Color::Indexed(2),
            Attrs::ITALIC.union(Attrs::INVERSE).union(Attrs::BLINK),
        );
        let g = st(Color::Default, Color::Default, Attrs::BOLD);
        assert_eq!(tr(f, g), "\x1b[0;1m");
        let u = Style {
            underline: Underline::Curly,
            underline_color: Color::Indexed(3),
            ..d
        };
        assert_eq!(tr(d, u), "\x1b[4:3;58;5;3m");
    }

    #[test]
    fn downgrade() {
        assert_eq!(rgb_to_256(0, 0, 0), 16);
        assert_eq!(rgb_to_256(255, 255, 255), 231);
        assert_eq!(rgb_to_256(255, 0, 0), 196);
        assert_eq!(rgb_to_256(95, 135, 175), 16 + 36 + 2 * 6 + 3);
        assert_eq!(rgb_to_256(128, 128, 128), 244);
        assert_eq!(rgb_to_256(18, 18, 18), 233);
        // A dark saturated blue stays blue rather than becoming a gray.
        let (r, g, b) = palette_rgb(rgb_to_256(25, 49, 110));
        assert!(b > r && b > g, "{r},{g},{b}");
    }

    #[test]
    fn downgrade_never_loses_hue_badly() {
        // Exact palette colors map to themselves, and saturated inputs never
        // map to the gray ramp.
        for i in 16..=255u8 {
            let (r, g, b) = palette_rgb(i);
            let j = rgb_to_256(r, g, b);
            assert_eq!(palette_rgb(j), (r, g, b), "{i} -> {j}");
        }
        for r in (0..=255).step_by(15) {
            for g in (0..=255).step_by(15) {
                for b in (0..=255).step_by(15) {
                    let (mx, mn) = (r.max(g).max(b), r.min(g).min(b));
                    if mx - mn >= 100 {
                        assert!(rgb_to_256(r as u8, g as u8, b as u8) < 232, "{r},{g},{b}");
                    }
                }
            }
        }
    }

    #[test]
    fn norm_downgrades_and_falls_back() {
        let mut n = Norm::new(OuterCaps::default());
        let mut s = Style {
            fg: Color::Rgb(255, 0, 0),
            underline: Underline::Curly,
            underline_color: Color::Indexed(4),
            attrs: Attrs::OVERLINE.union(Attrs::BOLD),
            ..Style::default()
        };
        n.style(&mut s);
        assert_eq!(s.fg, Color::Indexed(196));
        assert_eq!(s.underline, Underline::Single);
        assert_eq!(s.underline_color, Color::Default);
        assert_eq!(s.attrs, Attrs::BOLD);
    }
}
