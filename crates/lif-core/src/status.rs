//! Styles (`bg=green,fg=black`, `#[fg=red,bold]`) and status line layout.
//!
//! The model expands the status formats; this module turns the expanded
//! text, which may still hold `#[...]` style directives, into styled spans
//! and lays out the left part, the window list and the right part on one
//! line the way tmux's default status line looks.

use lif_types::{Attrs, Color, Span, Style, StyledLine, Underline};

/// Display width of one character (0 for combining marks, 2 for wide East
/// Asian characters and emoji, 1 otherwise). An approximation of wcwidth
/// good enough for status text; panes use the VT engine's widths.
pub(crate) fn char_width(c: char) -> usize {
    let u = c as u32;
    if u < 0x300 {
        return if u < 0x20 || (0x7f..0xa0).contains(&u) { 0 } else { 1 };
    }
    if (0x300..0x370).contains(&u)
        || (0x200b..0x2010).contains(&u)
        || (0xfe00..0xfe10).contains(&u)
        || (0x1ab0..0x1b00).contains(&u)
        || (0x20d0..0x2100).contains(&u)
        || (0xe0100..0xe01f0).contains(&u)
    {
        return 0;
    }
    let wide = (0x1100..=0x115f).contains(&u)
        || (0x2e80..=0x303e).contains(&u)
        || (0x3041..=0x33ff).contains(&u)
        || (0x3400..=0x4dbf).contains(&u)
        || (0x4e00..=0x9fff).contains(&u)
        || (0xa000..=0xa4cf).contains(&u)
        || (0xac00..=0xd7a3).contains(&u)
        || (0xf900..=0xfaff).contains(&u)
        || (0xfe30..=0xfe4f).contains(&u)
        || (0xff00..=0xff60).contains(&u)
        || (0xffe0..=0xffe6).contains(&u)
        || (0x1f300..=0x1f64f).contains(&u)
        || (0x1f900..=0x1f9ff).contains(&u)
        || (0x20000..=0x3fffd).contains(&u);
    if wide { 2 } else { 1 }
}

/// Display width of a string.
pub(crate) fn text_width(s: &str) -> usize {
    s.chars().map(char_width).sum()
}

fn parse_color(s: &str) -> Option<Color> {
    let names = ["black", "red", "green", "yellow", "blue", "magenta", "cyan", "white"];
    let lower = s.to_ascii_lowercase();
    if lower == "default" || lower == "terminal" {
        return Some(Color::Default);
    }
    if let Some(i) = names.iter().position(|n| *n == lower) {
        return Some(Color::Indexed(i as u8));
    }
    if let Some(rest) = lower.strip_prefix("bright")
        && let Some(i) = names.iter().position(|n| *n == rest)
    {
        return Some(Color::Indexed(8 + i as u8));
    }
    if let Some(n) = lower.strip_prefix("colour").or_else(|| lower.strip_prefix("color")) {
        return n.parse::<u8>().ok().map(Color::Indexed);
    }
    if let Some(hex) = lower.strip_prefix('#')
        && hex.len() == 6
    {
        let v = u32::from_str_radix(hex, 16).ok()?;
        return Some(Color::Rgb((v >> 16) as u8, (v >> 8) as u8, v as u8));
    }
    if (90..=97).contains(&lower.parse::<u8>().unwrap_or(0)) {
        return Some(Color::Indexed(lower.parse::<u8>().ok()? - 90 + 8));
    }
    None
}

/// Apply a tmux style string to `style`. `default` resets to `base` (the
/// style of the area being drawn). Unknown directives (alignment, ranges,
/// lists, push/pop) are ignored.
pub(crate) fn apply_style(spec: &str, style: &mut Style, base: Style) {
    for tok in spec.split([',', ' ']).filter(|t| !t.is_empty()) {
        let (key, val) = match tok.split_once('=') {
            Some((k, v)) => (k, Some(v)),
            None => (tok, None),
        };
        match (key, val) {
            ("fg", Some(v)) => {
                if let Some(c) = parse_color(v) {
                    style.fg = if v == "default" { base.fg } else { c };
                }
            }
            ("bg", Some(v)) => {
                if let Some(c) = parse_color(v) {
                    style.bg = if v == "default" { base.bg } else { c };
                }
            }
            ("us", Some(v)) => {
                if let Some(c) = parse_color(v) {
                    style.underline_color = c;
                }
            }
            ("default", None) => *style = base,
            ("none", None) | ("noattr", None) => {
                style.attrs = Attrs::NONE;
                style.underline = Underline::None;
            }
            (name, None) => {
                let (on, name) = match name.strip_prefix("no") {
                    Some(n) if n != "ne" => (false, n),
                    _ => (true, name),
                };
                let attr = match name {
                    "bright" | "bold" => Some(Attrs::BOLD),
                    "dim" => Some(Attrs::FAINT),
                    "italics" => Some(Attrs::ITALIC),
                    "blink" => Some(Attrs::BLINK),
                    "reverse" => Some(Attrs::INVERSE),
                    "hidden" => Some(Attrs::INVISIBLE),
                    "strikethrough" => Some(Attrs::STRIKETHROUGH),
                    "overline" => Some(Attrs::OVERLINE),
                    _ => None,
                };
                if let Some(a) = attr {
                    if on { style.attrs.insert(a) } else { style.attrs.remove(a) }
                    continue;
                }
                let ul = match name {
                    "underscore" => Some(Underline::Single),
                    "double-underscore" => Some(Underline::Double),
                    "curly-underscore" => Some(Underline::Curly),
                    "dotted-underscore" => Some(Underline::Dotted),
                    "dashed-underscore" => Some(Underline::Dashed),
                    _ => None,
                };
                if let Some(u) = ul {
                    style.underline = if on { u } else { Underline::None };
                }
            }
            _ => {}
        }
    }
}

/// A style string applied to the default style.
pub(crate) fn parse_style(spec: &str) -> Style {
    let mut s = Style::default();
    apply_style(spec, &mut s, Style::default());
    s
}

/// Split expanded text with `#[...]` directives into spans starting in
/// `style`; `#[default]` returns to `base`.
pub(crate) fn styled_spans(text: &str, start: Style, base: Style, out: &mut Vec<Span>) {
    let mut style = start;
    let mut rest = text;
    while let Some(i) = rest.find("#[") {
        if i > 0 {
            push(out, &rest[..i], style);
        }
        let after = &rest[i + 2..];
        match after.find(']') {
            Some(j) => {
                apply_style(&after[..j], &mut style, base);
                rest = &after[j + 1..];
            }
            None => {
                rest = "";
            }
        }
    }
    if !rest.is_empty() {
        push(out, rest, style);
    }
}

/// Expanded text with `#[...]` directives as one line of at most `width`
/// cells (pane border labels).
pub(crate) fn label(text: &str, base: Style, width: usize) -> StyledLine {
    let mut spans = Vec::new();
    styled_spans(text, base, base, &mut spans);
    if spans_width(&spans) <= width {
        return spans;
    }
    let mut out = Vec::new();
    crop(&spans, 0, width, &mut out);
    out
}

fn push(out: &mut Vec<Span>, text: &str, style: Style) {
    let text: String = text.chars().filter(|c| !c.is_control()).collect();
    if text.is_empty() {
        return;
    }
    match out.last_mut() {
        Some(last) if last.style == style => last.text.push_str(&text),
        _ => out.push(Span::new(text, style)),
    }
}

fn spans_width(spans: &[Span]) -> usize {
    spans.iter().map(|s| text_width(&s.text)).sum()
}

/// The part of `spans` in display columns `[start, start+len)`. A wide
/// character cut by either edge becomes a space.
fn crop(spans: &[Span], start: usize, len: usize, out: &mut Vec<Span>) {
    let end = start + len;
    let mut col = 0;
    for s in spans {
        let mut piece = String::new();
        for c in s.text.chars() {
            let w = char_width(c);
            if col >= start && col + w <= end {
                piece.push(c);
            } else if col < end && col + w > start {
                piece.push(' ');
            }
            col += w;
        }
        push(out, &piece, s.style);
        if col >= end {
            break;
        }
    }
}

/// One window in the status line's window list.
pub(crate) struct WindowEntry {
    /// Expanded `window-status(-current)-format`.
    pub text: String,
    /// Style the entry starts in.
    pub style: Style,
    pub current: bool,
}

/// Everything the status line needs, already expanded.
pub(crate) struct StatusParts {
    pub width: usize,
    pub base: Style,
    pub left: String,
    pub left_style: Style,
    pub left_length: usize,
    pub right: String,
    pub right_style: Style,
    pub right_length: usize,
    pub windows: Vec<WindowEntry>,
    pub separator: String,
    /// 0 left, 1 centre, 2 right, 3 absolute-centre.
    pub justify: usize,
}

/// Where the parts of a status line came out, as columns: for the mouse.
#[derive(Clone, Debug, Default, PartialEq, Eq)]
pub struct StatusRanges {
    /// `status-left`: start and end (exclusive).
    pub left: (usize, usize),
    /// `status-right`.
    pub right: (usize, usize),
    /// The visible part of each window's entry, in window order; empty
    /// ranges for windows scrolled out.
    pub windows: Vec<(usize, usize)>,
}

/// Lay the status line out, exactly `width` cells wide.
pub(crate) fn render(p: &StatusParts) -> StyledLine {
    render_with(p, None)
}

/// [`render`], also saying where everything went.
pub(crate) fn render_with(p: &StatusParts, mut ranges: Option<&mut StatusRanges>) -> StyledLine {
    let mut left_full = Vec::new();
    styled_spans(&p.left, p.left_style, p.base, &mut left_full);
    let mut left = Vec::new();
    crop(&left_full, 0, p.left_length.min(p.width), &mut left);
    let lw = spans_width(&left);

    let mut right_full = Vec::new();
    styled_spans(&p.right, p.right_style, p.base, &mut right_full);
    let rlen = p.right_length.min(p.width - lw);
    let mut right = Vec::new();
    let rfw = spans_width(&right_full);
    if rfw > rlen {
        // tmux keeps the start of status-right when it is too long.
        crop(&right_full, 0, rlen, &mut right);
    } else {
        right = right_full;
    }
    let rw = spans_width(&right);

    let mut list = Vec::new();
    let (mut cur_start, mut cur_end) = (0, 0);
    for (i, w) in p.windows.iter().enumerate() {
        if i > 0 {
            styled_spans(&p.separator, p.base, p.base, &mut list);
        }
        let start = spans_width(&list);
        styled_spans(&w.text, w.style, p.base, &mut list);
        if let Some(r) = ranges.as_deref_mut() {
            // In the list for now; moved to the line below.
            r.windows.push((start, spans_width(&list)));
        }
        if w.current {
            (cur_start, cur_end) = (start, spans_width(&list));
        }
    }
    let list_w = spans_width(&list);
    let avail = p.width - lw - rw;

    let mut line = left;
    let mut placed = Vec::new();
    // The list's first visible column, and the column of the line it is at.
    let mut shown_from = 0;
    let (pad_before, used) = if list_w <= avail {
        crop(&list, 0, list_w, &mut placed);
        let pad = match p.justify {
            1 => (avail - list_w) / 2,
            2 => avail - list_w,
            3 => (p.width / 2).saturating_sub(list_w / 2).saturating_sub(lw).min(avail - list_w),
            _ => 0,
        };
        (pad, list_w)
    } else if avail > 0 {
        // Scroll so the current window is visible, with < and > markers on
        // the cut sides.
        let mut offset = 0;
        if cur_end > avail {
            offset = (cur_end + 1).saturating_sub(avail).min(list_w - avail);
            if cur_start < offset {
                offset = cur_start;
            }
        }
        // The < and > markers need room besides the list: with one or two
        // columns left the list is cut without them.
        let markers = avail > 2;
        let cut_left = markers && offset > 0;
        let cut_right = markers && offset + avail < list_w;
        let inner_start = offset + cut_left as usize;
        shown_from = offset;
        let inner_len = avail - cut_left as usize - cut_right as usize;
        if cut_left {
            push(&mut placed, "<", p.base);
        }
        crop(&list, inner_start, inner_len, &mut placed);
        if cut_right {
            push(&mut placed, ">", p.base);
        }
        (0, avail)
    } else {
        (0, 0)
    };
    if let Some(r) = ranges {
        r.left = (0, lw);
        r.right = (p.width - rw, p.width);
        let (first, end) = (lw + pad_before, lw + pad_before + used);
        for w in &mut r.windows {
            let at = |c: usize| (first + c.saturating_sub(shown_from)).min(end);
            *w = (at(w.0), at(w.1));
        }
    }
    if pad_before > 0 {
        push(&mut line, &" ".repeat(pad_before), p.base);
    }
    for s in placed {
        push(&mut line, &s.text, s.style);
    }
    let fill = p.width - lw - rw - used - pad_before;
    if fill > 0 {
        push(&mut line, &" ".repeat(fill), p.base);
    }
    for s in right {
        push(&mut line, &s.text, s.style);
    }
    line
}

#[cfg(test)]
mod tests {
    use super::*;

    fn text(line: &StyledLine) -> String {
        line.iter().map(|s| s.text.as_str()).collect()
    }

    fn green() -> Style {
        parse_style("bg=green,fg=black")
    }

    fn parts(width: usize, windows: Vec<(&str, bool)>) -> StatusParts {
        StatusParts {
            width,
            base: green(),
            left: "[0] ".into(),
            left_style: green(),
            left_length: 10,
            right: "\"host\" 21:50 14-Nov-26".into(),
            right_style: green(),
            right_length: 40,
            windows: windows
                .into_iter()
                .map(|(t, c)| WindowEntry { text: t.into(), style: green(), current: c })
                .collect(),
            separator: " ".into(),
            justify: 0,
        }
    }

    #[test]
    fn styles() {
        let s = green();
        assert_eq!(s.bg, Color::Indexed(2));
        assert_eq!(s.fg, Color::Indexed(0));
        let mut t = s;
        apply_style("fg=colour123,bold,underscore,bg=#ff0080", &mut t, s);
        assert_eq!(t.fg, Color::Indexed(123));
        assert_eq!(t.bg, Color::Rgb(255, 0, 128));
        assert!(t.attrs.contains(Attrs::BOLD));
        assert_eq!(t.underline, Underline::Single);
        apply_style("nobold nounderscore", &mut t, s);
        assert!(!t.attrs.contains(Attrs::BOLD));
        assert_eq!(t.underline, Underline::None);
        apply_style("default", &mut t, s);
        assert_eq!(t, s);
        apply_style("fg=brightred,align=right,list=on", &mut t, s);
        assert_eq!(t.fg, Color::Indexed(9));
    }

    #[test]
    fn default_look() {
        // tmux 3.8 default: "[0] 0:zsh* 1:vim-" left, right-aligned
        // "\"host\" 21:50 14-Nov-26", green fill.
        let line = render(&parts(80, vec![("0:zsh*", true), ("1:vim-", false)]));
        let t = text(&line);
        assert_eq!(t.chars().count(), 80);
        assert!(t.starts_with("[0] 0:zsh* 1:vim- "), "{t}");
        assert!(t.ends_with("\"host\" 21:50 14-Nov-26"), "{t}");
        assert!(line.iter().all(|s| s.style.bg == Color::Indexed(2)));
    }

    #[test]
    fn styled_entries_and_truncation() {
        let mut p = parts(40, vec![("#[fg=red]0:zsh#[default]*", true)]);
        p.left = "[a-very-long-session] ".into();
        let line = render(&p);
        let t = text(&line);
        assert_eq!(t.chars().count(), 40);
        assert!(t.starts_with("[a-very-lo0:zsh*"), "{t}");
        assert_eq!(line[1].text, "0:zsh");
        assert_eq!(line[1].style.fg, Color::Indexed(1));
        // Too narrow for everything: left first, then right.
        let line = render(&parts(12, vec![("0:zsh*", true)]));
        assert_eq!(text(&line).chars().count(), 12);
    }

    #[test]
    fn long_window_list_scrolls_to_current() {
        let wins: Vec<(String, bool)> = (0..20).map(|i| (format!("{i}:win{i}"), i == 15)).collect();
        let mut p = parts(60, wins.iter().map(|(t, c)| (t.as_str(), *c)).collect());
        p.right = String::new();
        let t = text(&render(&p));
        assert_eq!(t.chars().count(), 60);
        assert!(t.contains("15:win15"), "{t}");
        assert!(t[4..].starts_with('<'), "{t}");
        assert!(t.ends_with('>') || t.ends_with("15:win15"), "{t}");
    }

    #[test]
    fn justify_and_wide_chars() {
        let mut p = parts(40, vec![("0:日本", true)]);
        p.right = String::new();
        p.justify = 2;
        let t = text(&render(&p));
        assert!(t.ends_with("0:日本"), "{t}");
        assert_eq!(t.chars().map(char_width).sum::<usize>(), 40);
        assert_eq!(text_width("日本a\u{301}"), 5);
    }
}
