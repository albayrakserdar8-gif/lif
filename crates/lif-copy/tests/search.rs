//! Search: directions, wrap, smart case, soft wraps, highlights,
//! incremental prompts, and bounded work over a million rows.

mod common;
use common::*;
use lif_copy::ModeKeys::{Emacs, Vi};
use lif_copy::{Command, Config, CopyMode, Enter, ModeKeys, Outcome, PaneView};
use lif_types::Pos;

// At 20 columns the last "beta" of line 4 starts on column 19 and
// continues on the next row.
const TEXT: &str = "alpha Beta gamma\nbeta alpha\n\nGAMMA beta 01234567beta\nend";

fn at(m: &CopyMode) -> (usize, usize) {
    (m.cursor().row, m.cursor().col)
}

#[test]
fn directions_wrap_and_smart_case() {
    let (mut v, mut m) = pane(20, 4, TEXT, Vi);
    x(&mut m, &mut v, "history-top");
    x(&mut m, &mut v, "search-forward beta");
    assert_eq!(at(&m), (0, 6), "lower-case pattern ignores case");
    x(&mut m, &mut v, "search-again");
    assert_eq!(at(&m), (1, 0));
    x(&mut m, &mut v, "search-again");
    assert_eq!(at(&m), (3, 6));
    x(&mut m, &mut v, "search-again");
    assert_eq!(at(&m), (3, 19), "match across the soft wrap");
    x(&mut m, &mut v, "search-again");
    assert_eq!(at(&m), (0, 6), "wrapped around");
    x(&mut m, &mut v, "search-reverse");
    assert_eq!(at(&m), (3, 19));
    x(&mut m, &mut v, "search-backward Beta");
    assert_eq!(at(&m), (0, 6), "upper case: exact");
    x(&mut m, &mut v, "search-again");
    assert_eq!(at(&m), (0, 6), "the only match, found again by wrapping");
    x(&mut m, &mut v, "2 search-forward-text a.");
    assert_eq!(m.search_string(), Some("a."));
    x(&mut m, &mut v, "search-forward gam+a$");
    assert_eq!(at(&m), (0, 11));

    let mut cfg = Config::new(Vi);
    cfg.wrap_search = false;
    m.set_config(cfg);
    x(&mut m, &mut v, "history-bottom");
    let o = x(&mut m, &mut v, "search-forward alpha");
    assert_eq!(o, Outcome::Message("No match: alpha".into()));
    let o = x(&mut m, &mut v, "search-backward alpha");
    assert_eq!(o, Outcome::Redraw);
    assert_eq!(at(&m), (1, 5));
}

#[test]
fn highlights() {
    let (mut v, mut m) = pane(20, 4, TEXT, Emacs);
    x(&mut m, &mut v, "history-top");
    x(&mut m, &mut v, "search-forward beta");
    let o = m.overlays(&v);
    let spans: Vec<_> = o.matches.iter().map(|s| (s.y, s.x0, s.x1, s.current)).collect();
    assert_eq!(spans, vec![(0, 6, 9, true), (1, 0, 3, false), (3, 6, 9, false), (3, 19, 19, false)]);
    assert_eq!(o.cursor, Pos { x: 10, y: 0 }, "emacs: after the match");
    x(&mut m, &mut v, "history-bottom");
    let o = m.overlays(&v);
    let spans: Vec<_> = o.matches.iter().map(|s| (s.y, s.x0, s.x1)).collect();
    // Viewport rows 2..6: the wrapped match is split over two rows.
    assert_eq!(spans, vec![(1, 6, 9), (1, 19, 19), (2, 0, 2)]);
    assert_eq!(o.indicator, Some("[0/2]"));
}

#[test]
fn incremental_update_and_cancel() {
    let lines: Vec<String> = (0..100).map(|i| format!("entry {i:03} value {}", i * 7)).collect();
    let text = lines.join("\n");
    let (mut v, mut m) = pane(30, 10, &text, Emacs);
    x(&mut m, &mut v, "5 page-up");
    let start = (m.cursor(), m.top());
    let o = keys(&mut m, &mut v, &["C-r"]);
    assert!(matches!(o, Outcome::Prompt { incremental: true, .. }));
    let drive = |m: &mut CopyMode, v: &mut lif_copy::MemoryView, mut o: Outcome| {
        while o == Outcome::Searching {
            o = m.search_step(v, 10);
        }
        o
    };
    let o = m.prompt_changed(&mut v, "entry 01");
    drive(&mut m, &mut v, o);
    let first = m.cursor();
    assert_eq!(v.row_text(first.row), "entry 019 value 133");
    let o = m.prompt_changed(&mut v, "entry 012");
    drive(&mut m, &mut v, o);
    assert_eq!(v.row_text(m.cursor().row), "entry 012 value 84");
    let o = m.prompt_changed(&mut v, "entry 0129");
    assert_eq!(drive(&mut m, &mut v, o), Outcome::Redraw, "no match is silent while typing");
    let o = m.prompt_again(&mut v, false);
    drive(&mut m, &mut v, o);
    m.prompt_cancel(&mut v);
    assert_eq!((m.cursor(), m.top()), start, "cancel restores position");
    assert_eq!(v.viewport_top, start.1);
    assert!(m.overlays(&v).matches.is_empty());

    // Accept keeps the position; an empty incremental search repeats.
    keys(&mut m, &mut v, &["C-s"]);
    let o = m.prompt_changed(&mut v, "value 7");
    drive(&mut m, &mut v, o);
    let found = m.cursor();
    let o = m.prompt_accept(&mut v, "value 7");
    drive(&mut m, &mut v, o);
    assert_eq!(m.cursor(), found);
    match keys(&mut m, &mut v, &["C-s"]) {
        Outcome::Prompt { initial, .. } => assert_eq!(initial, "value 7"),
        o => panic!("{o:?}"),
    }
    let o = m.prompt_accept(&mut v, "");
    drive(&mut m, &mut v, o);
    assert!(m.cursor() > found);
}

#[test]
fn key_press_cancels_pending_search() {
    let mut s = Synth::new(200_000, Some(3));
    let mut m = CopyMode::new(&mut s, Config::new(ModeKeys::Vi), Pos { x: 0, y: 23 }, Enter::default());
    let mut cfg = Config::new(ModeKeys::Vi);
    cfg.search_step_rows = 1000;
    m.set_config(cfg);
    let o = m.run_command(&mut s, Command::SearchBackward, Some("needle"), 1);
    assert_eq!(o, Outcome::Searching);
    assert!(m.searching());
    let before = m.cursor();
    m.handle_key(&key("k"), &mut s);
    assert!(!m.searching());
    assert_eq!(m.cursor().row, before.row - 1);
}

#[test]
fn resumable_search_over_a_million_rows() {
    const ROWS: usize = 1_000_000;
    const BUDGET: usize = 10_000;
    let mut s = Synth::new(ROWS, Some(5));
    let mut m = CopyMode::new(&mut s, Config::new(ModeKeys::Vi), Pos { x: 0, y: 23 }, Enter::default());
    let mut o = m.run_command(&mut s, Command::SearchBackward, Some("needle"), 1);
    let mut steps = 0;
    while o == Outcome::Searching {
        let before = s.reads.get();
        o = m.search_step(&mut s, BUDGET);
        let read = s.reads.get() - before;
        assert!(read as usize <= BUDGET + 8, "step read {read} rows");
        steps += 1;
        assert!(steps < 1000);
    }
    assert_eq!(o, Outcome::Redraw);
    assert_eq!(m.cursor().row, 5);
    assert!(steps >= ROWS / BUDGET - 5, "{steps} steps");
    assert_eq!(s.top, 0, "viewport moved to the match");

    // Exhausting the history without a match.
    let mut s = Synth::new(ROWS, None);
    let mut m = CopyMode::new(&mut s, Config::new(ModeKeys::Vi), Pos { x: 0, y: 0 }, Enter::default());
    let mut o = m.run_command(&mut s, Command::SearchForward, Some("ne+dle"), 1);
    let mut steps = 0;
    while o == Outcome::Searching {
        o = m.search_step(&mut s, BUDGET);
        steps += 1;
    }
    assert_eq!(o, Outcome::Message("No match: ne+dle".into()));
    assert!(steps >= ROWS / BUDGET - 5);
    let _ = s.history_rows();
}
