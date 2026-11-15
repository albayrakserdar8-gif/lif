//! Mouse keys: the root table's `MouseDown1Pane`, `WheelUpStatus` and so
//! on, with tmux 3.8's names and default bindings. What lif does by itself
//! for the defaults is covered in `basic.rs` and `copy.rs`; this is
//! binding, rebinding and unbinding them.
mod common;

use common::*;
use lif_types::Size;

fn attach(env: &Env) -> Term {
    env.ok(&["new-session", "-d", "-s", "s", "-x", "80", "-y", "24"]);
    let mut t = Term::spawn(env, &["attach", "-t", "s:"], Size::new(80, 24));
    t.wait_text("$");
    t.wait_for("mouse reporting on", secs(10), |t| t.modes().mouse_tracking);
    t
}

fn show(env: &Env, fmt: &str) -> String {
    env.ok(&["display-message", "-p", "-t", "s:", fmt]).trim_end().to_string()
}

/// An SGR mouse report: `b` is the button code (0 left, 1 middle, 2 right,
/// +32 motion, 64/65 wheel, +8 Meta, +16 Control), `x`/`y` are 0-based.
fn sgr(b: u8, x: u16, y: u16, press: bool) -> Vec<u8> {
    format!("\x1b[<{b};{};{}{}", x + 1, y + 1, if press { 'M' } else { 'm' }).into_bytes()
}

/// The status line up to status-right (which starts with a quote and
/// holds the time, whose digits look like window indexes).
fn window_list(t: &Term) -> String {
    let line = t.lines()[23].clone();
    line.split('"').next().unwrap_or("").to_string()
}

fn click(t: &mut Term, b: u8, x: u16, y: u16) {
    t.write(&sgr(b, x, y, true));
    t.write(&sgr(b, x, y, false));
}

#[test]
fn a_bound_mouse_key_runs_with_the_mouse_as_target() {
    let env = Env::new();
    let mut t = attach(&env);
    t.type_str("echo alpha be''ta gamma\r");
    t.wait_text("alpha beta gamma");
    env.ok(&["split-window", "-h", "-t", "s:"]);
    t.wait_for("split", secs(10), |t| t.cursor().is_some_and(|(x, _)| x > 40));
    let y = t.lines().iter().position(|l| l.starts_with("alpha beta gamma")).unwrap() as u16;
    // -t = is the pane under the mouse, not the active one; the mouse_*
    // formats are relative to that pane.
    env.ok(&["bind", "-n", "MouseDown2Pane", "display-message", "-t", "=", "#{pane_index} #{mouse_x},#{mouse_y} [#{mouse_word}] [#{mouse_line}]"]);
    click(&mut t, 1, 7, y);
    t.wait_text(&format!("0 7,{y} [beta] [alpha beta gamma]"));
    assert_eq!(show(&env, "#{pane_index}"), "1", "the binding did not select the pane");
    // Modifiers are part of the key.
    env.ok(&["bind", "-n", "C-MouseDown1Pane", "display-message", "ctrl #{mouse_x}"]);
    env.ok(&["bind", "-n", "M-MouseDown3Pane", "display-message", "meta #{mouse_x}"]);
    click(&mut t, 16, 3, y);
    t.wait_text("ctrl 3");
    click(&mut t, 2 + 8, 4, y);
    t.wait_text("meta 4");
    // Outside a mouse key there is no mouse target.
    let o = env.run(&["select-pane", "-t", "="]);
    assert!(!o.status.success());
    assert_eq!(String::from_utf8_lossy(&o.stderr), "no mouse target\n");
    // Names are tmux's, and checked.
    let o = env.run(&["bind", "-n", "MouseDown9Pane", "kill-server"]);
    assert_eq!(String::from_utf8_lossy(&o.stderr), "unknown key: MouseDown9Pane\n");
    assert!(env.ok(&["list-keys", "-T", "root", "C-MouseDown1Pane"]).contains("C-MouseDown1Pane"));
}

#[test]
fn unbinding_a_mouse_key_removes_what_it_did() {
    let env = Env::new();
    let mut t = attach(&env);
    env.ok(&["split-window", "-h", "-t", "s:"]);
    t.wait_for("split", secs(10), |t| t.cursor().is_some_and(|(x, _)| x > 40));
    env.ok(&["unbind", "-n", "MouseDown1Pane"]);
    env.ok(&["unbind", "-n", "MouseDrag1Border"]);
    click(&mut t, 0, 5, 5);
    // The border (column 40) dragged to column 30: nothing.
    t.write(&sgr(0, 40, 5, true));
    t.write(&sgr(32, 35, 5, true));
    t.write(&sgr(32, 30, 5, true));
    t.write(&sgr(0, 30, 5, false));
    // A key that is still bound shows the others were seen and ignored.
    env.ok(&["bind", "-n", "MouseDown2Pane", "display-message", "seen"]);
    click(&mut t, 1, 5, 5);
    t.wait_text("seen");
    assert_eq!(show(&env, "#{pane_index} #{pane_width}"), "1 39");
    // Bound again to tmux's default commands, they work again.
    env.ok(&["bind", "-n", "MouseDown1Pane", "select-pane -t = ; send-keys -M"]);
    env.ok(&["bind", "-n", "MouseDrag1Border", "resize-pane", "-M"]);
    click(&mut t, 0, 5, 5);
    assert!(env.wait(secs(10), || show(&env, "#{pane_index}") == "0"));
    t.write(&sgr(0, 40, 5, true));
    t.write(&sgr(32, 35, 5, true));
    t.write(&sgr(32, 30, 5, true));
    t.write(&sgr(0, 30, 5, false));
    assert!(env.wait(secs(10), || show(&env, "#{pane_width}") == "30"), "{}", env.ok(&["list-panes", "-t", "s:"]));
}

/// tmux's default MouseDown3Pane and MouseDown3Status: menus that are up
/// while the button is held (no `-O`).
#[test]
fn right_button_menus() {
    let env = Env::new();
    let mut t = attach(&env);
    t.write(&sgr(2, 40, 4, true));
    t.wait_text("Horizontal Split");
    let l = t.lines();
    // tmux's -x M -y M: centred on the mouse, the top border on its line.
    let top = l.iter().position(|l| l.contains('┌')).unwrap();
    assert_eq!((l[top].chars().position(|c| c == '┌'), top), (Some(28), 4), "{l:#?}");
    assert!(l[top].contains("0 (%0)"), "{:?}", l[top]);
    // Letting go beside the items closes it, as in tmux.
    t.write(&sgr(2, 40, 4, false));
    t.wait_for("the menu gone", secs(10), |t| !t.screen().contains("Horizontal Split"));
    assert_eq!(show(&env, "#{window_panes}"), "1");
    // Moving onto an item selects it; letting go there chooses it.
    t.write(&sgr(2, 40, 4, true));
    t.wait_text("Horizontal Split");
    let row = t.lines().iter().position(|l| l.contains("Horizontal Split")).unwrap() as u16;
    t.write(&sgr(34, 42, row, true));
    t.write(&sgr(2, 42, row, false));
    assert!(env.wait(secs(10), || show(&env, "#{window_panes}") == "2"));
    t.wait_for("the menu gone", secs(10), |t| !t.screen().contains("Vertical Split"));
    // On a window in the status line: the window menu, above the line and
    // starting where the window's entry does (-x W -y W). Its keys work
    // while the button is down.
    let col = window_list(&t).find("0:").unwrap() as u16;
    t.write(&sgr(2, col + 2, 23, true));
    t.wait_text("New After");
    let l = t.lines();
    assert!(l[22].contains('└') && l[22].chars().position(|c| c == '└') == Some(col as usize), "{l:#?}");
    t.write(b"w");
    assert!(env.wait(secs(10), || show(&env, "#{session_windows}") == "2"));
    t.write(&sgr(2, col + 2, 23, false));
    // On status-left: the session menu.
    t.wait_for("the menu gone", secs(10), |t| !t.screen().contains("New After"));
    t.write(&sgr(2, 1, 23, true));
    t.wait_text("Renumber");
    assert!(t.screen().contains("New Session") && t.lines().iter().any(|l| l.starts_with("┌") && l.contains('s')), "{}", t.screen());
    t.write(&sgr(2, 1, 23, false));
    t.wait_for("the menu gone", secs(10), |t| !t.screen().contains("Renumber"));
    // -O keeps a menu opened with the mouse up after the button is let go;
    // a click then chooses.
    env.ok(&["bind", "-n", "MouseDown3Pane", "display-menu", "-O", "-t", "=", "-x", "M", "-y", "M", "Name it", "n", "rename-window picked"]);
    click(&mut t, 2, 20, 4);
    t.wait_text("Name it");
    t.settle(std::time::Duration::from_millis(100));
    assert!(t.screen().contains("Name it"));
    let row = t.lines().iter().position(|l| l.contains("Name it")).unwrap() as u16;
    click(&mut t, 0, 20, row);
    assert!(env.wait(secs(10), || show(&env, "#{window_name}") == "picked"));
}

#[test]
fn status_line_mouse_keys() {
    let env = Env::new();
    let mut t = attach(&env);
    env.ok(&["new-window", "-t", "s:"]);
    env.ok(&["new-window", "-t", "s:"]);
    t.wait_for("three windows", secs(10), |t| window_list(t).contains("2:"));
    // The wheel over the status line: previous and next window.
    let col = window_list(&t).find("1:").unwrap() as u16;
    click(&mut t, 64, col, 23);
    assert!(env.wait(secs(10), || show(&env, "#{window_index}") == "1"));
    click(&mut t, 65, col, 23);
    click(&mut t, 65, col, 23);
    assert!(env.wait(secs(10), || show(&env, "#{window_index}") == "0"));
    // The parts of the line are places of their own.
    env.ok(&["bind", "-n", "MouseDown1StatusLeft", "display-message", "left part"]);
    env.ok(&["bind", "-n", "MouseDown1StatusRight", "display-message", "right part"]);
    env.ok(&["bind", "-n", "MouseDown1StatusDefault", "display-message", "no part"]);
    env.ok(&["bind", "-n", "MouseDown1Status", "display-message", "window #{window_index}"]);
    click(&mut t, 0, 1, 23);
    t.wait_text("left part");
    click(&mut t, 0, 78, 23);
    t.wait_text("right part");
    click(&mut t, 0, 40, 23);
    t.wait_text("no part");
    // The window under the mouse is the target, not the current one.
    env.ok(&["bind", "-n", "MouseDown1Status", "display-message", "-t", "=", "window #{window_index}"]);
    env.ok(&["set", "-g", "display-time", "200"]);
    t.wait_for("the status line back", secs(10), |t| window_list(t).contains("2:"));
    let col = window_list(&t).find("2:").unwrap() as u16;
    click(&mut t, 0, col, 23);
    t.wait_text("window 2");
    assert_eq!(show(&env, "#{window_index}"), "0");
}

#[test]
fn a_program_using_the_mouse_gets_it_unless_the_key_is_rebound() {
    let env = Env::new();
    let mut t = attach(&env);
    t.type_str("printf '\\033[?1000h\\033[?1006h'; cat -v\r");
    assert!(env.wait(secs(10), || show(&env, "#{mouse_any_flag}") == "1"));
    click(&mut t, 0, 3, 10);
    t.wait_text("^[[<0;4;11M^[[<0;4;11m");
    click(&mut t, 2, 5, 10);
    t.wait_text("^[[<2;6;11M");
    assert!(!t.screen().contains("Horizontal Split"), "the program got the right click, no menu");
    env.ok(&["bind", "-n", "MouseDown3Pane", "display-message", "mine"]);
    click(&mut t, 2, 7, 10);
    t.wait_text("mine");
    assert!(!t.screen().contains("^[[<2;8;11M"));
    t.write(b"\x03");
}

#[test]
fn rebound_drag_and_middle_click() {
    let env = Env::new();
    let mut t = attach(&env);
    t.type_str("echo drag''me please\r");
    t.wait_text("dragme please");
    let y = t.lines().iter().position(|l| l.starts_with("dragme please")).unwrap() as u16;
    // tmux's own commands for the drag, bound by hand: as the default.
    env.ok(&["bind", "-n", "MouseDrag1Pane", "if-shell -F '#{||:#{pane_in_mode},#{mouse_any_flag}}' { send-keys -M } { copy-mode -M }"]);
    t.write(&sgr(0, 0, y, true));
    t.write(&sgr(32, 2, y, true));
    t.write(&sgr(32, 6, y, true));
    t.write(&sgr(0, 6, y, false));
    t.wait_for("selection copied", secs(10), |_| env.ok(&["show-buffer"]) == "dragme\n");
    assert!(env.wait(secs(10), || show(&env, "#{pane_in_mode}") == "0"));
    // A drag bound to something else, and its end.
    env.ok(&["bind", "-n", "MouseDrag1Pane", "display-message", "drag from #{mouse_x}"]);
    env.ok(&["bind", "-n", "MouseDragEnd1Pane", "display-message", "dropped"]);
    t.write(&sgr(0, 4, y, true));
    t.write(&sgr(32, 9, y, true));
    t.wait_text("drag from 4");
    t.write(&sgr(32, 12, y, true));
    t.write(&sgr(0, 12, y, false));
    t.wait_text("dropped");
    assert_eq!(show(&env, "#{pane_in_mode}"), "0");
    // The middle button pastes (tmux's default MouseDown2Pane).
    env.ok(&["set-buffer", "echo pas''ted"]);
    click(&mut t, 1, 3, 3);
    t.wait_text("echo pas''ted");
    t.write(b"\r");
    t.wait_text("pasted");
}

/// The keys around a press: MouseUp, MouseDrag and MouseDragEnd for every
/// button, SecondClick, and the wheel going down.
#[test]
fn up_drag_second_click_and_wheel_down_keys() {
    let env = Env::new();
    let mut t = attach(&env);
    env.ok(&["set", "-g", "display-time", "300"]);
    let gone = |t: &mut Term, what: &str| t.wait_for("the message gone", secs(10), |t| !t.screen().contains(what));
    // A click that is not a drag ends with MouseUp, also with the default
    // (built-in) drag binding.
    env.ok(&["bind", "-n", "MouseUp1Pane", "display-message", "up1 #{mouse_x}"]);
    click(&mut t, 0, 6, 5);
    t.wait_text("up1 6");
    gone(&mut t, "up1 6");
    // The middle button: down, drag, drag end; no MouseUp after a drag.
    env.ok(&["bind", "-n", "MouseDown2Pane", "display-message", "down2"]);
    env.ok(&["bind", "-n", "MouseDrag2Pane", "display-message", "drag2 from #{mouse_x}"]);
    env.ok(&["bind", "-n", "MouseDragEnd2Pane", "display-message", "end2"]);
    env.ok(&["bind", "-n", "MouseUp2Pane", "display-message", "up2"]);
    t.write(&sgr(1, 3, 5, true));
    t.wait_text("down2");
    t.write(&sgr(33, 8, 5, true));
    t.wait_text("drag2 from 3");
    t.write(&sgr(33, 12, 5, true));
    t.write(&sgr(1, 12, 5, false));
    t.wait_text("end2");
    gone(&mut t, "end2");
    click(&mut t, 1, 30, 9);
    t.wait_text("up2");
    gone(&mut t, "up2");
    // On the status line too.
    env.ok(&["bind", "-n", "MouseUp1StatusRight", "display-message", "up on the right"]);
    click(&mut t, 0, 78, 23);
    t.wait_text("up on the right");
    gone(&mut t, "up on the right");
    // SecondClick is the second press itself; the double click follows.
    t.type_str("echo one tw''o\r");
    t.wait_text("one two");
    let y = t.lines().iter().position(|l| l.starts_with("one two")).unwrap() as u16;
    env.ok(&["unbind", "-n", "MouseUp1Pane"]);
    env.ok(&["bind", "-n", "SecondClick1Pane", "set-buffer", "second"]);
    click(&mut t, 0, 5, y);
    click(&mut t, 0, 5, y);
    t.wait_for("the word copied after the second click's key", secs(10), |_| env.ok(&["list-buffers", "-F", "#{buffer_sample}"]) == "two\nsecond\n");
    // A double click nothing is bound to is two presses.
    env.ok(&["set-buffer", "-b", "acc", ""]);
    env.ok(&["bind", "-n", "MouseDown2Pane", "set-buffer", "-a", "-b", "acc", "x"]);
    click(&mut t, 1, 9, 9);
    click(&mut t, 1, 9, 9);
    click(&mut t, 1, 9, 9);
    t.wait_for("three presses", secs(10), |_| env.ok(&["show-buffer", "-b", "acc"]) == "xxx\n");
    // WheelDownPane has no default; bound, it runs.
    env.ok(&["bind", "-n", "WheelDownPane", "display-message", "wheel down"]);
    click(&mut t, 65, 4, 4);
    t.wait_text("wheel down");
}
