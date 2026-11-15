//! The chooser modes as a user meets them: the tree behind `C-b w` and
//! `C-b s`, buffers behind `C-b =`, clients behind `C-b D`, and
//! `find-window` behind `C-b f`. The screens are tmux 3.8's.
mod common;

use common::*;
use lif_types::Size;

fn attach(env: &Env) -> Term {
    env.ok(&["new-session", "-d", "-s", "alpha", "-x", "80", "-y", "24"]);
    env.ok(&["new-window", "-d", "-t", "alpha", "-n", "second"]);
    env.ok(&["new-session", "-d", "-s", "beta", "-x", "80", "-y", "24"]);
    let mut t = Term::spawn(env, &["attach", "-t", "alpha"], Size::new(80, 24));
    t.wait_text("$");
    t
}

fn show(env: &Env, fmt: &str) -> String {
    env.ok(&["display-message", "-p", "-t", "alpha", fmt]).trim_end().to_string()
}

#[test]
fn window_tree_navigates_and_switches() {
    let env = Env::new();
    let mut t = attach(&env);
    t.write(b"\x02w");
    t.wait_text("(sort: index) (view: preview)");
    let l = t.lines();
    assert_eq!(l[0], "(0) - alpha: 2 windows (attached)");
    assert!(l[1].starts_with("(1) ├─→ + 0: [lif]*"), "{:?}", l[1]);
    assert!(l[2].starts_with("(2) └─→ + 1: second"), "{:?}", l[2]);
    assert_eq!(l[3], "(3) - beta: 1 windows");
    assert!(l[11].starts_with("┌ 0 (sort: index) (view: preview) ─") && l[11].ends_with('┐'), "{:?}", l[11]);
    assert!(l[22].starts_with('└') && l[22].ends_with('┘'), "{:?}", l[22]);
    assert_eq!(show(&env, "#{pane_in_mode}"), "1");
    // Down to window 1; its preview has the window's pane in a label box.
    t.write(b"\x1b[B");
    t.wait_for("window 1 selected", secs(10), |t| t.lines()[11].starts_with("┌ 1 (sort"));
    t.write(b"\r");
    assert!(env.wait(secs(10), || show(&env, "#{window_index} #{pane_in_mode}") == "1 0"), "{}", show(&env, "#{window_index} #{pane_in_mode}"));
    // The shortcut on a line chooses it at once: a session, from C-b s.
    t.write(b"\x02s");
    t.wait_text("(1) + beta: 1 windows");
    t.write(b"1");
    assert!(env.wait(secs(10), || env.ok(&["list-clients", "-F", "#{client_session}"]) == "beta\n"));
    // q leaves without choosing.
    t.write(b"\x02w");
    t.wait_text("(view: preview)");
    t.write(b"q");
    assert!(env.wait(secs(10), || env.ok(&["display-message", "-p", "-t", "beta", "#{pane_in_mode}"]) == "0\n"));
    assert_eq!(env.ok(&["list-clients", "-F", "#{client_session}"]), "beta\n");
}

#[test]
fn tree_kills_follow_the_server_and_zoom_is_restored() {
    let env = Env::new();
    let mut t = attach(&env);
    env.ok(&["split-window", "-d", "-t", "alpha:0"]);
    t.write(b"\x02w");
    t.wait_text("(view: preview)");
    assert_eq!(show(&env, "#{window_zoomed_flag}"), "1", "-Z zooms the pane the mode is in");
    // x asks on the status line; anything but y keeps the window.
    t.write(b"\x1b[Bx");
    t.wait_text("Kill window 1?");
    t.write(b"n");
    t.wait_for("the prompt gone", secs(10), |t| !t.screen().contains("Kill window"));
    assert_eq!(env.ok(&["list-windows", "-t", "alpha", "-F", "#{window_index}"]), "0\n1\n");
    t.write(b"x");
    t.wait_text("Kill window 1?");
    t.write(b"y");
    assert!(env.wait(secs(10), || env.ok(&["list-windows", "-t", "alpha", "-F", "#{window_index}"]) == "0\n"));
    t.wait_for("the list without it", secs(10), |t| t.lines()[0] == "(0) - alpha: 1 windows (attached)" && !t.screen().contains("second"));
    // Something else changes the server: the list keeps up.
    env.ok(&["new-session", "-d", "-s", "gamma"]);
    t.wait_text("gamma: 1 windows");
    env.ok(&["kill-session", "-t", "beta"]);
    t.wait_for("beta gone", secs(10), |t| !t.screen().contains("beta"));
    // A smaller terminal: drawn again at the new size.
    t.resize(Size::new(60, 20));
    t.wait_for("redrawn at 60 columns", secs(10), |t| t.lines().iter().any(|l| l.starts_with("┌ ") && l.chars().count() == 60));
    t.write(b"q");
    assert!(env.wait(secs(10), || show(&env, "#{pane_in_mode} #{window_zoomed_flag}") == "0 0"), "{}", show(&env, "#{pane_in_mode} #{window_zoomed_flag}"));
}

#[test]
fn buffers_clients_and_find_window() {
    let env = Env::new();
    let mut t = attach(&env);
    env.ok(&["set-buffer", "-b", "named", "echo from''buffer"]);
    env.ok(&["set-buffer", "newest text"]);
    // C-b =: the newest first, its text in the preview; Enter pastes.
    t.write(b"\x02=");
    t.wait_text("(sort: creation)");
    let l = t.lines();
    assert!(l[0].starts_with("(0) buffer0000: ") && l[0].ends_with(": newest text"), "{:?}", l[0]);
    assert!(l[1].starts_with("(1) named: "), "{:?}", l[1]);
    assert!(l[12].contains("newest text"), "the preview: {:?}", l[12]);
    t.write(b"\x1b[B\r");
    t.wait_text("$ echo from''buffer");
    t.write(b"\r");
    t.wait_text("frombuffer");
    // d deletes and the list follows.
    t.write(b"\x02=");
    t.wait_text("(sort: creation)");
    t.write(b"d");
    assert!(env.wait(secs(10), || env.ok(&["list-buffers", "-F", "#{buffer_name}"]) == "named\n"));
    t.wait_for("one buffer listed", secs(10), |t| t.lines()[0].starts_with("(0) named: "));
    t.write(b"q");
    // C-b f asks for text and shows the windows that have it.
    t.write(b"\x02f");
    t.wait_text("(find-window)");
    t.type_str("sec\r");
    t.wait_text("(view: preview)");
    let l = t.lines();
    assert_eq!(l[0], "(0) - alpha: 2 windows (attached)");
    assert!(l[1].starts_with("(1) └─→ - 1: second"), "{:?}", l[1]);
    assert!(!t.screen().contains("beta"), "{}", t.screen());
    t.write(b"\x1b[B\r");
    assert!(env.wait(secs(10), || show(&env, "#{window_index}") == "1"));
    // C-b D lists the clients; Enter detaches the chosen one.
    t.write(b"\x02D");
    t.wait_text("session alpha");
    assert!(t.lines()[0].starts_with("(0) client-"), "{:?}", t.lines()[0]);
    t.write(b"\r");
    assert_eq!(t.wait_exit(), 0);
    assert!(t.screen().contains("[detached (from session alpha)]"), "{}", t.screen());
    // No client: the commands say so, as tmux's do.
    let o = env.run(&["choose-tree", "-t", "alpha"]);
    assert_eq!((o.status.code(), String::from_utf8_lossy(&o.stderr).as_ref()), (Some(1), "no clients\n"));
}

#[test]
fn mouse_selects_and_double_click_chooses() {
    let env = Env::new();
    let mut t = attach(&env);
    t.write(b"\x02w");
    t.wait_text("(view: preview)");
    // SGR mouse: a click on line 2 (window 1) selects it.
    t.write(b"\x1b[<0;5;3M\x1b[<0;5;3m");
    t.wait_for("window 1 selected", secs(10), |t| t.lines()[11].starts_with("┌ 1 (sort"));
    assert_eq!(show(&env, "#{window_index}"), "0");
    t.write(b"\x1b[<0;5;3M\x1b[<0;5;3m");
    assert!(env.wait(secs(10), || show(&env, "#{window_index} #{pane_in_mode}") == "1 0"));
}
