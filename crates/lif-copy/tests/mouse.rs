//! Mouse selection, multi-clicks, wheel and edge auto-scroll.

mod common;
use common::*;
use lif_copy::ModeKeys::{Emacs, Vi};
use lif_copy::{Config, CopyMode, DragEnd, Enter, MemoryView, Outcome};
use lif_types::{Mods, MouseAction, MouseButton, MouseEvent, Pos};

fn ev(action: MouseAction, button: Option<MouseButton>, x: u16, y: u16) -> MouseEvent {
    MouseEvent { action, button, mods: Mods::NONE, cell: Pos { x, y }, pixel: None }
}
fn press(x: u16, y: u16) -> MouseEvent {
    ev(MouseAction::Press, Some(MouseButton::Left), x, y)
}
fn drag(x: u16, y: u16) -> MouseEvent {
    ev(MouseAction::Motion, Some(MouseButton::Left), x, y)
}
fn release(x: u16, y: u16) -> MouseEvent {
    ev(MouseAction::Release, Some(MouseButton::Left), x, y)
}

fn setup(keys: lif_copy::ModeKeys) -> (MemoryView, CopyMode) {
    let lines: Vec<String> = (0..30).map(|i| format!("row {i:02} some words")).collect();
    let text = lines.join("\n");
    pane(20, 6, &text, keys)
}

#[test]
fn drag_selects_and_release_copies() {
    let (mut v, mut m) = setup(Vi);
    let t = 0;
    assert_eq!(m.handle_mouse(&press(4, 1), t, &mut v), Outcome::None, "a click alone does nothing");
    assert_eq!(m.handle_mouse(&drag(6, 2), t + 10, &mut v), Outcome::Redraw);
    let out = m.handle_mouse(&release(6, 2), t + 20, &mut v);
    assert_eq!(copied(&out), "25 some words\nrow 26 ");
    assert!(matches!(out, Outcome::Copy { exit: false, .. }), "lif stays in copy mode");
    assert!(m.selection().is_some(), "and keeps the selection shown");

    // Dragging past the end of a line takes the newline (cursor past the text).
    m.handle_mouse(&press(0, 1), 1000, &mut v);
    m.handle_mouse(&drag(19, 1), 1010, &mut v);
    assert_eq!(copied(&m.handle_mouse(&release(19, 1), 1020, &mut v)), "row 25 some words\n");

    let mut cfg = Config::new(Vi);
    cfg.mouse.drag_end = DragEnd::CopyAndCancel;
    m.set_config(cfg);
    m.handle_mouse(&press(0, 0), 2000, &mut v);
    m.handle_mouse(&drag(2, 0), 2010, &mut v);
    assert!(matches!(m.handle_mouse(&release(2, 0), 2020, &mut v), Outcome::Copy { exit: true, .. }));
}

#[test]
fn drag_edges_auto_scroll() {
    let (mut v, mut m) = setup(Emacs);
    let top0 = m.top();
    m.handle_mouse(&press(3, 3), 0, &mut v);
    m.handle_mouse(&drag(3, 0), 5, &mut v);
    assert_eq!(m.top(), top0 - 1, "touching the top edge scrolls");
    assert!(m.autoscroll_active());
    for _ in 0..5 {
        assert_eq!(m.autoscroll_tick(&mut v), Outcome::Redraw);
    }
    assert_eq!(m.top(), top0 - 6);
    assert_eq!(v.viewport_top, m.top());
    assert_eq!(m.cursor().row, m.top());
    m.handle_mouse(&drag(3, 2), 10, &mut v);
    assert!(!m.autoscroll_active());
    m.handle_mouse(&drag(3, 5), 11, &mut v);
    assert_eq!(m.top(), top0 - 5, "bottom edge scrolls back down");
    let text = copied(&m.handle_mouse(&release(3, 5), 20, &mut v));
    // From the cursor (row 24, column 3) to the press point (row 27, column
    // 3), which emacs excludes.
    assert_eq!(text, " 24 some words\nrow 25 some words\nrow 26 some words\nrow");
    assert!(!m.autoscroll_active());
    assert_eq!(m.autoscroll_tick(&mut v), Outcome::None);
}

#[test]
fn double_and_triple_click() {
    let (mut v, mut m) = setup(Vi);
    m.handle_mouse(&press(9, 2), 100, &mut v);
    m.handle_mouse(&release(9, 2), 110, &mut v);
    let out = m.handle_mouse(&press(9, 2), 200, &mut v);
    assert_eq!(copied(&out), "some");
    m.handle_mouse(&release(9, 2), 210, &mut v);
    let out = m.handle_mouse(&press(9, 2), 300, &mut v);
    assert_eq!(copied(&out), "row 26 some words\n");
    // Too slow: a new single click.
    assert_eq!(m.handle_mouse(&press(9, 2), 1000, &mut v), Outcome::None);
}

#[test]
fn wheel_scrolls_five_lines_and_exit_at_bottom() {
    let lines: Vec<String> = (0..30).map(|i| format!("{i}")).collect();
    let text = lines.join("\n");
    let lines: Vec<&str> = text.split('\n').collect();
    let mut v = MemoryView::from_lines(10, 6, &lines);
    let enter = Enter { exit_at_bottom: true, ..Enter::default() };
    let mut m = CopyMode::new(&mut v, Config::new(Vi), Pos { x: 0, y: 5 }, enter);
    let up = ev(MouseAction::Press, Some(MouseButton::WheelUp), 0, 0);
    let down = ev(MouseAction::Press, Some(MouseButton::WheelDown), 0, 0);
    m.handle_mouse(&up, 0, &mut v);
    m.handle_mouse(&up, 0, &mut v);
    assert_eq!(m.position().0, 10);
    m.handle_mouse(&down, 0, &mut v);
    assert_eq!(m.position().0, 5);
    assert_eq!(m.handle_mouse(&down, 0, &mut v), Outcome::Exit, "copy-mode -e leaves at the bottom");
}
