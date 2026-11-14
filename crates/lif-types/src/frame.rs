use crate::{Pos, Size};

/// A terminal color.
#[derive(Clone, Copy, Debug, Default, PartialEq, Eq, Hash)]
pub enum Color {
    /// The terminal's default foreground / background.
    #[default]
    Default,
    /// One of the 256 palette colors.
    Indexed(u8),
    Rgb(u8, u8, u8),
}

/// Boolean text attributes, as a bit set.
#[derive(Clone, Copy, Debug, Default, PartialEq, Eq, Hash)]
pub struct Attrs(pub u16);

impl Attrs {
    pub const NONE: Attrs = Attrs(0);
    pub const BOLD: Attrs = Attrs(1 << 0);
    pub const FAINT: Attrs = Attrs(1 << 1);
    pub const ITALIC: Attrs = Attrs(1 << 2);
    pub const BLINK: Attrs = Attrs(1 << 3);
    pub const INVERSE: Attrs = Attrs(1 << 4);
    pub const INVISIBLE: Attrs = Attrs(1 << 5);
    pub const STRIKETHROUGH: Attrs = Attrs(1 << 6);
    pub const OVERLINE: Attrs = Attrs(1 << 7);

    pub const fn contains(self, other: Attrs) -> bool {
        self.0 & other.0 == other.0
    }

    pub const fn union(self, other: Attrs) -> Attrs {
        Attrs(self.0 | other.0)
    }

    pub fn insert(&mut self, other: Attrs) {
        self.0 |= other.0;
    }

    pub fn remove(&mut self, other: Attrs) {
        self.0 &= !other.0;
    }

    pub const fn is_empty(self) -> bool {
        self.0 == 0
    }
}

#[derive(Clone, Copy, Debug, Default, PartialEq, Eq, Hash)]
pub enum Underline {
    #[default]
    None,
    Single,
    Double,
    Curly,
    Dotted,
    Dashed,
}

/// The full visual style of one cell.
#[derive(Clone, Copy, Debug, Default, PartialEq, Eq, Hash)]
pub struct Style {
    pub fg: Color,
    pub bg: Color,
    /// `Color::Default` means "same as the foreground".
    pub underline_color: Color,
    pub attrs: Attrs,
    pub underline: Underline,
}

#[derive(Clone, Copy, Debug, Default, PartialEq, Eq, Hash)]
pub enum CellWidth {
    #[default]
    Narrow,
    /// The first cell of a double-width character.
    Wide,
    /// The second cell of a double-width character. It carries no text.
    WideTail,
}

/// One cell of a [`PaneFrame`].
#[derive(Clone, Copy, Debug, PartialEq, Eq, Hash)]
pub struct Cell {
    /// The first codepoint. `' '` for an empty cell.
    pub ch: char,
    /// If non-zero, the cell holds a multi-codepoint grapheme cluster and
    /// `FrameRow::clusters[extra - 1]` is its full text (including `ch`).
    pub extra: u16,
    pub width: CellWidth,
    pub style: Style,
    /// OSC 8 hyperlink id, 0 for none. Ids are only meaningful to the engine
    /// that produced the frame (see `VtEngine::hyperlink`).
    pub link: u32,
}

impl Cell {
    pub const BLANK: Cell = Cell {
        ch: ' ',
        extra: 0,
        width: CellWidth::Narrow,
        style: Style {
            fg: Color::Default,
            bg: Color::Default,
            underline_color: Color::Default,
            attrs: Attrs::NONE,
            underline: Underline::None,
        },
        link: 0,
    };
}

impl Default for Cell {
    fn default() -> Self {
        Cell::BLANK
    }
}

/// One row of a [`PaneFrame`].
#[derive(Clone, Debug, Default, PartialEq, Eq)]
pub struct FrameRow {
    /// Always exactly `PaneFrame::size.cols` cells.
    pub cells: Vec<Cell>,
    /// Full text of multi-codepoint grapheme clusters, see [`Cell::extra`].
    pub clusters: Vec<Box<str>>,
    /// The row soft-wraps onto the next one.
    pub wrapped: bool,
}

impl FrameRow {
    /// The text of the cell at `x`, appended to `out`. Nothing for a wide tail.
    pub fn push_cell_text(&self, x: usize, out: &mut String) {
        let cell = &self.cells[x];
        if cell.width == CellWidth::WideTail {
            return;
        }
        match cell.extra {
            0 => out.push(cell.ch),
            n => out.push_str(&self.clusters[n as usize - 1]),
        }
    }
}

#[derive(Clone, Copy, Debug, Default, PartialEq, Eq, Hash)]
pub enum CursorShape {
    #[default]
    Block,
    Underline,
    Bar,
}

#[derive(Clone, Copy, Debug, Default, PartialEq, Eq)]
pub struct Cursor {
    /// `None` when the cursor is outside the visible viewport (scrolled back).
    pub pos: Option<Pos>,
    pub visible: bool,
    pub shape: CursorShape,
    pub blinking: bool,
}

/// The visible contents of one pane, in engine-neutral form.
///
/// A pane has exactly one frame, owned by the server and updated in place by
/// `VtEngine::refresh`, which rewrites only the rows that changed and sets
/// their `dirty` flag (it never clears one). Every client showing the pane
/// reads the same frame, so no renderer may clear the flags. Each frame tick:
///
/// 1. the server refreshes every pane's frame once;
/// 2. every client's renderer either renders the frame or skips it (a client
///    that cannot take output now remembers the dirty rows itself), reading
///    the flags only;
/// 3. the server clears the flags (`PaneFrame::clear_dirty`).
#[derive(Clone, Debug, Default, PartialEq, Eq)]
pub struct PaneFrame {
    pub size: Size,
    /// Exactly `size.rows` rows.
    pub rows: Vec<FrameRow>,
    /// Exactly `size.rows` flags; `dirty[y]` means row `y` changed since the
    /// server last cleared the flags (the end of the previous frame tick).
    pub dirty: Vec<bool>,
    pub cursor: Cursor,
}

impl PaneFrame {
    pub fn new(size: Size) -> Self {
        let mut f = PaneFrame::default();
        f.reset(size);
        f
    }

    /// Resize to `size`, blank every cell and mark everything dirty.
    pub fn reset(&mut self, size: Size) {
        self.size = size;
        self.rows.resize_with(size.rows as usize, FrameRow::default);
        for row in &mut self.rows {
            row.cells.clear();
            row.cells.resize(size.cols as usize, Cell::BLANK);
            row.clusters.clear();
            row.wrapped = false;
        }
        self.dirty.clear();
        self.dirty.resize(size.rows as usize, true);
        self.cursor = Cursor::default();
    }

    pub fn mark_all_dirty(&mut self) {
        self.dirty.fill(true);
    }

    pub fn clear_dirty(&mut self) {
        self.dirty.fill(false);
    }

    pub fn any_dirty(&self) -> bool {
        self.dirty.iter().any(|d| *d)
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn cell_stays_small() {
        // The renderer keeps two frames per client; keep cells compact.
        assert!(std::mem::size_of::<Cell>() <= 32, "{}", std::mem::size_of::<Cell>());
    }

    #[test]
    fn reset_shapes_frame() {
        let mut f = PaneFrame::new(Size::new(4, 2));
        assert_eq!(f.rows.len(), 2);
        assert!(f.rows.iter().all(|r| r.cells.len() == 4));
        assert!(f.any_dirty());
        f.clear_dirty();
        assert!(!f.any_dirty());
        f.reset(Size::new(3, 5));
        assert_eq!(f.rows.len(), 5);
        assert_eq!(f.dirty.len(), 5);
        assert!(f.rows.iter().all(|r| r.cells.len() == 3));
    }

    #[test]
    fn cell_text() {
        let mut row = FrameRow { cells: vec![Cell::BLANK; 3], ..Default::default() };
        row.cells[0].ch = 'a';
        row.clusters.push("e\u{301}".into());
        row.cells[1].ch = 'e';
        row.cells[1].extra = 1;
        row.cells[2].width = CellWidth::WideTail;
        let mut s = String::new();
        for x in 0..3 {
            row.push_cell_text(x, &mut s);
        }
        assert_eq!(s, "ae\u{301}");
    }
}
