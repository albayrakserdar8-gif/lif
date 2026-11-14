//! Cell access with a small row cache, and tmux's grid-reader motions.

use crate::view::PaneView;
use lif_types::{Cell, CellWidth, FrameRow};

pub(crate) const WHITESPACE: &str = "\t ";
const SLOTS: usize = 4;

#[derive(Default, Debug)]
struct Slot {
    row: usize,
    used: u64,
    len: usize,
    valid: bool,
    data: FrameRow,
}

/// The last few rows read. Motions inspect cells one by one, mostly on one
/// or two rows; caching them keeps a motion to a couple of row reads.
/// Invalidated at the start of every public call because the pane's
/// contents may have changed in between.
#[derive(Default, Debug)]
pub(crate) struct RowCache {
    slots: Vec<Slot>,
    clock: u64,
}

impl RowCache {
    pub fn invalidate(&mut self) {
        for s in &mut self.slots {
            s.valid = false;
        }
    }
}

/// Trimmed length of a row: up to the last cell that is not a plain blank.
fn trimmed_len(r: &FrameRow) -> usize {
    let mut n = r.cells.len();
    while n > 0 {
        let c = &r.cells[n - 1];
        if c.width == CellWidth::Narrow && c.extra == 0 && c.ch == ' ' {
            n -= 1;
        } else {
            break;
        }
    }
    n
}

pub(crate) struct Grid<'a> {
    pub view: &'a dyn PaneView,
    cache: &'a mut RowCache,
    pub cols: usize,
    pub total: usize,
}

impl<'a> Grid<'a> {
    pub fn new(view: &'a dyn PaneView, cache: &'a mut RowCache, cols: usize, total: usize) -> Grid<'a> {
        Grid { view, cache, cols, total }
    }

    fn slot(&mut self, y: usize) -> usize {
        self.cache.clock += 1;
        let clock = self.cache.clock;
        if let Some(i) = self.cache.slots.iter().position(|s| s.valid && s.row == y) {
            self.cache.slots[i].used = clock;
            return i;
        }
        let i = if self.cache.slots.len() < SLOTS {
            self.cache.slots.push(Slot::default());
            self.cache.slots.len() - 1
        } else {
            let mut best = 0;
            for (k, s) in self.cache.slots.iter().enumerate() {
                if !s.valid {
                    best = k;
                    break;
                }
                if s.used < self.cache.slots[best].used {
                    best = k;
                }
            }
            best
        };
        let s = &mut self.cache.slots[i];
        if y < self.total {
            self.view.row(y, &mut s.data);
        } else {
            s.data.cells.clear();
            s.data.clusters.clear();
            s.data.wrapped = false;
        }
        if s.data.cells.len() != self.cols {
            s.data.cells.resize(self.cols, Cell::BLANK);
        }
        // A tail with no head (a cut-off engine row) is just a blank.
        if let Some(c) = s.data.cells.first_mut()
            && c.width == CellWidth::WideTail
        {
            *c = Cell::BLANK;
        }
        s.len = trimmed_len(&s.data);
        s.row = y;
        s.valid = true;
        s.used = clock;
        i
    }

    pub fn row(&mut self, y: usize) -> &FrameRow {
        let i = self.slot(y);
        &self.cache.slots[i].data
    }

    /// Trimmed length in cells.
    pub fn len(&mut self, y: usize) -> usize {
        let i = self.slot(y);
        self.cache.slots[i].len
    }

    /// The row soft-wraps onto the next. The last row never does.
    pub fn wrapped(&mut self, y: usize) -> bool {
        y + 1 < self.total && self.row(y).wrapped
    }

    pub fn cell(&mut self, y: usize, x: usize) -> Cell {
        if x >= self.cols {
            return Cell::BLANK;
        }
        self.row(y).cells[x]
    }

    pub fn is_tail(&mut self, y: usize, x: usize) -> bool {
        x < self.cols && self.row(y).cells[x].width == CellWidth::WideTail
    }

    /// Move `x` off a wide tail onto its head.
    pub fn head(&mut self, y: usize, mut x: usize) -> usize {
        while x > 0 && self.is_tail(y, x) {
            x -= 1;
        }
        x
    }

    /// Width in cells of the character starting at `x`.
    pub fn width_at(&mut self, y: usize, x: usize) -> usize {
        if self.cell(y, x).width == CellWidth::Wide { 2 } else { 1 }
    }

    /// tmux's grid_in_set: is the cell a single character from `set`? Wide
    /// tails and multi-codepoint clusters never are.
    pub fn in_set(&mut self, y: usize, x: usize, set: &str) -> bool {
        let c = self.cell(y, x);
        if c.width == CellWidth::WideTail || c.extra != 0 {
            return false;
        }
        set.contains(c.ch)
    }

    /// The single character of a cell, if it holds exactly one.
    pub fn single_char(&mut self, y: usize, x: usize) -> Option<char> {
        let c = self.cell(y, x);
        (c.width != CellWidth::WideTail && c.extra == 0).then_some(c.ch)
    }

    /// The text of the cell at `x` appended to `out` (nothing for a tail).
    pub fn push_cell_text(&mut self, y: usize, x: usize, out: &mut String) {
        if x < self.cols {
            self.row(y).push_cell_text(x, out);
        }
    }

    /// First row of the logical (hard) line containing `y`, looking back at
    /// most `limit` rows.
    pub fn line_start(&mut self, mut y: usize, limit: usize) -> usize {
        let stop = y.saturating_sub(limit);
        while y > stop && self.wrapped(y - 1) {
            y -= 1;
        }
        y
    }

    /// Last row of the logical line containing `y`, looking ahead at most
    /// `limit` rows.
    pub fn line_end(&mut self, mut y: usize, limit: usize) -> usize {
        let stop = y.saturating_add(limit).min(self.total - 1);
        while y < stop && self.wrapped(y) {
            y += 1;
        }
        y
    }
}

/// A cursor over the grid with tmux's grid_reader motions (grid-reader.c),
/// ported closely so word motions land exactly where tmux's do.
pub(crate) struct Reader<'g, 'a> {
    pub g: &'g mut Grid<'a>,
    pub x: usize,
    pub y: usize,
}

impl<'g, 'a> Reader<'g, 'a> {
    pub fn new(g: &'g mut Grid<'a>, x: usize, y: usize) -> Self {
        Reader { g, x, y }
    }

    fn last(&self) -> usize {
        self.g.total - 1
    }

    fn line_length(&mut self) -> usize {
        self.g.len(self.y)
    }

    fn in_set(&mut self, set: &str) -> bool {
        self.g.in_set(self.y, self.x, set)
    }

    pub fn right(&mut self, wrap: bool, all: bool) {
        let px = if all { self.g.cols } else { self.line_length() };
        if wrap && self.x >= px && self.y < self.last() {
            self.start_of_line(false);
            self.down();
        } else if self.x < px {
            self.x += 1;
            while self.x < px && self.g.is_tail(self.y, self.x) {
                self.x += 1;
            }
        }
    }

    pub fn left(&mut self, wrap: bool) {
        self.x = self.g.head(self.y, self.x);
        if self.x == 0 && self.y > 0 && (wrap || self.g.wrapped(self.y - 1)) {
            self.up();
            self.end_of_line(false, false);
        } else if self.x > 0 {
            self.x = self.g.head(self.y, self.x - 1);
        }
    }

    pub fn up(&mut self) {
        if self.y > 0 {
            self.y -= 1;
        }
    }

    pub fn down(&mut self) {
        if self.y < self.last() {
            self.y += 1;
        }
    }

    pub fn start_of_line(&mut self, wrap: bool) {
        if wrap {
            while self.y > 0 && self.g.wrapped(self.y - 1) {
                self.y -= 1;
            }
        }
        self.x = 0;
    }

    pub fn end_of_line(&mut self, wrap: bool, all: bool) {
        if wrap {
            while self.y < self.last() && self.g.wrapped(self.y) {
                self.y += 1;
            }
        }
        self.x = if all { self.g.cols } else { self.line_length() };
    }

    fn handle_wrap(&mut self, xx: &mut usize, yy: usize) -> bool {
        while self.x > *xx {
            if self.y == yy {
                return false;
            }
            self.start_of_line(false);
            self.down();
            *xx = if self.g.wrapped(self.y) { self.g.cols - 1 } else { self.line_length() };
        }
        true
    }

    fn start_xx(&mut self) -> usize {
        if self.g.wrapped(self.y) { self.g.cols - 1 } else { self.line_length() }
    }

    pub fn next_word(&mut self, seps: &str) {
        let mut xx = self.start_xx();
        let yy = self.last();
        if !self.handle_wrap(&mut xx, yy) {
            return;
        }
        if !self.in_set(WHITESPACE) {
            if self.in_set(seps) {
                loop {
                    self.x += 1;
                    if !(self.handle_wrap(&mut xx, yy) && self.in_set(seps) && !self.in_set(WHITESPACE)) {
                        break;
                    }
                }
            } else {
                loop {
                    self.x += 1;
                    if !(self.handle_wrap(&mut xx, yy) && !(self.in_set(seps) || self.in_set(WHITESPACE))) {
                        break;
                    }
                }
            }
        }
        while self.handle_wrap(&mut xx, yy) && self.in_set(WHITESPACE) {
            self.x += 1;
        }
    }

    /// Leaves the cursor one past the end of the next word.
    pub fn next_word_end(&mut self, seps: &str) {
        let mut xx = self.start_xx();
        let yy = self.last();
        while self.handle_wrap(&mut xx, yy) {
            if self.in_set(WHITESPACE) {
                self.x += 1;
            } else if self.in_set(seps) {
                loop {
                    self.x += 1;
                    if !(self.handle_wrap(&mut xx, yy) && self.in_set(seps) && !self.in_set(WHITESPACE)) {
                        break;
                    }
                }
                return;
            } else {
                loop {
                    self.x += 1;
                    if !(self.handle_wrap(&mut xx, yy) && !(self.in_set(WHITESPACE) || self.in_set(seps))) {
                        break;
                    }
                }
                return;
            }
        }
    }

    pub fn previous_word(&mut self, seps: &str, already: bool, stop_at_eol: bool) {
        let word_is_letters;
        if already || self.in_set(WHITESPACE) {
            loop {
                if self.x > 0 {
                    self.x -= 1;
                    if !self.in_set(WHITESPACE) {
                        word_is_letters = !self.in_set(seps);
                        break;
                    }
                } else {
                    if self.y == 0 {
                        return;
                    }
                    self.up();
                    self.end_of_line(false, false);
                    if stop_at_eol && self.x > 0 {
                        let oldx = self.x;
                        self.x -= 1;
                        let at_eol = self.in_set(WHITESPACE);
                        self.x = oldx;
                        if at_eol {
                            word_is_letters = false;
                            break;
                        }
                    }
                }
            }
        } else {
            word_is_letters = !self.in_set(seps);
        }
        loop {
            let (oldx, oldy) = (self.x, self.y);
            if self.x == 0 {
                if self.y == 0 || !self.g.wrapped(self.y - 1) {
                    break;
                }
                self.up();
                self.end_of_line(false, true);
            }
            if self.x > 0 {
                self.x -= 1;
            }
            if !(!self.in_set(WHITESPACE) && word_is_letters != self.in_set(seps)) {
                self.x = oldx;
                self.y = oldy;
                break;
            }
        }
    }

    pub fn back_to_indentation(&mut self) {
        self.start_of_line(true);
        let first = self.y;
        loop {
            let xx = self.line_length();
            for px in 0..xx {
                if self.g.single_char(self.y, px) != Some(' ') {
                    self.x = px;
                    return;
                }
            }
            if self.g.wrapped(self.y) {
                self.down();
            } else {
                break;
            }
        }
        self.y = first;
        self.x = 0;
    }
}
