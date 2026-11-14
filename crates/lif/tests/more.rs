//! Pane environment, modes, prompts, signals and input bounds.
mod common;

use std::time::{Duration, Instant};

use common::*;
use lif_types::Size;

fn attach_new(env: &Env, name: &str, size: Size) -> Term {
    let mut t = Term::spawn(env, &["new-session", "-s", name], size);
    t.wait_text("$");
    t
}

#[test]
fn pane_environment() {
    let env = Env::new();
    let mut t = Term::spawn_with(
        &env,
        &["new-session", "-s", "e"],
        Size::new(120, 24),
        &[("TMUX", "/tmp/x,1,0"), ("TMUX_PANE", "%9"), ("ZELLIJ", "0"), ("SSH_AUTH_SOCK", "/first"), ("LIFBIN", BIN)],
    );
    t.wait_text("$");
    t.type_str("echo \"T=$TERM C=$COLORTERM P=$TERM_PROGRAM X=$TMUX Y=$TMUX_PANE Z=$ZELLIJ S=$SSH_AUTH_SOCK N=$LIF_PANE\"\r");
    t.wait_text("T=xterm-256color C=truecolor P=lif X= Y= Z= S=/first N=%0");
    t.type_str("echo \"L=$LIF\"\r");
    let pid = env.server_pid();
    t.wait_text(&format!("L={},{pid},0", env.sock.display()));
    t.write(b"\x02d");
    assert_eq!(t.wait_exit(), 0);
    // Reattaching from a client with a new agent socket refreshes it for new panes.
    let mut t = Term::spawn_with(&env, &["attach", "-t", "e"], Size::new(120, 24), &[("SSH_AUTH_SOCK", "/second")]);
    t.wait_text("$");
    t.write(b"\x02c");
    t.wait_for("new window", secs(10), |t| t.lines()[23].contains("1:"));
    t.wait_for("prompt", secs(10), |t| t.lines()[0].starts_with('$'));
    t.type_str("echo \"S=$SSH_AUTH_SOCK\"\r");
    t.wait_text("S=/second");
    // A command run from inside a pane targets that pane.
    t.type_str("$LIFBIN split-window -h; echo split''done\r");
    t.wait_text("splitdone");
}

#[test]
fn copy_mode_scrolls_and_exits() {
    let env = Env::new();
    let mut t = attach_new(&env, "c", Size::new(80, 24));
    t.type_str("i=0; while [ $i -lt 100 ]; do echo row$i; i=$((i+1)); done\r");
    t.wait_text("row99");
    t.write(b"\x02[");
    t.wait_for("position indicator", secs(10), |t| t.lines()[0].trim_end().ends_with(']'));
    t.write(b"\x1b[5~");
    t.wait_for("scrolled up a page", secs(10), |t| t.screen().contains("row60") && !t.screen().contains("row99"));
    // tmux's page-up moves a screen less two rows (23 - 2 here).
    assert!(t.lines()[0].contains("[21/"), "{}", t.lines()[0]);
    // emacs keys (no VISUAL / EDITOR in the test environment): M-< is
    // history-top.
    t.write(b"\x1b<");
    t.wait_for("top of history", secs(10), |t| t.screen().contains("row0\n") || t.lines().iter().any(|l| l.starts_with("row0 ") || l == "row0"));
    t.write(b"q");
    t.wait_for("back to live", secs(10), |t| t.screen().contains("row99") && !t.lines()[0].trim_end().ends_with(']'));
    // The wheel scrolls through the same mode and leaves it at the bottom.
    t.wait_for("mouse on", secs(10), |t| t.modes().mouse_tracking);
    t.write(b"\x1b[<64;10;10M");
    // tmux's WheelUpPane: copy-mode -e, then five lines per notch.
    t.wait_for("wheel scrolled", secs(10), |t| t.lines()[0].contains("[5/"));
    t.write(b"\x1b[<65;10;10M");
    t.wait_for("wheel back at bottom leaves the mode", secs(10), |t| !t.lines()[0].trim_end().ends_with(']'));
}

#[test]
fn display_panes_and_confirm_before() {
    let env = Env::new();
    let mut t = attach_new(&env, "d", Size::new(80, 24));
    t.write(b"\x02%");
    t.wait_for("right pane active", secs(10), |t| t.cursor().is_some_and(|(x, _)| x > 40));
    t.write(b"\x02q");
    t.wait_for("pane numbers", secs(10), |t| t.lines()[0].trim_end().ends_with('1') && t.lines()[0][..40].trim_end().ends_with('0'));
    t.write(b"0");
    t.wait_for("pane 0 selected", secs(10), |t| t.cursor().is_some_and(|(x, _)| x < 40));
    t.write(b"\x02x");
    t.wait_text("kill-pane 0? (y/n)");
    t.write(b"y");
    t.wait_for("pane killed", secs(10), |t| !t.screen().contains('│'));
    assert_eq!(env.ok(&["list-panes", "-t", "d", "-F", "#{pane_id}"]), "%1\n");
}

#[test]
fn automatic_window_name_follows_program() {
    let env = Env::new();
    let mut t = attach_new(&env, "n", Size::new(80, 24));
    t.type_str("cat\r");
    t.wait_for("window named after cat", secs(10), |t| t.lines()[23].contains("0:cat*"));
    t.write(b"\x04");
    t.wait_for("name back to the shell", secs(10), |t| !t.lines()[23].contains("0:cat*"));
}

#[test]
fn sigterm_shuts_down_cleanly() {
    let env = Env::new();
    let mut t = attach_new(&env, "t", Size::new(80, 24));
    let pid = env.server_pid();
    // SAFETY: signalling our own test server.
    unsafe { libc::kill(pid as i32, libc::SIGTERM) };
    assert_eq!(t.wait_exit(), 0);
    assert!(t.screen().contains("[server exited]"), "{}", t.screen());
    assert!(!t.modes().alt_screen);
    assert!(env.wait(Duration::from_secs(5), || !path_exists(&env.sock)));
}

#[test]
fn input_to_a_stuck_pane_is_bounded() {
    let env = Env::new();
    let mut t = attach_new(&env, "stuck", Size::new(80, 24));
    // A program that never reads its input.
    t.type_str("stty raw -echo; sleep 30\r");
    t.wait_for("sleep running", secs(10), |t| t.lines()[23].contains("sleep"));
    let pid = env.server_pid();
    let rss0 = rss_kib(pid);
    let chunk = vec![b'x'; 64 * 1024];
    let mut sent = 0usize;
    let end = Instant::now() + Duration::from_secs(3);
    while Instant::now() < end && sent < 64 << 20 {
        match t.pty.write(&chunk) {
            Ok(n) => sent += n,
            Err(_) => {
                t.pump(Duration::from_millis(5));
            }
        }
    }
    let rss1 = rss_kib(pid);
    eprintln!("stuck pane: client accepted {sent} bytes of typing, server rss {rss0} -> {rss1} KiB");
    assert!(rss1 < rss0 + 4096, "server buffered input without bound: {rss0} -> {rss1} KiB");
    // Other commands still work.
    assert_eq!(env.ok(&["display-message", "-p", "ok"]), "ok\n");
}

#[test]
fn socket_label() {
    let env = Env::new();
    let label = format!("lif-test-{}", std::process::id());
    let run = |args: &[&str]| {
        let mut c = std::process::Command::new(BIN);
        for k in Env::REMOVE {
            c.env_remove(k);
        }
        c.envs(env.vars()).env("TMPDIR", &env.dir).arg("-L").arg(&label).args(args).output().unwrap()
    };
    assert!(run(&["new-session", "-d", "-s", "lbl"]).status.success());
    let o = run(&["ls", "-F", "#{session_name}"]);
    assert_eq!(String::from_utf8_lossy(&o.stdout), "lbl\n");
    // SAFETY: getuid cannot fail.
    let uid = unsafe { libc::getuid() };
    let sock = env.dir.join(format!("lif-{uid}")).join(&label);
    assert!(path_exists(&sock), "{}", sock.display());
    use std::os::unix::fs::PermissionsExt;
    assert_eq!(std::fs::metadata(&sock).unwrap().permissions().mode() & 0o777, 0o600);
    assert_eq!(std::fs::metadata(sock.parent().unwrap()).unwrap().permissions().mode() & 0o777, 0o700);
    assert!(run(&["kill-server"]).status.success());
}
