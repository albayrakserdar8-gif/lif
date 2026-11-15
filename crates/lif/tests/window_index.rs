//! Window indices at the top of the range, against a real server: they
//! used to wrap at u32::MAX, overwrite a window, leak its pane's process and
//! crash the server on a later break-pane.
mod common;

use common::*;

fn err(env: &Env, args: &[&str]) -> String {
    let o = env.run(args);
    assert!(!o.status.success(), "lif {args:?} should fail: {}", String::from_utf8_lossy(&o.stdout));
    String::from_utf8_lossy(&o.stderr).trim().to_string()
}

/// Shell processes whose parent is the server: one per pane, none leaked.
fn children(pid: u32) -> usize {
    let o = std::process::Command::new("pgrep").args(["-P", &pid.to_string()]).output().unwrap();
    String::from_utf8_lossy(&o.stdout).lines().count()
}

#[test]
fn window_index_past_the_maximum_is_refused_and_the_server_lives() {
    let env = Env::new();
    env.ok(&["new-session", "-d", "-s", "s"]);
    let server = env.server_pid();
    // The reported sequence.
    assert_eq!(
        err(&env, &["new-window", "-t", "s:4294967295"]),
        "create window failed: window index too big: 4294967295 (the largest is 2147483647)"
    );
    env.ok(&["new-window", "-a"]);
    env.ok(&["break-pane"]);
    assert_eq!(env.ok(&["list-windows", "-t", "s", "-F", "#{window_index}"]), "0\n2\n");

    // At the largest index nothing fits after it.
    env.ok(&["new-window", "-t", "s:2147483647"]);
    assert_eq!(err(&env, &["new-window", "-a", "-t", "s:2147483647"]), "create window failed: no free window index");
    env.ok(&["split-window", "-t", "s:2147483647"]);
    assert_eq!(err(&env, &["break-pane", "-a", "-s", "s:2147483647.1"]), "no free window index");
    assert_eq!(
        err(&env, &["move-window", "-s", "s:0", "-t", "s:4294967295"]),
        "window index too big: 4294967295 (the largest is 2147483647)"
    );
    env.ok(&["break-pane", "-s", "s:2147483647.1"]);

    assert_eq!(
        env.ok(&["list-windows", "-t", "s", "-F", "#{window_index} #{window_panes}"]),
        "0 1\n1 1\n2 1\n2147483647 1\n"
    );
    assert_eq!(env.server_pid(), server, "the server restarted");
    let panes = env.ok(&["list-panes", "-a", "-F", "#{pane_pid}"]).lines().count();
    assert_eq!(panes, 4);
    assert!(env.wait(secs(5), || children(server) == panes), "server has {} children for {panes} panes", children(server));
}
