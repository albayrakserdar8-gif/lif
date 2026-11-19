//! Keystroke echo: never paced, and not relayed through the client.
mod common;

use std::time::{Duration, Instant};

use common::*;
use lif_types::Size;

/// Types `c` and returns how long until the client's terminal received it.
fn echo_time(t: &mut Term, c: u8) -> Duration {
    let from = t.raw.len();
    let t0 = Instant::now();
    t.write(&[c]);
    let end = t0 + Duration::from_secs(5);
    loop {
        if t.raw[from..].contains(&c) {
            return t0.elapsed();
        }
        assert!(Instant::now() < end, "no echo of {:?}", c as char);
        t.pump(Duration::from_millis(1));
    }
}

fn median(mut v: Vec<Duration>) -> Duration {
    v.sort();
    v[v.len() / 2]
}

/// The server reads keys from and writes frames to the client's terminal
/// itself: with the client process stopped, typing still works.
#[test]
fn server_uses_the_terminal_directly() {
    let env = Env::new();
    let mut t = Term::spawn(&env, &["new-session", "-s", "direct"], Size::new(80, 24));
    t.wait_text("$");
    t.settle(Duration::from_millis(100));
    let pid = t.pty.pid() as i32;
    // SAFETY: signalling our own child.
    unsafe { libc::kill(pid, libc::SIGSTOP) };
    t.type_str("echo DIRECT''OK\r");
    t.wait_text("DIRECTOK");
    // SAFETY: as above.
    unsafe { libc::kill(pid, libc::SIGCONT) };
    t.write(b"\x02d");
    let code = t.wait_exit();
    assert_eq!(code, 0, "client exit status; screen:\n{}", t.screen());
    let m = t.modes();
    assert!(!m.alt_screen && !m.bracketed_paste && !m.mouse_tracking, "terminal not restored: {m:?}");
    assert!(t.screen().contains("[detached"), "{}", t.screen());
}

/// Frame pacing coalesces output nobody typed for; it must never hold back
/// an echo, also not while another pane floods the same client.
#[test]
fn echo_is_not_paced_next_to_a_flood() {
    let env = Env::new();
    env.ok(&["new-session", "-d", "-s", "echo", "-x", "120", "-y", "40"]);
    env.wait_capture("echo", "$");
    env.ok(&["send-keys", "-t", "echo", "exec cat", "Enter"]);
    env.ok(&["split-window", "-d", "-t", "echo", "yes"]);
    let mut t = Term::spawn(&env, &["attach", "-t", "echo"], Size::new(120, 41));
    t.wait_text("[echo]");
    t.settle(Duration::from_millis(200));
    let mut lat = Vec::new();
    for i in 0..40u8 {
        // Typed right after the previous echo's frame: inside the pacing
        // interval, where a paced echo would wait.
        lat.push(echo_time(&mut t, b'a' + i % 20));
        t.write(b"\x7f");
        std::thread::sleep(Duration::from_millis(2));
        t.pump(Duration::ZERO);
    }
    let med = median(lat.clone());
    eprintln!("echo next to a flood: median {med:?}, max {:?}", lat.iter().max().unwrap());
    assert!(med < Duration::from_millis(3), "echo median {med:?}: held back by frame pacing");
}

/// Median time from typing `key` until the pane's terminal echoes Escape
/// (`^[`, from `cat` under the tty's echo).
fn escape_echo(t: &mut Term, key: &[u8]) -> Duration {
    let mut lat = Vec::new();
    for _ in 0..15 {
        let t0 = Instant::now();
        t.write(key);
        let end = t0 + Duration::from_secs(5);
        while !t.screen().contains("^[") {
            assert!(Instant::now() < end, "Escape never reached the pane; screen:\n{}", t.screen());
            t.pump(Duration::from_millis(1));
        }
        lat.push(t0.elapsed());
        // Kill the line (the tty erases the echo too).
        t.write(b"\x15");
        t.wait_for("the line erased", secs(5), |t| !t.screen().contains("^["));
        t.settle(Duration::from_millis(10));
    }
    median(lat)
}

fn escape_session(env: &Env, answer: bool) -> Term {
    let mut t = Term::spawn(env, &["new-session", "-s", "esc"], Size::new(80, 24));
    t.answer = answer;
    t.wait_text("$");
    // echoctl: the tty echoes a control character as ^X, at once.
    t.type_str("stty echoctl; exec cat\r");
    t.settle(Duration::from_millis(200));
    t
}

/// A legacy terminal's lone Escape waits `escape-time` (5 ms by default)
/// for the rest of a sequence, and not at all with `escape-time 0`.
#[test]
fn escape_time_is_honored_on_legacy_terminals() {
    let env = Env::new();
    let mut t = escape_session(&env, false);
    assert_eq!(t.modes().kitty_keyboard, 0);
    let waited = escape_echo(&mut t, b"\x1b");
    env.ok(&["set-option", "-s", "escape-time", "0"]);
    let at_once = escape_echo(&mut t, b"\x1b");
    eprintln!("legacy Escape: escape-time 5: {waited:?}, escape-time 0: {at_once:?}");
    assert!(waited >= Duration::from_millis(4), "escape-time 5 waited only {waited:?}");
    assert!(at_once < Duration::from_millis(3), "escape-time 0 still waited {at_once:?}");
}

/// With the kitty keyboard protocol the Escape key is unambiguous
/// (`CSI 27 u`): no wait, whatever escape-time says.
#[test]
fn escape_never_waits_with_the_kitty_protocol() {
    let env = Env::new();
    let mut t = escape_session(&env, true);
    t.wait_for("kitty flags pushed", secs(10), |t| t.modes().kitty_keyboard != 0);
    env.ok(&["set-option", "-s", "escape-time", "500"]);
    let lat = escape_echo(&mut t, b"\x1b[27u");
    eprintln!("kitty Escape: {lat:?}");
    assert!(lat < Duration::from_millis(3), "Escape waited {lat:?}");
}
