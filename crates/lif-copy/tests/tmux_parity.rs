//! Scenarios replayed against tmux 3.8 (\`tmux -L lifcopy-ref -f /dev/null\`,
//! the same text printed with \`printf '%s' "$(cat file)"\` in a pane of the
//! same size, then \`send-keys -X\` steps and \`display -p
//! '#{copy_cursor_x},#{copy_cursor_y},#{scroll_position}'\` /
//! \`show-buffer\`). The expected values below are tmux's output, except
//! where a comment says lif deliberately differs.

mod common;
use common::*;
use lif_copy::ModeKeys::{Emacs, Vi};

const C1: &str = "漢字テスト end\nfoo.bar  baz-qux(1)\nshort\na much longer line here\n\nlast\n";
const C2: &str = "hello world  \nfoo.bar  baz-qux(1)\n0123456789012345678901234567890123456789XYZ\nabc\n\nlast\n";

fn c3() -> String {
    let mut lines = Vec::new();
    for i in 0..60 {
        lines.push(if i % 7 == 6 { String::new() } else { format!("line {i} (a[b]c) x") });
    }
    lines.join("\n")
}

#[test]
fn wide_chars_and_line_ends_vi() {
    let (mut v, mut m) = pane(30, 8, C1, Vi);
    script(
        &mut m,
        &mut v,
        &[
            ("history-top", (0, 0, 0)),
            ("cursor-right", (2, 0, 0)),
            ("cursor-right", (4, 0, 0)),
            ("cursor-right", (6, 0, 0)),
            // tmux 3.8 lands on column 5, the tail of テ; lif never rests
            // on a tail and goes to its head.
            ("cursor-left", (4, 0, 0)),
            ("end-of-line", (13, 0, 0)),
            ("cursor-left", (12, 0, 0)),
            ("cursor-right", (13, 0, 0)),
            ("cursor-right", (0, 1, 0)),
        ],
    );
}

#[test]
fn line_ends_emacs() {
    let (mut v, mut m) = pane(30, 8, C1, Emacs);
    script(&mut m, &mut v, &[("history-top", (0, 0, 0)), ("end-of-line", (14, 0, 0)), ("cursor-right", (0, 1, 0))]);
}

#[test]
fn vertical_column_memory() {
    let (mut v, mut m) = pane(30, 8, C1, Vi);
    script(
        &mut m,
        &mut v,
        &[
            ("history-top", (0, 0, 0)),
            ("cursor-down", (0, 1, 0)),
            ("end-of-line", (18, 1, 0)),
            ("cursor-down", (4, 2, 0)),
            ("cursor-down", (4, 3, 0)),
            ("cursor-down", (0, 4, 0)),
            ("cursor-down", (3, 5, 0)),
            ("cursor-up", (0, 4, 0)),
            ("start-of-line", (0, 4, 0)),
            ("15 cursor-right", (0, 7, 0)),
            ("cursor-up", (0, 6, 0)),
            ("cursor-down", (0, 7, 0)),
            ("cursor-down", (0, 7, 0)),
        ],
    );
}

#[test]
fn word_motions() {
    let (mut v, mut m) = pane(30, 8, C1, Vi);
    x(&mut m, &mut v, "history-top");
    x(&mut m, &mut v, "cursor-down");
    let mut got = Vec::new();
    for _ in 0..9 {
        x(&mut m, &mut v, "next-word");
        got.push(st(&m));
    }
    let w = |c| (c, 1, 0);
    assert_eq!(got, vec![w(3), w(4), w(9), w(12), w(13), w(16), w(17), w(18), (0, 2, 0)]);

    x(&mut m, &mut v, "history-top");
    x(&mut m, &mut v, "cursor-down");
    let mut got = Vec::new();
    for _ in 0..8 {
        x(&mut m, &mut v, "next-word-end");
        got.push(st(&m).0);
    }
    assert_eq!(got, vec![2, 3, 6, 11, 12, 15, 16, 17]);

    x(&mut m, &mut v, "end-of-line");
    assert_eq!(st(&m), (18, 1, 0));
    let mut got = Vec::new();
    for _ in 0..8 {
        x(&mut m, &mut v, "previous-word");
        got.push(st(&m).0);
    }
    assert_eq!(got, vec![17, 16, 13, 12, 9, 4, 3, 0]);
}

#[test]
fn stream_selection_vi() {
    let (mut v, mut m) = pane(40, 8, C2, Vi);
    x(&mut m, &mut v, "history-top");
    x(&mut m, &mut v, "begin-selection");
    x(&mut m, &mut v, "cursor-right");
    x(&mut m, &mut v, "cursor-right");
    assert_eq!(yank(&mut m, &mut v), "hel");
    x(&mut m, &mut v, "cursor-down");
    assert_eq!(st(&m), (2, 1, 0));
    assert_eq!(yank(&mut m, &mut v), "hello world\nfoo");
    x(&mut m, &mut v, "end-of-line");
    assert_eq!(st(&m), (18, 1, 0));
    assert_eq!(yank(&mut m, &mut v), "hello world\nfoo.bar  baz-qux(1)");
    x(&mut m, &mut v, "cursor-down");
    assert_eq!(st(&m), (18, 2, 0));
    assert_eq!(yank(&mut m, &mut v), "hello world\nfoo.bar  baz-qux(1)\n0123456789012345678");
    x(&mut m, &mut v, "cursor-down");
    assert_eq!(st(&m), (2, 3, 0));
    x(&mut m, &mut v, "cursor-down");
    assert_eq!(st(&m), (2, 4, 0));
    assert_eq!(
        yank(&mut m, &mut v),
        "hello world\nfoo.bar  baz-qux(1)\n0123456789012345678901234567890123456789XYZ\nabc"
    );
}

#[test]
fn line_selection_vi_and_emacs() {
    let (mut v, mut m) = pane(40, 8, C2, Vi);
    x(&mut m, &mut v, "history-top");
    x(&mut m, &mut v, "select-line");
    assert_eq!(st(&m), (10, 0, 0));
    assert_eq!(yank(&mut m, &mut v), "hello world\n");
    x(&mut m, &mut v, "cursor-down");
    assert_eq!(st(&m), (18, 1, 0));
    assert_eq!(yank(&mut m, &mut v), "hello world\nfoo.bar  baz-qux(1)\n");
    x(&mut m, &mut v, "other-end");
    assert_eq!(st(&m), (0, 0, 0));
    x(&mut m, &mut v, "cursor-down");
    assert_eq!(st(&m), (0, 1, 0));
    assert_eq!(yank(&mut m, &mut v), "foo.bar  baz-qux(1)\n");

    let (mut v, mut m) = pane(40, 8, C2, Emacs);
    x(&mut m, &mut v, "history-top");
    x(&mut m, &mut v, "begin-selection");
    x(&mut m, &mut v, "cursor-right");
    x(&mut m, &mut v, "cursor-right");
    assert_eq!(yank(&mut m, &mut v), "he", "emacs: the cursor cell is not selected");
    x(&mut m, &mut v, "cursor-down");
    x(&mut m, &mut v, "end-of-line");
    assert_eq!(st(&m), (19, 1, 0));
    assert_eq!(yank(&mut m, &mut v), "hello world\nfoo.bar  baz-qux(1)");
    x(&mut m, &mut v, "select-line");
    assert_eq!(st(&m), (19, 1, 0));
    assert_eq!(yank(&mut m, &mut v), "foo.bar  baz-qux(1)");
}

#[test]
fn backward_selection_and_wrapped_end_of_line() {
    let (mut v, mut m) = pane(40, 8, C2, Vi);
    x(&mut m, &mut v, "history-top");
    x(&mut m, &mut v, "cursor-down");
    x(&mut m, &mut v, "end-of-line");
    x(&mut m, &mut v, "begin-selection");
    x(&mut m, &mut v, "history-top");
    assert_eq!(yank(&mut m, &mut v), "hello world\nfoo.bar  baz-qux(1)");
    x(&mut m, &mut v, "cursor-down");
    x(&mut m, &mut v, "cursor-down");
    // end-of-line follows the soft wrap to the end of the logical line.
    x(&mut m, &mut v, "end-of-line");
    assert_eq!(st(&m), (2, 3, 0));
    assert_eq!(yank(&mut m, &mut v), ")\n0123456789012345678901234567890123456789XYZ");
}

#[test]
fn rectangle_selection_vi() {
    let (mut v, mut m) = pane(40, 8, C2, Vi);
    x(&mut m, &mut v, "history-top");
    x(&mut m, &mut v, "cursor-right");
    x(&mut m, &mut v, "cursor-right");
    x(&mut m, &mut v, "begin-selection");
    x(&mut m, &mut v, "rectangle-toggle");
    x(&mut m, &mut v, "3 cursor-down");
    x(&mut m, &mut v, "cursor-right");
    assert_eq!(st(&m), (3, 3, 0));
    assert_eq!(yank(&mut m, &mut v), "ll\no.\n23\nZ\n");
    x(&mut m, &mut v, "cursor-down");
    assert_eq!(yank(&mut m, &mut v), "ll\no.\n23\nZ\nc\n");
}

#[test]
fn copy_line_and_end_of_line() {
    for keys in [Vi, Emacs] {
        let (mut v, mut m) = pane(40, 8, C2, keys);
        x(&mut m, &mut v, "history-top");
        x(&mut m, &mut v, "cursor-right");
        assert_eq!(copied(&x(&mut m, &mut v, "copy-line")), "hello world");
        assert_eq!(copied(&x(&mut m, &mut v, "copy-end-of-line")), "ello world");
        assert_eq!(copied(&x(&mut m, &mut v, "2 copy-line")), "hello world\nfoo.bar  baz-qux(1)");
        assert_eq!(st(&m), (1, 0, 0), "copying does not move the cursor");
        assert!(m.selection().is_none());
    }
}

#[test]
fn select_word_cases() {
    let (mut v, mut m) = pane(40, 8, "x (1) a b .. qq", Vi);
    // (cursor column, selected text) from tmux.
    let cases = [
        (0, "x"),
        (1, "x"),
        (2, "(1"),
        (3, "1)"),
        (4, ")"),
        (5, ")"),
        (6, "a"),
        (7, "a"),
        (8, "b"),
        (9, "b"),
        (10, ".."),
        (11, ".."),
        (12, ".."),
        (13, "qq"),
    ];
    for (col, want) in cases {
        x(&mut m, &mut v, "clear-selection");
        x(&mut m, &mut v, "start-of-line");
        if col > 0 {
            x(&mut m, &mut v, &format!("{col} cursor-right"));
        }
        x(&mut m, &mut v, "select-word");
        assert_eq!(yank(&mut m, &mut v), want, "select-word at {col}");
    }
    let (mut v, mut m) = pane(40, 8, C2, Vi);
    x(&mut m, &mut v, "history-top");
    x(&mut m, &mut v, "cursor-down");
    x(&mut m, &mut v, "3 cursor-right");
    x(&mut m, &mut v, "select-word");
    assert_eq!(st(&m), (6, 1, 0));
    assert_eq!(yank(&mut m, &mut v), ".bar");
}

#[test]
fn scrolling() {
    let text = c3();
    let (mut v, mut m) = pane(30, 10, &text, Vi);
    script(
        &mut m,
        &mut v,
        &[
            ("cursor-right", (16, 9, 0)),
            ("cursor-right", (16, 9, 0)),
            ("page-up", (16, 9, 8)),
            ("page-up", (16, 9, 16)),
            ("page-down", (16, 9, 8)),
            ("halfpage-up", (16, 9, 13)),
            ("halfpage-down", (16, 9, 8)),
            ("scroll-up", (16, 9, 9)),
            ("scroll-up", (16, 9, 10)),
            ("cursor-down", (16, 9, 9)),
            ("scroll-down", (16, 8, 8)),
            ("top-line", (0, 0, 8)),
            ("scroll-down", (0, 0, 7)),
            ("bottom-line", (0, 9, 7)),
            ("middle-line", (0, 4, 7)),
            ("history-top", (0, 0, 50)),
            ("history-bottom", (16, 9, 0)),
            ("goto-line 30", (16, 9, 30)),
            ("scroll-middle", (16, 4, 25)),
            // tmux leaves the vi cursor one past the end of the 16-column
            // line here (column 16); lif keeps vi's cursor on a character.
            ("3 page-up", (15, 4, 49)),
            ("history-top", (0, 0, 50)),
            ("scroll-up", (0, 0, 50)),
            ("page-up", (0, 0, 50)),
            ("scroll-down", (0, 0, 49)),
            ("scroll-down", (0, 0, 48)),
        ],
    );
}

#[test]
fn scrolling_emacs_keeps_screen_row() {
    let text = c3();
    let (mut v, mut m) = pane(30, 10, &text, Emacs);
    script(
        &mut m,
        &mut v,
        &[("page-up", (17, 9, 8)), ("scroll-down", (17, 9, 7)), ("scroll-down", (17, 9, 6)), ("scroll-up", (17, 9, 7))],
    );
}

#[test]
fn paragraphs() {
    let text = c3();
    let (mut v, mut m) = pane(30, 10, &text, Vi);
    script(
        &mut m,
        &mut v,
        &[
            ("history-top", (0, 0, 50)),
            ("next-paragraph", (0, 6, 50)),
            ("next-paragraph", (0, 8, 45)),
            ("previous-paragraph", (0, 1, 45)),
        ],
    );
}

#[test]
fn jumps_and_brackets() {
    let text = c3();
    let (mut v, mut m) = pane(30, 10, &text, Vi);
    script(
        &mut m,
        &mut v,
        &[
            ("history-top", (0, 0, 50)),
            ("jump-forward a", (8, 0, 50)),
            ("jump-again", (8, 0, 50)),
            ("jump-reverse", (8, 0, 50)),
            ("jump-to-forward ]", (10, 0, 50)),
            ("jump-again", (10, 0, 50)),
            ("jump-again", (10, 0, 50)),
            ("jump-backward l", (0, 0, 50)),
            ("start-of-line", (0, 0, 50)),
            ("2 jump-forward c", (12, 0, 50)),
            ("jump-to-backward a", (9, 0, 50)),
            ("jump-to-backward a", (9, 0, 50)),
            ("jump-again", (9, 0, 50)),
            ("start-of-line", (0, 0, 50)),
            ("next-matching-bracket", (13, 0, 50)),
            ("next-matching-bracket", (7, 0, 50)),
            ("cursor-right", (8, 0, 50)),
            ("next-matching-bracket", (11, 0, 50)),
            ("previous-matching-bracket", (9, 0, 50)),
            ("end-of-line", (15, 0, 50)),
            ("jump-forward a", (15, 0, 50)),
            ("cursor-down", (15, 1, 50)),
            ("jump-reverse", (8, 1, 50)),
        ],
    );
    let (mut v, mut m) = pane(30, 8, "{ a ( b ] c ) }\nx(\ny)z", Vi);
    script(
        &mut m,
        &mut v,
        &[
            ("history-top", (0, 0, 0)),
            ("next-matching-bracket", (14, 0, 0)),
            ("next-matching-bracket", (0, 0, 0)),
            ("previous-matching-bracket", (0, 0, 0)),
            ("3 cursor-right", (3, 0, 0)),
            ("previous-matching-bracket", (3, 0, 0)),
            ("cursor-down", (1, 1, 0)),
            ("start-of-line", (0, 1, 0)),
            ("cursor-right", (1, 1, 0)),
            ("next-matching-bracket", (1, 2, 0)),
            ("next-matching-bracket", (1, 1, 0)),
        ],
    );
}

#[test]
fn search_regex_smart_case_wrap() {
    let text = c3();
    let (mut v, mut m) = pane(30, 10, &text, Vi);
    script(
        &mut m,
        &mut v,
        &[
            ("history-top", (0, 0, 50)),
            ("search-forward line 1", (0, 1, 50)),
            ("search-again", (0, 8, 48)),
            ("search-again", (0, 9, 48)),
            ("search-reverse", (0, 8, 48)),
            ("search-backward LINE 2", (0, 8, 48)),
            ("search-backward Line 2", (0, 8, 48)),
            ("search-again", (0, 8, 48)),
            ("history-bottom", (16, 9, 0)),
            ("search-forward line 3", (0, 3, 50)),
            ("search-forward line 59", (0, 9, 0)),
            ("search-again", (0, 9, 0)),
            ("search-forward b.c", (11, 9, 0)),
            ("search-forward nomatch", (11, 9, 0)),
        ],
    );
}

#[test]
fn search_across_soft_wrap_and_emacs_cursor() {
    let (mut v, mut m) = pane(40, 8, C2, Vi);
    script(
        &mut m,
        &mut v,
        &[
            ("history-top", (0, 0, 0)),
            ("search-forward 789XY", (37, 2, 0)),
            ("search-forward hello", (0, 0, 0)),
            ("history-top", (0, 0, 0)),
            ("search-forward hello", (0, 0, 0)),
            ("search-backward abc", (0, 4, 0)),
        ],
    );
    let (mut v, mut m) = pane(40, 8, C2, Emacs);
    script(
        &mut m,
        &mut v,
        &[("history-top", (0, 0, 0)), ("search-forward world", (11, 0, 0)), ("search-backward foo", (0, 1, 0))],
    );
}
