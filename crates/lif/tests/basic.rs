//! Attached-client scenarios: what the user sees.
mod common;

use std::time::Duration;

use common::*;
use lif_types::Size;
use lif_vt::VtEngine;

fn attach_new(env: &Env, name: &str, size: Size) -> Term {
    let mut t = Term::spawn(env, &["new-session", "-s", name], size);
    t.wait_text("$");
    t
}

#[test]
fn new_session_prompt_typing_and_detach() {
    let env = Env::new();
    let mut t = attach_new(&env, "main", Size::new(80, 24));
    t.type_str("echo hel''lo\r");
    t.wait_text("hello");
    let lines = t.lines();
    assert!(lines[23].starts_with("[main] 0:"), "status line: {:?}", lines[23]);
    assert!(t.modes().alt_screen, "client should be on the alternate screen");
    t.write(b"\x02d");
    assert_eq!(t.wait_exit(), 0);
    let m = t.modes();
    assert!(!m.alt_screen, "alt screen left after detach");
    assert!(!m.mouse_tracking, "mouse reporting off after detach");
    assert!(!m.bracketed_paste, "bracketed paste off after detach");
    assert!(!m.focus_events, "focus reporting off after detach");
    assert_eq!(m.kitty_keyboard, 0, "kitty keyboard flags popped after detach");
    assert!(t.screen().contains("[detached (from session main)]"), "{}", t.screen());
    // The session lives on.
    assert_eq!(env.ok(&["has-session", "-t", "main"]), "");
}

#[test]
fn split_both_directions_and_select_pane() {
    let env = Env::new();
    let mut t = attach_new(&env, "s", Size::new(80, 24));
    t.write(b"\x02%");
    t.wait_for("vertical border", secs(10), |t| t.screen().contains('│'));
    t.wait_for("cursor in right pane", secs(10), |t| t.cursor().is_some_and(|(x, _)| x > 40));
    t.type_str("echo right''pane\r");
    t.wait_text("rightpane");
    // Prefix + Left moves to the left pane.
    t.write(b"\x02\x1b[D");
    t.wait_for("cursor in left pane", secs(10), |t| t.cursor().is_some_and(|(x, _)| x < 40));
    t.type_str("echo left''pane\r");
    t.wait_text("leftpane");
    let lines = t.lines();
    let row = lines.iter().find(|l| l.contains("leftpane")).unwrap();
    assert!(row.find("leftpane").unwrap() < 40, "{row:?}");
    // Prefix + " splits the left pane top/bottom.
    t.write(b"\x02\"");
    t.wait_for("horizontal border", secs(10), |t| t.screen().contains('─'));
    t.type_str("echo bottom''pane\r");
    t.wait_text("bottompane");
    let panes = env.ok(&["list-panes", "-t", "s"]);
    assert_eq!(panes.lines().count(), 3, "{panes}");
    // Up goes back to the top-left pane.
    t.write(b"\x02\x1b[A");
    t.wait_for("cursor in top-left pane", secs(10), |t| t.cursor().is_some_and(|(x, y)| x < 40 && y < 11));
}

#[test]
fn windows_status_line_rename_and_zoom() {
    let env = Env::new();
    let mut t = attach_new(&env, "win", Size::new(80, 24));
    t.write(b"\x02c");
    t.wait_for("second window", secs(10), |t| t.lines()[23].contains(" 1:"));
    assert!(t.lines()[23].contains('*'));
    // Rename through the prompt (prefix ,), which offers the current name.
    t.write(b"\x02,");
    t.wait_text("(rename-window)");
    t.write(b"\x15beta\r");
    t.wait_for("renamed window", secs(10), |t| t.lines()[23].contains("1:beta*"));
    t.write(b"\x02p");
    t.wait_for("previous window current", secs(10), |t| t.lines()[23].contains("1:beta-"));
    t.write(b"\x02n");
    t.wait_for("next window current", secs(10), |t| t.lines()[23].contains("1:beta*"));
    assert_eq!(env.ok(&["display-message", "-p", "-t", "win", "#{window_index}:#{window_name}"]), "1:beta\n");
    // Zoom: split, then prefix z hides the border and flags the window.
    t.write(b"\x02%");
    t.wait_for("split", secs(10), |t| t.screen().contains('│'));
    t.write(b"\x02z");
    t.wait_for("zoomed", secs(10), |t| !t.screen().contains('│') && t.lines()[23].contains("1:beta*Z"));
    t.write(b"\x02z");
    t.wait_for("unzoomed", secs(10), |t| t.screen().contains('│'));
}

// A program sets its pane's title (OSC 2); the prompts that offer a name for
// editing must put it back as one argument, never as more commands.
#[test]
fn pane_title_in_the_title_prompt_is_not_a_command() {
    let env = Env::new();
    let mut t = attach_new(&env, "ti", Size::new(80, 24));
    let title = "a\"; set -g @owned \"yes";
    t.type_str("printf '\\033]2;a\"; set -g @owned \"yes\\007'\r");
    let shown = |env: &Env, f: &str| env.ok(&["display-message", "-p", "-t", "ti:", f]);
    assert!(env.wait(secs(10), || shown(&env, "#{pane_title}").trim_end() == title), "{}", shown(&env, "#{pane_title}"));
    t.write(b"\x02T");
    t.wait_text("(select-pane)");
    t.write(b"\r");
    t.wait_for("prompt closed", secs(10), |t| t.lines()[23].starts_with("[ti]"));
    assert_eq!(shown(&env, "#{@owned}|#{pane_title}"), format!("|{title}\n"));
}

#[test]
fn command_prompt_runs_commands() {
    let env = Env::new();
    let mut t = attach_new(&env, "p", Size::new(80, 24));
    t.write(b"\x02:");
    t.wait_for("prompt", secs(10), |t| t.lines()[23].starts_with(':'));
    t.type_str("rename-sesion x");
    // Left, insert the missing letter, then Enter.
    t.write(b"\x1b[D\x1b[D\x1b[D\x1b[D\x1b[Ds\r");
    t.wait_for("session renamed", secs(10), |t| t.lines()[23].starts_with("[x]"));
    // An error is shown as a message.
    t.write(b"\x02:");
    t.wait_for("prompt", secs(10), |t| t.lines()[23].starts_with(':'));
    t.type_str("nosuchcommand\r");
    t.wait_text("unknown command: nosuchcommand");
}

#[test]
fn client_resize_resizes_panes() {
    let env = Env::new();
    let mut t = attach_new(&env, "r", Size::new(80, 24));
    t.type_str("stty size\r");
    t.wait_text("23 80");
    t.resize(Size::new(100, 30));
    t.type_str("clear; stty size\r");
    t.wait_text("29 100");
    assert_eq!(env.ok(&["display-message", "-p", "-t", "r", "#{pane_width}x#{pane_height}"]), "100x29\n");
}

#[test]
fn reattach_and_two_clients() {
    let env = Env::new();
    let mut a = attach_new(&env, "two", Size::new(80, 24));
    a.type_str("echo mark''er1\r");
    a.wait_text("marker1");
    a.write(b"\x02d");
    assert_eq!(a.wait_exit(), 0);
    let mut b = Term::spawn(&env, &["attach", "-t", "two"], Size::new(80, 24));
    b.wait_text("marker1");
    let mut c = Term::spawn(&env, &["attach", "-t", "two"], Size::new(80, 24));
    c.wait_text("marker1");
    c.type_str("echo from''c\r");
    c.wait_text("fromc");
    b.wait_text("fromc");
    b.type_str("echo from''b\r");
    b.wait_text("fromb");
    c.wait_text("fromb");
    let clients = env.ok(&["list-clients"]);
    assert_eq!(clients.lines().count(), 2, "{clients}");
}

#[test]
fn alternate_screen_program_restores() {
    let env = Env::new();
    let mut t = attach_new(&env, "alt", Size::new(80, 24));
    t.type_str("echo before''alt\r");
    t.wait_text("beforealt");
    if std::path::Path::new("/usr/bin/less").exists() {
        std::fs::write(env.home.join("f.txt"), "LESS CONTENT LINE\n".repeat(5)).unwrap();
        t.type_str("less f.txt\r");
        t.wait_text("LESS CONTENT LINE");
        assert!(!t.screen().contains("beforealt"), "less runs on its own screen");
        t.type_str("q");
    } else {
        t.type_str("printf '\\033[?1049hALTSCREEN'; sleep 1; printf '\\033[?1049l'\r");
        t.wait_text("ALTSCREEN");
    }
    t.wait_for("screen restored", secs(10), |t| {
        let s = t.screen();
        s.contains("beforealt") && !s.contains("LESS CONTENT LINE") && !s.contains("ALTSCREEN")
    });
}

#[test]
fn utf8_wide_and_emoji() {
    let env = Env::new();
    let text = "héllo 世界|x 👍 ok\n";
    std::fs::write(env.home.join("u.txt"), text).unwrap();
    let mut t = attach_new(&env, "utf", Size::new(80, 24));
    t.type_str("cat u.txt\r");
    t.wait_text("héllo 世界|x 👍 ok");
    // The wide characters take two cells each: '|' is in column 10.
    t.vt.refresh(&mut t.frame);
    let row = t.frame.rows.iter().find(|r| r.cells.iter().any(|c| c.ch == '世')).expect("row");
    let bar = row.cells.iter().position(|c| c.ch == '|').unwrap();
    assert_eq!(bar, 10);
    let pane = env.ok(&["capture-pane", "-p", "-t", "utf"]);
    assert!(pane.contains("héllo 世界|x 👍 ok"), "{pane}");
}

#[test]
fn nested_attach_is_refused() {
    let env = Env::new();
    env.ok(&["new-session", "-d", "-s", "n"]);
    let o = env.command().env("LIF", "/x/y,1,0").args(["attach", "-t", "n"]).output().unwrap();
    assert_eq!(o.status.code(), Some(1));
    assert_eq!(String::from_utf8_lossy(&o.stderr), "sessions should be nested with care, unset $LIF to force\n");
    let o = env.command().env("LIF", "/x/y,1,0").args(["new-session"]).output().unwrap();
    assert_eq!(o.status.code(), Some(1));
    // Detached sessions are fine from inside.
    let o = env.command().env("LIF", "/x/y,1,0").args(["new-session", "-d", "-s", "inner"]).output().unwrap();
    assert!(o.status.success());
}

#[test]
fn last_pane_exit_ends_everything() {
    let env = Env::new();
    let mut t = attach_new(&env, "end", Size::new(80, 24));
    t.type_str("exit\r");
    assert_eq!(t.wait_exit(), 0);
    assert!(t.screen().contains("[exited]"), "{}", t.screen());
    assert!(!t.modes().alt_screen);
    assert!(env.wait(Duration::from_secs(5), || !path_exists(&env.sock)), "socket removed");
    // The lock file stays (removing it would let two clients starting a
    // server each lock a different file), but nobody holds its lock.
    // (The socket goes first; the lock goes with the process.)
    let lock = std::fs::File::open(env.sock.with_extension("lock")).expect("lock file");
    let free = env.wait(Duration::from_secs(5), || {
        // SAFETY: flock on a file we opened; closing it releases the lock.
        unsafe { libc::flock(std::os::fd::AsRawFd::as_raw_fd(&lock), libc::LOCK_EX | libc::LOCK_NB) == 0 }
    });
    assert!(free, "the exited server still holds the lock");
    let o = env.run(&["ls"]);
    assert_eq!(o.status.code(), Some(1));
    assert_eq!(String::from_utf8_lossy(&o.stderr), format!("no server running on {}\n", env.sock.display()));
}

#[test]
fn mouse_click_selects_pane() {
    let env = Env::new();
    let mut t = attach_new(&env, "m", Size::new(80, 24));
    t.write(b"\x02%");
    t.wait_for("split", secs(10), |t| t.cursor().is_some_and(|(x, _)| x > 40));
    t.wait_for("mouse reporting on", secs(10), |t| t.modes().mouse_tracking);
    // SGR click at column 5, row 5 (1-based) in the left pane.
    t.write(b"\x1b[<0;5;5M\x1b[<0;5;5m");
    t.wait_for("left pane active", secs(10), |t| t.cursor().is_some_and(|(x, _)| x < 40));
    // Drag the border (column 40) to column 30.
    t.write(b"\x1b[<0;41;5M\x1b[<32;36;5M\x1b[<32;31;5M\x1b[<0;31;5m");
    assert!(
        env.wait(secs(10), || env.ok(&["list-panes", "-t", "m", "-F", "#{pane_width}"]).starts_with("30\n")),
        "{}",
        env.ok(&["list-panes", "-t", "m"])
    );
    // Click a window in the status line.
    t.write(b"\x02c");
    t.wait_for("second window", secs(10), |t| t.lines()[23].contains(" 1:"));
    let status = t.lines()[23].clone();
    let col = status.find(" 0:").unwrap() + 1;
    t.write(format!("\x1b[<0;{};24M\x1b[<0;{};24m", col + 1, col + 1).as_bytes());
    t.wait_for("window 0 selected", secs(10), |t| {
        let s = t.lines()[23].clone();
        s[s.find(" 0:").unwrap()..].split(' ').nth(1).is_some_and(|w| w.ends_with('*'))
    });
}
