//! Character widths for lif's own UI text (status line, messages,
//! indicators). Pane contents carry their widths from the VT engine; this
//! table is only used for strings lif itself lays out, so a compact table of
//! the common wide and zero-width ranges is enough and avoids a dependency.

use crate::cell::{RCell, W};
use lif_types::{Span, Style};

const ZERO: &[(u32, u32)] = &[
    (0x0300, 0x036F),
    (0x0483, 0x0489),
    (0x0591, 0x05BD),
    (0x05BF, 0x05BF),
    (0x05C1, 0x05C2),
    (0x05C4, 0x05C5),
    (0x05C7, 0x05C7),
    (0x0610, 0x061A),
    (0x064B, 0x065F),
    (0x0670, 0x0670),
    (0x06D6, 0x06DC),
    (0x06DF, 0x06E4),
    (0x06E7, 0x06E8),
    (0x06EA, 0x06ED),
    (0x0900, 0x0902),
    (0x093A, 0x093A),
    (0x093C, 0x093C),
    (0x0941, 0x0948),
    (0x094D, 0x094D),
    (0x0951, 0x0957),
    (0x0E31, 0x0E31),
    (0x0E34, 0x0E3A),
    (0x0E47, 0x0E4E),
    (0x1AB0, 0x1AFF),
    (0x1DC0, 0x1DFF),
    (0x200B, 0x200F),
    (0x202A, 0x202E),
    (0x2060, 0x2064),
    (0x20D0, 0x20FF),
    (0x302A, 0x302D),
    (0x3099, 0x309A),
    (0xFE00, 0xFE0F),
    (0xFE20, 0xFE2F),
    (0xFEFF, 0xFEFF),
    (0xE0000, 0xE0FFF),
];

const WIDE: &[(u32, u32)] = &[
    (0x1100, 0x115F),
    (0x231A, 0x231B),
    (0x2329, 0x232A),
    (0x23E9, 0x23EC),
    (0x23F0, 0x23F0),
    (0x23F3, 0x23F3),
    (0x25FD, 0x25FE),
    (0x2614, 0x2615),
    (0x2648, 0x2653),
    (0x267F, 0x267F),
    (0x2693, 0x2693),
    (0x26A1, 0x26A1),
    (0x26AA, 0x26AB),
    (0x26BD, 0x26BE),
    (0x26C4, 0x26C5),
    (0x26CE, 0x26CE),
    (0x26D4, 0x26D4),
    (0x26EA, 0x26EA),
    (0x26F2, 0x26F3),
    (0x26F5, 0x26F5),
    (0x26FA, 0x26FA),
    (0x26FD, 0x26FD),
    (0x2705, 0x2705),
    (0x270A, 0x270B),
    (0x2728, 0x2728),
    (0x274C, 0x274C),
    (0x274E, 0x274E),
    (0x2753, 0x2755),
    (0x2757, 0x2757),
    (0x2795, 0x2797),
    (0x27B0, 0x27B0),
    (0x27BF, 0x27BF),
    (0x2B1B, 0x2B1C),
    (0x2B50, 0x2B50),
    (0x2B55, 0x2B55),
    (0x2E80, 0x3029),
    (0x302E, 0x303E),
    (0x3041, 0x3098),
    (0x309B, 0x33FF),
    (0x3400, 0x4DBF),
    (0x4E00, 0x9FFF),
    (0xA000, 0xA4CF),
    (0xA960, 0xA97F),
    (0xAC00, 0xD7A3),
    (0xF900, 0xFAFF),
    (0xFE10, 0xFE19),
    (0xFE30, 0xFE6F),
    (0xFF00, 0xFF60),
    (0xFFE0, 0xFFE6),
    (0x16FE0, 0x16FE4),
    (0x17000, 0x18CFF),
    (0x1B000, 0x1B2FF),
    (0x1F004, 0x1F004),
    (0x1F0CF, 0x1F0CF),
    (0x1F18E, 0x1F18E),
    (0x1F191, 0x1F19A),
    (0x1F200, 0x1F202),
    (0x1F210, 0x1F23B),
    (0x1F240, 0x1F248),
    (0x1F250, 0x1F251),
    (0x1F260, 0x1F265),
    (0x1F300, 0x1F320),
    (0x1F32D, 0x1F335),
    (0x1F337, 0x1F37C),
    (0x1F37E, 0x1F393),
    (0x1F3A0, 0x1F3CA),
    (0x1F3CF, 0x1F3D3),
    (0x1F3E0, 0x1F3F0),
    (0x1F3F4, 0x1F3F4),
    (0x1F3F8, 0x1F43E),
    (0x1F440, 0x1F440),
    (0x1F442, 0x1F4FC),
    (0x1F4FF, 0x1F53D),
    (0x1F54B, 0x1F54E),
    (0x1F550, 0x1F567),
    (0x1F57A, 0x1F57A),
    (0x1F595, 0x1F596),
    (0x1F5A4, 0x1F5A4),
    (0x1F5FB, 0x1F64F),
    (0x1F680, 0x1F6C5),
    (0x1F6CC, 0x1F6CC),
    (0x1F6D0, 0x1F6D2),
    (0x1F6D5, 0x1F6D7),
    (0x1F6DC, 0x1F6DF),
    (0x1F6EB, 0x1F6EC),
    (0x1F6F4, 0x1F6FC),
    (0x1F7E0, 0x1F7EB),
    (0x1F7F0, 0x1F7F0),
    (0x1F90C, 0x1F93A),
    (0x1F93C, 0x1F945),
    (0x1F947, 0x1F9FF),
    (0x1FA70, 0x1FAFF),
    (0x20000, 0x2FFFD),
    (0x30000, 0x3FFFD),
];

fn in_table(u: u32, t: &[(u32, u32)]) -> bool {
    t.binary_search_by(|&(lo, hi)| {
        if hi < u {
            std::cmp::Ordering::Less
        } else if lo > u {
            std::cmp::Ordering::Greater
        } else {
            std::cmp::Ordering::Equal
        }
    })
    .is_ok()
}

/// Display width of `c` in cells: 0 for control and combining characters,
/// 2 for East Asian wide characters and emoji presentation, else 1.
pub fn char_width(c: char) -> usize {
    let u = c as u32;
    if u < 0x7f {
        return usize::from(u >= 0x20);
    }
    if u < 0xa0 {
        return 0;
    }
    if in_table(u, ZERO) {
        0
    } else if in_table(u, WIDE) {
        2
    } else {
        1
    }
}

#[inline]
pub(crate) fn is_control(c: char) -> bool {
    let u = c as u32;
    u < 0x20 || (0x7f..0xa0).contains(&u)
}

/// Lay out styled text into at most `width` cells appended to `dst`;
/// clusters go into `arena`. With `fill`, pads to exactly `width`. A wide
/// character that does not fit ends the text.
pub(crate) fn layout_spans<'s>(
    spans: impl Iterator<Item = (&'s str, Style)>,
    width: usize,
    fill: Option<Style>,
    dst: &mut Vec<RCell>,
    arena: &mut String,
) {
    let start = dst.len();
    'outer: for (text, style) in spans {
        for c in text.chars() {
            if is_control(c) {
                continue;
            }
            let w = char_width(c);
            if w == 0 {
                let n = dst.len();
                if n == start {
                    continue;
                }
                let hi = if dst[n - 1].w == W::Tail {
                    n - 2
                } else {
                    n - 1
                };
                let head = &mut dst[hi];
                if head.len == 0 {
                    head.off = arena.len() as u32;
                    arena.push(head.ch);
                    head.len = head.ch.len_utf8() as u16;
                }
                if head.len as usize + c.len_utf8() > u16::MAX as usize {
                    continue;
                }
                arena.push(c);
                head.len += c.len_utf8() as u16;
                continue;
            }
            if dst.len() - start + w > width {
                break 'outer;
            }
            let cell = RCell {
                ch: c,
                w: if w == 2 { W::Wide } else { W::Narrow },
                style,
                ..RCell::BLANK
            };
            dst.push(cell);
            if w == 2 {
                dst.push(RCell {
                    w: W::Tail,
                    ch: ' ',
                    len: 0,
                    ..cell
                });
            }
        }
    }
    if let Some(fill) = fill {
        while dst.len() - start < width {
            dst.push(RCell {
                style: fill,
                ..RCell::BLANK
            });
        }
    }
}

pub(crate) fn line_spans(line: &[Span]) -> impl Iterator<Item = (&str, Style)> {
    line.iter().map(|s| (s.text.as_str(), s.style))
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn widths() {
        assert_eq!(char_width('a'), 1);
        assert_eq!(char_width('\u{7}'), 0);
        assert_eq!(char_width('\u{301}'), 0);
        assert_eq!(char_width('中'), 2);
        assert_eq!(char_width('\u{1F600}'), 2);
        assert_eq!(char_width('é'), 1);
        assert_eq!(char_width('─'), 1);
    }

    #[test]
    fn layout_wide_cut_and_cluster() {
        let mut dst = Vec::new();
        let mut arena = String::new();
        let line = vec![Span::plain("ae\u{301}中文")];
        layout_spans(
            line_spans(&line),
            4,
            Some(Style::default()),
            &mut dst,
            &mut arena,
        );
        assert_eq!(dst.len(), 4);
        assert_eq!(dst[1].text(&arena), Some("e\u{301}"));
        assert_eq!(dst[2].w, W::Wide);
        assert_eq!(dst[3].w, W::Tail);
        dst.clear();
        layout_spans(
            line_spans(&line),
            3,
            Some(Style::default()),
            &mut dst,
            &mut arena,
        );
        // The wide char does not fit in the last cell: blank instead.
        assert_eq!(dst.len(), 3);
        assert_eq!(dst[2].ch, ' ');
        assert_eq!(dst[2].w, W::Narrow);
    }
}
