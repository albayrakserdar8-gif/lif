//! RenderState -> PaneFrame conversion.

use std::collections::HashMap;

use libghostty_vt::mode::Mode;
use libghostty_vt::point::{Coordinate, Point};
use libghostty_vt::render::{CellIterator, CursorVisualStyle, Dirty, RowIteration, RowIterator};
use libghostty_vt::screen::{CellContentTag, CellWide, cell};
use libghostty_vt::style::{self, StyleColor};
use libghostty_vt::{RenderState, Terminal};
use lif_types::{
    Attrs, Cell, CellWidth, Color, Cursor, CursorShape, FrameRow, PaneFrame, Pos, Size, Style,
    Underline,
};

/// Distinct hyperlink URIs kept before the table is reset (which forces a
/// full frame rewrite so no row keeps a stale id).
const MAX_LINKS: usize = 1024;

/// Hyperlink URIs seen in frames; a cell's `link` id is index + 1.
#[derive(Default)]
pub(super) struct Links {
    uris: Vec<Box<str>>,
    ids: HashMap<Box<str>, u32>,
}

impl Links {
    pub(super) fn get(&self, id: u32) -> Option<&str> {
        let i = (id as usize).checked_sub(1)?;
        self.uris.get(i).map(|s| &**s)
    }

    fn intern(&mut self, uri: &str) -> u32 {
        if let Some(&id) = self.ids.get(uri) {
            return id;
        }
        self.uris.push(uri.into());
        let id = self.uris.len() as u32;
        self.ids.insert(uri.into(), id);
        id
    }

    fn clear(&mut self) {
        self.uris.clear();
        self.ids.clear();
    }
}

/// Buffers reused across refreshes.
#[derive(Default)]
pub(super) struct FrameScratch {
    grapheme: String,
    uri: Vec<u8>,
}

fn color(c: StyleColor) -> Color {
    match c {
        StyleColor::None => Color::Default,
        StyleColor::Palette(p) => Color::Indexed(p.0),
        StyleColor::Rgb(rgb) => Color::Rgb(rgb.r, rgb.g, rgb.b),
    }
}

pub(super) fn convert_style(s: style::Style) -> Style {
    let mut attrs = Attrs::NONE;
    for (on, a) in [
        (s.bold, Attrs::BOLD),
        (s.faint, Attrs::FAINT),
        (s.italic, Attrs::ITALIC),
        (s.blink, Attrs::BLINK),
        (s.inverse, Attrs::INVERSE),
        (s.invisible, Attrs::INVISIBLE),
        (s.strikethrough, Attrs::STRIKETHROUGH),
        (s.overline, Attrs::OVERLINE),
    ] {
        if on {
            attrs.insert(a);
        }
    }
    let underline = match s.underline {
        style::Underline::None => Underline::None,
        style::Underline::Single => Underline::Single,
        style::Underline::Double => Underline::Double,
        style::Underline::Curly => Underline::Curly,
        style::Underline::Dotted => Underline::Dotted,
        style::Underline::Dashed => Underline::Dashed,
        // A newer libghostty style we do not know: still underlined.
        _ => Underline::Single,
    };
    Style {
        fg: color(s.fg_color),
        bg: color(s.bg_color),
        underline_color: color(s.underline_color),
        attrs,
        underline,
    }
}

#[allow(clippy::too_many_arguments)]
pub(super) fn refresh(
    term: &Terminal<'static, 'static>,
    render: &mut RenderState<'static>,
    rows: &mut RowIterator<'static>,
    cells: &mut CellIterator<'static>,
    links: &mut Links,
    scratch: &mut FrameScratch,
    size: Size,
    reverse: &mut bool,
    frame: &mut PaneFrame,
) -> bool {
    let mut full = false;
    // DECSCNM (mode 5) is applied by the renderer, so libghostty does not
    // dirty any row for it. PaneFrame has no screen-wide flag, so fold it
    // into every cell's INVERSE bit and rewrite the frame when it flips.
    let rev = term.mode(Mode::REVERSE_COLORS).unwrap_or(false);
    if rev != *reverse {
        *reverse = rev;
        full = true;
    }
    if frame.size != size
        || frame.rows.len() != size.rows as usize
        || frame.dirty.len() != size.rows as usize
    {
        frame.reset(size);
        full = true;
    }
    if links.uris.len() >= MAX_LINKS {
        links.clear();
        full = true;
    }
    let Ok(snap) = render.update(term) else { return false };
    let dirty = snap.dirty().unwrap_or(Dirty::Full);
    full |= dirty == Dirty::Full;
    let mut changed = false;
    if (full || dirty == Dirty::Partial)
        && let Ok(mut it) = rows.update(&snap)
    {
        let nrows = frame.rows.len();
        if full {
            // next_dirty() yields nothing for Dirty::Full, so walk every row.
            let mut y = 0usize;
            while y < nrows
                && let Some(row) = it.next()
            {
                fill_row(term, row, cells, links, scratch, y, &mut frame.rows[y]);
                if rev {
                    invert(&mut frame.rows[y]);
                }
                frame.dirty[y] = true;
                y += 1;
            }
            for yy in y..nrows {
                blank_row(&mut frame.rows[yy]);
                if rev {
                    invert(&mut frame.rows[yy]);
                }
                frame.dirty[yy] = true;
            }
            changed = true;
        } else {
            while let Some((y, row)) = it.next_dirty() {
                let y = y as usize;
                if y >= nrows {
                    break;
                }
                fill_row(term, row, cells, links, scratch, y, &mut frame.rows[y]);
                if rev {
                    invert(&mut frame.rows[y]);
                }
                frame.dirty[y] = true;
                changed = true;
            }
        }
    }
    if let Ok(c) = snap.cursor() {
        let cursor = Cursor {
            pos: c.viewport.map(|v| Pos { x: v.x, y: v.y }),
            visible: c.visible,
            shape: match c.visual_style {
                CursorVisualStyle::Bar => CursorShape::Bar,
                CursorVisualStyle::Underline => CursorShape::Underline,
                _ => CursorShape::Block,
            },
            blinking: c.blinking,
        };
        if cursor != frame.cursor {
            frame.cursor = cursor;
            changed = true;
        }
    }
    let _ = snap.clean();
    changed
}

fn invert(dst: &mut FrameRow) {
    for c in &mut dst.cells {
        c.style.attrs.0 ^= Attrs::INVERSE.0;
    }
}

fn blank_row(dst: &mut FrameRow) {
    dst.cells.fill(Cell::BLANK);
    dst.clusters.clear();
    dst.wrapped = false;
}

fn fill_row(
    term: &Terminal<'static, 'static>,
    row: &RowIteration<'static, '_>,
    cells: &mut CellIterator<'static>,
    links: &mut Links,
    scratch: &mut FrameScratch,
    y: usize,
    dst: &mut FrameRow,
) {
    dst.wrapped = row.raw_row().and_then(|r| r.is_wrapped()).unwrap_or(false);
    let ncols = dst.cells.len();
    let (Ok(raws), Ok(mut ci)) = (row.cells_raw(), cells.update(row)) else {
        blank_row(dst);
        return;
    };
    // Rows never span pages, so a style id is stable within one row: cache
    // the last lookup, since runs of equally styled cells are the norm.
    let mut last: Option<(style::Id, Style)> = None;
    let mut nclusters = 0usize;
    let mut x = 0usize;
    for raw in raws {
        if x >= ncols {
            break;
        }
        let Ok((cp, tag, wide, styled, sid, linked)) = raw.get_multi((
            cell::Codepoint,
            cell::ContentTag,
            cell::Wide,
            cell::HasStyling,
            cell::StyleId,
            cell::HasHyperlink,
        )) else {
            dst.cells[x] = Cell::BLANK;
            x += 1;
            continue;
        };
        let mut out = Cell::BLANK;
        let xs = x as u16;
        if styled {
            out.style = match last {
                Some((id, st)) if id == sid => st,
                _ => {
                    let st = ci
                        .select(xs)
                        .and_then(|()| ci.style())
                        .map(convert_style)
                        .unwrap_or_default();
                    last = Some((sid, st));
                    st
                }
            };
        }
        match tag {
            CellContentTag::Codepoint => {
                out.ch = char::from_u32(cp).filter(|&c| c != '\0').unwrap_or(' ');
            }
            CellContentTag::CodepointGrapheme => {
                out.ch = char::from_u32(cp).unwrap_or(' ');
                if ci.select(xs).and_then(|()| ci.graphemes_utf8(&mut scratch.grapheme)).is_ok()
                    && scratch.grapheme.chars().nth(1).is_some()
                {
                    let g = scratch.grapheme.as_str();
                    // Keep the old allocation when the cluster did not change.
                    match dst.clusters.get_mut(nclusters) {
                        Some(old) if **old == *g => {}
                        Some(old) => *old = g.into(),
                        None => dst.clusters.push(g.into()),
                    }
                    nclusters += 1;
                    out.extra = nclusters as u16;
                }
            }
            CellContentTag::BgColorPalette => {
                if let Ok(p) = raw.bg_color_palette() {
                    out.style.bg = Color::Indexed(p.0);
                }
            }
            CellContentTag::BgColorRgb => {
                if let Ok(c) = raw.bg_color_rgb() {
                    out.style.bg = Color::Rgb(c.r, c.g, c.b);
                }
            }
        }
        match wide {
            CellWide::Wide => out.width = CellWidth::Wide,
            CellWide::SpacerTail => {
                out.width = CellWidth::WideTail;
                out.ch = ' ';
                out.extra = 0;
            }
            _ => {}
        }
        if linked {
            out.link = link_id(term, links, scratch, xs, y);
        }
        dst.cells[x] = out;
        x += 1;
    }
    dst.cells[x..].fill(Cell::BLANK);
    dst.clusters.truncate(nclusters);
}

fn link_id(
    term: &Terminal<'static, 'static>,
    links: &mut Links,
    scratch: &mut FrameScratch,
    x: u16,
    y: usize,
) -> u32 {
    // The render state exposes only "has a hyperlink"; the URI needs a grid
    // ref. This is a slow path, but only for cells that carry a link.
    let Ok(gr) = term.grid_ref(Point::Viewport(Coordinate { x, y: y as u32 })) else {
        return 0;
    };
    let buf = &mut scratch.uri;
    if buf.len() < 256 {
        buf.resize(256, 0);
    }
    let n = match gr.hyperlink_uri(buf) {
        Ok(n) => n,
        Err(libghostty_vt::Error::OutOfSpace { required }) if required <= 64 * 1024 => {
            buf.resize(required, 0);
            match gr.hyperlink_uri(buf) {
                Ok(n) => n,
                Err(_) => return 0,
            }
        }
        Err(_) => return 0,
    };
    if n == 0 {
        return 0;
    }
    match std::str::from_utf8(&buf[..n]) {
        Ok(uri) => links.intern(uri),
        Err(_) => links.intern(&String::from_utf8_lossy(&buf[..n])),
    }
}
