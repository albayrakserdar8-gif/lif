//! Selected text and selection overlays for every kind, in both
//! directions, in vi and emacs mode.

mod common;
use common::*;
use lif_copy::ModeKeys::{Emacs, Vi};
use lif_copy::{Outcome, SelectionShape};
use lif_types::Pos;

// 10 columns: line 2 soft-wraps; line 1 has trailing blanks.
const TEXT: &str = "ab漢字cd\nfoo bar   \n0123456789wrapped\ne\u{301}\u{1F44D}\u{1F3FD}z\n\nend";

fn sel(keys: lif_copy::ModeKeys, steps: &[&str]) -> (String, lif_copy::Overlays<'static>) {
    let (mut v, mut m) = pane(10, 8, TEXT, keys);
    x(&mut m, &mut v, "history-top");
    for s in steps {
        x(&mut m, &mut v, s);
    }
    let text = m.selection_text(&v).unwrap_or_default();
    let o = m.overlays(&v);
    let o = lif_copy::Overlays { indicator: None, matches: &[], ..o };
    (text, o)
}

#[test]
fn stream_both_directions() {
    // Forward over a wide char: ending on it includes all of it.
    let (t, o) = sel(Vi, &["begin-selection", "2 cursor-right"]);
    assert_eq!(t, "ab漢");
    let s = o.selection.unwrap();
    assert_eq!((s.start, s.end, s.shape), (Pos { x: 0, y: 0 }, Pos { x: 3, y: 0 }, SelectionShape::Stream));
    // Backward: the anchor character is included.
    let (t, _) = sel(Vi, &["end-of-line", "begin-selection", "2 cursor-left"]);
    assert_eq!(t, "字cd");
    // Across lines: trailing blanks trimmed, soft wrap joined.
    let (t, _) = sel(Vi, &["cursor-down", "begin-selection", "cursor-down", "cursor-down", "end-of-line"]);
    assert_eq!(t, "foo bar\n0123456789wrapped");
    let (t, _) = sel(Vi, &["3 cursor-down", "end-of-line", "begin-selection", "2 cursor-up", "start-of-line"]);
    assert_eq!(t, "foo bar\n0123456789wrapped");
    // Clusters copy whole.
    let (t, _) = sel(Vi, &["4 cursor-down", "begin-selection", "end-of-line"]);
    assert_eq!(t, "e\u{301}\u{1F44D}\u{1F3FD}z");
    // emacs: the cursor's cell is not part of the selection, either way.
    let (t, o) = sel(Emacs, &["begin-selection", "2 cursor-right"]);
    assert_eq!(t, "ab");
    assert_eq!(o.selection.unwrap().end, Pos { x: 1, y: 0 });
    let (t, _) = sel(Emacs, &["end-of-line", "begin-selection", "2 cursor-left"]);
    assert_eq!(t, "cd");
    let (t, o) = sel(Emacs, &["begin-selection"]);
    assert_eq!(t, "");
    assert_eq!(o.selection, None, "an empty emacs selection draws nothing");
}

#[test]
fn line_wise() {
    let (t, o) = sel(Vi, &["cursor-down", "select-line", "cursor-down"]);
    assert_eq!(t, "foo bar\n0123456789wrapped\n");
    let s = o.selection.unwrap();
    assert_eq!(s.shape, SelectionShape::Lines);
    assert_eq!((s.start.y, s.end.y), (1, 3));
    // Upwards from the anchored line.
    let (t, _) = sel(Vi, &["3 cursor-down", "select-line", "cursor-up"]);
    assert_eq!(t, "0123456789wrapped\n", "select-line on a wrapped row takes the logical line");
    let (t, _) = sel(Vi, &["4 cursor-down", "select-line", "2 cursor-up"]);
    assert_eq!(t, "0123456789wrapped\ne\u{301}\u{1F44D}\u{1F3FD}z\n");
    let (t, _) = sel(Emacs, &["cursor-down", "select-line", "cursor-down"]);
    assert_eq!(t, "foo bar\n0123456789wrapped", "emacs drops the final newline");
    let (t, _) = sel(Vi, &["2 select-line"]);
    assert_eq!(t, "ab漢字cd\nfoo bar\n", "a count selects more lines");
}

#[test]
fn rectangles() {
    let (t, o) = sel(Vi, &["cursor-right", "begin-selection", "rectangle-toggle", "2 cursor-down", "2 cursor-right"]);
    // Ends inside a line: no final newline (tmux's rule).
    assert_eq!(t, "b漢\noo \n123");
    let s = o.selection.unwrap();
    assert_eq!((s.shape, s.start, s.end), (SelectionShape::Rect, Pos { x: 1, y: 0 }, Pos { x: 3, y: 2 }));
    // Cursor left of the anchor.
    // (3 cursor-right from column 0 lands on 字 at column 4.)
    let (t, _) = sel(Vi, &["3 cursor-right", "begin-selection", "rectangle-toggle", "2 cursor-down", "2 cursor-left"]);
    assert_eq!(t, "漢字\no b\n234");
    // A rectangle can extend past the end of short lines.
    let (t, _) = sel(Vi, &["4 cursor-down", "begin-selection", "rectangle-toggle", "cursor-down", "cursor-down", "9 cursor-right"]);
    assert_eq!(t, "e\u{301}\u{1F44D}\u{1F3FD}z\n\nend\n");
    // emacs rectangles exclude the cursor column when it is right of the anchor.
    let (t, _) = sel(Emacs, &["cursor-right", "begin-selection", "rectangle-toggle", "cursor-down", "2 cursor-right"]);
    assert_eq!(t, "b漢\noo");
    // Toggling back gives a stream again.
    let (t, _) = sel(Vi, &["begin-selection", "rectangle-toggle", "cursor-down", "rectangle-toggle"]);
    assert_eq!(t, "ab漢字cd\nf");
}

#[test]
fn overlay_clips_to_viewport() {
    let lines: Vec<String> = (0..40).map(|i| format!("line {i}")).collect();
    let text = lines.join("\n");
    let (mut v, mut m) = pane(20, 5, &text, Vi);
    x(&mut m, &mut v, "history-top");
    x(&mut m, &mut v, "3 cursor-right");
    x(&mut m, &mut v, "begin-selection");
    x(&mut m, &mut v, "20 cursor-down");
    let o = m.overlays(&v);
    let s = o.selection.unwrap();
    assert_eq!(s.start, Pos { x: 0, y: 0 }, "anchor above the screen: from the top-left");
    assert_eq!(s.end, Pos { x: 3, y: 4 });
    x(&mut m, &mut v, "other-end");
    let o = m.overlays(&v);
    let s = o.selection.unwrap();
    assert_eq!((s.start, s.end), (Pos { x: 3, y: 0 }, Pos { x: 19, y: 4 }));
    x(&mut m, &mut v, "clear-selection");
    assert!(m.overlays(&v).selection.is_none());
}

#[test]
fn copy_outcomes() {
    let (mut v, mut m) = pane(10, 8, TEXT, Vi);
    x(&mut m, &mut v, "history-top");
    assert_eq!(x(&mut m, &mut v, "copy-selection"), Outcome::Redraw, "nothing selected");
    assert_eq!(x(&mut m, &mut v, "copy-selection-and-cancel"), Outcome::Exit);
    x(&mut m, &mut v, "begin-selection");
    x(&mut m, &mut v, "cursor-right");
    assert_eq!(x(&mut m, &mut v, "copy-selection-no-clear"), Outcome::Copy { data: b"ab".to_vec(), append: false, exit: false });
    assert!(m.selection().is_some());
    assert_eq!(x(&mut m, &mut v, "append-selection"), Outcome::Copy { data: b"ab".to_vec(), append: true, exit: false });
    assert!(m.selection().is_none(), "append clears");
    x(&mut m, &mut v, "begin-selection");
    x(&mut m, &mut v, "stop-selection");
    x(&mut m, &mut v, "3 cursor-right");
    assert_eq!(m.selection_text(&v).unwrap(), "b", "stopped selections no longer follow the cursor");
    assert_eq!(
        x(&mut m, &mut v, "copy-selection-and-cancel"),
        Outcome::Copy { data: b"b".to_vec(), append: false, exit: true }
    );
    assert_eq!(copied(&x(&mut m, &mut v, "copy-end-of-line-and-cancel")), "cd");
}
