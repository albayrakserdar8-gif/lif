//! What copy mode needs from a pane, and an in-memory implementation.

use lif_types::{Cell, CellWidth, FrameRow, Size};

/// A pane's contents as copy mode sees them: the scrollback followed by the
/// screen, addressed by absolute row (0 = the oldest row still held).
///
/// Every method is called with bounded work in mind: copy mode reads only
/// the rows a command touches (a handful for a motion, one window per
/// [`CopyMode::search_step`](crate::CopyMode::search_step) call for a
/// search) and never asks for the whole history at once.
pub trait PaneView {
    /// The screen size (columns, and rows of the visible screen).
    fn size(&self) -> Size;

    /// Rows of scrollback above the screen.
    fn history_rows(&self) -> usize;

    /// Write absolute row `row` (`0..history_rows() + size().rows`) into
    /// `out`: exactly `size().cols` cells with their widths and grapheme
    /// clusters, and whether the row soft-wraps onto the next one. `out`'s
    /// buffers are reused between calls. `dirty` flags do not exist here.
    fn row(&self, row: usize, out: &mut FrameRow);

    /// The next row at or after `from` (`forward`) or at or before `from`
    /// (backward) that starts a shell prompt (OSC 133;A). The default knows
    /// of no prompts, which makes next-prompt / previous-prompt do nothing.
    fn find_prompt(&self, _from: usize, _forward: bool) -> Option<usize> {
        None
    }

    /// How many rows have been dropped from the top of the scrollback since
    /// the pane was created (a monotonic counter). Copy mode uses it to keep
    /// its positions on the same text when the scrollback limit evicts old
    /// rows while it is open. The default never drops rows.
    fn trimmed_rows(&self) -> u64 {
        0
    }

    /// Show the rows starting at absolute row `top` (`0..=history_rows()`)
    /// on the screen. `top == history_rows()` is the live bottom.
    fn set_viewport_top(&mut self, top: usize);
}

/// Display width of a character in cells: 0 for combining and other
/// zero-width characters, 2 for East Asian wide and emoji, 1 otherwise.
/// Good enough to lay out test text; real panes get widths from the engine.
pub fn char_width(c: char) -> usize {
    let u = c as u32;
    const ZERO: &[(u32, u32)] = &[
        (0x0300, 0x036F),
        (0x0483, 0x0489),
        (0x0591, 0x05BD),
        (0x0610, 0x061A),
        (0x064B, 0x065F),
        (0x0E31, 0x0E31),
        (0x0E34, 0x0E3A),
        (0x1AB0, 0x1AFF),
        (0x1DC0, 0x1DFF),
        (0x200B, 0x200F),
        (0x20D0, 0x20FF),
        (0xFE00, 0xFE0F),
        (0xFE20, 0xFE2F),
        (0x1F3FB, 0x1F3FF),
        (0xE0020, 0xE007F),
        (0xE0100, 0xE01EF),
    ];
    const WIDE: &[(u32, u32)] = &[
        (0x1100, 0x115F),
        (0x231A, 0x231B),
        (0x2329, 0x232A),
        (0x23E9, 0x23EC),
        (0x25FD, 0x25FE),
        (0x2614, 0x2615),
        (0x2648, 0x2653),
        (0x26A1, 0x26A1),
        (0x26AA, 0x26AB),
        (0x26BD, 0x26BE),
        (0x26C4, 0x26C5),
        (0x26CE, 0x26CE),
        (0x26D4, 0x26D4),
        (0x26EA, 0x26EA),
        (0x26F2, 0x26F5),
        (0x26FA, 0x26FD),
        (0x2705, 0x2705),
        (0x270A, 0x270B),
        (0x2728, 0x2728),
        (0x274C, 0x274C),
        (0x2753, 0x2755),
        (0x2795, 0x2797),
        (0x2B1B, 0x2B1C),
        (0x2E80, 0x303E),
        (0x3041, 0x33FF),
        (0x3400, 0x4DBF),
        (0x4E00, 0x9FFF),
        (0xA000, 0xA4CF),
        (0xAC00, 0xD7A3),
        (0xF900, 0xFAFF),
        (0xFE30, 0xFE4F),
        (0xFF00, 0xFF60),
        (0xFFE0, 0xFFE6),
        (0x1F004, 0x1F004),
        (0x1F0CF, 0x1F0CF),
        (0x1F18E, 0x1F18E),
        (0x1F191, 0x1F19A),
        (0x1F200, 0x1F251),
        (0x1F300, 0x1F64F),
        (0x1F680, 0x1F6FF),
        (0x1F7E0, 0x1F7EB),
        (0x1F900, 0x1F9FF),
        (0x1FA70, 0x1FAFF),
        (0x20000, 0x3FFFD),
    ];
    let within = |t: &[(u32, u32)]| t.iter().any(|&(a, b)| a <= u && u <= b);
    if u == 0x200D || within(ZERO) {
        0
    } else if within(WIDE) {
        2
    } else {
        1
    }
}

/// An in-memory pane: rows of cells laid out from text, for tests and for
/// panes whose contents are not in a terminal engine.
#[derive(Clone, Debug, Default)]
pub struct MemoryView {
    pub size: Size,
    pub rows: Vec<FrameRow>,
    pub prompts: Vec<bool>,
    pub trimmed: u64,
    pub viewport_top: usize,
    /// Number of `row` calls, so tests can check how much a command read.
    pub reads: std::cell::Cell<u64>,
}

impl MemoryView {
    /// An empty pane (a blank screen and no history).
    pub fn new(cols: u16, screen_rows: u16) -> MemoryView {
        let mut v = MemoryView { size: Size::new(cols, screen_rows), ..Default::default() };
        v.pad();
        v
    }

    /// A pane showing `lines` (each one a hard line, soft-wrapped at the
    /// pane width) followed by blank rows to fill the screen; rows beyond the
    /// screen become history. The viewport is at the bottom.
    pub fn from_lines(cols: u16, screen_rows: u16, lines: &[&str]) -> MemoryView {
        let mut v = MemoryView { size: Size::new(cols, screen_rows), ..Default::default() };
        for l in lines {
            v.push_line(l);
        }
        v.pad();
        v
    }

    /// Like [`from_lines`](Self::from_lines), with the lines starting with
    /// `prompt` flagged as shell prompts.
    pub fn with_prompts(cols: u16, screen_rows: u16, lines: &[&str], prompt: &str) -> MemoryView {
        let mut v = MemoryView { size: Size::new(cols, screen_rows), ..Default::default() };
        for l in lines {
            let first = v.rows.len();
            v.push_line(l);
            v.prompts[first] = l.starts_with(prompt);
        }
        v.pad();
        v
    }

    fn pad(&mut self) {
        while self.rows.len() < self.size.rows as usize {
            self.rows.push(self.blank());
            self.prompts.push(false);
        }
        self.viewport_top = self.history_rows();
    }

    fn blank(&self) -> FrameRow {
        FrameRow { cells: vec![Cell::BLANK; self.size.cols as usize], clusters: Vec::new(), wrapped: false }
    }

    /// Append one hard line, wrapping it over as many rows as it needs.
    pub fn push_line(&mut self, text: &str) {
        let cols = self.size.cols as usize;
        let mut row = self.blank();
        let mut x = 0usize;
        let mut join_next = false;
        for c in text.chars() {
            let w = char_width(c);
            if (w == 0 || join_next) && x > 0 {
                // Extend the cluster in the previous cell.
                let mut hx = x - 1;
                if row.cells[hx].width == CellWidth::WideTail {
                    hx -= 1;
                }
                let cell = &mut row.cells[hx];
                if cell.extra == 0 {
                    row.clusters.push(cell.ch.to_string().into_boxed_str());
                    cell.extra = row.clusters.len() as u16;
                }
                let k = cell.extra as usize - 1;
                let mut s = row.clusters[k].to_string();
                s.push(c);
                row.clusters[k] = s.into_boxed_str();
                join_next = c == '\u{200D}';
                continue;
            }
            join_next = false;
            // A wide character in a one-column pane is shown narrow.
            let w = if w > cols { 1 } else { w.max(1) };
            if x + w > cols {
                row.wrapped = true;
                let full = std::mem::replace(&mut row, self.blank());
                self.rows.push(full);
                self.prompts.push(false);
                x = 0;
            }
            row.cells[x].ch = c;
            if w == 2 {
                row.cells[x].width = CellWidth::Wide;
                row.cells[x + 1].width = CellWidth::WideTail;
                row.cells[x + 1].ch = ' ';
            }
            x += w;
        }
        self.rows.push(row);
        self.prompts.push(false);
        self.viewport_top = self.history_rows();
    }

    /// Drop `n` rows from the top, as a scrollback limit would.
    pub fn trim(&mut self, n: usize) {
        let n = n.min(self.history_rows());
        self.rows.drain(..n);
        self.prompts.drain(..n);
        self.trimmed += n as u64;
        self.viewport_top = self.viewport_top.saturating_sub(n);
    }

    /// Plain text of a row (wide tails skipped), for test assertions.
    pub fn row_text(&self, row: usize) -> String {
        let r = &self.rows[row];
        let mut s = String::new();
        for x in 0..r.cells.len() {
            r.push_cell_text(x, &mut s);
        }
        s.trim_end().to_string()
    }
}

impl PaneView for MemoryView {
    fn size(&self) -> Size {
        self.size
    }

    fn history_rows(&self) -> usize {
        self.rows.len().saturating_sub(self.size.rows as usize)
    }

    fn row(&self, row: usize, out: &mut FrameRow) {
        self.reads.set(self.reads.get() + 1);
        match self.rows.get(row) {
            Some(r) => {
                out.cells.clear();
                out.cells.extend_from_slice(&r.cells);
                out.clusters.clone_from(&r.clusters);
                out.wrapped = r.wrapped;
            }
            None => {
                out.cells.clear();
                out.cells.resize(self.size.cols as usize, Cell::BLANK);
                out.clusters.clear();
                out.wrapped = false;
            }
        }
    }

    fn find_prompt(&self, from: usize, forward: bool) -> Option<usize> {
        if forward {
            (from..self.prompts.len()).find(|&r| self.prompts[r])
        } else {
            (0..=from.min(self.prompts.len().saturating_sub(1))).rev().find(|&r| self.prompts[r])
        }
    }

    fn trimmed_rows(&self) -> u64 {
        self.trimmed
    }

    fn set_viewport_top(&mut self, top: usize) {
        self.viewport_top = top;
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn layout_wraps_wide_and_clusters() {
        let v = MemoryView::from_lines(5, 3, &["ab漢字x", "e\u{301}👍🏽z"]);
        assert_eq!(v.rows.len(), 3);
        assert!(v.rows[0].wrapped);
        assert_eq!(v.row_text(0), "ab漢");
        assert_eq!(v.rows[0].cells[2].width, CellWidth::Wide);
        assert_eq!(v.rows[0].cells[3].width, CellWidth::WideTail);
        assert_eq!(v.row_text(1), "字x");
        assert_eq!(v.row_text(2), "e\u{301}👍🏽z");
        assert_eq!(v.rows[2].cells[1].width, CellWidth::Wide);
        assert_eq!(v.rows[2].cells[3].ch, 'z');
    }
}
