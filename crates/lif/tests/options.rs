//! Options whose effect needs the real server: timers, clients coming and
//! going, events from the outer terminal.
mod common;

use std::time::{Duration, Instant};

use common::*;
use lif_types::Size;

fn attach(env: &Env, name: &str) -> Term {
    let mut t = Term::spawn(env, &["new-session", "-s", name], Size::new(80, 24));
    t.wait_text("$");
    t
}

fn show(env: &Env, target: &str, fmt: &str) -> String {
    env.ok(&["display-message", "-p", "-t", target, fmt]).trim_end().to_string()
}

/// `monitor-silence`: a window that prints nothing for that long is
/// flagged, once, and output starts the count again.
#[test]
fn monitor_silence_flags_a_quiet_window() {
    let env = Env::new();
    let mut t = attach(&env, "s");
    env.ok(&["set", "-g", "visual-silence", "on"]);
    env.ok(&["set", "-g", "display-time", "3000"]);
    // A window that prints once after half a second and is then quiet.
    env.ok(&["new-window", "-d", "-t", "s:", "sleep 0.5; echo printed; sleep 30"]);
    env.ok(&["setw", "-t", "s:1", "monitor-silence", "1"]);
    let start = Instant::now();
    assert_eq!(show(&env, "s:1", "#{window_silence_flag}"), "0");
    t.wait_text("Silence in window 1");
    let took = start.elapsed();
    assert!(took >= Duration::from_millis(1400), "flagged {took:?} after the option was set: the output at 0.5 s should have started the count again");
    assert_eq!(show(&env, "s:1", "#{window_silence_flag} #{window_flags}"), "1 ~");
    assert!(t.lines()[23].contains("Silence in window 1") || t.screen().contains("1:sleep~"), "{}", t.screen());
    // The current window is not flagged however quiet it is.
    env.ok(&["setw", "-t", "s:0", "monitor-silence", "1"]);
    std::thread::sleep(Duration::from_millis(1300));
    assert_eq!(show(&env, "s:0", "#{window_silence_flag}"), "0");
    // Selecting the window clears the flag.
    env.ok(&["select-window", "-t", "s:1"]);
    assert_eq!(show(&env, "s:1", "#{window_silence_flag}"), "0");
    // Off again: no timer is left (the server is idle).
    env.ok(&["setw", "-t", "s:1", "monitor-silence", "0"]);
    env.ok(&["setw", "-t", "s:0", "monitor-silence", "0"]);
    env.ok(&["select-window", "-t", "s:0"]);
    std::thread::sleep(Duration::from_millis(1300));
    assert_eq!(show(&env, "s:1", "#{window_silence_flag}"), "0");
}

/// `destroy-unattached`: the session goes when its last client leaves.
#[test]
fn destroy_unattached_follows_the_last_client() {
    let env = Env::new();
    env.ok(&["new-session", "-d", "-s", "stay"]);
    let mut t = attach(&env, "temp");
    env.ok(&["set", "-t", "temp", "destroy-unattached", "on"]);
    assert_eq!(env.ok(&["list-sessions", "-F", "#{session_name}"]), "stay\ntemp\n");
    t.write(b"\x02d");
    assert_eq!(t.wait_exit(), 0);
    assert!(env.wait(secs(10), || env.ok(&["list-sessions", "-F", "#{session_name}"]) == "stay\n"), "{}", env.ok(&["list-sessions"]));
    // A client that just disappears counts as leaving.
    let t = attach(&env, "temp2");
    env.ok(&["set", "-t", "temp2", "destroy-unattached", "on"]);
    drop(t);
    assert!(env.wait(secs(10), || env.ok(&["list-sessions", "-F", "#{session_name}"]) == "stay\n"), "{}", env.ok(&["list-sessions"]));
}

/// `focus-events`: on by default a program that asks for focus reports
/// gets the terminal's; off, it does not.
#[test]
fn focus_events_can_be_turned_off() {
    let env = Env::new();
    let mut t = attach(&env, "f");
    t.type_str("printf '\\033[?1004h'; cat -v\r");
    t.wait_for("focus reporting asked of the outer terminal", secs(10), |t| t.modes().focus_events);
    t.settle(Duration::from_millis(150));
    t.write(b"\x1b[O");
    t.wait_text("^[[O");
    env.ok(&["set", "-s", "focus-events", "off"]);
    t.write(b"\x1b[I");
    t.type_str("mark\r");
    t.wait_text("mark");
    assert!(!t.screen().contains("^[[I"), "{}", t.screen());
    env.ok(&["set", "-s", "focus-events", "on"]);
    t.write(b"\x1b[I");
    t.wait_text("^[[I");
    t.write(b"\x03");
}
