//! Test support: a virtual outer terminal (libghostty-vt), a deliberately
//! simple reference compositor, and the comparison between them.
#![allow(dead_code)]

use libghostty_vt::point::{Coordinate, Point};
use libghostty_vt::render::{CellIterator, CursorVisualStyle, RowIterator};
use libghostty_vt::screen::CellWide;
use libghostty_vt::style::{StyleColor, Underline as GUnderline};
use libghostty_vt::{RenderState, Terminal};
use lif_render::{Scene, SelectionKind, StatusPosition, border_glyph, char_width, rgb_to_256};
use lif_types::{Attrs, CellWidth, Color, CursorShape, OuterCaps, Style, Underline};

/// One cell as seen on the screen.
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct ECell {
    pub text: String,
    /// 0 narrow, 1 wide head, 2 wide tail.
    pub wide: u8,
    pub style: Style,
    pub link: Option<String>,
}

impl ECell {
    pub fn blank() -> Self {
        ECell {
            text: " ".into(),
            wide: 0,
            style: Style::default(),
            link: None,
        }
    }
    fn styled_blank(style: Style) -> Self {
        ECell {
            style,
            ..ECell::blank()
        }
    }
}

/// Make cells that look the same compare the same: a space whose style is
/// invisible on a space, and tails (whose look is their head's).
pub fn normalize(c: &mut ECell) {
    if c.wide == 2 {
        c.text.clear();
        c.style = Style::default();
        c.link = None;
        return;
    }
    let s = &c.style;
    if c.text == " "
        && c.wide == 0
        && s.bg == Color::Default
        && s.underline == Underline::None
        && !s.attrs.contains(Attrs::INVERSE)
        && !s.attrs.contains(Attrs::STRIKETHROUGH)
        && !s.attrs.contains(Attrs::OVERLINE)
    {
        c.style = Style::default();
    }
}

#[derive(Clone, Debug, PartialEq, Eq)]
pub struct Shot {
    pub cols: usize,
    pub rows: usize,
    pub cells: Vec<ECell>,
    /// Cursor position and shape, `None` if hidden.
    pub cursor: Option<(u16, u16, CursorShape, bool)>,
    /// Cursor position whether visible or not.
    pub raw_cursor: (u16, u16),
}

impl Shot {
    pub fn row_text(&self, y: usize) -> String {
        self.cells[y * self.cols..(y + 1) * self.cols]
            .iter()
            .map(|c| if c.wide == 2 { "" } else { c.text.as_str() })
            .collect()
    }
}

/// The client's terminal.
pub struct Vt {
    term: Terminal<'static, 'static>,
    state: RenderState<'static>,
    rit: RowIterator<'static>,
    cit: CellIterator<'static>,
    pub cols: u16,
    pub rows: u16,
}

fn gcolor(c: StyleColor) -> Color {
    match c {
        StyleColor::None => Color::Default,
        StyleColor::Palette(p) => Color::Indexed(p.0),
        StyleColor::Rgb(rgb) => Color::Rgb(rgb.r, rgb.g, rgb.b),
    }
}

fn gstyle(s: libghostty_vt::style::Style) -> Style {
    let mut a = Attrs::NONE;
    for (on, bit) in [
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
            a.insert(bit);
        }
    }
    Style {
        fg: gcolor(s.fg_color),
        bg: gcolor(s.bg_color),
        underline_color: gcolor(s.underline_color),
        attrs: a,
        underline: match s.underline {
            GUnderline::Single => Underline::Single,
            GUnderline::Double => Underline::Double,
            GUnderline::Curly => Underline::Curly,
            GUnderline::Dotted => Underline::Dotted,
            GUnderline::Dashed => Underline::Dashed,
            _ => Underline::None,
        },
    }
}

impl Vt {
    pub fn new(cols: u16, rows: u16) -> Self {
        Vt {
            term: Terminal::new(cols, rows).unwrap(),
            state: RenderState::new().unwrap(),
            rit: RowIterator::new().unwrap(),
            cit: CellIterator::new().unwrap(),
            cols,
            rows,
        }
    }

    pub fn feed(&mut self, b: &[u8]) {
        self.term.vt_write(b);
    }

    pub fn resize(&mut self, cols: u16, rows: u16) {
        self.term.resize(cols, rows, 8, 16).unwrap();
        self.cols = cols;
        self.rows = rows;
    }

    pub fn shot(&mut self) -> Shot {
        let snap = self.state.update(&self.term).unwrap();
        let cur = snap.cursor().unwrap();
        let mut cells = Vec::with_capacity(self.cols as usize * self.rows as usize);
        let mut it = self.rit.update(&snap).unwrap();
        let mut s = String::new();
        let mut buf = vec![0u8; 4096];
        let mut y = 0u32;
        while let Some(row) = it.next() {
            let mut ci = self.cit.update(row).unwrap();
            let mut x = 0u16;
            while let Some(c) = ci.next() {
                let raw = c.raw_cell().unwrap();
                c.graphemes_utf8(&mut s).unwrap();
                let wide = match raw.wide().unwrap() {
                    CellWide::Wide => 1,
                    CellWide::SpacerTail => 2,
                    _ => 0,
                };
                let link = if raw.has_hyperlink().unwrap() {
                    let g = self
                        .term
                        .grid_ref(Point::Viewport(Coordinate { x, y }))
                        .unwrap();
                    let n = g.hyperlink_uri(&mut buf).unwrap();
                    Some(String::from_utf8_lossy(&buf[..n]).into_owned())
                } else {
                    None
                };
                let mut e = ECell {
                    text: if s.is_empty() { " ".into() } else { s.clone() },
                    wide,
                    style: gstyle(c.style().unwrap()),
                    link,
                };
                normalize(&mut e);
                cells.push(e);
                x += 1;
            }
            y += 1;
        }
        let cursor = match (cur.visible, cur.viewport) {
            (true, Some(v)) => {
                let shape = match cur.visual_style {
                    CursorVisualStyle::Bar => CursorShape::Bar,
                    CursorVisualStyle::Underline => CursorShape::Underline,
                    _ => CursorShape::Block,
                };
                Some((v.x, v.y, shape, cur.blinking))
            }
            _ => None,
        };
        let raw_cursor = (self.term.cursor_x().unwrap(), self.term.cursor_y().unwrap());
        Shot {
            cols: self.cols as usize,
            rows: self.rows as usize,
            cells,
            cursor,
            raw_cursor,
        }
    }
}

fn norm_style(s: &mut Style, caps: OuterCaps) {
    if !caps.truecolor {
        for c in [&mut s.fg, &mut s.bg, &mut s.underline_color] {
            if let Color::Rgb(r, g, b) = *c {
                *c = Color::Indexed(rgb_to_256(r, g, b));
            }
        }
    }
    if !caps.styled_underline && s.underline != Underline::None {
        s.underline = Underline::Single;
    }
    if !caps.underline_color {
        s.underline_color = Color::Default;
    }
    if !caps.overline {
        s.attrs.remove(Attrs::OVERLINE);
    }
}

/// Lay out lif UI text the simple way: one cell per character of width 1,
/// two for width 2, zero-width characters joined to the previous cell.
fn ui_text(text: &[(String, Style)], width: usize) -> Vec<ECell> {
    let mut v: Vec<ECell> = Vec::new();
    'all: for (t, st) in text {
        for ch in t.chars() {
            if (ch as u32) < 0x20 || (0x7f..0xa0).contains(&(ch as u32)) {
                continue;
            }
            match char_width(ch) {
                0 => {
                    if let Some(i) = v.iter().rposition(|c| c.wide != 2) {
                        v[i].text.push(ch);
                    }
                }
                w => {
                    if v.len() + w > width {
                        break 'all;
                    }
                    v.push(ECell {
                        text: ch.to_string(),
                        wide: if w == 2 { 1 } else { 0 },
                        style: *st,
                        link: None,
                    });
                    if w == 2 {
                        v.push(ECell {
                            text: String::new(),
                            wide: 2,
                            style: *st,
                            link: None,
                        });
                    }
                }
            }
        }
    }
    v
}

/// The screen the scene should produce, computed cell by cell without any
/// of the renderer's machinery. Panes must not overlap.
pub fn reference(scene: &Scene<'_>, caps: OuterCaps) -> Shot {
    let (cols, rows) = (scene.size.cols as usize, scene.size.rows as usize);
    let chrome = scene.chrome_row().map(|c| c as usize);
    let mut g = vec![ECell::blank(); cols * rows];
    let pane_at = |x: isize, y: isize| -> Option<usize> {
        if x < 0 || y < 0 || x >= cols as isize || y >= rows as isize || Some(y as usize) == chrome
        {
            return None;
        }
        scene
            .panes
            .iter()
            .rposition(|p| p.rect.contains(x as u16, y as u16))
    };
    // Panes.
    for (i, p) in scene.panes.iter().enumerate() {
        let r = p.rect;
        let f = p.frame;
        let x_end = (r.right() as usize).min(cols);
        for py in 0..r.rows as usize {
            let y = r.y as usize + py;
            if y >= rows || Some(y) == chrome {
                continue;
            }
            let frow = (py < f.size.rows as usize)
                .then(|| f.rows.get(py))
                .flatten();
            let mut x = r.x as usize;
            while x < x_end {
                let px = x - r.x as usize;
                let cell = frow.and_then(|fr| {
                    if px < f.size.cols as usize {
                        fr.cells.get(px)
                    } else {
                        None
                    }
                });
                let selected = p
                    .selection
                    .is_some_and(|s| s.contains(px as u16, py as u16));
                // Matches go under the selection; a current one wins.
                let (mut matched, mut current) = (false, false);
                for m in p.matches {
                    if m.y as usize == py && (m.x0 as usize..=m.x1 as usize).contains(&px) {
                        matched = true;
                        current |= m.current;
                    }
                }
                let sel_or = |st: Style| {
                    if selected {
                        scene.theme.selection
                    } else if current {
                        scene.theme.current_match
                    } else if matched {
                        scene.theme.search_match
                    } else {
                        st
                    }
                };
                let Some(cell) = cell else {
                    g[y * cols + x] = ECell::styled_blank(sel_or(Style::default()));
                    x += 1;
                    continue;
                };
                let fr = frow.unwrap();
                let mut text = cell.ch.to_string();
                if cell.extra != 0
                    && let Some(cl) = fr.clusters.get(cell.extra as usize - 1)
                    && cl.chars().count() > 1
                    && !cl
                        .chars()
                        .any(|c| (c as u32) < 0x20 || (0x7f..0xa0).contains(&(c as u32)))
                {
                    text = cl.to_string();
                }
                if text.chars().count() == 1 {
                    let c = text.chars().next().unwrap();
                    if (c as u32) < 0x20
                        || (0x7f..0xa0).contains(&(c as u32))
                        || ((c as u32) >= 0x300 && char_width(c) == 0)
                    {
                        text = " ".into();
                    }
                }
                let link = if caps.hyperlinks && cell.link != 0 {
                    scene
                        .links
                        .and_then(|l| l(i, cell.link))
                        .filter(|u| !u.is_empty())
                        .map(str::to_string)
                } else {
                    None
                };
                let style = sel_or(cell.style);
                match cell.width {
                    CellWidth::Narrow => {
                        g[y * cols + x] = ECell {
                            text,
                            wide: 0,
                            style,
                            link,
                        };
                        x += 1;
                    }
                    CellWidth::Wide
                        if x + 1 < x_end
                            && px + 1 < f.size.cols as usize
                            && fr
                                .cells
                                .get(px + 1)
                                .is_some_and(|t| t.width == CellWidth::WideTail) =>
                    {
                        g[y * cols + x] = ECell {
                            text,
                            wide: 1,
                            style,
                            link: link.clone(),
                        };
                        g[y * cols + x + 1] = ECell {
                            text: String::new(),
                            wide: 2,
                            style,
                            link,
                        };
                        x += 2;
                    }
                    _ => {
                        g[y * cols + x] = ECell::styled_blank(style);
                        x += 1;
                    }
                }
            }
            if py == 0
                && let Some(ind) = p.indicator
            {
                let cells = ui_text(&[(ind.to_string(), scene.theme.indicator)], r.cols as usize);
                let start = r.right() as usize - cells.len();
                for (k, c) in cells.into_iter().enumerate() {
                    if start + k < x_end {
                        g[y * cols + start + k] = c;
                    }
                }
                if start + 1 > r.x as usize + 1
                    && start - 1 < x_end
                    && g[y * cols + start - 1].wide == 1
                {
                    let st = g[y * cols + start - 1].style;
                    g[y * cols + start - 1] = ECell::styled_blank(st);
                }
                if x_end > r.x as usize && g[y * cols + x_end - 1].wide == 1 {
                    let st = g[y * cols + x_end - 1].style;
                    g[y * cols + x_end - 1] = ECell::styled_blank(st);
                }
            }
        }
    }
    // Borders.
    let is_border = |x: isize, y: isize| -> bool {
        if x < 0 || y < 0 || x >= cols as isize || y >= rows as isize || Some(y as usize) == chrome
        {
            return false;
        }
        if pane_at(x, y).is_some() {
            return false;
        }
        (-1..=1).any(|dy| (-1..=1).any(|dx| pane_at(x + dx, y + dy).is_some()))
    };
    let active = scene.panes.iter().position(|p| p.active);
    for y in 0..rows as isize {
        for x in 0..cols as isize {
            if !is_border(x, y) {
                continue;
            }
            let mut act = active.is_some_and(|a| {
                (-1..=1).any(|dy| (-1..=1).any(|dx| pane_at(x + dx, y + dy) == Some(a)))
            });
            if let (true, 2, Some(a)) = (act, scene.panes.len(), active) {
                let (ar, or) = (scene.panes[a].rect, scene.panes[1 - a].rect);
                let (x, y) = (x as u16, y as u16);
                act = if or.x >= ar.right() && x >= ar.right() {
                    y < ar.y + ar.rows.div_ceil(2)
                } else if or.right() <= ar.x && x < ar.x {
                    y >= ar.y + ar.rows / 2
                } else if or.y >= ar.bottom() && y >= ar.bottom() {
                    x < ar.x + ar.cols.div_ceil(2)
                } else if or.bottom() <= ar.y && y < ar.y {
                    x >= ar.x + ar.cols / 2
                } else {
                    true
                };
            }
            let ch = border_glyph(
                is_border(x, y - 1),
                is_border(x, y + 1),
                is_border(x - 1, y),
                is_border(x + 1, y),
            );
            let style = if act {
                scene.theme.active_border
            } else {
                scene.theme.border
            };
            g[y as usize * cols + x as usize] = ECell {
                text: ch.to_string(),
                wide: 0,
                style,
                link: None,
            };
        }
    }
    // Status line or message.
    if let Some(cy) = chrome {
        let (line, fill) = match (scene.message, scene.status) {
            (Some(m), _) => (m.line, m.fill),
            (None, Some(s)) => (s.line, s.fill),
            _ => unreachable!(),
        };
        let spans: Vec<(String, Style)> = line.iter().map(|s| (s.text.clone(), s.style)).collect();
        let mut cells = ui_text(&spans, cols);
        while cells.len() < cols {
            cells.push(ECell::styled_blank(fill));
        }
        for (x, c) in cells.into_iter().enumerate() {
            g[cy * cols + x] = c;
        }
    }
    for c in &mut g {
        norm_style(&mut c.style, caps);
        normalize(c);
    }
    let cur = scene.cursor;
    let cursor = match cur.pos {
        Some(p) if cur.visible && (p.x as usize) < cols && (p.y as usize) < rows => {
            Some((p.x, p.y, cur.shape, cur.blinking))
        }
        _ => None,
    };
    let _ = (SelectionKind::Stream, StatusPosition::Top);
    Shot {
        cols,
        rows,
        cells: g,
        cursor,
        raw_cursor: (0, 0),
    }
}

/// Describe the first differences between two screens, or `None`.
pub fn diff(got: &Shot, want: &Shot) -> Option<String> {
    if got.cols != want.cols || got.rows != want.rows {
        return Some(format!(
            "size {}x{} != {}x{}",
            got.cols, got.rows, want.cols, want.rows
        ));
    }
    let mut msg = String::new();
    let mut n = 0;
    for i in 0..got.cells.len() {
        if got.cells[i] != want.cells[i] {
            if n < 6 {
                msg += &format!(
                    "  at ({}, {}): got {:?}\n               want {:?}\n",
                    i % got.cols,
                    i / got.cols,
                    got.cells[i],
                    want.cells[i]
                );
            }
            n += 1;
        }
    }
    if got.cursor != want.cursor {
        msg += &format!("  cursor: got {:?} want {:?}\n", got.cursor, want.cursor);
        n += 1;
    }
    if n == 0 {
        return None;
    }
    msg += &format!("  {n} differences\n  got:\n");
    for y in 0..got.rows {
        msg += &format!("  |{}|\n", got.row_text(y));
    }
    msg += "  want:\n";
    for y in 0..want.rows {
        msg += &format!("  |{}|\n", want.row_text(y));
    }
    Some(msg)
}

pub fn escape(b: &[u8]) -> String {
    let mut s = String::new();
    for ch in String::from_utf8_lossy(b).chars() {
        match ch {
            '\x1b' => s.push_str("\\e"),
            '\r' => s.push_str("\\r"),
            '\n' => s.push_str("\\n"),
            '\x08' => s.push_str("\\b"),
            c if (c as u32) < 0x20 => s.push_str(&format!("\\x{:02x}", c as u32)),
            c => s.push(c),
        }
    }
    s
}

/// xorshift64*: deterministic, no dependency.
pub struct Rng(pub u64);

impl Rng {
    pub fn next(&mut self) -> u64 {
        let mut x = self.0;
        x ^= x >> 12;
        x ^= x << 25;
        x ^= x >> 27;
        self.0 = x;
        x.wrapping_mul(0x2545_F491_4F6C_DD1D)
    }
    pub fn below(&mut self, n: u64) -> u64 {
        if n == 0 { 0 } else { self.next() % n }
    }
    pub fn range(&mut self, lo: u64, hi: u64) -> u64 {
        lo + self.below(hi - lo + 1)
    }
    pub fn chance(&mut self, p: f64) -> bool {
        (self.next() >> 11) as f64 / (1u64 << 53) as f64 <= p
    }
}
