//! The configuration file end to end: start-up, diagnostics, reload, -f,
//! the binary's own config commands, and themes reaching the screen.
mod common;

use std::path::PathBuf;
use std::time::Duration;

use common::*;
use lif_types::{Color, Size};
use lif_vt::VtEngine;

fn write_config(env: &Env, name: &str, text: &str) -> PathBuf {
    let p = env.dir.join(name);
    std::fs::write(&p, text).unwrap();
    p
}

#[test]
fn config_file_applies_at_server_start() {
    let env = Env::new();
    let cfg = write_config(
        &env,
        "c.toml",
        "prefix = \"C-a\"\nbase-index = 1\n[status]\nposition = \"top\"\nleft = \"[CFG] \"\nleft-length = 20\n[keys]\n\"|\" = \"split-window -h\"\n",
    );
    let path = cfg.to_str().unwrap();
    let mut t = Term::spawn(&env, &["-f", path, "new-session", "-s", "c"], Size::new(80, 24));
    t.wait_text("$");
    assert_eq!(env.ok(&["show", "-gv", "prefix"]), "C-a\n");
    assert_eq!(env.ok(&["display", "-p", "#I"]), "1\n", "base-index");
    t.wait_for("status at the top", secs(10), |t| t.lines()[0].starts_with("[CFG] 1:"));
    // The new prefix and the custom binding work; C-b is an ordinary key.
    t.write(b"\x01|");
    t.wait_for("split", secs(10), |_| env.ok(&["list-panes"]).lines().count() == 2);
    t.write(b"\x02%");
    t.settle(Duration::from_millis(200));
    assert_eq!(env.ok(&["list-panes"]).lines().count(), 2, "C-b is not a prefix any more");
    assert!(env.ok(&["list-keys", "-T", "prefix"]).contains("C-a"));
    // #{pid}, #{socket_path} and #{version}.
    let pid = env.server_pid();
    assert_eq!(env.ok(&["display", "-p", "#{pid}"]), format!("{pid}\n"));
    assert_eq!(env.ok(&["display", "-p", "#{socket_path}"]), format!("{}\n", env.sock.display()));
    assert_eq!(env.ok(&["display", "-p", "#{version}"]), format!("{}\n", env!("CARGO_PKG_VERSION")));
}

#[test]
fn a_config_with_errors_still_starts_and_says_so() {
    let env = Env::new();
    let cfg = write_config(&env, "bad.toml", "mouze = true\nbase-index = 2\n[keys]\nx = \"slect-pane -L\"\n");
    let path = cfg.to_str().unwrap();
    let mut t = Term::spawn(&env, &["-f", path, "new-session", "-s", "b"], Size::new(250, 24));
    t.wait_for("first problem on the status line", secs(10), |t| {
        let l = &t.lines()[23];
        l.contains("bad.toml:1:1: error: unknown option `mouze`") && l.contains("(and 1 more, see show-messages)")
    });
    // What could be applied was.
    assert_eq!(env.ok(&["show", "-gv", "base-index"]), "2\n");
    let log = env.ok(&["show-messages"]);
    assert!(log.contains("bad.toml:1:1: error") && log.contains("bad.toml:4:5: error"), "{log}");
    // The message stays until a key is pressed.
    t.type_str("x");
    t.wait_for("message gone", secs(10), |t| !t.lines()[23].contains("mouze"));
}

#[test]
fn source_file_reloads_bindings_live() {
    let env = Env::new();
    let cfg = write_config(&env, "r.toml", "[keys]\nX = \"display-message one\"\n");
    let path = cfg.to_str().unwrap();
    let mut t = Term::spawn(&env, &["-f", path, "new-session", "-s", "r"], Size::new(80, 24));
    t.wait_text("$");
    t.write(b"\x02X");
    t.wait_for("one", secs(10), |t| t.lines()[23].contains("one"));
    std::fs::write(&cfg, "[keys]\nX = \"display-message two\"\n").unwrap();
    env.ok(&["source-file", path]);
    t.write(b"\x02X");
    t.wait_for("two", secs(10), |t| t.lines()[23].contains("two"));
    // Problems in the file are the command's error.
    std::fs::write(&cfg, "mouze = 1\n").unwrap();
    let o = env.run(&["source-file", path]);
    assert!(!o.status.success());
    assert!(String::from_utf8_lossy(&o.stderr).contains("r.toml:1:1: error: unknown option `mouze`"));
    // A tmux file gets pointed at the translator.
    let conf = write_config(&env, "x.tmux.conf", "set -g mouse on\n");
    let o = env.run(&["source-file", conf.to_str().unwrap()]);
    assert!(!o.status.success());
    assert!(String::from_utf8_lossy(&o.stderr).contains("lif migrate-tmux-conf"), "{o:?}");
    assert!(env.run(&["source-file", "-q", "/no/such/file.toml"]).status.success());
}

#[test]
fn dash_f_reaches_a_new_server_only() {
    let env = Env::new();
    let one = write_config(&env, "one.toml", "base-index = 1\n");
    let two = write_config(&env, "two.toml", "base-index = 5\n");
    env.ok(&["-f", one.to_str().unwrap(), "new-session", "-d", "-s", "a"]);
    assert_eq!(env.ok(&["show", "-gv", "base-index"]), "1\n");
    // A running server keeps its configuration, as tmux does.
    env.ok(&["-f", two.to_str().unwrap(), "new-session", "-d", "-s", "b"]);
    assert_eq!(env.ok(&["show", "-gv", "base-index"]), "1\n");
    // A missing -f file is reported, and the server still runs.
    env.ok(&["kill-server"]);
    env.ok(&["-f", "/no/such/lif.toml", "new-session", "-d", "-s", "c"]);
    assert!(env.ok(&["show-messages"]).contains("/no/such/lif.toml: No such file or directory"));
}

#[test]
fn default_config_location_is_read() {
    let env = Env::new();
    let dir = env.home.join(".config/lif");
    std::fs::create_dir_all(&dir).unwrap();
    std::fs::write(dir.join("config.toml"), "base-index = 3\n").unwrap();
    env.ok(&["new-session", "-d", "-s", "a"]);
    assert_eq!(env.ok(&["show", "-gv", "base-index"]), "3\n");
    assert_eq!(env.ok(&["show-messages"]), "");
}

#[test]
fn check_print_and_migrate_config() {
    let env = Env::new();
    let good = write_config(&env, "good.toml", "mouse = false\n");
    let o = env.run(&["check-config", good.to_str().unwrap()]);
    assert!(o.status.success());
    assert_eq!(o.stdout, b"");
    let bad = write_config(&env, "bad.toml", "mouze = true\n[theme]\nborder = { fg = \"rd\" }\n");
    let o = env.run(&["check-config", bad.to_str().unwrap()]);
    assert_eq!(o.status.code(), Some(1));
    let b = bad.display();
    assert_eq!(
        String::from_utf8_lossy(&o.stdout),
        format!(
            "{b}:1:1: error: unknown option `mouze`, did you mean `mouse`?\n{b}:3:17: error: `theme.border`: unknown color \"rd\", did you mean \"red\"?\n"
        )
    );
    assert_eq!(env.run(&["check-config", "/no/such.toml"]).status.code(), Some(1));
    // The default location, with no file there.
    assert_eq!(env.run(&["check-config"]).status.code(), Some(1));
    // None of these need (or start) a server.
    assert!(env.server_pids().is_empty());
    let o = env.run(&["print-config"]);
    assert!(o.status.success());
    let text = String::from_utf8(o.stdout).unwrap();
    assert!(text.starts_with("# lif configuration file"));
    let example = write_config(&env, "example.toml", &text);
    assert!(env.run(&["check-config", example.to_str().unwrap()]).status.success());
    // migrate-tmux-conf: TOML on stdout, what was left out on stderr.
    std::fs::write(env.home.join(".tmux.conf"), "set -g mouse off\nset -g default-terminal tmux-256color\nbind | split-window -h\n").unwrap();
    let o = env.run(&["migrate-tmux-conf"]);
    assert!(o.status.success());
    let toml = String::from_utf8_lossy(&o.stdout);
    assert!(toml.contains("mouse = false") && toml.contains("\"|\" = \"split-window -h\""), "{toml}");
    let err = String::from_utf8_lossy(&o.stderr);
    assert!(err.contains(".tmux.conf:2: not translated (lif has no option default-terminal)"), "{err}");
    assert!(env.server_pids().is_empty());
}

#[test]
fn theme_changes_the_border_colors_on_screen() {
    let env = Env::new();
    let cfg = write_config(&env, "t.toml", "[theme]\nborder = { fg = \"blue\" }\nactive-border = \"fg=red\"\n");
    let mut t = Term::spawn(&env, &["-f", cfg.to_str().unwrap(), "new-session", "-s", "t"], Size::new(80, 24));
    t.wait_text("$");
    env.ok(&["split-window", "-h"]);
    t.wait_for("border", secs(10), |t| t.lines()[5].chars().nth(40) == Some('│'));
    t.settle(Duration::from_millis(100));
    t.vt.refresh(&mut t.frame);
    let colors: Vec<Color> = (0..23).map(|y| t.frame.rows[y].cells[40].style.fg).collect();
    assert!(colors.contains(&Color::Indexed(1)), "active border red: {colors:?}");
    assert!(colors.contains(&Color::Indexed(4)), "other border blue: {colors:?}");
    // Changing the option at runtime redraws in the new colors.
    env.ok(&["set", "-g", "pane-active-border-style", "fg=yellow"]);
    t.wait_for("yellow", secs(10), |t| {
        t.vt.refresh(&mut t.frame);
        (0..23).any(|y| t.frame.rows[y].cells[40].style.fg == Color::Indexed(3))
    });
    // message-style colors messages.
    env.ok(&["set", "-g", "message-style", "bg=blue"]);
    t.write(b"\x02:");
    t.type_str("display-message hi\r");
    t.wait_for("blue message", secs(10), |t| {
        t.vt.refresh(&mut t.frame);
        t.lines()[23].starts_with("hi") && t.frame.rows[23].cells[0].style.bg == Color::Indexed(4)
    });
}

#[test]
fn capture_pane_keeps_spaces_and_escapes_on_request() {
    let env = Env::new();
    let mut t = Term::spawn(&env, &["new-session", "-s", "k"], Size::new(80, 24));
    t.wait_text("$");
    t.type_str("printf '\\033[31mred\\033[0m  x   \\n'\r");
    t.wait_text("red  x");
    let plain = env.ok(&["capture-pane", "-p"]);
    assert!(plain.lines().any(|l| l == "red  x"), "{plain:?}");
    let keep = env.ok(&["capture-pane", "-p", "-N"]);
    assert!(keep.lines().any(|l| l.starts_with("red  x   ")), "{keep:?}");
    let esc = env.ok(&["capture-pane", "-p", "-e"]);
    let line = esc.lines().find(|l| l.contains("red") && !l.contains("printf")).unwrap();
    assert!(line.contains("\x1b[") && (line.contains("31") || line.contains("38;5;1")), "{line:?}");
    assert!(!plain.contains('\x1b'));
}
