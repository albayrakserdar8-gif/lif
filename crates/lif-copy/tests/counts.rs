//! Repeat counts: a huge count (send-keys -X -N, or typed) must cost no
//! more than the motion can achieve, and a repeated search must stay in
//! bounded steps.

mod common;
use common::*;
use lif_copy::ModeKeys::{Emacs, Vi};
use lif_copy::{Command, Config, Outcome};
use std::time::{Duration, Instant};

fn quick<T>(what: &str, f: impl FnOnce() -> T) -> T {
    let start = Instant::now();
    let r = f();
    assert!(start.elapsed() < Duration::from_secs(2), "{what} took {:?}", start.elapsed());
    r
}

#[test]
fn motions_stop_repeating_when_the_cursor_stops() {
    let (mut v, mut m) = pane(20, 4, "alpha beta\ngamma\ndelta (x) y\nend", Vi);
    for cmd in [
        "cursor-up",
        "cursor-down",
        "cursor-left",
        "cursor-right",
        "next-word",
        "previous-word",
        "next-word-end",
        "page-up",
        "page-down",
        "halfpage-up",
        "end-of-line",
        "next-paragraph",
        "previous-paragraph",
    ] {
        quick(cmd, || x(&mut m, &mut v, &format!("4294967295 {cmd}")));
    }
    x(&mut m, &mut v, "history-top");
    quick("jump", || x(&mut m, &mut v, "4294967295 jump-forward a"));
    assert_eq!(m.cursor().col, 9, "last a of the line");
}

#[test]
fn bracket_matching_cycles_in_two_steps() {
    let (mut v, mut m) = pane(20, 4, "f(x)", Vi);
    x(&mut m, &mut v, "start-of-line");
    x(&mut m, &mut v, "cursor-right");
    assert_eq!(m.cursor().col, 1);
    quick("odd", || x(&mut m, &mut v, "999999 next-matching-bracket"));
    assert_eq!(m.cursor().col, 3, "odd count: the other bracket");
    quick("even", || x(&mut m, &mut v, "1000000 next-matching-bracket"));
    assert_eq!(m.cursor().col, 3, "even count: back where it was");
    x(&mut m, &mut v, "3 next-matching-bracket");
    assert_eq!(m.cursor().col, 1);
    // Clamped to MAX_COUNT (even).
    quick("huge", || x(&mut m, &mut v, "4294967295 next-matching-bracket"));
    assert_eq!(m.cursor().col, 1);
}

#[test]
fn repeated_wrapping_search_counts_laps() {
    // Three matches; with wrap-search, 3k+1 searches land on the first one.
    let (mut v, mut m) = pane(20, 4, "a1\nb\na2\nc\na3", Emacs);
    x(&mut m, &mut v, "history-top");
    x(&mut m, &mut v, "search-forward-text a");
    let first = m.cursor();
    x(&mut m, &mut v, "history-top");
    quick("search", || x(&mut m, &mut v, "1000000 search-forward-text a"));
    assert_eq!(m.cursor(), first, "1000000 = 3 * 333333 + 1");
    x(&mut m, &mut v, "history-top");
    x(&mut m, &mut v, "2 search-forward-text a");
    let second = m.cursor();
    x(&mut m, &mut v, "history-top");
    quick("search", || x(&mut m, &mut v, "4294967295 search-forward-text a"));
    // The count is clamped to 1000000 first.
    assert_eq!(m.cursor(), first);
    assert_ne!(first, second);
}

#[test]
fn repeated_search_runs_in_bounded_steps() {
    let text: String = (0..200).map(|i| format!("a{i}\n")).collect();
    let (mut v, mut m) = pane(20, 4, &text, Vi);
    let mut cfg = Config::new(Vi);
    cfg.search_step_rows = 10;
    m.set_config(cfg);
    x(&mut m, &mut v, "history-top");
    let out = m.run_command(&mut v, Command::SearchForwardText, Some("a"), 100);
    assert_eq!(out, Outcome::Searching, "100 matches are more than one step's rows");
    let mut steps = 1;
    let mut out = out;
    while out == Outcome::Searching {
        out = m.search_step(&mut v, 10);
        steps += 1;
    }
    assert!(steps >= 10, "{steps}");
    assert_eq!(m.cursor().row, 100, "the 100th match");
}
