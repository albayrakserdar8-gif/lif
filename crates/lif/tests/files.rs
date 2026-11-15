//! Files the server reads on behalf of a command (`source-file`, `-f`): it
//! is one thread serving every session, so a file that blocks, never ends
//! or nests without bound must give an error, never a stall or a crash.
mod common;

use std::path::{Path, PathBuf};
use std::process::{Output, Stdio};
use std::time::{Duration, Instant};

use common::*;

/// Runs `lif -S sock args...`, failing the test if it has not finished
/// within 5 s. `unstick` runs first then, so that a server stuck in the
/// command does not hang the rest of the test (and its clean-up).
fn run_within(env: &Env, args: &[&str], unstick: impl FnOnce()) -> Output {
    let mut child = env.command().args(args).stdout(Stdio::piped()).stderr(Stdio::piped()).spawn().expect("run lif");
    let end = Instant::now() + Duration::from_secs(5);
    while child.try_wait().unwrap().is_none() {
        if Instant::now() >= end {
            unstick();
            let _ = child.kill();
            let _ = child.wait();
            panic!("lif {args:?} did not finish within 5 s");
        }
        std::thread::sleep(Duration::from_millis(10));
    }
    child.wait_with_output().unwrap()
}

fn mkfifo(env: &Env, name: &str) -> PathBuf {
    let p = env.dir.join(name);
    assert!(std::process::Command::new("mkfifo").arg(&p).status().unwrap().success());
    p
}

/// Opening the FIFO for writing (and closing it) ends a reader blocked on
/// it. Non-blocking, because with no reader a blocking open would wait in
/// turn (it fails with ENXIO instead, which is just as well).
fn release(fifo: &Path) {
    use std::os::unix::fs::OpenOptionsExt;
    let _ = std::fs::OpenOptions::new().write(true).custom_flags(libc::O_NONBLOCK).open(fifo);
}

/// SIGKILL for this test's own server, stuck reading a file that never ends.
fn kill_server(pid: u32) {
    let _ = std::process::Command::new("kill").args(["-9", &pid.to_string()]).status();
}

fn stderr(o: &Output) -> String {
    String::from_utf8_lossy(&o.stderr).into_owned()
}

#[test]
fn source_file_on_a_fifo_fails_at_once_and_the_server_still_answers() {
    let env = Env::new();
    env.ok(&["new-session", "-d", "-s", "a"]);
    let fifo = mkfifo(&env, "fifo.toml");
    let path = fifo.to_str().unwrap();
    let o = run_within(&env, &["source-file", path], || release(&fifo));
    assert!(!o.status.success());
    assert!(stderr(&o).contains(&format!("{path}: not a regular file")), "{}", stderr(&o));
    assert_eq!(env.ok(&["list-sessions", "-F", "#{session_name}"]), "a\n");
    // -q only hides missing files.
    assert!(!run_within(&env, &["source-file", "-q", path], || release(&fifo)).status.success());
}

#[test]
fn source_file_on_a_device_fails_at_once() {
    let env = Env::new();
    env.ok(&["new-session", "-d", "-s", "a"]);
    let pid = env.server_pid();
    let o = run_within(&env, &["source-file", "/dev/zero"], || kill_server(pid));
    assert!(!o.status.success());
    assert!(stderr(&o).contains("/dev/zero: not a regular file"), "{}", stderr(&o));
    assert_eq!(env.ok(&["list-sessions", "-F", "#{session_name}"]), "a\n");
}

#[test]
fn source_file_refuses_a_file_over_one_mib() {
    let env = Env::new();
    env.ok(&["new-session", "-d", "-s", "a"]);
    let big = env.dir.join("big.toml");
    let mut text = "#".repeat(1024 * 1024);
    text.push_str("\nbase-index = 7\n");
    std::fs::write(&big, &text).unwrap();
    let o = env.run(&["source-file", big.to_str().unwrap()]);
    assert!(!o.status.success());
    let want = format!("{}: file too large ({} bytes, limit 1 MiB)", big.display(), text.len());
    assert!(stderr(&o).contains(&want), "{}", stderr(&o));
    assert_eq!(env.ok(&["show", "-gv", "base-index"]), "0\n", "nothing applied");
    // At the limit it is read.
    let ok = env.dir.join("ok.toml");
    let line = "base-index = 7\n";
    std::fs::write(&ok, format!("{}\n{line}", "#".repeat(1024 * 1024 - line.len() - 1))).unwrap();
    assert_eq!(std::fs::metadata(&ok).unwrap().len(), 1024 * 1024);
    env.ok(&["source-file", ok.to_str().unwrap()]);
    assert_eq!(env.ok(&["show", "-gv", "base-index"]), "7\n");
}

#[test]
fn source_file_of_deeply_nested_commands_is_an_error() {
    let env = Env::new();
    env.ok(&["new-session", "-d", "-s", "a"]);
    let deep = "{ ".repeat(100_000);
    // Not TOML: the server looks at the first line to see whether it is a
    // tmux command (to point at migrate-tmux-conf).
    let raw = env.dir.join("raw.toml");
    std::fs::write(&raw, format!("{deep}\n")).unwrap();
    assert!(!env.run(&["source-file", raw.to_str().unwrap()]).status.success());
    // A key binding of nested blocks.
    let bind = env.dir.join("bind.toml");
    std::fs::write(&bind, format!("[keys]\nx = '{deep}'\n")).unwrap();
    let o = env.run(&["source-file", bind.to_str().unwrap()]);
    assert!(!o.status.success());
    assert!(stderr(&o).contains("nested more than"), "{}", &stderr(&o)[..stderr(&o).len().min(300)]);
    assert_eq!(env.ok(&["list-sessions", "-F", "#{session_name}"]), "a\n");
}

/// `-f` naming a FIFO: like a missing `-f` file, the problem is reported
/// (show-messages, and on the status line of the first client) and the
/// server starts without it, rather than refusing to start.
#[test]
fn dash_f_naming_a_fifo_starts_the_server_without_it() {
    let env = Env::new();
    let fifo = mkfifo(&env, "f.toml");
    let path = fifo.to_str().unwrap();
    let o = run_within(&env, &["-f", path, "new-session", "-d", "-s", "a"], || {
        release(&fifo);
        for pid in env.server_pids() {
            kill_server(pid);
        }
    });
    assert!(o.status.success(), "{}", stderr(&o));
    assert!(env.ok(&["show-messages"]).contains(&format!("{path}: not a regular file")));
    assert_eq!(env.ok(&["list-sessions", "-F", "#{session_name}"]), "a\n");
}

#[test]
fn a_missing_default_config_is_ignored() {
    let env = Env::new();
    env.ok(&["new-session", "-d", "-s", "a"]);
    assert_eq!(env.ok(&["show-messages"]), "");
}

#[test]
fn check_and_migrate_refuse_fifos_and_big_files() {
    let env = Env::new();
    let fifo = mkfifo(&env, "f.toml");
    let o = run_within(&env, &["check-config", fifo.to_str().unwrap()], || release(&fifo));
    assert_eq!(o.status.code(), Some(1));
    assert!(String::from_utf8_lossy(&o.stdout).contains("cannot read file: not a regular file"), "{o:?}");
    let o = run_within(&env, &["migrate-tmux-conf", fifo.to_str().unwrap()], || release(&fifo));
    assert_eq!(o.status.code(), Some(1));
    assert!(stderr(&o).contains("not a regular file"), "{o:?}");
    let o = run_within(&env, &["migrate-tmux-conf", "/dev/zero"], || {});
    assert_eq!(o.status.code(), Some(1));
    let big = env.dir.join("big.conf");
    std::fs::write(&big, "#".repeat(2 * 1024 * 1024)).unwrap();
    let o = env.run(&["migrate-tmux-conf", big.to_str().unwrap()]);
    assert_eq!(o.status.code(), Some(1));
    assert!(stderr(&o).contains("file too large (2097152 bytes, limit 1 MiB)"), "{o:?}");
    assert!(env.server_pids().is_empty());
}
