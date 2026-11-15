//! The smaller commands added in 0.3: `pipe-pane`, `clock-mode`,
//! `resize-window`, `respawn-window`, the prompt history commands,
//! `select-pane -d` and `split-window -f`. Output as tmux 3.8's.
mod common;

use std::time::Duration;

use common::*;
use lif_types::Size;

fn session(env: &Env) {
    env.ok(&["new-session", "-d", "-s", "s", "-x", "80", "-y", "24"]);
    env.wait_capture("s:", "$");
}

fn show(env: &Env, target: &str, fmt: &str) -> String {
    env.ok(&["display-message", "-p", "-t", target, fmt]).trim_end().to_string()
}

#[test]
fn pipe_pane_copies_output_to_a_command() {
    let env = Env::new();
    session(&env);
    let log = env.home.join("pane.log");
    env.ok(&["pipe-pane", "-t", "s:", &format!("cat > {}", log.display())]);
    assert_eq!(show(&env, "s:", "#{pane_pipe}"), "1");
    env.ok(&["send-keys", "-t", "s:", "echo pi''ped-$((40+2))", "Enter"]);
    env.wait_capture("s:", "piped-42");
    assert!(env.wait(secs(10), || std::fs::read_to_string(&log).is_ok_and(|l| l.contains("piped-42"))), "{:?}", std::fs::read_to_string(&log));
    // Stopped: later output is not piped, and the command saw its input end.
    env.ok(&["pipe-pane", "-t", "s:"]);
    assert_eq!(show(&env, "s:", "#{pane_pipe}"), "0");
    env.ok(&["send-keys", "-t", "s:", "echo af''ter", "Enter"]);
    env.wait_capture("s:", "after");
    std::thread::sleep(Duration::from_millis(200));
    assert!(!std::fs::read_to_string(&log).unwrap().contains("after"));
    // -o toggles.
    env.ok(&["pipe-pane", "-o", "-t", "s:", "cat > /dev/null"]);
    assert_eq!(show(&env, "s:", "#{pane_pipe}"), "1");
    env.ok(&["pipe-pane", "-o", "-t", "s:", "cat > /dev/null"]);
    assert_eq!(show(&env, "s:", "#{pane_pipe}"), "0");
    // A command that exits closes the pipe.
    env.ok(&["pipe-pane", "-t", "s:", "true"]);
    env.ok(&["send-keys", "-t", "s:", "echo one; echo two", "Enter"]);
    assert!(env.wait(secs(10), || show(&env, "s:", "#{pane_pipe}") == "0"));
}

#[test]
fn pipe_pane_to_a_command_that_never_reads_does_not_hold_the_pane() {
    let env = Env::new();
    session(&env);
    env.ok(&["pipe-pane", "-t", "s:", "sleep 300"]);
    // 8 MB of output: far more than waits for the command (1 MiB).
    env.ok(&["send-keys", "-t", "s:", "head -c 8000000 /dev/zero | tr '\\0' x; echo; echo DO''NE", "Enter"]);
    env.wait_capture("s:", "DONE");
    assert_eq!(show(&env, "s:", "#{pane_pipe}"), "1");
    let rss = rss_kib(env.server_pid());
    assert!(rss < 200 * 1024, "the server holds {rss} KiB");
    // kill-server ends the command with the pane.
    env.ok(&["kill-server"]);
}

#[test]
fn clock_mode_shows_the_time_until_a_key() {
    let env = Env::new();
    session(&env);
    let mut t = Term::spawn(&env, &["attach", "-t", "s:"], Size::new(80, 24));
    t.wait_text("$");
    t.write(b"\x02t");
    t.wait_for("the clock", secs(10), |t| t.lines().iter().filter(|l| l.contains('#')).count() == 5);
    assert_eq!(show(&env, "s:", "#{pane_in_mode}"), "1");
    // Five rows of digits, centered: tmux's placement.
    let l = t.lines();
    let rows: Vec<&String> = l.iter().filter(|l| l.contains('#')).collect();
    assert!(rows.iter().all(|r| r.starts_with(&" ".repeat(25)) && r.trim_end().len() <= 54), "{rows:#?}");
    assert_eq!(l.iter().position(|x| x.contains('#')), Some(8));
    // 12-hour style adds AM or PM: more glyphs, so it starts further left.
    env.ok(&["set-option", "-g", "clock-mode-style", "12"]);
    t.wait_for("the 12-hour clock", secs(10), |t| t.lines().iter().any(|l| l.contains('#') && !l.starts_with(&" ".repeat(25))));
    t.write(b"x");
    assert!(env.wait(secs(10), || show(&env, "s:", "#{pane_in_mode}") == "0"));
    t.wait_for("the pane back", secs(10), |t| !t.screen().contains("#####"));
    // The key that ended the clock was not typed into the shell.
    assert!(!env.wait_capture("s:", "$").contains("$ x"));
}

#[test]
fn input_off_drops_what_is_typed() {
    let env = Env::new();
    session(&env);
    let mut t = Term::spawn(&env, &["attach", "-t", "s:"], Size::new(80, 24));
    t.wait_text("$");
    env.ok(&["select-pane", "-d", "-t", "s:"]);
    assert_eq!(show(&env, "s:", "#{pane_input_off}"), "1");
    t.type_str("echo dro''pped\r");
    // Prefix keys still work with the input off.
    t.write(b"\x02:");
    t.type_str("select-pane -e\r");
    assert!(env.wait(secs(10), || show(&env, "s:", "#{pane_input_off}") == "0"));
    t.type_str("echo ke''pt\r");
    t.wait_text("kept");
    assert!(!t.screen().contains("dropped"), "{}", t.screen());
}

#[test]
fn window_commands_and_prompt_history() {
    let env = Env::new();
    session(&env);
    env.ok(&["new-window", "-d", "-t", "s:", "-n", "two"]);
    env.ok(&["resize-window", "-t", "s:1", "-x", "100", "-y", "30"]);
    assert_eq!(show(&env, "s:1", "#{window_width}x#{window_height} #{pane_width}x#{pane_height}"), "100x30 100x30");
    let o = env.run(&["resize-window", "-t", "s:1", "-x", "0"]);
    assert_eq!(String::from_utf8_lossy(&o.stderr), "width too small\n");
    // split-window -f spans the window.
    env.ok(&["split-window", "-d", "-h", "-t", "s:1"]);
    env.ok(&["split-window", "-d", "-f", "-t", "s:1.0"]);
    assert_eq!(
        env.ok(&["list-panes", "-t", "s:1", "-F", "#{pane_left},#{pane_top} #{pane_width}x#{pane_height}"]),
        "0,0 50x15\n51,0 49x15\n0,16 100x14\n"
    );
    // respawn-window: back to one pane, a new program in it.
    let o = env.run(&["respawn-window", "-t", "s:1"]);
    assert_eq!(String::from_utf8_lossy(&o.stderr), "respawn window failed: window s:1 still active\n");
    env.ok(&["respawn-window", "-k", "-e", "WHO=again", "-t", "s:1", "echo \"respawned $WHO\"; sleep 30"]);
    env.wait_capture("s:1", "respawned again");
    assert_eq!(env.ok(&["list-panes", "-t", "s:1", "-F", "#{pane_width}x#{pane_height}"]), "100x30\n");
    // What is typed at the prompt is remembered; C-b / describes a key.
    let mut t = Term::spawn(&env, &["attach", "-t", "s:"], Size::new(80, 24));
    t.wait_text("$");
    assert_eq!(env.ok(&["show-prompt-history", "-T", "command"]), "History for command:\n\n\n");
    t.write(b"\x02:");
    t.type_str("rename-window typed\r");
    assert!(env.wait(secs(10), || show(&env, "s:0", "#{window_name}") == "typed"));
    assert_eq!(env.ok(&["show-prompt-history"]), "History for command:\n\n1: rename-window typed\n\nHistory for search:\n\n\n");
    env.ok(&["clear-prompt-history"]);
    assert_eq!(env.ok(&["show-prompt-history", "-T", "command"]), "History for command:\n\n\n");
    t.write(b"\x02/");
    t.wait_for("the key prompt", secs(10), |t| t.lines()[23].starts_with("key"));
    t.write(b"c");
    t.wait_text("C-b c Create a new window");
}
