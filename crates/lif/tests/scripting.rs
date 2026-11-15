//! Scripting: shell commands run by the server (`run-shell`, `if-shell`,
//! `#()`), `wait-for`, hooks, the environment commands, and the pane
//! commands that came with them. Expected output is tmux 3.8's for the same
//! commands (`tmux -L ref -f /dev/null`), except where noted.
mod common;

use std::process::Stdio;
use std::time::{Duration, Instant};

use common::*;
use lif_types::Size;

fn out(o: &std::process::Output) -> (i32, String, String) {
    (o.status.code().unwrap_or(-1), String::from_utf8_lossy(&o.stdout).into_owned(), String::from_utf8_lossy(&o.stderr).into_owned())
}

fn session(env: &Env) {
    env.ok(&["new-session", "-d", "-s", "s", "-x", "80", "-y", "24"]);
    env.wait_capture("s:", "$");
}

#[test]
fn run_shell_prints_output_and_passes_the_status_on() {
    let env = Env::new();
    session(&env);
    assert_eq!(out(&env.run(&["run-shell", "echo hi; echo err >&2; exit 3"])), (3, "hi\n'echo hi; echo err >&2; exit 3' returned 3\n".into(), String::new()));
    assert_eq!(out(&env.run(&["run-shell", "-E", "echo err >&2"])), (0, "err\n".into(), String::new()));
    assert_eq!(out(&env.run(&["run-shell", "printf 'no newline'"])), (0, "no newline\n".into(), String::new()));
    assert_eq!(out(&env.run(&["run-shell", "kill -9 $$"])), (137, "'kill -9 $$' terminated by signal 9\n".into(), String::new()));
    // The list goes on after a failing command; the status is the command's.
    assert_eq!(out(&env.run(&["run-shell", "exit 1", ";", "display-message", "-p", "after"])), (1, "'exit 1' returned 1\nafter\n".into(), String::new()));
    // It waits for the command, in order.
    let t = Instant::now();
    assert_eq!(env.ok(&["run-shell", "sleep 0.3; echo slow", ";", "display-message", "-p", "after"]), "slow\nafter\n");
    assert!(t.elapsed() >= Duration::from_millis(300));
    // -b does not; -d waits before starting; -C runs lif commands.
    let t = Instant::now();
    assert_eq!(env.ok(&["run-shell", "-b", "sleep 2; echo late"]), "");
    assert!(t.elapsed() < Duration::from_millis(1500), "{:?}", t.elapsed());
    let t = Instant::now();
    assert_eq!(env.ok(&["run-shell", "-d", "0.3", "echo delayed"]), "delayed\n");
    assert!(t.elapsed() >= Duration::from_millis(300));
    assert_eq!(env.ok(&["run-shell", "-C", "display-message -p '#{session_name}'"]), "s\n");
    assert_eq!(out(&env.run(&["run-shell", "-d", "x"])), (1, String::new(), "invalid delay time: x\n".into()));
    // The command is a format; it runs in the client's directory, or -c's.
    assert_eq!(env.ok(&["run-shell", "echo #{session_name}:#{pane_id}"]), "s:%0\n");
    let here = std::env::current_dir().unwrap();
    assert_eq!(env.ok(&["run-shell", "pwd"]), format!("{}\n", here.display()));
    assert_eq!(env.ok(&["run-shell", "-c", "/usr/bin", "pwd"]), "/usr/bin\n");
    // The command finds this server and its target pane.
    let inner = format!("{BIN} display-message -p \"inner #{{session_name}} $LIF_PANE\"");
    assert_eq!(env.ok(&["run-shell", &inner]), "inner s %0\n");
}

#[test]
fn if_shell_picks_a_branch() {
    let env = Env::new();
    session(&env);
    let yes_no = ["display-message -p yes", "display-message -p no"];
    assert_eq!(env.ok(&["if-shell", "true", yes_no[0], yes_no[1]]), "yes\n");
    assert_eq!(env.ok(&["if-shell", "false", yes_no[0], yes_no[1]]), "no\n");
    assert_eq!(env.ok(&["if-shell", "false", yes_no[0]]), "");
    assert_eq!(env.ok(&["if-shell", "-F", "#{session_name}", yes_no[0], yes_no[1]]), "yes\n");
    assert_eq!(env.ok(&["if-shell", "-F", "#{==:#{session_name},x}", yes_no[0], yes_no[1]]), "no\n");
    assert_eq!(env.ok(&["if-shell", "test #{session_windows} -eq 1", yes_no[0], yes_no[1]]), "yes\n");
    assert_eq!(env.ok(&["if-shell", "sleep 0.2", "display-message -p first", ";", "display-message", "-p", "second"]), "first\nsecond\n");
    assert_eq!(out(&env.run(&["if-shell", "true", "nosuchcmd"])), (1, String::new(), "unknown command: nosuchcmd\n".into()));
    // In a key binding: the branch runs for the client that pressed it.
    env.ok(&["bind-key", "y", "if-shell 'true' 'rename-window viakey'"]);
    let mut t = Term::spawn(&env, &["attach", "-t", "s:"], Size::new(80, 24));
    t.wait_text("$");
    t.write(b"\x02y");
    assert!(env.wait(secs(10), || env.ok(&["display-message", "-p", "-t", "s:", "#{window_name}"]) == "viakey\n"));
    // Output of a command run from a key shows up as a message.
    env.ok(&["bind-key", "u", "run-shell 'echo from-the-shell'"]);
    t.write(b"\x02u");
    t.wait_text("from-the-shell");
}

#[test]
fn wait_for_blocks_until_signalled() {
    let env = Env::new();
    session(&env);
    let mut waiter = env.command().args(["wait-for", "ch", ";", "display-message", "-p", "woke"]).stdout(Stdio::piped()).spawn().unwrap();
    std::thread::sleep(Duration::from_millis(300));
    assert!(waiter.try_wait().unwrap().is_none(), "wait-for returned before the signal");
    env.ok(&["wait-for", "-S", "ch"]);
    let o = waiter.wait_with_output().unwrap();
    assert_eq!(out(&o), (0, "woke\n".into(), String::new()));
    // Signalled first: the next wait returns at once.
    env.ok(&["wait-for", "-S", "early"]);
    env.ok(&["wait-for", "early"]);
    // A lock serializes; unlocking one that is not held is an error.
    env.ok(&["wait-for", "-L", "lk"]);
    let mut second = env.command().args(["wait-for", "-L", "lk"]).spawn().unwrap();
    std::thread::sleep(Duration::from_millis(300));
    assert!(second.try_wait().unwrap().is_none(), "two holders of one lock");
    env.ok(&["wait-for", "-U", "lk"]);
    assert!(second.wait().unwrap().success());
    env.ok(&["wait-for", "-U", "lk"]);
    assert_eq!(out(&env.run(&["wait-for", "-U", "lk"])), (1, String::new(), "channel lk not locked\n".into()));
    // A waiter that is killed leaves nothing behind that a signal wakes.
    let mut gone = env.command().args(["wait-for", "dead"]).spawn().unwrap();
    std::thread::sleep(Duration::from_millis(200));
    gone.kill().unwrap();
    gone.wait().unwrap();
    env.ok(&["wait-for", "-S", "dead"]);
    // The usual use: a script waits for a pane's command to finish.
    env.ok(&["send-keys", "-t", "s:", &format!("sleep 0.3; {BIN} wait-for -S done"), "Enter"]);
    let t = Instant::now();
    env.ok(&["wait-for", "done"]);
    assert!(t.elapsed() >= Duration::from_millis(200), "{:?}", t.elapsed());
}

#[test]
fn status_line_shows_shell_command_output() {
    let env = Env::new();
    let file = env.home.join("status-text");
    std::fs::write(&file, "one\n").unwrap();
    env.ok(&["new-session", "-d", "-s", "s", "-x", "80", "-y", "24"]);
    env.ok(&["set-option", "-g", "status-interval", "1"]);
    env.ok(&["set-option", "-g", "status-right", &format!("<#(cat {})>", file.display())]);
    let mut t = Term::spawn(&env, &["attach", "-t", "s:"], Size::new(80, 24));
    t.wait_text("<one>");
    // It is run again every status-interval.
    std::fs::write(&file, "ignored line\ntwo\n").unwrap();
    t.wait_text("<two>");
    assert!(!t.screen().contains("ignored"), "only the last line is shown");
    // Without #() the status line asks for nothing: no job is left running
    // and none starts.
    env.ok(&["set-option", "-g", "status-right", "plain"]);
    t.wait_text("plain");
    let marker = env.home.join("ran");
    env.ok(&["set-option", "-g", "@unused", &format!("#(touch {})", marker.display())]);
    std::thread::sleep(Duration::from_millis(1500));
    assert!(!marker.exists(), "a format nobody expands runs nothing");
}

#[test]
fn hooks_run_commands_when_things_happen() {
    let env = Env::new();
    session(&env);
    let log = env.home.join("hook-log");
    // After a command, with its output; tmux prints the hook's command as
    // it parsed it.
    env.ok(&["set-hook", "-g", "after-new-window", "display -p 'hooked #{hook} #{hook_pane}'"]);
    assert_eq!(env.ok(&["show-hooks", "-g", "after-new-window"]), "after-new-window[0] display-message -p \"hooked #{hook} #{hook_pane}\"\n");
    assert_eq!(env.ok(&["new-window", "-d", "-t", "s:"]), "hooked after-new-window %1\n");
    // When a pane's program exits: nobody ran a command, the server runs it.
    env.ok(&["set-hook", "-g", "pane-exited", &format!("run-shell 'echo exited #{{hook_pane}} >> {}'", log.display())]);
    env.ok(&["set-hook", "-g", "session-closed", &format!("run-shell 'echo closed #{{hook_session_name}} >> {}'", log.display())]);
    env.ok(&["split-window", "-d", "-t", "s:0", "sleep 0.2"]);
    assert!(env.wait(secs(10), || std::fs::read_to_string(&log).is_ok_and(|l| l == "exited %2\n")), "{:?}", std::fs::read_to_string(&log));
    env.ok(&["new-session", "-d", "-s", "doomed"]);
    env.ok(&["kill-session", "-t", "doomed"]);
    assert!(env.wait(secs(10), || std::fs::read_to_string(&log).is_ok_and(|l| l.ends_with("closed doomed\n"))), "{:?}", std::fs::read_to_string(&log));
    // A client attaching.
    env.ok(&["set-hook", "-g", "client-attached", "rename-window -t s:0 attached"]);
    let mut t = Term::spawn(&env, &["attach", "-t", "s:"], Size::new(80, 24));
    t.wait_text("attached");
    assert_eq!(out(&env.run(&["set-hook", "-g", "no-such-hook", "display x"])), (1, String::new(), "invalid option: no-such-hook\n".into()));
    env.ok(&["set-hook", "-gu", "after-new-window"]);
    assert_eq!(env.ok(&["new-window", "-d", "-t", "s:"]), "");
}

#[test]
fn hooks_and_environment_from_the_config_file() {
    let mut env = Env::new();
    let cfg = env.home.join("lif.toml");
    let log = env.home.join("created");
    std::fs::write(
        &cfg,
        format!("[hooks]\nsession-created = \"run-shell 'echo #{{hook_session_name}} >> {}'\"\n\n[env]\nFROM_CONFIG = \"yes it is\"\nHOME_TOWN = false\n", log.display()),
    )
    .unwrap();
    env.extra.push(("LIF_CONFIG".into(), cfg.display().to_string()));
    env.extra.push(("HOME_TOWN".into(), "set outside".into()));
    env.ok(&["new-session", "-d", "-s", "first", "-x", "80", "-y", "24"]);
    assert_eq!(env.ok(&["show-messages"]), "", "the file loads cleanly");
    assert!(env.wait(secs(10), || std::fs::read_to_string(&log).is_ok_and(|l| l == "first\n")), "{:?}", std::fs::read_to_string(&log));
    assert_eq!(env.ok(&["show-environment", "-g"]), "FROM_CONFIG=yes it is\n-HOME_TOWN\n");
    env.wait_capture("first", "$");
    env.ok(&["send-keys", "-t", "first", "echo \"[$FROM_CONFIG][${HOME_TOWN-unset}]\"", "Enter"]);
    env.wait_capture("first", "[yes it is][unset]");
    assert_eq!(env.ok(&["run-shell", "echo \"[$FROM_CONFIG][${HOME_TOWN-unset}]\""]), "[yes it is][unset]\n");
}

#[test]
fn environment_commands_reach_new_panes() {
    let env = Env::new();
    session(&env);
    env.ok(&["set-environment", "-g", "GV", "global"]);
    env.ok(&["set-environment", "-t", "s:", "SV", "of session"]);
    assert_eq!(env.ok(&["show-environment", "-t", "s:"]), "SV=of session\n");
    assert_eq!(env.ok(&["show-environment", "-t", "s:", "-s", "SV"]), "SV=\"of session\"; export SV;\n");
    assert_eq!(out(&env.run(&["show-environment", "-t", "s:", "NOPE"])), (1, String::new(), "unknown variable: NOPE\n".into()));
    env.ok(&["new-window", "-t", "s:", "-e", "EV=flag", "-e", "GV=overridden"]);
    env.wait_capture("s:", "$");
    env.ok(&["send-keys", "-t", "s:", "echo \"[$GV][$SV][$EV]\"", "Enter"]);
    env.wait_capture("s:", "[overridden][of session][flag]");
    // A session made later has the global ones only; the pane before the
    // change keeps what it started with.
    env.ok(&["new-session", "-d", "-s", "other"]);
    env.wait_capture("other", "$");
    env.ok(&["send-keys", "-t", "other", "echo \"[$GV][${SV-unset}]\"", "Enter"]);
    env.wait_capture("other", "[global][unset]");
    env.ok(&["split-window", "-t", "other", "-e", "A=1", "echo \"split:$A:$GV\"; sleep 30"]);
    env.wait_capture("other", "split:1:global");
    env.ok(&["set-option", "-t", "other", "remain-on-exit", "on"]);
    env.ok(&["respawn-pane", "-k", "-t", "other", "-e", "A=2", "echo \"respawned:$A\"; sleep 30"]);
    env.wait_capture("other", "respawned:2");
}

#[test]
fn clear_history_join_pane_and_synchronized_typing() {
    let env = Env::new();
    session(&env);
    env.ok(&["send-keys", "-t", "s:", "seq 1 300", "Enter"]);
    env.wait_capture("s:", "300");
    assert!(env.wait(secs(10), || env.ok(&["display-message", "-p", "-t", "s:", "#{history_size}"]).trim().parse::<u32>().is_ok_and(|n| n > 200)));
    env.ok(&["clear-history", "-t", "s:"]);
    assert_eq!(env.ok(&["display-message", "-p", "-t", "s:", "#{history_size}"]), "0\n");
    assert!(env.wait_capture("s:", "300").contains("299"), "the screen stays");
    // join-pane: sizes as tmux gives them.
    env.ok(&["new-window", "-d", "-t", "s:"]);
    env.ok(&["split-window", "-d", "-t", "s:0"]);
    env.ok(&["join-pane", "-h", "-s", "s:0.1", "-t", "s:1.0"]);
    let panes = ["list-panes", "-s", "-t", "s:", "-F", "#{window_index}.#{pane_index} #{pane_id} #{pane_width}x#{pane_height} #{pane_active}"];
    assert_eq!(env.ok(&panes), "0.0 %0 80x24 1\n1.0 %1 40x24 0\n1.1 %2 39x24 1\n");
    assert_eq!(out(&env.run(&["join-pane", "-s", "s:1.1", "-t", "s:1.1"])), (1, String::new(), "source and target panes must be different\n".into()));
    // The pane that moved is the same process, still running.
    env.ok(&["send-keys", "-t", "%2", "echo mo''ved", "Enter"]);
    env.wait_capture("%2", "moved");
    // What is typed goes to every synchronized pane, not to others.
    env.ok(&["set-option", "-w", "-t", "s:1", "synchronize-panes", "on"]);
    let mut t = Term::spawn(&env, &["attach", "-t", "s:"], Size::new(80, 24));
    t.wait_text("$");
    t.type_str("echo sy''nc$((40+2))\r");
    env.wait_capture("%1", "sync42");
    env.wait_capture("%2", "sync42");
    assert!(!env.wait_capture("%0", "$").contains("sync"), "another window's pane got the keys");
    env.ok(&["set-option", "-w", "-t", "s:1", "synchronize-panes", "off"]);
    t.type_str("echo so''lo\r");
    env.wait_capture("%2", "solo");
    assert!(!env.wait_capture("%1", "sync42").contains("solo"));
}

#[test]
fn new_window_from_a_shell_in_a_pane() {
    // `lif new-window` typed in a pane used to fail with "index 0 in use".
    let env = Env::new();
    session(&env);
    env.ok(&["send-keys", "-t", "s:", &format!("{BIN} new-window -d; echo rc=$?"), "Enter"]);
    env.wait_capture("s:0", "rc=0");
    assert_eq!(env.ok(&["list-windows", "-t", "s:", "-F", "#{window_index}"]), "0\n1\n");
}

#[test]
fn shell_commands_are_bounded_and_end_with_the_server() {
    let env = Env::new();
    session(&env);
    // Output past 1 MiB is dropped, and the command is not held up by it.
    let o = env.run(&["run-shell", "head -c 3000000 /dev/zero | tr '\\0' x; echo; echo end >&2"]);
    assert!(o.status.success());
    assert_eq!(o.stdout.len(), 1024 * 1024 + 1, "1 MiB of it and the newline lif adds");
    // At most 64 at once: the next one is refused, and says so.
    for _ in 0..64 {
        env.ok(&["run-shell", "-b", "sleep 30"]);
    }
    let (code, _, err) = out(&env.run(&["run-shell", "true"]));
    assert_eq!(code, 1);
    assert_eq!(err, "64 shell commands are still running; not running true\n");
    // A command still running when the server stops is stopped too.
    let pidfile = env.home.join("job-pid");
    env.ok(&["kill-server"]);
    assert!(env.wait(secs(10), || !path_exists(&env.sock)));
    env.ok(&["new-session", "-d", "-s", "again"]);
    env.ok(&["run-shell", "-b", &format!("echo $$ > {}; exec sleep 300", pidfile.display())]);
    assert!(env.wait(secs(10), || std::fs::read_to_string(&pidfile).is_ok_and(|p| p.ends_with('\n'))));
    let pid: u32 = std::fs::read_to_string(&pidfile).unwrap().trim().parse().unwrap();
    assert!(pid_alive(pid));
    env.ok(&["kill-server"]);
    assert!(env.wait(secs(10), || !pid_alive(pid)), "the job outlived the server");
}
