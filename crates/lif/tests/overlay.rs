//! Popups and menus over a client's panes: `display-popup` and
//! `display-menu`. Placement, borders and exit statuses are tmux 3.8's.
mod common;

use std::process::Stdio;
use std::time::Duration;

use common::*;
use lif_types::Size;

fn attach(env: &Env) -> Term {
    env.ok(&["new-session", "-d", "-s", "s", "-x", "80", "-y", "24"]);
    let mut t = Term::spawn(env, &["attach", "-t", "s"], Size::new(80, 24));
    t.wait_text("$");
    t
}

fn show(env: &Env, fmt: &str) -> String {
    env.ok(&["display-message", "-p", "-t", "s", fmt]).trim_end().to_string()
}

#[test]
fn popup_runs_a_program_over_the_panes() {
    let env = Env::new();
    let mut t = attach(&env);
    t.type_str("echo under''neath\r");
    t.wait_text("underneath");
    // A popup with cat: what is typed goes to it, not to the pane.
    let mut waiter = env.command().args(["display-popup", "-E", "-T", " cat ", "cat"]).stdout(Stdio::piped()).spawn().unwrap();
    t.wait_text("┌─ cat ─");
    let l = t.lines();
    // tmux's default: half the screen, one up and left of the middle.
    assert_eq!(l[5].chars().position(|c| c == '┌'), Some(19), "{:?}", l[5]);
    assert_eq!(l[5].chars().filter(|c| "┌─┐".contains(*c)).count() + " cat ".len(), 40, "{:?}", l[5]);
    assert!(l[16].contains("└") && !l[17].contains('└'), "{l:#?}");
    assert!(l[0].contains("underneath") || l[1].contains("underneath"), "the pane shows around the popup");
    t.type_str("in the popup\r");
    t.wait_for("the echo inside the box", secs(10), |t| t.lines().iter().any(|l| l.contains("│in the popup")));
    assert!(!String::from_utf8_lossy(&env.run(&["capture-pane", "-p", "-t", "s"]).stdout).contains("in the popup"));
    assert_eq!(show(&env, "#{window_panes}"), "1", "the popup is not a pane of the window");
    // The command that opened it waits; the program's end (C-d) closes it.
    std::thread::sleep(Duration::from_millis(200));
    assert!(waiter.try_wait().unwrap().is_none(), "display-popup returned while the popup was up");
    t.write(b"\x04");
    assert!(waiter.wait().unwrap().success());
    t.wait_for("the popup gone", secs(10), |t| !t.screen().contains('┌'));
    // What was under it is back, and keys reach the pane again.
    t.type_str("echo ba''ck\r");
    t.wait_text("back");
}

#[test]
fn popup_status_and_closing() {
    let env = Env::new();
    let mut t = attach(&env);
    // -E: closes when the program exits; its status is the command's.
    let o = env.run(&["display-popup", "-E", "exit 3"]);
    assert_eq!(o.status.code(), Some(3));
    assert!(env.run(&["display-popup", "-E", "true"]).status.success());
    // Without -E it stays, showing what the program left, until Escape.
    let mut waiter = env.command().args(["display-popup", "-w", "30", "-h", "8", "-x", "5", "-y", "12", "echo le''ft; exit 4"]).spawn().unwrap();
    t.wait_text("│left");
    let l = t.lines();
    assert_eq!(l[4].chars().position(|c| c == '┌'), Some(5), "-y names the row below the popup: {l:#?}");
    assert!(l[11].starts_with("     └"), "{l:#?}");
    std::thread::sleep(Duration::from_millis(300));
    assert!(waiter.try_wait().unwrap().is_none());
    t.write(b"x");
    std::thread::sleep(Duration::from_millis(200));
    assert!(t.screen().contains("│left"), "another key does not close it");
    t.write(b"\x1b");
    assert_eq!(waiter.wait().unwrap().code(), Some(0), "closed by a key: status 0");
    t.wait_for("the popup gone", secs(10), |t| !t.screen().contains('└'));
    // A second popup is not opened; -C closes the one that is up.
    let mut first = env.command().args(["display-popup", "-E", "sleep 30"]).spawn().unwrap();
    t.wait_text("┌──");
    assert!(env.run(&["display-popup", "-E", "echo se''cond; sleep 5"]).status.success());
    assert!(!t.screen().contains("second"));
    env.ok(&["display-popup", "-C"]);
    assert_eq!(first.wait().unwrap().code(), Some(0));
    t.wait_for("the popup gone", secs(10), |t| !t.screen().contains('┌'));
    // Errors, as tmux words them.
    let o = env.run(&["display-popup", "-w", "0", "true"]);
    assert_eq!(String::from_utf8_lossy(&o.stderr), "width too small\n");
    let o = env.run(&["display-popup", "-h", "500", "true"]);
    assert_eq!(String::from_utf8_lossy(&o.stderr), "height too large\n");
}

#[test]
fn popup_ends_with_its_client() {
    let env = Env::new();
    let mut t = attach(&env);
    let pidfile = env.home.join("popup-pid");
    let mut waiter = env.command().args(["display-popup", "-E", &format!("echo $$ > {}; exec sleep 300", pidfile.display())]).spawn().unwrap();
    t.wait_text("┌──");
    assert!(env.wait(secs(10), || std::fs::read_to_string(&pidfile).is_ok_and(|p| p.ends_with('\n'))));
    let pid: u32 = std::fs::read_to_string(&pidfile).unwrap().trim().parse().unwrap();
    assert!(pid_alive(pid));
    // The client detaches (the popup takes the keys, so by command).
    env.ok(&["detach-client", "-s", "s"]);
    assert_eq!(t.wait_exit(), 0);
    assert!(waiter.wait().unwrap().success(), "the waiting command is released");
    assert!(env.wait(secs(10), || !pid_alive(pid)), "the popup's program outlived its client");
    // A popup still up when the server stops goes with it.
    let mut t = Term::spawn(&env, &["attach", "-t", "s"], Size::new(80, 24));
    t.wait_text("$");
    let _ = std::fs::remove_file(&pidfile);
    let mut waiter = env.command().args(["display-popup", "-E", &format!("echo $$ > {}; exec sleep 300", pidfile.display())]).spawn().unwrap();
    assert!(env.wait(secs(10), || std::fs::read_to_string(&pidfile).is_ok_and(|p| p.ends_with('\n'))));
    let pid: u32 = std::fs::read_to_string(&pidfile).unwrap().trim().parse().unwrap();
    env.ok(&["kill-server"]);
    let _ = waiter.wait();
    assert!(env.wait(secs(10), || !pid_alive(pid)), "the popup's program outlived the server");
}

#[test]
fn menu_chooses_with_keys_and_mouse() {
    let env = Env::new();
    let mut t = attach(&env);
    let menu = ["display-menu", "-T", "Title", "First", "f", "rename-window first", "Second item", "s", "rename-window second", "", "-Disabled", "d", "rename-window no", "Last", "", "rename-window last"];
    env.ok(&menu);
    t.wait_text("┌──────Title──────┐");
    let l = t.lines();
    assert_eq!(&l[7..14], ["                              ┌──────Title──────┐", "                              │ First       (f) │", "                              │ Second item (s) │", "                              ├─────────────────┤", "                              │ Disabled        │", "                              │ Last            │", "                              └─────────────────┘"]);
    // Down twice from the first item: past the separator and the disabled one.
    t.write(b"\x1b[B\x1b[B\r");
    assert!(env.wait(secs(10), || show(&env, "#{window_name}") == "last"));
    t.wait_for("the menu gone", secs(10), |t| !t.screen().contains("Title"));
    // An item's key.
    env.ok(&menu);
    t.wait_text("Title");
    t.write(b"s");
    assert!(env.wait(secs(10), || show(&env, "#{window_name}") == "second"));
    // A click on an item; q and a click outside only close.
    env.ok(&menu);
    t.wait_text("Title");
    t.write(b"\x1b[<0;33;9M\x1b[<0;33;9m");
    assert!(env.wait(secs(10), || show(&env, "#{window_name}") == "first"));
    env.ok(&menu);
    t.wait_text("Title");
    t.write(b"q");
    t.wait_for("the menu gone", secs(10), |t| !t.screen().contains("Title"));
    env.ok(&menu);
    t.wait_text("Title");
    t.write(b"\x1b[<0;2;2M\x1b[<0;2;2m");
    t.wait_for("the menu gone", secs(10), |t| !t.screen().contains("Title"));
    assert_eq!(show(&env, "#{window_name}"), "first");
    // The default pane menu (C-b >): z zooms.
    env.ok(&["split-window", "-d", "-t", "s"]);
    t.write(b"\x02>");
    t.wait_text("Horizontal Split (h)");
    t.write(b"z");
    assert!(env.wait(secs(10), || show(&env, "#{window_zoomed_flag}") == "1"));
    // Keys go to the pane again once the menu is gone.
    t.type_str("echo ty''ped\r");
    t.wait_text("typed");
}
