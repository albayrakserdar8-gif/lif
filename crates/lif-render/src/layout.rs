//! Which source owns each screen cell, and the pane borders derived from
//! the pane rectangles.
//!
//! A border cell is a cell covered by no pane whose 8-neighborhood touches a
//! pane. Its glyph joins with the border cells above, below, left and right,
//! which yields the junctions (├ ┤ ┬ ┴ ┼) of tmux-style layouts. A border cell
//! next to the active pane is drawn in the active style; with exactly two
//! panes only the half of the shared border nearest the active pane is, as
//! tmux does, because the whole border would touch both panes.

use crate::scene::ScenePane;

pub(crate) const BORDER: u16 = 0x40;
pub(crate) const ACTIVE: u16 = 0x10;
pub(crate) const CHROME: u16 = 0x80;
pub(crate) const PANE: u16 = 0x100;

const UP: u16 = 1;
const DOWN: u16 = 2;
const LEFT: u16 = 4;
const RIGHT: u16 = 8;

#[inline]
pub(crate) fn is_border(o: u16) -> bool {
    o < CHROME && o & BORDER != 0
}

/// The box-drawing glyph for a border cell connected in the given
/// directions.
pub fn border_glyph(up: bool, down: bool, left: bool, right: bool) -> char {
    match (up, down, left, right) {
        (true, true, true, true) => '┼',
        (true, true, false, true) => '├',
        (true, true, true, false) => '┤',
        (false, true, true, true) => '┬',
        (true, false, true, true) => '┴',
        (false, true, false, true) => '┌',
        (false, true, true, false) => '┐',
        (true, false, false, true) => '└',
        (true, false, true, false) => '┘',
        (true, _, false, false) | (_, true, false, false) => '│',
        _ => '─',
    }
}

#[inline]
pub(crate) fn glyph_of(o: u16) -> char {
    border_glyph(o & UP != 0, o & DOWN != 0, o & LEFT != 0, o & RIGHT != 0)
}

pub(crate) fn build(
    owner: &mut Vec<u16>,
    cols: usize,
    rows: usize,
    panes: &[ScenePane<'_>],
    chrome: Option<u16>,
) {
    owner.clear();
    owner.resize(cols * rows, 0);
    for (i, p) in panes.iter().enumerate() {
        let r = p.rect;
        let (x1, y1) = (
            (r.right() as usize).min(cols),
            (r.bottom() as usize).min(rows),
        );
        for y in r.y as usize..y1 {
            owner[y * cols + r.x as usize..y * cols + x1].fill(PANE + i as u16);
        }
    }
    if let Some(cy) = chrome {
        let cy = cy as usize;
        owner[cy * cols..(cy + 1) * cols].fill(CHROME);
    }
    let active = panes.iter().position(|p| p.active).map(|a| PANE + a as u16);
    for y in 0..rows {
        for x in 0..cols {
            if owner[y * cols + x] != 0 {
                continue;
            }
            let (mut border, mut act) = (false, false);
            for ny in y.saturating_sub(1)..(y + 2).min(rows) {
                for nx in x.saturating_sub(1)..(x + 2).min(cols) {
                    let o = owner[ny * cols + nx];
                    if o >= PANE {
                        border = true;
                        act |= Some(o) == active;
                    }
                }
            }
            if border {
                owner[y * cols + x] = BORDER | if act { ACTIVE } else { 0 };
            }
        }
    }
    if let (2, Some(a)) = (panes.len(), active) {
        let a = (a - PANE) as usize;
        let (ar, or) = (panes[a].rect, panes[1 - a].rect);
        for y in 0..rows {
            for x in 0..cols {
                let o = &mut owner[y * cols + x];
                if !is_border(*o) || *o & ACTIVE == 0 {
                    continue;
                }
                let (x16, y16) = (x as u16, y as u16);
                let keep = if or.x >= ar.right() && x16 >= ar.right() {
                    y16 < ar.y + ar.rows.div_ceil(2)
                } else if or.right() <= ar.x && x16 < ar.x {
                    y16 >= ar.y + ar.rows / 2
                } else if or.y >= ar.bottom() && y16 >= ar.bottom() {
                    x16 < ar.x + ar.cols.div_ceil(2)
                } else if or.bottom() <= ar.y && y16 < ar.y {
                    x16 >= ar.x + ar.cols / 2
                } else {
                    true
                };
                if !keep {
                    *o &= !ACTIVE;
                }
            }
        }
    }
    for y in 0..rows {
        for x in 0..cols {
            let i = y * cols + x;
            if !is_border(owner[i]) {
                continue;
            }
            let mut m = 0;
            if y > 0 && is_border(owner[i - cols]) {
                m |= UP;
            }
            if y + 1 < rows && is_border(owner[i + cols]) {
                m |= DOWN;
            }
            if x > 0 && is_border(owner[i - 1]) {
                m |= LEFT;
            }
            if x + 1 < cols && is_border(owner[i + 1]) {
                m |= RIGHT;
            }
            owner[i] |= m;
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use lif_types::{PaneFrame, Rect};

    fn draw(cols: u16, rows: u16, rects: &[Rect], active: usize) -> Vec<String> {
        let frames: Vec<PaneFrame> = rects.iter().map(|r| PaneFrame::new(r.size())).collect();
        let panes: Vec<ScenePane> = rects
            .iter()
            .zip(&frames)
            .enumerate()
            .map(|(i, (r, f))| ScenePane {
                id: i as u64,
                rect: *r,
                frame: f,
                active: i == active,
                selection: None,
                matches: &[],
                indicator: None,
                base: Default::default(),
            })
            .collect();
        let mut owner = Vec::new();
        build(&mut owner, cols as usize, rows as usize, &panes, None);
        (0..rows as usize)
            .map(|y| {
                (0..cols as usize)
                    .map(|x| {
                        let o = owner[y * cols as usize + x];
                        if o >= PANE {
                            char::from(b'0' + (o - PANE) as u8)
                        } else if is_border(o) {
                            if o & ACTIVE != 0 { '#' } else { glyph_of(o) }
                        } else {
                            ' '
                        }
                    })
                    .collect()
            })
            .collect()
    }

    fn glyphs(cols: u16, rows: u16, rects: &[Rect]) -> Vec<String> {
        draw(cols, rows, rects, usize::MAX)
    }

    #[test]
    fn one_pane() {
        assert_eq!(glyphs(3, 2, &[Rect::new(0, 0, 3, 2)]), ["000", "000"]);
    }

    #[test]
    fn two_panes() {
        assert_eq!(
            glyphs(5, 2, &[Rect::new(0, 0, 2, 2), Rect::new(3, 0, 2, 2)]),
            ["00│11", "00│11"]
        );
        assert_eq!(
            glyphs(2, 3, &[Rect::new(0, 0, 2, 1), Rect::new(0, 2, 2, 1)]),
            ["00", "──", "11"]
        );
    }

    #[test]
    fn three_panes() {
        // Left pane, right column split in two: a ├ junction.
        let r = [
            Rect::new(0, 0, 2, 5),
            Rect::new(3, 0, 2, 2),
            Rect::new(3, 3, 2, 2),
        ];
        assert_eq!(
            glyphs(5, 5, &r),
            ["00│11", "00│11", "00├──", "00│22", "00│22"]
        );
        // Top pane, bottom row split in two: a ┬ junction.
        let r = [
            Rect::new(0, 0, 5, 2),
            Rect::new(0, 3, 2, 2),
            Rect::new(3, 3, 2, 2),
        ];
        assert_eq!(
            glyphs(5, 5, &r),
            ["00000", "00000", "──┬──", "11│22", "11│22"]
        );
        // Mirrored: ┤ and ┴.
        let r = [
            Rect::new(0, 0, 2, 2),
            Rect::new(0, 3, 2, 2),
            Rect::new(3, 0, 2, 5),
        ];
        assert_eq!(
            glyphs(5, 5, &r),
            ["00│22", "00│22", "──┤22", "11│22", "11│22"]
        );
        let r = [
            Rect::new(0, 0, 2, 2),
            Rect::new(3, 0, 2, 2),
            Rect::new(0, 3, 5, 2),
        ];
        assert_eq!(
            glyphs(5, 5, &r),
            ["00│11", "00│11", "──┴──", "22222", "22222"]
        );
    }

    #[test]
    fn four_panes() {
        let r = [
            Rect::new(0, 0, 2, 2),
            Rect::new(3, 0, 2, 2),
            Rect::new(0, 3, 2, 2),
            Rect::new(3, 3, 2, 2),
        ];
        assert_eq!(
            glyphs(5, 5, &r),
            ["00│11", "00│11", "──┼──", "22│33", "22│33"]
        );
    }

    #[test]
    fn active_border() {
        // Four panes: everything around the active pane, the junction too.
        let r = [
            Rect::new(0, 0, 2, 2),
            Rect::new(3, 0, 2, 2),
            Rect::new(0, 3, 2, 2),
            Rect::new(3, 3, 2, 2),
        ];
        assert_eq!(
            draw(5, 5, &r, 3),
            ["00│11", "00│11", "──###", "22#33", "22#33"]
        );
        // Two panes side by side: top half when the left is active, bottom
        // half when the right is.
        let r = [Rect::new(0, 0, 2, 4), Rect::new(3, 0, 2, 4)];
        assert_eq!(draw(5, 4, &r, 0), ["00#11", "00#11", "00│11", "00│11"]);
        assert_eq!(draw(5, 4, &r, 1), ["00│11", "00│11", "00#11", "00#11"]);
        let r = [Rect::new(0, 0, 4, 1), Rect::new(0, 2, 4, 1)];
        assert_eq!(draw(4, 3, &r, 0), ["0000", "##──", "1111"]);
        assert_eq!(draw(4, 3, &r, 1), ["0000", "──##", "1111"]);
    }
}
