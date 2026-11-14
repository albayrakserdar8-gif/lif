//! Reading any row, scrollback included, and keeping count of rows the
//! scrollback limit evicted.
//!
//! libghostty's grid references resolve a screen point by walking the list
//! of storage pages from the top, reading only each page's header (its row
//! count); the cell data read afterwards lives in the one page holding the
//! row, which is restored if it was compressed. Nothing else is touched, so
//! reading a row deep in a large scrollback costs one header walk per cell
//! plus the cell itself.

use libghostty_vt::grid_ref::TrackedGridRef;
use libghostty_vt::point::{Coordinate, Point, Space};
use libghostty_vt::screen::{CellContentTag, CellWide, RowSemanticPrompt, cell};
use libghostty_vt::style;
use libghostty_vt::Terminal;
use lif_types::{Cell, CellWidth, Color, FrameRow, Style};

use super::frame::convert_style;

/// Longest grapheme cluster kept, in codepoints; longer ones are cut.
const MAX_CLUSTER: usize = 32;

/// Write absolute row `y` of `term` (`cols` wide) into `out`.
pub(super) fn read_row(term: &Terminal<'static, 'static>, cols: u16, y: usize, out: &mut FrameRow) {
    let ncols = cols as usize;
    out.cells.clear();
    out.cells.resize(ncols, Cell::BLANK);
    out.wrapped = false;
    let total = term.total_rows().unwrap_or(0);
    let Ok(yy) = u32::try_from(y) else {
        out.clusters.clear();
        return;
    };
    if y >= total {
        out.clusters.clear();
        return;
    }
    // Points below the viewport's top resolve from the viewport, walking
    // only the pages between it and the row; others from the top of the
    // scrollback. Copy mode keeps the viewport where it reads most.
    let top = term.scrollbar().map_or(u64::MAX, |s| s.offset);
    let at = |x: u16| {
        if u64::from(yy) >= top {
            Point::Viewport(Coordinate { x, y: yy - top as u32 })
        } else {
            Point::Screen(Coordinate { x, y: yy })
        }
    };
    if let Ok(gr) = term.grid_ref(at(0))
        && let Ok(r) = gr.row()
    {
        out.wrapped = r.is_wrapped().unwrap_or(false);
    }
    let mut last: Option<(style::Id, Style)> = None;
    let mut nclusters = 0usize;
    let mut cps = ['\0'; MAX_CLUSTER];
    let mut utf8 = [0u8; MAX_CLUSTER * 4];
    for x in 0..ncols {
        let Ok(gr) = term.grid_ref(at(x as u16)) else { continue };
        let Ok(raw) = gr.cell() else { continue };
        let Ok((cp, tag, wide, styled, sid)) =
            raw.get_multi((cell::Codepoint, cell::ContentTag, cell::Wide, cell::HasStyling, cell::StyleId))
        else {
            continue;
        };
        let mut c = Cell::BLANK;
        if styled {
            c.style = match last {
                Some((id, st)) if id == sid => st,
                _ => {
                    let st = gr.style().map(convert_style).unwrap_or_default();
                    last = Some((sid, st));
                    st
                }
            };
        }
        match tag {
            CellContentTag::Codepoint => c.ch = char::from_u32(cp).filter(|&ch| ch != '\0').unwrap_or(' '),
            CellContentTag::CodepointGrapheme => {
                c.ch = char::from_u32(cp).unwrap_or(' ');
                let n = match gr.graphemes(&mut cps) {
                    Ok(n) => n,
                    Err(libghostty_vt::Error::OutOfSpace { .. }) => MAX_CLUSTER,
                    Err(_) => 0,
                };
                if n > 1 {
                    let mut len = 0;
                    for ch in &cps[..n] {
                        len += ch.encode_utf8(&mut utf8[len..]).len();
                    }
                    // Only chars went in, so this is valid UTF-8.
                    let g = std::str::from_utf8(&utf8[..len]).unwrap_or("");
                    // Keep the old allocation when the cluster is the same.
                    match out.clusters.get_mut(nclusters) {
                        Some(old) if **old == *g => {}
                        Some(old) => *old = g.into(),
                        None => out.clusters.push(g.into()),
                    }
                    nclusters += 1;
                    c.extra = nclusters as u16;
                }
            }
            CellContentTag::BgColorPalette => {
                if let Ok(p) = raw.bg_color_palette() {
                    c.style.bg = Color::Indexed(p.0);
                }
            }
            CellContentTag::BgColorRgb => {
                if let Ok(rgb) = raw.bg_color_rgb() {
                    c.style.bg = Color::Rgb(rgb.r, rgb.g, rgb.b);
                }
            }
        }
        match wide {
            CellWide::Wide if x + 1 < ncols => c.width = CellWidth::Wide,
            CellWide::SpacerTail => {
                c.width = CellWidth::WideTail;
                c.ch = ' ';
                c.extra = 0;
            }
            _ => {}
        }
        out.cells[x] = c;
    }
    out.clusters.truncate(nclusters);
}

/// Whether absolute row `y` starts a prompt.
fn is_prompt(term: &Terminal<'static, 'static>, y: usize) -> bool {
    let Ok(yy) = u32::try_from(y) else { return false };
    term.grid_ref(Point::Screen(Coordinate { x: 0, y: yy }))
        .and_then(|g| g.row())
        .and_then(|r| r.semantic_prompt())
        .is_ok_and(|p| p == RowSemanticPrompt::Prompt)
}

pub(super) fn find_prompt(term: &Terminal<'static, 'static>, from: usize, forward: bool) -> Option<usize> {
    let total = term.total_rows().unwrap_or(0);
    if total == 0 {
        return None;
    }
    if forward {
        (from..total).find(|&y| is_prompt(term, y))
    } else {
        (0..=from.min(total - 1)).rev().find(|&y| is_prompt(term, y))
    }
}

/// Counts rows evicted from the top of the primary screen's scrollback,
/// while tracking is on.
///
/// A tracked reference sits on the newest row; libghostty moves it with its
/// text, so when rows are evicted above it its screen row number drops by
/// exactly the number evicted. It is moved back to the newest row after
/// every write, so it is never old enough to be evicted itself unless one
/// write brings more than a whole scrollback of output; then the count only
/// knows that every row older than the anchor is gone.
#[derive(Default)]
pub(super) struct TrimCounter {
    anchor: Option<TrackedGridRef>,
    /// Screen row of the anchor when it was placed.
    placed_at: u32,
    pub(super) trimmed: u64,
}

impl TrimCounter {
    /// After a write: count what was evicted, then move the anchor to the
    /// newest row. `primary`: the primary screen is active (the alternate
    /// screen has no scrollback and leaves the anchor alone).
    pub(super) fn after_write(&mut self, term: &mut Terminal<'static, 'static>, rows: u16, primary: bool) {
        if !primary || self.anchor.is_none() {
            return;
        }
        if let Some(a) = &self.anchor {
            match a.point(Space::Screen) {
                Ok(Some(p)) if p.y < self.placed_at => self.trimmed += u64::from(self.placed_at - p.y),
                Ok(Some(_)) => {}
                // The anchor itself was evicted: one write brought more than
                // the whole scrollback. Every row it had seen is gone, which
                // is all the count is used for; how many newer rows went
                // too is unknown.
                _ if a.has_value() => {}
                _ => self.trimmed += u64::from(self.placed_at) + 1,
            }
        }
        self.place(term, rows);
    }

    /// Start or stop counting. Without an anchor nothing is counted and
    /// libghostty has no reference to keep up to date.
    pub(super) fn set_tracking(&mut self, term: &mut Terminal<'static, 'static>, rows: u16, on: bool) {
        if on && self.anchor.is_none() {
            self.place(term, rows);
        } else if !on {
            self.anchor = None;
        }
    }

    /// Whether counting is on.
    pub(super) fn tracking(&self) -> bool {
        self.anchor.is_some()
    }

    /// Move the anchor without counting (after a resize, whose reflow
    /// renumbers rows without evicting any).
    pub(super) fn place(&mut self, term: &mut Terminal<'static, 'static>, rows: u16) {
        let newest = Point::Active(Coordinate { x: 0, y: u32::from(rows.max(1) - 1) });
        let ok = match self.anchor.as_mut() {
            Some(a) => a.set(term, newest).is_ok(),
            None => match term.track_grid_ref(newest) {
                Ok(a) => {
                    self.anchor = Some(a);
                    true
                }
                Err(_) => false,
            },
        };
        self.placed_at = match (ok, self.anchor.as_ref()) {
            (true, Some(a)) => a.point(Space::Screen).ok().flatten().map_or(0, |p| p.y),
            _ => 0,
        };
    }
}
