//! Motions on awkward text: wide characters, clusters, combining marks,
//! soft wraps, blank lines, expanded tabs and prompts; counts and bounds.

mod common;
use common::*;
use lif_copy::{Config, CopyMode, Enter, MemoryView, ModeKeys, PaneView};
use lif_types::{CellWidth, Pos};

const LINES: &[&str] = &[
    "$ ls -la",
    "    indented line",
    "漢字かな交じり文 mixed",
    "e\u{301}clair cafe\u{301} nai\u{308}ve",
    "👍🏽 thumbs 👨\u{200d}👩\u{200d}👧 family",
    "xxxxxxxxxxxxxxxxxxxxxxxxx wrapped tail",
    "",
    "col1    col2    col3",
    "$ echo done",
    "done",
];

fn view() -> MemoryView {
    MemoryView::with_prompts(20, 5, LINES, "$ ")
}

fn enter(v: &mut MemoryView, keys: ModeKeys) -> CopyMode {
    CopyMode::new(v, Config::new(keys), Pos { x: 4, y: 4 }, Enter::default())
}

fn not_on_tail(m: &CopyMode, v: &MemoryView) {
    let c = m.cursor();
    assert_ne!(v.rows[c.row].cells[c.col].width, CellWidth::WideTail, "cursor on a wide tail at {c:?}");
}

/// Absolute row of the first row of LINES[i].
fn row_of(v: &MemoryView, i: usize) -> usize {
    let mut n = 0;
    for (r, row) in v.rows.iter().enumerate() {
        if n == i {
            return r;
        }
        if !row.wrapped {
            n += 1;
        }
    }
    panic!("no line {i}")
}

fn goto(m: &mut CopyMode, v: &mut MemoryView, row: usize) {
    x(m, v, "history-top");
    if row > 0 {
        x(m, v, &format!("{row} cursor-down"));
    }
    while m.cursor().col > 0 {
        x(m, v, "cursor-left");
    }
    assert_eq!(m.cursor().row, row);
}

#[test]
fn layout_is_as_expected() {
    let v = view();
    assert_eq!(v.row_text(row_of(&v, 2)), "漢字かな交じり文 mix");
    assert!(v.rows[row_of(&v, 5)].wrapped);
    assert_eq!(v.history_rows(), v.rows.len() - 5);
}

#[test]
fn walking_over_wide_and_clusters() {
    for keys in [ModeKeys::Vi, ModeKeys::Emacs] {
        let mut v = view();
        let mut m = enter(&mut v, keys);
        for line in [2, 3, 4] {
            let row = row_of(&v, line);
            goto(&mut m, &mut v, row);
            let mut cols = vec![0];
            loop {
                x(&mut m, &mut v, "cursor-right");
                not_on_tail(&m, &v);
                if m.cursor().row != row {
                    break;
                }
                cols.push(m.cursor().col);
            }
            // Back again with cursor-left, through the same columns.
            goto(&mut m, &mut v, row);
            if cols.len() > 1 {
                x(&mut m, &mut v, &format!("{} cursor-right", cols.len() - 1));
            }
            assert_eq!(m.cursor().row, row);
            let mut back = vec![m.cursor().col];
            while m.cursor().col > 0 {
                x(&mut m, &mut v, "cursor-left");
                not_on_tail(&m, &v);
                assert_eq!(m.cursor().row, row);
                back.push(m.cursor().col);
            }
            back.reverse();
            let mut fwd = cols.clone();
            fwd.dedup();
            assert_eq!(fwd[..back.len().min(fwd.len())], back[..back.len().min(fwd.len())], "{keys:?} line {line}");
            match line {
                2 => assert_eq!(&cols[..5], &[0, 2, 4, 6, 8]),
                3 => assert_eq!(&cols[..3], &[0, 1, 2], "é is one cell"),
                _ => assert_eq!(&cols[..3], &[0, 2, 3], "👍🏽 is one wide cell"),
            }
        }
    }
}

#[test]
fn vertical_motion_never_rests_on_a_tail() {
    let mut v = MemoryView::from_lines(10, 4, &["abcdefgh", "漢字漢字漢", "a漢字漢字", "xyz"]);
    let mut m = enter(&mut v, ModeKeys::Vi);
    x(&mut m, &mut v, "history-top");
    x(&mut m, &mut v, "3 cursor-right");
    assert_eq!(m.cursor().col, 3);
    x(&mut m, &mut v, "cursor-down");
    assert_eq!(m.cursor().col, 2, "column 3 is a tail: go to its head");
    x(&mut m, &mut v, "cursor-down");
    not_on_tail(&m, &v);
    x(&mut m, &mut v, "cursor-up");
    x(&mut m, &mut v, "cursor-up");
    assert_eq!(m.cursor().col, 3, "the remembered column comes back");
}

#[test]
fn words_on_wide_and_combining_text() {
    let mut v = view();
    let mut m = enter(&mut v, ModeKeys::Vi);
    let r = row_of(&v, 2);
    goto(&mut m, &mut v, r);
    x(&mut m, &mut v, "next-word");
    assert_eq!((m.cursor().row, m.cursor().col), (r, 17), "CJK run is one word");
    x(&mut m, &mut v, "next-word");
    assert_eq!((m.cursor().row, m.cursor().col), (r + 2, 0), "\"mix|ed\" is one wrapped word");
    x(&mut m, &mut v, "previous-word");
    assert_eq!((m.cursor().row, m.cursor().col), (r, 17));
    x(&mut m, &mut v, "previous-word");
    assert_eq!((m.cursor().row, m.cursor().col), (r, 0));
    let r = row_of(&v, 3);
    goto(&mut m, &mut v, r);
    x(&mut m, &mut v, "next-word-end");
    assert_eq!(m.cursor().col, 5, "éclair ends at column 5");
    x(&mut m, &mut v, "select-word");
    assert_eq!(yank(&mut m, &mut v), "e\u{301}clair");
    x(&mut m, &mut v, "clear-selection");
    x(&mut m, &mut v, "next-word");
    x(&mut m, &mut v, "select-word");
    assert_eq!(yank(&mut m, &mut v), "cafe\u{301}");
}

#[test]
fn space_motions_and_tabs() {
    let mut v = view();
    let mut m = enter(&mut v, ModeKeys::Vi);
    let r = row_of(&v, 7);
    goto(&mut m, &mut v, r);
    let mut got = vec![];
    for _ in 0..2 {
        x(&mut m, &mut v, "next-space");
        got.push(m.cursor().col);
    }
    assert_eq!(got, vec![8, 16]);
    x(&mut m, &mut v, "previous-space");
    assert_eq!(m.cursor().col, 8);
    x(&mut m, &mut v, "next-space-end");
    assert_eq!(m.cursor().col, 11);
    let r0 = row_of(&v, 0);
    goto(&mut m, &mut v, r0);
    x(&mut m, &mut v, "2 next-space");
    assert_eq!(m.cursor().col, 5, "\"$ ls -la\": -la is one space-word");
    x(&mut m, &mut v, "next-word");
    assert_eq!(m.cursor().col, 6, "- is a separator, la a word");
}

#[test]
fn line_starts_on_wrapped_lines_and_indentation() {
    let mut v = view();
    let mut m = enter(&mut v, ModeKeys::Vi);
    let r = row_of(&v, 5);
    goto(&mut m, &mut v, r + 1);
    x(&mut m, &mut v, "3 cursor-right");
    x(&mut m, &mut v, "start-of-line");
    assert_eq!((m.cursor().row, m.cursor().col), (r, 0), "start of the logical line");
    x(&mut m, &mut v, "end-of-line");
    assert_eq!((m.cursor().row, m.cursor().col), (r + 1, 17));
    let r1 = row_of(&v, 1);
    goto(&mut m, &mut v, r1);
    x(&mut m, &mut v, "end-of-line");
    x(&mut m, &mut v, "back-to-indentation");
    assert_eq!(m.cursor().col, 4);
    let r6 = row_of(&v, 6);
    goto(&mut m, &mut v, r6);
    x(&mut m, &mut v, "back-to-indentation");
    assert_eq!(m.cursor().col, 0, "blank line");
}

#[test]
fn counts_and_bounds() {
    let mut v = view();
    let mut m = enter(&mut v, ModeKeys::Vi);
    let total = v.rows.len();
    x(&mut m, &mut v, "1000 cursor-up");
    assert_eq!((m.cursor().row, m.top()), (0, 0));
    x(&mut m, &mut v, "1000 cursor-down");
    assert_eq!(m.cursor().row, total - 1);
    assert_eq!(m.top(), v.history_rows());
    x(&mut m, &mut v, "1000 page-up");
    assert_eq!(m.top(), 0);
    x(&mut m, &mut v, "1000 halfpage-down");
    assert_eq!(m.top(), v.history_rows());
    x(&mut m, &mut v, "history-top");
    x(&mut m, &mut v, "1000 cursor-left");
    assert_eq!((m.cursor().row, m.cursor().col), (0, 0));
    x(&mut m, &mut v, "100 next-word");
    assert_eq!(m.cursor().row, total - 1);
    x(&mut m, &mut v, "100 previous-word");
    assert_eq!((m.cursor().row, m.cursor().col), (0, 0));
    x(&mut m, &mut v, "3 next-paragraph");
    x(&mut m, &mut v, "3 previous-paragraph");
    assert_eq!(m.cursor().row, 0);
    x(&mut m, &mut v, "1000 scroll-down");
    assert_eq!(m.top(), v.history_rows());
    assert!(m.cursor().row >= m.top());
}

#[test]
fn viewport_follows_the_cursor() {
    let mut v = view();
    let mut m = enter(&mut v, ModeKeys::Vi);
    x(&mut m, &mut v, "history-top");
    assert_eq!(v.viewport_top, 0);
    x(&mut m, &mut v, "4 cursor-down");
    assert_eq!(v.viewport_top, 0);
    x(&mut m, &mut v, "cursor-down");
    assert_eq!(v.viewport_top, 1, "one row past the bottom scrolls one row");
    x(&mut m, &mut v, "bottom-line");
    x(&mut m, &mut v, "next-word");
    assert!(m.cursor().row < v.viewport_top + 5);
    let (off, hist) = m.position();
    assert_eq!(off, hist - v.viewport_top);
}

#[test]
fn prompts() {
    let mut v = view();
    let mut m = enter(&mut v, ModeKeys::Emacs);
    let p0 = row_of(&v, 0);
    let p1 = row_of(&v, 8);
    x(&mut m, &mut v, "previous-prompt");
    assert_eq!(m.cursor().row, p1);
    x(&mut m, &mut v, "previous-prompt");
    assert_eq!((m.cursor().row, m.cursor().col), (p0, 0));
    assert_eq!(v.viewport_top, 0);
    x(&mut m, &mut v, "previous-prompt");
    assert_eq!(m.cursor().row, p0);
    x(&mut m, &mut v, "2 next-prompt");
    assert_eq!(m.cursor().row, p1);
}

#[test]
fn marks() {
    let mut v = view();
    let mut m = enter(&mut v, ModeKeys::Vi);
    x(&mut m, &mut v, "history-top");
    x(&mut m, &mut v, "set-mark");
    x(&mut m, &mut v, "history-bottom");
    let bottom = m.cursor();
    x(&mut m, &mut v, "jump-to-mark");
    assert_eq!((m.cursor().row, m.cursor().col), (0, 0));
    x(&mut m, &mut v, "jump-to-mark");
    assert_eq!(m.cursor(), bottom);
}

#[test]
fn trimmed_history_keeps_positions_on_their_text() {
    let mut v = view();
    let mut m = enter(&mut v, ModeKeys::Vi);
    let r = row_of(&v, 4);
    goto(&mut m, &mut v, r);
    x(&mut m, &mut v, "begin-selection");
    x(&mut m, &mut v, "end-of-line");
    let before = m.selection_text(&v).unwrap();
    v.trim(2);
    v.push_line("new output");
    m.refresh(&mut v);
    assert_eq!(m.cursor().row, r - 2);
    assert_eq!(m.selection_text(&v).unwrap(), before);
    assert_eq!(v.viewport_top, m.top());
}

#[test]
fn resize_keeps_everything_in_bounds() {
    let mut v = view();
    let mut m = enter(&mut v, ModeKeys::Vi);
    x(&mut m, &mut v, "end-of-line");
    v.size.cols = 6;
    for r in &mut v.rows {
        r.cells.truncate(6);
        if r.cells.last().is_some_and(|c| c.width == CellWidth::Wide) {
            *r.cells.last_mut().unwrap() = lif_types::Cell::BLANK;
        }
    }
    m.refresh(&mut v);
    assert!(m.cursor().col < 6);
    v.size.rows = 2;
    m.refresh(&mut v);
    let o = m.overlays(&v);
    assert!(o.cursor.y < 2);
}
