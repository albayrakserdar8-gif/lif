//! What Claude Code needs from a terminal multiplexer: the tmux commands its
//! agent-teams backend runs (through the `tmux` shim), `load-buffer -w -`
//! for copying, `tmux-compat` (`$TMUX`) and tmux's passthrough wrapper,
//! and `set-titles`. The command sequences are the ones Claude Code 2.1
//! runs, with its arguments.
mod common;

use std::io::Write;
use std::path::{Path, PathBuf};
use std::process::{Command, Output, Stdio};

use common::*;
use lif_types::Size;
use lif_vt::{VtEngine, VtEvent};

/// `dir/tmux`, a link to the lif binary (what `install-tmux-shim` makes).
fn shim(env: &Env) -> PathBuf {
    let dir = env.dir.join("shim");
    std::fs::create_dir_all(&dir).unwrap();
    let link = dir.join("tmux");
    if !link.exists() {
        std::os::unix::fs::symlink(BIN, &link).unwrap();
    }
    link
}

/// Runs the shim as a program in pane `pane` of `env`'s server would:
/// with `$LIF`, `$TMUX` and `$TMUX_PANE` set.
fn tmux(env: &Env, pane: &str, args: &[&str]) -> Output {
    let mut c = Command::new(shim(env));
    for k in Env::REMOVE {
        c.env_remove(k);
    }
    let sock = env.sock.display().to_string();
    c.envs(env.vars())
        .env("LIF", format!("{sock},1,0"))
        .env("LIF_PANE", pane)
        .env("TMUX", format!("{sock},1,0"))
        .env("TMUX_PANE", pane)
        .args(args);
    c.output().expect("run the tmux shim")
}

fn tmux_ok(env: &Env, pane: &str, args: &[&str]) -> String {
    let o = tmux(env, pane, args);
    assert!(o.status.success(), "tmux {args:?} failed: {}", String::from_utf8_lossy(&o.stderr));
    String::from_utf8(o.stdout).unwrap()
}

#[test]
fn agent_team_panes_through_the_tmux_shim() {
    let env = Env::new();
    let mut t = Term::spawn(&env, &["new-session", "-s", "lead"], Size::new(100, 30));
    t.wait_text("$");
    let sock = env.sock.display().to_string();
    let s = ["-S", sock.as_str()];
    let v = tmux_ok(&env, "%0", &["-V"]);
    assert!(v.starts_with("tmux 3.8 (lif "), "{v}");
    assert_eq!(tmux_ok(&env, "%0", &[s[0], s[1], "show-options", "-g", "prefix"]), "prefix C-b\n");
    let window = tmux_ok(&env, "%0", &[s[0], s[1], "display-message", "-t", "%0", "-p", "#{window_id}"]);
    assert_eq!(window, "@0\n");
    let w = window.trim();
    // TmuxBackend.createTeammatePaneWithLeader, first teammate.
    let pane = tmux_ok(&env, "%0", &[s[0], s[1], "split-window", "-d", "-t", "%0", "-h", "-P", "-F", "#{pane_id}", "--", "cat"]);
    let p = pane.trim().to_string();
    assert_eq!(p, "%1");
    for (opt, val) in [
        ("window-style", "bg=default,fg=colour33"),
        ("pane-border-style", "fg=colour33"),
        ("pane-active-border-style", "fg=colour33"),
    ] {
        tmux_ok(&env, "%0", &[s[0], s[1], "set-option", "-p", "-t", &p, opt, val]);
    }
    tmux_ok(&env, "%0", &[s[0], s[1], "select-pane", "-t", &p, "-T", "researcher"]);
    tmux_ok(&env, "%0", &[s[0], s[1], "set-option", "-p", "-t", &p, "pane-border-format", "#[fg=colour33,bold] #{pane_title} #[default]"]);
    tmux_ok(&env, "%0", &[s[0], s[1], "set-option", "-w", "-t", w, "pane-border-status", "top"]);
    assert_eq!(tmux_ok(&env, "%0", &[s[0], s[1], "list-panes", "-t", w, "-F", "#{pane_id}"]), "%0\n%1\n");
    tmux_ok(&env, "%0", &[s[0], s[1], "select-layout", "-t", w, "main-vertical"]);
    tmux_ok(&env, "%0", &[s[0], s[1], "resize-pane", "-t", "%0", "-x", "30%"]);
    // sendCommandToPane: remain-on-exit, then respawn with the agent.
    tmux_ok(&env, "%0", &[s[0], s[1], "set-option", "-p", "-t", &p, "remain-on-exit", "failed"]);
    tmux_ok(&env, "%0", &[s[0], s[1], "respawn-pane", "-k", "-t", &p, "--", "echo teammate-running; sleep 30"]);
    env.wait_capture(&p, "teammate-running");
    t.wait_for("the teammate's title on its border line", secs(10), |t| t.lines()[0].contains(" researcher "));
    t.wait_text("teammate-running");
    // The leader keeps 30% of the width; the label row is above both panes.
    let geo = env.ok(&["list-panes", "-t", "lead", "-F", "#{pane_id} #{pane_left},#{pane_top} #{pane_width}x#{pane_height}"]);
    assert_eq!(geo, "%0 0,0 30x28\n%1 31,0 69x28\n");
    assert_eq!(
        tmux_ok(&env, "%0", &[s[0], s[1], "show-options", "-p", "-t", &p]),
        "pane-active-border-style fg=colour33\npane-border-format \"#[fg=colour33,bold] #{pane_title} #[default]\"\npane-border-style fg=colour33\nremain-on-exit failed\nwindow-style bg=default,fg=colour33\n"
    );
    // The teammate's text takes window-style's foreground.
    t.wait_for("teammate text in its color", secs(10), |t| {
        t.vt.refresh(&mut t.frame);
        let row = t.lines().iter().position(|l| l.contains("teammate-running"));
        row.is_some_and(|y| {
            let line = &t.lines()[y];
            let x = line[..line.find("teammate-running").unwrap()].chars().count();
            t.frame.rows[y].cells[x].style.fg == lif_types::Color::Indexed(33)
        })
    });
    assert!(tmux(&env, "%0", &[s[0], s[1], "kill-pane", "-t", &p]).status.success());
    t.wait_for("teammate pane gone", secs(10), |t| !t.screen().contains("teammate-running"));
    assert!(tmux(&env, "%0", &[s[0], s[1], "has-session", "-t", "lead"]).status.success());
}

#[test]
fn swarm_session_outside_the_leader_window() {
    // Not inside tmux: Claude Code makes its own server with -L and a
    // session of `cat` panes, tiled.
    let env = Env::new();
    env.ok(&["new-session", "-d", "-s", "lead"]);
    let label = format!("claude-swarm-test-{}", std::process::id());
    let l = ["-L", label.as_str()];
    let p0 = tmux_ok(&env, "%0", &[l[0], l[1], "new-session", "-d", "-s", "claude-swarm", "-n", "swarm-view", "-P", "-F", "#{pane_id}", "--", "cat"]);
    assert_eq!(p0, "%0\n");
    let p1 = tmux_ok(&env, "%0", &[l[0], l[1], "split-window", "-d", "-t", "%0", "-v", "-P", "-F", "#{pane_id}", "--", "cat"]);
    assert_eq!(p1, "%1\n");
    tmux_ok(&env, "%0", &[l[0], l[1], "select-layout", "-t", "claude-swarm:swarm-view", "tiled"]);
    assert_eq!(tmux_ok(&env, "%0", &[l[0], l[1], "list-panes", "-t", "claude-swarm:swarm-view", "-F", "#{pane_id}"]), "%0\n%1\n");
    assert!(tmux(&env, "%0", &[l[0], l[1], "kill-session", "-t", "claude-swarm"]).status.success());
}

#[test]
fn load_buffer_from_stdin_copies_to_the_clipboard() {
    let env = Env::new();
    let mut t = Term::spawn(&env, &["new-session", "-s", "c"], Size::new(80, 24));
    t.wait_text("$");
    let mut c = env.command();
    let mut child = c.args(["load-buffer", "-w", "-"]).stdin(Stdio::piped()).stdout(Stdio::piped()).spawn().unwrap();
    child.stdin.take().unwrap().write_all("copied ç text\n".as_bytes()).unwrap();
    assert!(child.wait_with_output().unwrap().status.success());
    t.wait_for("OSC 52 on the outer terminal", secs(10), |t| {
        t.events.iter().any(|e| matches!(e, VtEvent::ClipboardWrite { data, .. } if data == "copied ç text\n".as_bytes()))
    });
    assert_eq!(env.ok(&["save-buffer", "-"]), "copied ç text\n");
    let file = env.dir.join("saved");
    env.ok(&["save-buffer", file.to_str().unwrap()]);
    assert_eq!(std::fs::read_to_string(&file).unwrap(), "copied ç text\n");
    env.ok(&["load-buffer", "-b", "f", file.to_str().unwrap()]);
    assert_eq!(env.ok(&["show-buffer", "-b", "f"]), "copied ç text\n");
}

#[test]
fn tmux_compat_sets_tmux_and_unwraps_passthrough() {
    let env = Env::new();
    let mut t = Term::spawn(&env, &["new-session", "-s", "p"], Size::new(100, 24));
    t.wait_text("$");
    // Off by default: no TMUX.
    t.type_str("echo \"X=$TMUX.\"\r");
    t.wait_text("X=.");
    env.ok(&["set-option", "-s", "tmux-compat", "on"]);
    env.ok(&["new-window"]);
    t.wait_text("$");
    let sock = env.sock.display().to_string();
    t.type_str("echo \"X=$TMUX Y=$TMUX_PANE\"\r");
    t.wait_for("TMUX in the new pane", secs(10), |t| t.screen().contains(&format!("X={sock},")) && t.screen().contains("Y=%1"));
    // ST-terminated inside the wrapper: the inner ST is a doubled ESC too.
    t.type_str("printf '\\033Ptmux;\\033\\033]9;wrapped hi\\033\\033\\\\\\033\\\\'\r");
    t.wait_for("unwrapped OSC 9 on the outer terminal", secs(10), |t| {
        t.events.iter().any(|e| matches!(e, VtEvent::Notification { body, .. } if body == "wrapped hi"))
    });
    // The shim inside a compat pane finds the server through $TMUX alone.
    let mut c = Command::new(shim(&env));
    for k in Env::REMOVE {
        c.env_remove(k);
    }
    let o = c.envs(env.vars()).env("LIF", format!("{sock},1,0")).env("TMUX", format!("{sock},1,0")).env("TMUX_PANE", "%1")
        .args(["display-message", "-p", "#{pane_id} #{session_name}"]).output().unwrap();
    assert_eq!(String::from_utf8_lossy(&o.stdout), "%1 p\n", "{}", String::from_utf8_lossy(&o.stderr));
}

#[test]
fn allow_passthrough_per_pane() {
    let env = Env::new();
    let mut t = Term::spawn(&env, &["new-session", "-s", "a"], Size::new(100, 24));
    t.wait_text("$");
    env.ok(&["set-option", "-p", "-t", "%0", "allow-passthrough", "on"]);
    t.type_str("printf '\\033Ptmux;\\033\\033]52;c;aGk=\\007\\033\\\\'\r");
    t.wait_for("unwrapped OSC 52", secs(10), |t| {
        t.events.iter().any(|e| matches!(e, VtEvent::ClipboardWrite { data, .. } if data == b"hi"))
    });
}

#[test]
fn set_titles_sets_the_outer_title() {
    let env = Env::new();
    let mut t = Term::spawn(&env, &["new-session", "-s", "work"], Size::new(80, 24));
    t.wait_text("$");
    env.ok(&["set-option", "-g", "set-titles", "on"]);
    t.type_str("printf '\\033]2;editing notes\\007'\r");
    t.wait_for("outer title", secs(10), |t| {
        let title = t.vt.title();
        title.starts_with("work:0:") && title.ends_with(" - \"editing notes\"")
    });
    env.ok(&["set-option", "-g", "set-titles-string", "#S/#T"]);
    t.wait_for("outer title from the new format", secs(10), |t| t.vt.title() == "work/editing notes");
}

#[test]
fn shim_outside_lif_runs_the_real_tmux() {
    let env = Env::new();
    let link = shim(&env);
    let real_dir = env.dir.join("real");
    std::fs::create_dir_all(&real_dir).unwrap();
    let real = real_dir.join("tmux");
    std::fs::write(&real, "#!/bin/sh\necho \"real tmux $*\"\n").unwrap();
    use std::os::unix::fs::PermissionsExt;
    std::fs::set_permissions(&real, std::fs::Permissions::from_mode(0o755)).unwrap();
    let path = format!("{}:{}:/usr/bin:/bin", link.parent().unwrap().display(), real_dir.display());
    let run = |lif: Option<&str>| {
        let mut c = Command::new("tmux");
        for k in Env::REMOVE {
            c.env_remove(k);
        }
        c.envs(env.vars()).env("PATH", &path);
        if let Some(v) = lif {
            c.env("LIF", v);
        }
        let o = c.arg("-V").output().unwrap();
        String::from_utf8_lossy(&o.stdout).into_owned()
    };
    assert_eq!(run(None), "real tmux -V\n");
    assert_eq!(run(Some("")), "real tmux -V\n", "an empty LIF is outside lif");
    assert!(run(Some("/x,1,0")).starts_with("tmux 3.8 (lif "));
}

#[test]
fn install_tmux_shim_links_and_never_replaces_a_file() {
    let env = Env::new();
    let dir = env.dir.join("bin");
    let o = env.command().args(["install-tmux-shim", dir.to_str().unwrap()]).output().unwrap();
    assert!(o.status.success(), "{}", String::from_utf8_lossy(&o.stderr));
    let target = std::fs::read_link(dir.join("tmux")).unwrap();
    assert_eq!(target, std::fs::canonicalize(BIN).unwrap());
    // Again: the old link is replaced.
    assert!(env.command().args(["install-tmux-shim", dir.to_str().unwrap()]).output().unwrap().status.success());
    let other = env.dir.join("other");
    std::fs::create_dir_all(&other).unwrap();
    std::fs::write(other.join("tmux"), "mine").unwrap();
    let o = env.command().args(["install-tmux-shim", other.to_str().unwrap()]).output().unwrap();
    assert!(!o.status.success());
    assert_eq!(std::fs::read_to_string(other.join("tmux")).unwrap(), "mine");
    let _ = Path::new("");
}
