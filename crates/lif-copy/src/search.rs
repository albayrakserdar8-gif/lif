//! Searching the history: logical-line text, the resumable search job and
//! the match highlights of the viewport.

use crate::Point;
use crate::grid::Grid;
use crate::regex::{Regex, Scratch, fold, smart_case};
use lif_types::CellWidth;

/// A logical line longer than this many rows is searched in pieces of this
/// size, so one call never builds an unbounded string. Matches that cross a
/// piece boundary are not found.
pub(crate) const MAX_LINE_ROWS: usize = 1024;

/// The text of one logical line (soft-wrapped rows joined) with the cell
/// each character came from.
#[derive(Default, Debug)]
pub(crate) struct LineText {
    pub first: usize,
    pub last: usize,
    pub chars: Vec<char>,
    /// (absolute row, column, width in cells) per char. Every char of a
    /// grapheme cluster maps to the cluster's cell.
    pub at: Vec<(usize, u16, u8)>,
}

impl LineText {
    /// Load the logical line starting at row `first`.
    pub fn load(&mut self, g: &mut Grid, first: usize, folded: bool) {
        self.chars.clear();
        self.at.clear();
        self.first = first;
        let mut y = first;
        loop {
            let wrapped = g.wrapped(y);
            let end = if wrapped { g.cols } else { g.len(y) };
            let row = g.row(y);
            for x in 0..end {
                let cell = &row.cells[x];
                let w = match cell.width {
                    CellWidth::WideTail => continue,
                    CellWidth::Wide => 2,
                    CellWidth::Narrow => 1,
                };
                let at = (y, x as u16, w);
                if cell.extra != 0 {
                    for c in row.clusters[cell.extra as usize - 1].chars() {
                        self.chars.push(if folded { fold(c) } else { c });
                        self.at.push(at);
                    }
                } else {
                    self.chars.push(if folded { fold(cell.ch) } else { cell.ch });
                    self.at.push(at);
                }
            }
            if !wrapped || y + 1 - first >= MAX_LINE_ROWS {
                break;
            }
            y += 1;
        }
        self.last = y;
    }

    pub fn rows(&self) -> usize {
        self.last + 1 - self.first
    }

    /// Index of the first char at or after `p` (after, with `strict`).
    pub fn index_of(&self, p: Point, strict: bool) -> usize {
        let key = (p.row, p.col);
        self.at.partition_point(|&(r, c, _)| {
            let k = (r, c as usize);
            if strict { k <= key } else { k < key }
        })
    }

    pub fn point(&self, i: usize) -> Point {
        let (row, col, _) = self.at[i];
        Point { row, col: col as usize }
    }

    /// The last cell of the char at index `i` (the tail of a wide char).
    pub fn end_point(&self, i: usize) -> Point {
        let (row, col, w) = self.at[i];
        Point { row, col: col as usize + w as usize - 1 }
    }
}

/// A compiled search.
#[derive(Clone, Debug)]
pub(crate) struct Pattern {
    pub text: String,
    pub regex: bool,
    pub re: Regex,
}

impl Pattern {
    /// Compile with tmux's smart case. An invalid regular expression is
    /// searched for literally rather than refused.
    pub fn new(text: &str, regex: bool) -> Pattern {
        let icase = smart_case(text);
        let re = if regex {
            Regex::new(text, icase).unwrap_or_else(|_| Regex::literal(text, icase))
        } else {
            Regex::literal(text, icase)
        };
        Pattern { text: text.to_string(), regex, re }
    }
}

/// A match: first and last cell (inclusive).
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub struct Match {
    pub start: Point,
    pub end: Point,
}

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
enum Phase {
    /// The rest of the origin's line in the search direction.
    First,
    /// Whole lines, moving away from the origin (and around, with wrap).
    Scan,
    /// Back at the origin's line after wrapping: the part not yet searched.
    Final,
}

/// What one bounded step of a search found.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub(crate) enum Step {
    Found(Match),
    Pending,
    NotFound,
}

/// A search in progress. It remembers where to continue, so the server can
/// spread a search through a huge history over several calls.
#[derive(Clone, Debug)]
pub(crate) struct Job {
    pub forward: bool,
    pub origin: Point,
    line_first: usize,
    line_last: usize,
    phase: Phase,
    /// Forward: the first row of the next line to search. Backward: one past
    /// the last row of the next line to search.
    next: usize,
    wrapped: bool,
    /// Matches still to find (a repeat count).
    pub remaining: u32,
    /// Rows the last [`step`](Self::step) examined.
    pub used: usize,
}

impl Job {
    pub fn new(g: &mut Grid, origin: Point, forward: bool, remaining: u32) -> Job {
        let line_first = g.line_start(origin.row, MAX_LINE_ROWS);
        let line_last = g.line_end(line_first, MAX_LINE_ROWS - 1);
        Job {
            forward,
            origin,
            line_first,
            line_last,
            phase: Phase::First,
            next: 0,
            wrapped: false,
            remaining: remaining.max(1),
            used: 0,
        }
    }

    fn found(line: &LineText, m: Option<(usize, usize)>) -> Option<Match> {
        m.map(|(s, e)| Match { start: line.point(s), end: line.end_point(e - 1) })
    }

    /// Search at most about `budget` rows (whole logical lines; at least
    /// one line per call).
    pub fn step(
        &mut self,
        g: &mut Grid,
        pat: &Pattern,
        line: &mut LineText,
        sc: &mut Scratch,
        budget: usize,
        wrap: bool,
    ) -> Step {
        let folded = pat.re.icase();
        self.used = 0;
        loop {
            match self.phase {
                Phase::First => {
                    line.load(g, self.line_first, folded);
                    self.used += line.rows();
                    let n = line.chars.len();
                    let m = if self.forward {
                        let k = line.index_of(self.origin, true);
                        pat.re.find(&line.chars, k, n + 1, false, sc)
                    } else {
                        let k = line.index_of(self.origin, false);
                        pat.re.find(&line.chars, 0, k, true, sc)
                    };
                    if let Some(m) = Self::found(line, m) {
                        return Step::Found(m);
                    }
                    self.line_last = line.last;
                    self.phase = Phase::Scan;
                    self.next = if self.forward { line.last + 1 } else { self.line_first };
                }
                Phase::Scan => {
                    if self.forward {
                        if self.next >= g.total {
                            if wrap && !self.wrapped {
                                self.wrapped = true;
                                self.next = 0;
                            } else {
                                        return Step::NotFound;
                            }
                        }
                        if self.wrapped && self.next >= self.line_first {
                            self.phase = Phase::Final;
                            continue;
                        }
                        line.load(g, self.next, folded);
                        self.used += line.rows();
                        let m = pat.re.find(&line.chars, 0, line.chars.len() + 1, false, sc);
                        if let Some(m) = Self::found(line, m) {
                                return Step::Found(m);
                        }
                        self.next = line.last + 1;
                    } else {
                        if self.next == 0 {
                            if wrap && !self.wrapped {
                                self.wrapped = true;
                                self.next = g.total;
                            } else {
                                        return Step::NotFound;
                            }
                        }
                        if self.wrapped && self.next <= self.line_last + 1 {
                            self.phase = Phase::Final;
                            continue;
                        }
                        let first = g.line_start(self.next - 1, MAX_LINE_ROWS - 1);
                        line.load(g, first, folded);
                        self.used += line.rows();
                        let m = pat.re.find(&line.chars, 0, line.chars.len() + 1, true, sc);
                        if let Some(m) = Self::found(line, m) {
                                return Step::Found(m);
                        }
                        self.next = first;
                    }
                    if self.used >= budget {
                        return Step::Pending;
                    }
                }
                Phase::Final => {
                    line.load(g, self.line_first, folded);
                    let n = line.chars.len();
                    let m = if self.forward {
                        let k = line.index_of(self.origin, true);
                        pat.re.find(&line.chars, 0, k, false, sc)
                    } else {
                        let k = line.index_of(self.origin, false);
                        pat.re.find(&line.chars, k, n + 1, true, sc)
                    };
                    return match Self::found(line, m) {
                        Some(m) => Step::Found(m),
                        None => Step::NotFound,
                    };
                }
            }
        }
    }
}

/// One highlighted run of a search match on one row of the viewport, in
/// pane coordinates; `x0..=x1`.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub struct MatchSpan {
    pub y: u16,
    pub x0: u16,
    pub x1: u16,
    /// Part of the match the cursor was moved to (tmux's search-active style).
    pub current: bool,
}

/// All matches in the rows `top..top + rows`, appended to `out`.
#[allow(clippy::too_many_arguments)]
pub(crate) fn viewport_matches(
    g: &mut Grid,
    pat: &Pattern,
    line: &mut LineText,
    sc: &mut Scratch,
    top: usize,
    rows: usize,
    current: Option<Match>,
    out: &mut Vec<MatchSpan>,
) {
    let bottom = (top + rows).min(g.total);
    let folded = pat.re.icase();
    let mut y = g.line_start(top, MAX_LINE_ROWS - 1);
    while y < bottom {
        line.load(g, y, folded);
        let n = line.chars.len();
        let mut i = 0;
        while i < n {
            let Some((s, e)) = pat.re.find(&line.chars, i, n + 1, false, sc) else { break };
            let is_current = current.is_some_and(|m| m.start == line.point(s));
            // Split the match into one span per row.
            let mut k = s;
            while k < e {
                let row = line.at[k].0;
                let x0 = line.at[k].1;
                let mut x1 = x0 + line.at[k].2 as u16 - 1;
                while k < e && line.at[k].0 == row {
                    x1 = x1.max(line.at[k].1 + line.at[k].2 as u16 - 1);
                    k += 1;
                }
                if row >= top && row < bottom {
                    out.push(MatchSpan { y: (row - top) as u16, x0, x1, current: is_current });
                }
            }
            i = e;
        }
        y = line.last + 1;
    }
}
