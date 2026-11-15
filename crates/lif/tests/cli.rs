//! Commands from the command line, server lifetime.
mod common;

use std::time::{Duration, Instant};

use common::*;
use lif_types::Size;

#[test]
fn cli_commands_exact_output() {
    let env = Env::new();
    assert_eq!(env.ok(&["new-session", "-d", "-s", "work", "-n", "win", "-x", "200", "-y", "50"]), "");
    let ls = env.ok(&["list-sessions"]);
    assert!(ls.starts_with("work: 1 windows (created ") && ls.ends_with(")\n") && ls.lines().count() == 1, "{ls:?}");
    let lw = env.ok(&["list-windows", "-t", "work"]);
    assert!(lw.starts_with("0: win* (1 panes) [200x50] [layout ") && lw.ends_with(",200x50,0,0,0] @0 (active)\n"), "{lw:?}");
    let lp = env.ok(&["list-panes", "-t", "work"]);
    assert!(lp.starts_with("0: [200x50] [history 0/4000, ") && lp.ends_with(" bytes] %0 (active)\n"), "{lp:?}");
    assert_eq!(env.ok(&["display-message", "-p", "-t", "work", "#{session_name}:#{window_index}.#{pane_index} #{pane_width}x#{pane_height} #{window_name}"]), "work:0.0 200x50 win\n");
    env.wait_capture("work", "$");
    assert_eq!(env.ok(&["send-keys", "-t", "work", "echo sent''keys", "Enter"]), "");
    let cap = env.wait_capture("work", "sentkeys");
    assert_eq!(cap.lines().count(), 50, "capture-pane prints every row");
    assert!(cap.lines().any(|l| l == "sentkeys"), "{cap}");
    assert_eq!(env.ok(&["has-session", "-t", "work"]), "");
    let o = env.run(&["has-session", "-t", "nope"]);
    assert_eq!(o.status.code(), Some(1));
    assert_eq!(String::from_utf8_lossy(&o.stderr), "can't find session: nope\n");
    let o = env.run(&["frobnicate"]);
    assert_eq!(o.status.code(), Some(1));
    assert_eq!(String::from_utf8_lossy(&o.stderr), "unknown command: frobnicate\n");
    // The benchmark harness's split.
    assert_eq!(env.ok(&["split-window", "-t", "work"]), "");
    assert_eq!(env.ok(&["list-panes", "-t", "work"]).lines().count(), 2);
    env.ok(&["new-session", "-d", "-s", "other"]);
    assert_eq!(env.ok(&["list-sessions", "-F", "#{session_name}"]), "other\nwork\n");
    assert_eq!(env.ok(&["kill-session", "-t", "other"]), "");
    assert_eq!(env.ok(&["list-sessions", "-F", "#{session_name}"]), "work\n");
    assert_eq!(env.ok(&["kill-server"]), "");
    assert!(env.wait(Duration::from_secs(5), || !path_exists(&env.sock)));
    let o = env.run(&["ls"]);
    assert_eq!(o.status.code(), Some(1));
    assert_eq!(String::from_utf8_lossy(&o.stderr), format!("no server running on {}\n", env.sock.display()));
}

#[test]
fn kill_session_of_last_session_stops_server() {
    let env = Env::new();
    env.ok(&["new-session", "-d", "-s", "only"]);
    let pid = env.server_pid();
    env.ok(&["kill-session", "-t", "only"]);
    assert!(env.wait(Duration::from_secs(5), || !path_exists(&env.sock)), "socket removed");
    // SAFETY: probing a pid with signal 0.
    assert!(env.wait(Duration::from_secs(5), || unsafe { libc::kill(pid as i32, 0) } != 0), "server process gone");
}

#[test]
fn pane_process_gets_sighup() {
    let env = Env::new();
    let flag = env.home.join("hup");
    env.ok(&["new-session", "-d", "-s", "h"]);
    let script = format!("trap 'echo got > {}; exit 0' HUP; while :; do sleep 0.05; done", flag.display());
    env.ok(&["split-window", "-t", "h", &script]);
    let target = env.ok(&["display-message", "-p", "-t", "h", "#{pane_id}"]);
    std::thread::sleep(Duration::from_millis(300)); // let sh install the trap
    env.ok(&["kill-pane", "-t", target.trim()]);
    assert!(env.wait(Duration::from_secs(5), || flag.exists()), "pane process did not get SIGHUP");
}

/// A pane running `script` in a second window of a new session; returns
/// its id and the pid of its process (the leader of its process group).
fn pane_running(env: &Env, script: &str) -> (String, u32) {
    env.ok(&["new-session", "-d", "-s", "k"]);
    env.ok(&["new-window", "-t", "k", script]);
    let got = env.ok(&["display-message", "-p", "-t", "k", "#{pane_id} #{pane_pid}"]);
    let (id, pid) = got.trim().split_once(' ').expect("pane id and pid");
    (id.to_string(), pid.parse().expect("pane pid"))
}

/// A shell that ignores SIGHUP, and the child it waits for (which inherits
/// that): what a hangup alone leaves running for good.
const IGNORES_HUP: &str = "trap '' HUP; sleep 1000";

#[test]
fn kill_pane_kills_a_process_that_ignores_sighup() {
    let env = Env::new();
    let (id, pid) = pane_running(&env, IGNORES_HUP);
    std::thread::sleep(Duration::from_millis(300)); // let sh install the trap
    env.ok(&["kill-pane", "-t", &id]);
    assert!(group_alive(pid), "killed before its grace period was over");
    assert!(env.wait(Duration::from_secs(3), || !group_alive(pid)), "the pane's process group survived kill-pane");
    assert_eq!(env.ok(&["display-message", "-p", "ok"]), "ok\n");
}

#[test]
fn kill_server_kills_a_process_that_ignores_sighup() {
    let env = Env::new();
    let (_, pid) = pane_running(&env, IGNORES_HUP);
    let server = env.server_pid();
    std::thread::sleep(Duration::from_millis(300)); // let sh install the trap
    env.ok(&["kill-server"]);
    assert!(env.wait(Duration::from_secs(3), || !pid_alive(server)), "server did not exit");
    assert!(!group_alive(pid), "the pane's process group outlived the server");
}

/// The wait for hung-up panes costs nothing when they exit on the hangup.
#[test]
fn kill_server_does_not_wait_for_panes_that_exit() {
    let env = Env::new();
    let (_, pid) = pane_running(&env, "exec sleep 1000");
    let server = env.server_pid();
    let t = std::time::Instant::now();
    env.ok(&["kill-server"]);
    assert!(env.wait(Duration::from_secs(3), || !pid_alive(server)), "server did not exit");
    let took = t.elapsed();
    assert!(took < Duration::from_millis(300), "the server took {took:?} to exit");
    assert!(!group_alive(pid));
}

#[test]
fn racing_clients_start_one_server() {
    for round in 0..4 {
        let env = Env::new();
        let children: Vec<_> =
            (0..4).map(|i| env.command().args(["new-session", "-d", "-s", &format!("r{i}")]).spawn().unwrap()).collect();
        for mut c in children {
            assert!(c.wait().unwrap().success(), "round {round}");
        }
        let ls = env.ok(&["list-sessions", "-F", "#{session_name}"]);
        assert_eq!(ls, "r0\nr1\nr2\nr3\n", "round {round}: all sessions in one server");
        assert_eq!(env.server_pids().len(), 1, "round {round}: exactly one server process");
    }
}

#[test]
fn no_server_running() {
    let env = Env::new();
    for cmd in [&["ls"][..], &["attach"], &["kill-server"], &["send-keys", "x"]] {
        let o = env.run(cmd);
        assert_eq!(o.status.code(), Some(1), "{cmd:?}");
        assert_eq!(String::from_utf8_lossy(&o.stderr), format!("no server running on {}\n", env.sock.display()));
    }
    assert!(!path_exists(&env.sock));
}

#[test]
fn version_and_usage() {
    let o = std::process::Command::new(BIN).arg("-V").output().unwrap();
    assert!(o.status.success());
    assert_eq!(String::from_utf8_lossy(&o.stdout), format!("lif {}\n", env!("CARGO_PKG_VERSION")));
    let o = std::process::Command::new(BIN).arg("-Q").output().unwrap();
    assert_eq!(o.status.code(), Some(1));
}

#[test]
fn bench_harness_cli() {
    let env = Env::new();
    env.ok(&["new-session", "-d", "-s", "B", "-x", "200", "-y", "50"]);
    env.ok(&["split-window", "-t", "B"]);
    env.wait_capture("B", "$");
    env.ok(&["send-keys", "-t", "B", "echo bench''ok", "Enter"]);
    env.wait_capture("B", "benchok");
    let mut t = Term::spawn(&env, &["attach", "-t", "B"], Size::new(200, 51));
    t.wait_text("benchok");
    let start = Instant::now();
    env.ok(&["kill-server"]);
    assert_eq!(t.wait_exit(), 0);
    assert!(t.screen().contains("[server exited]"), "{}", t.screen());
    assert!(!t.modes().alt_screen);
    assert!(start.elapsed() < Duration::from_secs(5));
}

#[test]
fn not_a_terminal() {
    let env = Env::new();
    env.ok(&["new-session", "-d", "-s", "x"]);
    let o = env.command().args(["attach", "-t", "x"]).stdin(std::process::Stdio::null()).output().unwrap();
    assert_eq!(o.status.code(), Some(1));
    assert_eq!(String::from_utf8_lossy(&o.stderr), "open terminal failed: not a terminal\n");
}

/// Inside a pane `lif` finds its server through `$LIF` (first field), as
/// tmux does through `$TMUX`; -S and -L still win, and `$LIF` beats
/// `$LIF_SOCKET` the way `$TMUX` beats `$TMUX_TMPDIR`.
#[test]
fn lif_inside_a_pane_finds_its_server_through_lif_env() {
    let env = Env::new();
    env.ok(&["new-session", "-d", "-s", "in"]);
    env.wait_capture("in", "$");
    let out = env.home.join("ls.out");
    let cmd = format!("LIF_SOCKET=/nonexistent {BIN} ls -F '#{{session_name}} #{{socket_path}}' > {} 2>&1", out.display());
    env.ok(&["send-keys", "-t", "in", &cmd, "Enter"]);
    let want = format!("in {}\n", env.sock.display());
    let mut got = String::new();
    let ok = env.wait(Duration::from_secs(10), || {
        got = std::fs::read_to_string(&out).unwrap_or_default();
        got == want
    });
    assert!(ok, "lif in a pane printed {got:?}, want {want:?}");
    // The same from outside, with $LIF set by hand.
    let lif = format!("{},1,0", env.sock.display());
    let bare = |args: &[&str], extra: &[(&str, &str)]| {
        let mut c = std::process::Command::new(BIN);
        for k in Env::REMOVE {
            c.env_remove(k);
        }
        c.envs(env.vars()).env("LIF", &lif).envs(extra.iter().copied()).args(args).output().unwrap()
    };
    let o = bare(&["ls", "-F", "#{session_name}"], &[("LIF_SOCKET", "/nonexistent")]);
    assert_eq!(String::from_utf8_lossy(&o.stdout), "in\n", "{}", String::from_utf8_lossy(&o.stderr));
    // -S beats $LIF.
    let other = env.dir.join("other");
    let o = bare(&["-S", other.to_str().unwrap(), "ls"], &[]);
    assert_eq!(String::from_utf8_lossy(&o.stderr), format!("no server running on {}\n", other.display()));
    // An empty $LIF counts as unset: $LIF_SOCKET is used.
    let o = bare(&["ls", "-F", "#{session_name}"], &[("LIF", ""), ("LIF_SOCKET", env.sock.to_str().unwrap())]);
    assert_eq!(String::from_utf8_lossy(&o.stdout), "in\n", "{}", String::from_utf8_lossy(&o.stderr));
}
