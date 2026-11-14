//! The diff renderer.

use crate::cell::{DEFAULT_STYLE, RCell, W, hash_cells, same, style_eq};
use crate::esc::{Sink, csi_n, num};
use crate::layout::{self, ACTIVE, CHROME, PANE, glyph_of, is_border};
use crate::moves::move_cursor;
use crate::scene::{LinkLookup, Scene, Selection, StatusPosition, Theme};
use crate::sgr::{self, Norm};
use crate::text;
use lif_types::{
    Cell, CellWidth, Cursor, CursorShape, FrameRow, OuterCaps, Rect, Size, Style, StyledLine,
};
use std::collections::HashMap;

/// Distinct hyperlink URIs remembered before the table is dropped (which
/// costs one full repaint). Bounds the memory a hostile program can make the
/// renderer hold.
const LINK_LIMIT: usize = 4096;
/// Shortest run of blanks erased with ECH instead of printed as spaces
/// (ECH plus the cursor movement after it costs about 8 bytes).
const ECH_MIN: usize = 10;
/// A scroll must make at least this many more rows match than before.
const SCROLL_MIN_GAIN: usize = 2;
/// Bound on a row's cluster arena before it is compacted.
const ARENA_SLACK: usize = 1024;

#[derive(Default)]
struct LinkTable {
    map: HashMap<Box<str>, u32>,
    uris: Vec<Box<str>>,
    /// One-entry cache of the last (pane index, link id) resolved this frame.
    key: Option<(usize, u32)>,
    val: u32,
}

impl LinkTable {
    fn uri(&self, rid: u32) -> &str {
        &self.uris[rid as usize - 1]
    }

    fn resolve(&mut self, pane: usize, id: u32, lookup: LinkLookup<'_>) -> u32 {
        if self.key == Some((pane, id)) {
            return self.val;
        }
        let rid = match lookup(pane, id) {
            Some(u) if !u.is_empty() => match self.map.get(u) {
                Some(&r) => r,
                None => {
                    self.uris.push(u.into());
                    let r = self.uris.len() as u32;
                    self.map.insert(u.into(), r);
                    r
                }
            },
            _ => 0,
        };
        self.key = Some((pane, id));
        self.val = rid;
        rid
    }

    fn clear(&mut self) {
        self.map.clear();
        self.uris.clear();
        self.key = None;
    }
}

/// What the client's terminal is believed to show, and its cursor, SGR and
/// hyperlink state.
struct Outer {
    cols: usize,
    rows: usize,
    cells: Vec<RCell>,
    /// Cluster text of each row's cells.
    arenas: Vec<String>,
    /// Cached hash of each full row, for scroll detection.
    rhash: Vec<u64>,
    rhash_ok: Vec<bool>,
    cx: Option<u16>,
    cy: Option<u16>,
    sgr: Style,
    link: u32,
    visible: Option<bool>,
    shape: Option<(CursorShape, bool)>,
    /// Bytes of this frame have been emitted (sync started / cursor hidden).
    begun: bool,
    caps: OuterCaps,
    tmp: String,
}

impl Outer {
    fn new(caps: OuterCaps) -> Self {
        Outer {
            cols: 0,
            rows: 0,
            cells: Vec::new(),
            arenas: Vec::new(),
            rhash: Vec::new(),
            rhash_ok: Vec::new(),
            cx: None,
            cy: None,
            sgr: DEFAULT_STYLE,
            link: 0,
            visible: None,
            shape: None,
            begun: false,
            caps,
            tmp: String::new(),
        }
    }

    fn size(&self) -> Size {
        Size::new(self.cols as u16, self.rows as u16)
    }

    fn resize(&mut self, size: Size) {
        self.cols = size.cols as usize;
        self.rows = size.rows as usize;
        self.cells.clear();
        self.cells.resize(self.cols * self.rows, RCell::BLANK);
        self.arenas.resize_with(self.rows, String::new);
        self.rhash.resize(self.rows, 0);
        self.rhash_ok.clear();
        self.rhash_ok.resize(self.rows, false);
    }

    /// Called before the first byte that changes the screen in this frame.
    #[inline]
    fn begin(&mut self, out: &mut Vec<u8>) {
        if self.begun {
            return;
        }
        self.begun = true;
        if self.caps.sync_output {
            out.extend_from_slice(b"\x1b[?2026h");
        } else if self.visible != Some(false) {
            // Without synchronized output the terminal may show the cursor
            // jumping around while the frame is drawn.
            out.extend_from_slice(b"\x1b[?25l");
            self.visible = Some(false);
        }
    }

    /// Put the outer terminal into a known state and clear it.
    fn full_reset(&mut self, out: &mut Vec<u8>) {
        self.visible = None;
        self.begin(out);
        out.extend_from_slice(b"\x1b[m");
        if self.caps.hyperlinks {
            out.extend_from_slice(b"\x1b]8;;\x1b\\");
        }
        // Full scroll region, autowrap off for as long as lif draws (so the
        // bottom-right cell can be written without scrolling), home, clear.
        out.extend_from_slice(b"\x1b[r\x1b[?7l\x1b[H\x1b[2J");
        self.cells.fill(RCell::BLANK);
        for a in &mut self.arenas {
            a.clear();
        }
        self.rhash_ok.fill(false);
        self.cx = Some(0);
        self.cy = Some(0);
        self.sgr = DEFAULT_STYLE;
        self.link = 0;
        self.shape = None;
    }

    fn move_to(&mut self, out: &mut Vec<u8>, x: usize, y: usize) {
        let (tx, ty) = (x as u16, y as u16);
        if self.cx == Some(tx) && self.cy == Some(ty) {
            return;
        }
        let mut rp = [0u8; 4];
        let mut n = 0;
        if let (Some(fx), Some(fy)) = (self.cx, self.cy)
            && fy == ty
            && tx > fx
            && (tx - fx) as usize <= rp.len()
        {
            let row = y * self.cols;
            let ok = (fx as usize..x).all(|i| {
                let c = &self.cells[row + i];
                c.w == W::Narrow
                    && c.len == 0
                    && (0x20..0x7f).contains(&(c.ch as u32))
                    && style_eq(&c.style, &self.sgr)
                    && c.link == self.link
            });
            if ok {
                for i in fx as usize..x {
                    rp[n] = self.cells[row + i].ch as u8;
                    n += 1;
                }
            }
        }
        move_cursor(out, self.cx, self.cy, tx, ty, &rp[..n]);
        self.cx = Some(tx);
        self.cy = Some(ty);
    }

    fn set_link(&mut self, rid: u32, links: &LinkTable, out: &mut Vec<u8>) {
        if rid == self.link {
            return;
        }
        out.extend_from_slice(b"\x1b]8;;");
        if rid != 0 {
            // The URI came from a program; never let it end the OSC early.
            out.extend(links.uri(rid).bytes().filter(|&b| b >= 0x20 && b != 0x7f));
        }
        out.extend_from_slice(b"\x1b\\");
        self.link = rid;
    }

    /// Erasing (EL, ECH, scrolling) fills with the current background, and
    /// terminals differ on which other attributes they keep, so erase only
    /// in the default style and outside any hyperlink.
    fn erase_safe(&mut self, links: &LinkTable, out: &mut Vec<u8>) {
        self.set_link(0, links, out);
        if self.sgr != DEFAULT_STYLE {
            sgr::transition(&self.sgr, &DEFAULT_STYLE, out);
            self.sgr = DEFAULT_STYLE;
        }
    }

    /// Overwriting the tail of a wide character at `x` destroys its head.
    #[inline]
    fn orphan_left(&mut self, row: usize, x: usize) {
        if x > 0
            && x < self.cols
            && self.cells[row + x].w == W::Tail
            && self.cells[row + x - 1].w == W::Wide
        {
            self.cells[row + x - 1] = RCell::UNKNOWN;
        }
    }

    /// Overwriting the head of a wide character just before `x` destroys
    /// the tail at `x`.
    #[inline]
    fn orphan_right(&mut self, row: usize, x: usize) {
        if x < self.cols && self.cells[row + x].w == W::Tail {
            self.cells[row + x] = RCell::UNKNOWN;
        }
    }

    /// Print desired cell `d` at the cursor, which is at (`x`, `y`).
    /// Returns the number of columns it covers.
    fn put(
        &mut self,
        out: &mut Vec<u8>,
        links: &LinkTable,
        x: usize,
        y: usize,
        d: &RCell,
        da: &str,
    ) -> usize {
        if !style_eq(&self.sgr, &d.style) {
            sgr::transition(&self.sgr, &d.style, out);
            self.sgr = d.style;
        }
        if self.link != d.link {
            self.set_link(d.link, links, out);
        }
        let text = d.text(da);
        let row = y * self.cols;
        let span = if d.w == W::Wide { 2 } else { 1 };
        match text {
            // With autowrap off, a terminal leaves the cursor on the last
            // column after printing there, and attaches the combining marks
            // that follow to the cell before it. With autowrap on it sets
            // the pending-wrap state instead, which attaches them correctly
            // (and does not scroll, as nothing is printed after them).
            Some(t) if x + span == self.cols => {
                out.extend_from_slice(b"\x1b[?7h");
                out.extend_from_slice(t.as_bytes());
                out.extend_from_slice(b"\x1b[?7l");
            }
            Some(t) => out.extend_from_slice(t.as_bytes()),
            None => {
                let mut b = [0u8; 4];
                out.extend_from_slice(d.ch.encode_utf8(&mut b).as_bytes());
            }
        }
        debug_assert!(x + span <= self.cols);
        self.orphan_left(row, x);
        self.orphan_right(row, x + span);
        let mut c = *d;
        if let Some(t) = text {
            let ar = &mut self.arenas[y];
            c.off = ar.len() as u32;
            ar.push_str(t);
        }
        self.cells[row + x] = c;
        if span == 2 {
            self.cells[row + x + 1] = RCell {
                w: W::Tail,
                ch: ' ',
                len: 0,
                ..c
            };
        }
        let nx = x + span;
        // With autowrap off the cursor sticks to the last column, but not
        // every terminal agrees, so forget the column there; likewise after
        // a cluster the outer terminal may have measured differently.
        self.cx = if nx >= self.cols || (d.len > 0 && !self.caps.grapheme_clusters) {
            None
        } else {
            Some(nx as u16)
        };
        self.rhash_ok[y] = false;
        span
    }

    /// Fast path after printing a plain narrow cell: keep printing the
    /// following differing cells as long as they are plain, narrow, in the
    /// current style and link, and nothing about them needs the general
    /// path (wide neighbors, the last column, erasable blank runs).
    /// Returns where it stopped; the cursor is there.
    #[allow(clippy::too_many_arguments)]
    #[inline]
    fn put_run(
        &mut self,
        out: &mut Vec<u8>,
        y: usize,
        mut x: usize,
        a: usize,
        b: usize,
        des: &[RCell],
        el_from: usize,
    ) -> usize {
        let row = y * self.cols;
        let last = self.cols - 1;
        let start = x;
        while x < b && x < last {
            let d = &des[x - a];
            let m = &self.cells[row + x];
            if d.w != W::Narrow
                || d.len != 0
                || !style_eq(&d.style, &self.sgr)
                || d.link != self.link
                || m.w == W::Wide
                || m.w == W::Tail
            {
                break;
            }
            if d.ch == m.ch
                && m.w == W::Narrow
                && m.len == 0
                && style_eq(&m.style, &d.style)
                && m.link == d.link
            {
                break;
            }
            if d.ch == ' '
                && d.is_blank()
                && (x >= el_from
                    || des[x - a..(x - a + ECH_MIN).min(b - a)]
                        .iter()
                        .all(RCell::is_blank))
            {
                break;
            }
            let ch = d.ch;
            if (ch as u32) < 0x80 {
                out.push(ch as u8);
            } else {
                let mut buf = [0u8; 4];
                out.extend_from_slice(ch.encode_utf8(&mut buf).as_bytes());
            }
            self.cells[row + x] = *d;
            x += 1;
        }
        if x != start {
            self.cx = Some(x as u16);
        }
        x
    }

    /// Bring row `y`, columns `a .. a + des.len()`, to the desired cells.
    /// Returns true when it disturbed cells outside the range (half of a wide
    /// character), in which case the caller must diff the whole row.
    fn diff(
        &mut self,
        out: &mut Vec<u8>,
        links: &LinkTable,
        y: usize,
        a: usize,
        des: &[RCell],
        da: &str,
    ) -> bool {
        let cols = self.cols;
        let b = a + des.len();
        let row = y * cols;
        let mut x = a;
        while x < b && same(&des[x - a], da, &self.cells[row + x], &self.arenas[y]) {
            x += 1;
        }
        if x == b {
            return false;
        }
        // From here to the end of the screen row everything is blank, so
        // EL can stand in for printing.
        let el_from = if b == cols {
            let mut f = b;
            while f > a && des[f - 1 - a].is_blank() {
                f -= 1;
            }
            f
        } else {
            usize::MAX
        };
        while x < b {
            let d = &des[x - a];
            if same(d, da, &self.cells[row + x], &self.arenas[y]) {
                x += 1;
                continue;
            }
            if d.w == W::Tail {
                // Drawn with its head; a tail without a head cannot occur in
                // composed rows.
                x += 1;
                continue;
            }
            if x >= el_from {
                let dirty = self.cells[row + x..row + b]
                    .iter()
                    .filter(|c| !c.is_blank())
                    .count();
                if dirty >= 2 {
                    self.begin(out);
                    self.move_to(out, x, y);
                    self.erase_safe(links, out);
                    out.extend_from_slice(b"\x1b[K");
                    self.orphan_left(row, x);
                    self.cells[row + x..row + cols].fill(RCell::BLANK);
                    self.rhash_ok[y] = false;
                    break;
                }
            }
            if d.is_blank() {
                let mut e = x;
                let mut last = x;
                while e < b && des[e - a].is_blank() {
                    if !self.cells[row + e].is_blank() {
                        last = e;
                    }
                    e += 1;
                }
                let n = last + 1 - x;
                if n >= ECH_MIN {
                    self.begin(out);
                    self.move_to(out, x, y);
                    self.erase_safe(links, out);
                    csi_n(out, n as u32, b'X');
                    self.orphan_left(row, x);
                    self.orphan_right(row, x + n);
                    self.cells[row + x..row + x + n].fill(RCell::BLANK);
                    self.rhash_ok[y] = false;
                    x += n;
                    continue;
                }
            }
            self.begin(out);
            self.move_to(out, x, y);
            let span = self.put(out, links, x, y, d, da);
            x += span;
            if span == 1 && d.len == 0 {
                x = self.put_run(out, y, x, a, b, des, el_from);
            }
        }
        self.maybe_compact(y);
        (a > 0 && self.cells[row + a - 1].w == W::Unknown)
            || (b < cols && self.cells[row + b].w == W::Unknown)
    }

    fn maybe_compact(&mut self, y: usize) {
        if self.arenas[y].len() <= ARENA_SLACK + 16 * self.cols {
            return;
        }
        let row = y * self.cols;
        self.tmp.clear();
        for c in &mut self.cells[row..row + self.cols] {
            if let Some(t) = c.text(&self.arenas[y]) {
                let off = self.tmp.len() as u32;
                self.tmp.push_str(t);
                c.off = off;
            }
        }
        std::mem::swap(&mut self.arenas[y], &mut self.tmp);
    }

    fn seg_hash(&mut self, y: usize, a: usize, b: usize) -> u64 {
        let row = y * self.cols;
        if a == 0 && b == self.cols {
            if !self.rhash_ok[y] {
                self.rhash[y] = hash_cells(&self.cells[row..row + b], &self.arenas[y]);
                self.rhash_ok[y] = true;
            }
            return self.rhash[y];
        }
        hash_cells(&self.cells[row + a..row + b], &self.arenas[y])
    }

    fn copy_seg(&mut self, src: usize, dst: usize, a: usize, b: usize) {
        let cols = self.cols;
        if a == 0 && b == cols {
            self.rhash[dst] = self.rhash[src];
            self.rhash_ok[dst] = self.rhash_ok[src];
        } else {
            self.rhash_ok[dst] = false;
        }
        for x in a..b {
            let mut c = self.cells[src * cols + x];
            if c.len > 0 {
                self.tmp.clear();
                self.tmp.push_str(c.text(&self.arenas[src]).unwrap_or(""));
                c.off = self.arenas[dst].len() as u32;
                self.arenas[dst].push_str(&self.tmp);
            }
            self.cells[dst * cols + x] = c;
        }
    }

    /// Mirror a scroll of rows `top..bot`, columns `a..b`, by `k` rows
    /// (positive: content moves up), with blank rows coming in.
    fn shift(&mut self, top: usize, bot: usize, a: usize, b: usize, k: isize) {
        let cols = self.cols;
        let n = k.unsigned_abs();
        if k > 0 {
            for y in top..bot - n {
                self.copy_seg(y + n, y, a, b);
            }
            for y in bot - n..bot {
                self.cells[y * cols + a..y * cols + b].fill(RCell::BLANK);
                self.rhash_ok[y] = false;
            }
        } else {
            for y in (top + n..bot).rev() {
                self.copy_seg(y - n, y, a, b);
            }
            for y in top..top + n {
                self.cells[y * cols + a..y * cols + b].fill(RCell::BLANK);
                self.rhash_ok[y] = false;
            }
        }
        for y in top..bot {
            self.maybe_compact(y);
        }
    }

    fn cursor(&mut self, out: &mut Vec<u8>, c: &Cursor) {
        let pos = c
            .pos
            .filter(|p| (p.x as usize) < self.cols && (p.y as usize) < self.rows);
        match pos {
            Some(p) if c.visible => {
                self.move_to(out, p.x as usize, p.y as usize);
                if self.shape != Some((c.shape, c.blinking)) {
                    let n = match c.shape {
                        CursorShape::Block => 1,
                        CursorShape::Underline => 3,
                        CursorShape::Bar => 5,
                    } + u32::from(!c.blinking);
                    out.extend_from_slice(b"\x1b[");
                    num(out, n);
                    out.extend_from_slice(b" q");
                    self.shape = Some((c.shape, c.blinking));
                }
                if self.visible != Some(true) {
                    out.extend_from_slice(b"\x1b[?25h");
                    self.visible = Some(true);
                }
            }
            _ => {
                if self.visible != Some(false) {
                    out.extend_from_slice(b"\x1b[?25l");
                    self.visible = Some(false);
                }
            }
        }
    }
}

/// Per-renderer memory of one pane of the last scene.
#[derive(Default)]
struct Track {
    id: u64,
    rect: Rect,
    fsize: Size,
    active: bool,
    /// Rows changed since this renderer last drew them. This is what makes
    /// skipped frames correct: dirty flags of frames this renderer did not
    /// draw are accumulated here (see [`Renderer::skip`]).
    stale: Vec<bool>,
    nstale: usize,
    sel: Option<Selection>,
    has_ind: bool,
    ind: String,
}

impl Track {
    fn mark(&mut self, lo: usize, hi: usize) {
        let hi = hi.min(self.stale.len().saturating_sub(1));
        for py in lo..=hi {
            if py < self.stale.len() && !self.stale[py] {
                self.stale[py] = true;
                self.nstale += 1;
            }
        }
    }

    fn clear(&mut self) {
        if self.nstale > 0 {
            self.stale.fill(false);
            self.nstale = 0;
        }
    }
}

/// Composition state borrowed from the renderer for one composed range.
struct Comp<'r, 's> {
    cols: usize,
    owner: &'r [u16],
    scene: &'r Scene<'s>,
    norm: &'r mut Norm,
    links: &'r mut LinkTable,
    crow: &'r [RCell],
    carena: &'r str,
    ind: &'r mut Vec<RCell>,
    iarena: &'r mut String,
}

/// Compose screen row `y`, columns `a..b`, into `dst[x - base]`.
fn compose(
    c: &mut Comp<'_, '_>,
    y: usize,
    a: usize,
    b: usize,
    dst: &mut [RCell],
    base: usize,
    arena: &mut String,
) {
    let row = y * c.cols;
    let mut x = a;
    while x < b {
        let o = c.owner[row + x];
        let mut e = x + 1;
        while e < b && c.owner[row + e] == o {
            e += 1;
        }
        if o >= PANE {
            pane_run(c, (o - PANE) as usize, y, x, e, dst, base, arena);
        } else if o == CHROME {
            for i in x..e {
                let mut cell = c.crow[i];
                if let Some(t) = cell.text(c.carena) {
                    cell.off = arena.len() as u32;
                    arena.push_str(t);
                }
                dst[i - base] = cell;
            }
        } else if is_border(o) {
            let mut style = if o & ACTIVE != 0 {
                c.scene.theme.active_border
            } else {
                c.scene.theme.border
            };
            c.norm.style(&mut style);
            let cell = RCell {
                ch: glyph_of(o),
                style,
                ..RCell::BLANK
            };
            dst[x - base..e - base].fill(cell);
        } else {
            dst[x - base..e - base].fill(RCell::BLANK);
        }
        x = e;
    }
}

#[inline]
fn conv(
    c: &mut Comp<'_, '_>,
    cell: &Cell,
    row: &FrameRow,
    pane: usize,
    w: W,
    arena: &mut String,
) -> RCell {
    let mut r = RCell {
        ch: cell.ch,
        w,
        style: cell.style,
        ..RCell::BLANK
    };
    if cell.extra != 0
        && let Some(s) = row.clusters.get(cell.extra as usize - 1)
        && let Some(first) = s.chars().next()
        && s.len() > first.len_utf8()
        && s.len() <= u16::MAX as usize
        && !s.chars().any(text::is_control)
    {
        r.ch = first;
        r.off = arena.len() as u32;
        arena.push_str(s);
        r.len = s.len() as u16;
    }
    if r.len == 0
        && (text::is_control(r.ch) || (r.ch as u32 >= 0x300 && text::char_width(r.ch) == 0))
    {
        // Printed alone it would not advance the cursor or would act as a
        // control: show a blank instead.
        r.ch = ' ';
    }
    if cell.link != 0
        && c.norm.hyperlinks()
        && let Some(l) = c.scene.links
    {
        r.link = c.links.resolve(pane, cell.link, l);
    }
    r
}

#[allow(clippy::too_many_arguments)]
fn pane_run(
    c: &mut Comp<'_, '_>,
    i: usize,
    y: usize,
    x0: usize,
    x1: usize,
    dst: &mut [RCell],
    base: usize,
    arena: &mut String,
) {
    let p = c.scene.panes[i];
    let r = p.rect;
    let f = p.frame;
    let py = y - r.y as usize;
    let fcols = f.size.cols as usize;
    let frow = if py < f.size.rows as usize {
        f.rows.get(py)
    } else {
        None
    };
    let mut x = x0;
    while x < x1 {
        let px = x - r.x as usize;
        let (Some(fr), true) = (frow, px < fcols) else {
            dst[x - base] = RCell::BLANK;
            x += 1;
            continue;
        };
        let Some(cell) = fr.cells.get(px) else {
            dst[x - base] = RCell::BLANK;
            x += 1;
            continue;
        };
        match cell.width {
            CellWidth::Wide
                if x + 1 < x1
                    && px + 1 < fcols
                    && fr
                        .cells
                        .get(px + 1)
                        .is_some_and(|t| t.width == CellWidth::WideTail) =>
            {
                let h = conv(c, cell, fr, i, W::Wide, arena);
                dst[x - base] = h;
                dst[x + 1 - base] = RCell {
                    w: W::Tail,
                    ch: ' ',
                    len: 0,
                    ..h
                };
                x += 2;
            }
            CellWidth::Narrow => {
                dst[x - base] = conv(c, cell, fr, i, W::Narrow, arena);
                x += 1;
            }
            // A wide character cut by the pane's edge, or a tail without its
            // head: a blank in the same colors.
            _ => {
                dst[x - base] = RCell {
                    style: cell.style,
                    ..RCell::BLANK
                };
                x += 1;
            }
        }
    }
    if let Some(sel) = p.selection {
        let style = c.scene.theme.selection;
        let mut x = x0;
        while x < x1 {
            let d = &mut dst[x - base];
            if d.w != W::Tail && sel.contains((x - r.x as usize) as u16, py as u16) {
                d.style = style;
                if d.w == W::Wide {
                    dst[x + 1 - base].style = style;
                }
            }
            x += 1;
        }
    }
    if py == 0
        && let Some(s) = p.indicator
    {
        c.ind.clear();
        c.iarena.clear();
        text::layout_spans(
            std::iter::once((s, c.scene.theme.indicator)),
            r.cols as usize,
            None,
            c.ind,
            c.iarena,
        );
        let start = r.right() as usize - c.ind.len();
        for x in start.max(x0)..x1 {
            let mut cell = c.ind[x - start];
            if let Some(t) = cell.text(c.iarena) {
                cell.off = arena.len() as u32;
                arena.push_str(t);
            }
            dst[x - base] = cell;
        }
        if x1 > x0 {
            // Never leave half a wide character at the run's edges.
            let l = start.max(x0);
            if l < x1 && dst[l - base].w == W::Tail {
                dst[l - base] = RCell {
                    style: dst[l - base].style,
                    ..RCell::BLANK
                };
            }
            if dst[x1 - 1 - base].w == W::Wide {
                dst[x1 - 1 - base] = RCell {
                    style: dst[x1 - 1 - base].style,
                    ..RCell::BLANK
                };
            }
        }
        if start > x0 && start <= x1 && dst[start - 1 - base].w == W::Wide {
            dst[start - 1 - base] = RCell {
                style: dst[start - 1 - base].style,
                ..RCell::BLANK
            };
        }
    }
    if !c.norm.is_full() {
        for d in &mut dst[x0 - base..x1 - base] {
            c.norm.style(&mut d.style);
        }
    }
    for d in &mut dst[x0 - base..x1 - base] {
        d.normalize_blank();
    }
}

fn best_shift(dh: &[u64], mh: &[u64]) -> isize {
    let h = dh.len();
    let base = (0..h).filter(|&i| dh[i] == mh[i]).count();
    let mut best = (0usize, 0isize);
    for k in 1..h {
        let up = (0..h - k).filter(|&i| dh[i] == mh[i + k]).count();
        let down = (k..h).filter(|&i| dh[i] == mh[i - k]).count();
        for (m, s) in [(up, k as isize), (down, -(k as isize))] {
            if m > base && m - base > best.0 {
                best = (m - base, s);
            }
        }
    }
    if best.0 >= SCROLL_MIN_GAIN { best.1 } else { 0 }
}

/// Brings one client's terminal up to date with a [`Scene`], writing as few
/// bytes as it can.
///
/// # Dirty flags and several clients
///
/// [`PaneFrame::dirty`](lif_types::PaneFrame::dirty) is shared by every
/// client showing the pane, so renderers only ever read it. Per frame tick
/// the server:
///
/// 1. refreshes every pane frame once (the engine sets dirty flags);
/// 2. for every client, calls [`Renderer::render`] if the client can take
///    output now, or [`Renderer::skip`] if it cannot (it is slow, its
///    socket is full);
/// 3. clears the dirty flags of every pane frame.
///
/// `skip` copies the flags into the renderer's own per-pane "stale rows",
/// so a renderer that skipped frames still knows which rows it has not drawn
/// yet and the next `render` brings it up to date in one frame, whatever
/// happened in between. There is never a queue of bytes: a frame is the
/// difference between what the client last received and the scene now.
///
/// # The outer terminal's state
///
/// While attached, autowrap (DECAWM) is kept off, set by every full repaint,
/// so that writing the bottom-right cell cannot scroll the screen; the
/// scroll region is kept full-screen except within a scroll operation.
/// [`Renderer::leave`] restores both. If anything else writes to the
/// client's terminal, call [`Renderer::invalidate`].
pub struct Renderer {
    caps: OuterCaps,
    norm: Norm,
    lr_margins: bool,
    scroll_opt: bool,
    full_pending: bool,
    layout_dirty: bool,
    o: Outer,
    links: LinkTable,
    owner: Vec<u16>,
    chrome: Option<u16>,
    theme: Theme,
    tracks: Vec<Track>,
    row_stale: Vec<bool>,
    any_row_stale: bool,
    has_status: bool,
    status_line: StyledLine,
    status_fill: Style,
    status_pos: StatusPosition,
    has_msg: bool,
    msg_line: StyledLine,
    msg_fill: Style,
    drow: Vec<RCell>,
    darena: String,
    crow: Vec<RCell>,
    carena: String,
    pbuf: Vec<RCell>,
    parena: String,
    dh: Vec<u64>,
    mh: Vec<u64>,
    ind: Vec<RCell>,
    iarena: String,
}

impl Renderer {
    /// A renderer for a newly attached client. Its first frame is a full
    /// repaint.
    pub fn new(caps: OuterCaps) -> Self {
        Renderer {
            caps,
            norm: Norm::new(caps),
            lr_margins: false,
            scroll_opt: true,
            full_pending: true,
            layout_dirty: true,
            o: Outer::new(caps),
            links: LinkTable::default(),
            owner: Vec::new(),
            chrome: None,
            theme: Theme::default(),
            tracks: Vec::new(),
            row_stale: Vec::new(),
            any_row_stale: false,
            has_status: false,
            status_line: Vec::new(),
            status_fill: DEFAULT_STYLE,
            status_pos: StatusPosition::Bottom,
            has_msg: false,
            msg_line: Vec::new(),
            msg_fill: DEFAULT_STYLE,
            drow: Vec::new(),
            darena: String::new(),
            crow: Vec::new(),
            carena: String::new(),
            pbuf: Vec::new(),
            parena: String::new(),
            dh: Vec::new(),
            mh: Vec::new(),
            ind: Vec::new(),
            iarena: String::new(),
        }
    }

    /// Where the renderer believes the outer terminal's cursor is (`None`
    /// on an axis it does not rely on). For tests.
    #[doc(hidden)]
    pub fn believed_cursor(&self) -> (Option<u16>, Option<u16>) {
        (self.o.cx, self.o.cy)
    }

    pub fn caps(&self) -> OuterCaps {
        self.caps
    }

    /// Change what the outer terminal is known to support (for example once
    /// the answers to lif's queries arrive). Repaints fully if it changed.
    pub fn set_caps(&mut self, caps: OuterCaps) {
        if caps != self.caps {
            self.caps = caps;
            self.norm = Norm::new(caps);
            self.o.caps = caps;
            self.full_pending = true;
        }
    }

    /// Whether the outer terminal supports DECLRMM/DECSLRM left and right
    /// margins, which lets panes narrower than the screen scroll in place.
    /// (`OuterCaps` has no such flag yet.)
    pub fn set_lr_margins(&mut self, on: bool) {
        self.lr_margins = on;
    }

    /// Scroll detection is on by default; off only for measurements.
    pub fn set_scroll_optimization(&mut self, on: bool) {
        self.scroll_opt = on;
    }

    /// Forget what the client's terminal shows: the next frame is a full
    /// repaint (after attach, resize, or anything else writing to it).
    pub fn invalidate(&mut self) {
        self.full_pending = true;
    }

    /// The whole scene as a fresh byte stream, for a newly attached client.
    pub fn render_full(&mut self, scene: &Scene<'_>, out: &mut Vec<u8>) {
        self.invalidate();
        self.render(scene, out);
    }

    /// Restore the outer terminal state lif changed (before detaching). The
    /// next frame after this is a full repaint.
    pub fn leave(&mut self, out: &mut Vec<u8>) {
        out.extend_from_slice(b"\x1b[m");
        if self.o.link != 0 || self.caps.hyperlinks {
            out.extend_from_slice(b"\x1b]8;;\x1b\\");
        }
        out.extend_from_slice(b"\x1b[r\x1b[?7h\x1b[0 q\x1b[?25h");
        self.o.link = 0;
        self.full_pending = true;
    }

    /// Record a frame this client will not get (it cannot take output now).
    /// Must be called, instead of `render`, for every frame tick between
    /// the server refreshing pane frames and clearing their dirty flags.
    pub fn skip(&mut self, scene: &Scene<'_>) {
        if self.full_pending || scene.size != self.o.size() {
            return;
        }
        if self.layout_matches(scene) {
            self.merge_dirty(scene);
        } else {
            self.layout_dirty = true;
        }
    }

    /// Append the bytes that bring the client's terminal from what it shows
    /// to `scene`. Appends nothing if nothing visible changed.
    pub fn render(&mut self, scene: &Scene<'_>, out: &mut Vec<u8>) {
        if scene.size.is_empty() {
            return;
        }
        self.o.begun = false;
        self.links.key = None;
        if scene.size != self.o.size() {
            self.o.resize(scene.size);
            self.row_stale.clear();
            self.row_stale.resize(self.o.rows, false);
            self.drow.resize(self.o.cols, RCell::BLANK);
            self.full_pending = true;
        }
        if self.links.uris.len() > LINK_LIMIT {
            self.links.clear();
            self.full_pending = true;
        }
        if self.full_pending {
            self.full_pending = false;
            self.o.full_reset(out);
            self.layout_dirty = true;
        }
        if self.layout_matches(scene) {
            self.merge_dirty(scene);
        } else {
            self.rebuild(scene);
        }
        self.update_chrome(scene);
        self.update_overlays(scene);

        if self.scroll_opt {
            for i in 0..self.tracks.len() {
                if self.scroll_candidate(i) {
                    self.pane_pass(scene, out, i);
                }
            }
        }
        for i in 0..self.tracks.len() {
            if self.tracks[i].nstale > 0 {
                self.stale_rows(scene, out, i);
            }
        }
        if self.any_row_stale {
            if let Some(cy) = self.chrome
                && self.row_stale[cy as usize]
            {
                self.build_chrome(scene);
            }
            let cols = self.o.cols;
            for y in 0..self.o.rows {
                if self.row_stale[y] {
                    self.full_row(scene, out, y, cols);
                }
            }
            self.row_stale.fill(false);
            self.any_row_stale = false;
        }
        if self.o.link != 0 {
            // Close links at the end of every frame so nothing written
            // between frames can end up inside one.
            self.o.set_link(0, &self.links, out);
        }
        self.o.cursor(out, &scene.cursor);
        if self.o.begun && self.caps.sync_output {
            out.extend_from_slice(b"\x1b[?2026l");
        }
    }

    fn full_row(&mut self, scene: &Scene<'_>, out: &mut Vec<u8>, y: usize, cols: usize) {
        self.compose_row(scene, y, 0, cols);
        let fix = self
            .o
            .diff(out, &self.links, y, 0, &self.drow[..cols], &self.darena);
        debug_assert!(!fix);
    }

    fn layout_matches(&self, scene: &Scene<'_>) -> bool {
        !self.layout_dirty
            && scene.theme == self.theme
            && scene.chrome_row() == self.chrome
            && scene.panes.len() == self.tracks.len()
            && scene.panes.iter().zip(&self.tracks).all(|(p, t)| {
                p.id == t.id && p.rect == t.rect && p.frame.size == t.fsize && p.active == t.active
            })
    }

    fn rebuild(&mut self, scene: &Scene<'_>) {
        self.chrome = scene.chrome_row();
        self.theme = scene.theme;
        layout::build(
            &mut self.owner,
            self.o.cols,
            self.o.rows,
            scene.panes,
            self.chrome,
        );
        self.tracks.truncate(scene.panes.len());
        self.tracks.resize_with(scene.panes.len(), Track::default);
        for (t, p) in self.tracks.iter_mut().zip(scene.panes) {
            t.id = p.id;
            t.rect = p.rect;
            t.fsize = p.frame.size;
            t.active = p.active;
            t.stale.clear();
            t.stale.resize(p.rect.rows as usize, false);
            t.nstale = 0;
            t.sel = None;
            t.has_ind = false;
            t.ind.clear();
        }
        self.row_stale.fill(true);
        self.any_row_stale = true;
        self.layout_dirty = false;
    }

    fn merge_dirty(&mut self, scene: &Scene<'_>) {
        for (p, t) in scene.panes.iter().zip(&mut self.tracks) {
            let n = t.stale.len().min(p.frame.dirty.len());
            for (s, &d) in t.stale[..n].iter_mut().zip(&p.frame.dirty[..n]) {
                if d && !*s {
                    *s = true;
                    t.nstale += 1;
                }
            }
        }
    }

    fn update_chrome(&mut self, scene: &Scene<'_>) {
        let mut changed = false;
        match scene.status {
            Some(s) => {
                if !self.has_status
                    || self.status_fill != s.fill
                    || self.status_pos != s.position
                    || self.status_line != *s.line
                {
                    changed = true;
                    self.has_status = true;
                    self.status_fill = s.fill;
                    self.status_pos = s.position;
                    self.status_line.clone_from(s.line);
                }
            }
            None => {
                changed |= self.has_status;
                self.has_status = false;
            }
        }
        match scene.message {
            Some(m) => {
                if !self.has_msg || self.msg_fill != m.fill || self.msg_line != *m.line {
                    changed = true;
                    self.has_msg = true;
                    self.msg_fill = m.fill;
                    self.msg_line.clone_from(m.line);
                }
            }
            None => {
                changed |= self.has_msg;
                self.has_msg = false;
            }
        }
        if changed && let Some(y) = self.chrome {
            self.row_stale[y as usize] = true;
            self.any_row_stale = true;
        }
    }

    fn update_overlays(&mut self, scene: &Scene<'_>) {
        for (p, t) in scene.panes.iter().zip(&mut self.tracks) {
            if p.selection != t.sel {
                for s in [t.sel, p.selection].into_iter().flatten() {
                    let (lo, hi) = s.rows();
                    t.mark(lo as usize, hi as usize);
                }
                t.sel = p.selection;
            }
            let changed = match p.indicator {
                Some(s) => !t.has_ind || t.ind != s,
                None => t.has_ind,
            };
            if changed {
                t.mark(0, 0);
                t.has_ind = p.indicator.is_some();
                t.ind.clear();
                t.ind.push_str(p.indicator.unwrap_or(""));
            }
        }
    }

    fn build_chrome(&mut self, scene: &Scene<'_>) {
        self.crow.clear();
        self.carena.clear();
        let (line, fill) = match (scene.message, scene.status) {
            (Some(m), _) => (m.line, m.fill),
            (None, Some(s)) => (s.line, s.fill),
            (None, None) => return,
        };
        text::layout_spans(
            text::line_spans(line),
            self.o.cols,
            Some(fill),
            &mut self.crow,
            &mut self.carena,
        );
        for c in &mut self.crow {
            self.norm.style(&mut c.style);
            c.normalize_blank();
        }
    }

    /// Compose row `y`, columns `a..b`, into `drow` (clearing its arena).
    fn compose_row(&mut self, scene: &Scene<'_>, y: usize, a: usize, b: usize) {
        self.darena.clear();
        let mut c = Comp {
            cols: self.o.cols,
            owner: &self.owner,
            scene,
            norm: &mut self.norm,
            links: &mut self.links,
            crow: &self.crow,
            carena: &self.carena,
            ind: &mut self.ind,
            iarena: &mut self.iarena,
        };
        compose(&mut c, y, a, b, &mut self.drow, 0, &mut self.darena);
    }

    fn scroll_candidate(&self, i: usize) -> bool {
        let t = &self.tracks[i];
        let r = t.rect;
        let (cols, rows) = (self.o.cols, self.o.rows);
        t.nstale * 2 >= r.rows as usize
            && r.rows >= 4
            && r.cols > 0
            && (r.bottom() as usize) <= rows
            && (r.right() as usize) <= cols
            && !self.chrome.is_some_and(|c| c >= r.y && c < r.bottom())
            // DECSLRM needs left < right.
            && ((r.x == 0 && r.cols as usize == cols) || (self.lr_margins && r.cols >= 2))
    }

    /// Draw a pane whose rows mostly changed: compose all of it, look for
    /// content that moved up or down, scroll the outer terminal to match,
    /// then diff.
    fn pane_pass(&mut self, scene: &Scene<'_>, out: &mut Vec<u8>, i: usize) {
        let r = self.tracks[i].rect;
        let (w, h) = (r.cols as usize, r.rows as usize);
        let (x0, x1) = (r.x as usize, r.right() as usize);
        self.pbuf.clear();
        self.pbuf.resize(w * h, RCell::BLANK);
        self.parena.clear();
        self.dh.clear();
        self.mh.clear();
        {
            let mut c = Comp {
                cols: self.o.cols,
                owner: &self.owner,
                scene,
                norm: &mut self.norm,
                links: &mut self.links,
                crow: &self.crow,
                carena: &self.carena,
                ind: &mut self.ind,
                iarena: &mut self.iarena,
            };
            for py in 0..h {
                let dst = &mut self.pbuf[py * w..(py + 1) * w];
                compose(&mut c, r.y as usize + py, x0, x1, dst, x0, &mut self.parena);
                self.dh.push(hash_cells(dst, &self.parena));
            }
        }
        for py in 0..h {
            let mh = self.o.seg_hash(r.y as usize + py, x0, x1);
            self.mh.push(mh);
        }
        let k = best_shift(&self.dh, &self.mh);
        if k != 0 {
            self.scroll(out, r, k);
        }
        for py in 0..h {
            let y = r.y as usize + py;
            if (k != 0 || self.tracks[i].stale[py]) && !self.row_stale[y] {
                let des = &self.pbuf[py * w..(py + 1) * w];
                if self.o.diff(out, &self.links, y, x0, des, &self.parena) {
                    self.row_stale[y] = true;
                    self.any_row_stale = true;
                } else if w == self.o.cols {
                    // The row now shows exactly what was composed, so its
                    // hash is known without hashing the model.
                    self.o.rhash[y] = self.dh[py];
                    self.o.rhash_ok[y] = true;
                }
            }
        }
        self.tracks[i].clear();
    }

    fn scroll(&mut self, out: &mut Vec<u8>, r: Rect, k: isize) {
        let o = &mut self.o;
        o.begin(out);
        o.erase_safe(&self.links, out);
        let (top, bot) = (r.y as usize, r.bottom() as usize);
        let (x0, x1) = (r.x as usize, r.right() as usize);
        let n = k.unsigned_abs() as u32;
        let fin = if k > 0 { b'S' } else { b'T' };
        let full_width = x0 == 0 && x1 == o.cols;
        if full_width && top == 0 && bot == o.rows {
            csi_n(out, n, fin);
        } else {
            if !full_width {
                out.extend_from_slice(b"\x1b[?69h");
            }
            out.extend_from_slice(b"\x1b[");
            num(out, top as u32 + 1);
            out.byte(b';');
            num(out, bot as u32);
            out.byte(b'r');
            if !full_width {
                out.extend_from_slice(b"\x1b[");
                num(out, x0 as u32 + 1);
                out.byte(b';');
                num(out, x1 as u32);
                out.byte(b's');
            }
            csi_n(out, n, fin);
            if !full_width {
                out.extend_from_slice(b"\x1b[s\x1b[?69l");
            }
            // Resetting the margins homes the cursor.
            out.extend_from_slice(b"\x1b[r");
            o.cx = Some(0);
            o.cy = Some(0);
        }
        o.shift(top, bot, x0, x1, k);
    }

    fn stale_rows(&mut self, scene: &Scene<'_>, out: &mut Vec<u8>, i: usize) {
        let r = self.tracks[i].rect;
        let (cols, rows) = (self.o.cols, self.o.rows);
        let (a, b) = ((r.x as usize).min(cols), (r.right() as usize).min(cols));
        for py in 0..self.tracks[i].stale.len() {
            if !self.tracks[i].stale[py] {
                continue;
            }
            let y = r.y as usize + py;
            if y >= rows || a >= b || self.chrome == Some(y as u16) || self.row_stale[y] {
                continue;
            }
            self.compose_row(scene, y, a, b);
            if self
                .o
                .diff(out, &self.links, y, a, &self.drow[a..b], &self.darena)
            {
                self.row_stale[y] = true;
                self.any_row_stale = true;
            }
        }
        self.tracks[i].clear();
    }
}
