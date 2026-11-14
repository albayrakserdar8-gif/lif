/// A size in character cells.
#[derive(Clone, Copy, Debug, Default, PartialEq, Eq, Hash)]
pub struct Size {
    pub cols: u16,
    pub rows: u16,
}

impl Size {
    pub const fn new(cols: u16, rows: u16) -> Self {
        Self { cols, rows }
    }

    pub const fn is_empty(self) -> bool {
        self.cols == 0 || self.rows == 0
    }
}

/// A size in pixels. Zero means "unknown" (the outer terminal did not report it).
#[derive(Clone, Copy, Debug, Default, PartialEq, Eq, Hash)]
pub struct PixelSize {
    pub width: u16,
    pub height: u16,
}

/// A rectangle in character cells, origin at the top-left of the client screen.
#[derive(Clone, Copy, Debug, Default, PartialEq, Eq, Hash)]
pub struct Rect {
    pub x: u16,
    pub y: u16,
    pub cols: u16,
    pub rows: u16,
}

impl Rect {
    pub const fn new(x: u16, y: u16, cols: u16, rows: u16) -> Self {
        Self { x, y, cols, rows }
    }

    pub const fn size(self) -> Size {
        Size::new(self.cols, self.rows)
    }

    /// One past the last column.
    pub const fn right(self) -> u16 {
        self.x + self.cols
    }

    /// One past the last row.
    pub const fn bottom(self) -> u16 {
        self.y + self.rows
    }

    pub const fn contains(self, x: u16, y: u16) -> bool {
        x >= self.x && x < self.right() && y >= self.y && y < self.bottom()
    }
}

/// A cell position, zero-based.
#[derive(Clone, Copy, Debug, Default, PartialEq, Eq, Hash)]
pub struct Pos {
    pub x: u16,
    pub y: u16,
}
