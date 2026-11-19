//! Control mode (`lif -C`): command answers, notifications and pane
//! output, in tmux 3.8's text (its transcript of the same session is the
//! reference these lines were checked against).
mod common;

use std::io::{BufRead, BufReader, Write};
use std::process::{Child, ChildStdin, Stdio};
use std::sync::mpsc::{Receiver, channel};
use std::time::{Duration, Instant};

use common::*;

struct Ctl {
    child: Child,
    stdin: Option<ChildStdin>,
    lines: Receiver<String>,
    seen: Vec<String>,
}

impl Ctl {
    fn start(env: &Env, args: &[&str]) -> Ctl {
        let mut child = env.command().args(args).stdin(Stdio::piped()).stdout(Stdio::piped()).spawn().unwrap();
        let stdout = child.stdout.take().unwrap();
        let (tx, rx) = channel();
        std::thread::spawn(move || {
            for l in BufReader::new(stdout).lines().map_while(Result::ok) {
                if tx.send(l).is_err() {
                    break;
                }
            }
        });
        Ctl { stdin: child.stdin.take(), child, lines: rx, seen: Vec::new() }
    }

    fn send(&mut self, line: &str) {
        let s = self.stdin.as_mut().unwrap();
        s.write_all(line.as_bytes()).unwrap();
        s.write_all(b"\n").unwrap();
        s.flush().unwrap();
    }

    /// Lines until one that `pred` accepts (returned), within 10 s.
    fn until(&mut self, what: &str, pred: impl Fn(&str) -> bool) -> String {
        let end = Instant::now() + Duration::from_secs(10);
        loop {
            let left = end.saturating_duration_since(Instant::now());
            match self.lines.recv_timeout(left) {
                Ok(l) => {
                    self.seen.push(l.clone());
                    if pred(&l) {
                        return l;
                    }
                }
                Err(_) => panic!("no {what}; got:\n{}", self.seen.join("\n")),
            }
        }
    }

    /// The lines of the next answer, `%begin` to `%end`/`%error`, without
    /// those two (and whether it was an error).
    fn answer(&mut self) -> (Vec<String>, bool) {
        self.until("%begin", |l| l.starts_with("%begin "));
        let mut body = Vec::new();
        loop {
            let l = self.until("answer line", |l| !l.starts_with("%output "));
            if l.starts_with("%end ") || l.starts_with("%error ") {
                return (body, l.starts_with("%error "));
            }
            body.push(l);
        }
    }
}

#[test]
fn control_mode_answers_and_notifies() {
    let env = Env::new();
    env.ok(&["new-session", "-d", "-s", "a", "-x", "80", "-y", "24"]);
    let mut c = Ctl::start(&env, &["-C", "attach", "-t", "a"]);
    assert_eq!(c.answer(), (vec![], false), "the attach command's answer");
    assert_eq!(c.until("%session-changed", |l| l.starts_with("%session-changed")), "%session-changed $0 a");
    c.send("display-message -p hello");
    assert_eq!(c.answer(), (vec!["hello".to_string()], false));
    c.send("bogus-command");
    assert_eq!(c.answer(), (vec!["parse error: unknown command: bogus-command".to_string()], true));
    c.send("new-window -n two");
    assert_eq!(c.answer(), (vec![], false));
    assert_eq!(c.until("notification", |l| l.starts_with("%session-window")), "%session-window-changed $0 @1");
    assert_eq!(c.until("%window-add", |l| l.starts_with("%window-add")), "%window-add @1");
    c.send("rename-window -t :1 renamed");
    c.answer();
    assert_eq!(c.until("%window-renamed", |l| l.starts_with("%window-renamed")), "%window-renamed @1 renamed");
    // Pane output, escaped: control characters and backslash in octal.
    c.send("send-keys -t a:1 'printf \"x\\ty\\\\z\\n\" > /dev/null; printf \"x\\ty\\\\z\\n\"' Enter");
    c.answer();
    // (How the output is cut into %output lines depends on the reads.)
    c.until("the printed line", |l| l.starts_with("%output %1 ") && l.contains("x\\011y\\134z\\015\\012"));
    c.send("split-window -t a:1");
    c.answer();
    assert_eq!(c.until("%window-pane-changed", |l| l.starts_with("%window-pane-changed")), "%window-pane-changed @1 %2");
    let l = c.until("%layout-change", |l| l.starts_with("%layout-change"));
    assert!(l.starts_with("%layout-change @1 ") && l.contains("[80x12,0,0,1,80x11,0,13,2]") && l.ends_with(" *"), "{l}");
    c.send("kill-window -t a:1");
    c.answer();
    assert_eq!(c.until("%window-close", |l| l.starts_with("%window-close")), "%window-close @1");
    c.send("rename-session b");
    c.answer();
    assert_eq!(c.until("%session-renamed", |l| l.starts_with("%session-renamed")), "%session-renamed $0 b");
    // Its size counts once it has one, with no status line.
    c.send("refresh-client -C 100x30");
    c.answer();
    let l = c.until("%layout-change", |l| l.starts_with("%layout-change"));
    assert!(l.starts_with("%layout-change @0 ") && l.contains(",100x30,0,0,0 "), "{l}");
    // An empty line ends it.
    c.send("");
    c.until("%exit", |l| l == "%exit");
    assert!(c.child.wait().unwrap().success());
    assert_eq!(env.ok(&["list-clients"]), "");
}

/// A command that waits (run-shell) is answered when it is done; what
/// comes after it waits for it, in order.
#[test]
fn control_mode_answers_waiting_commands_in_order() {
    let env = Env::new();
    env.ok(&["new-session", "-d", "-s", "a"]);
    let mut c = Ctl::start(&env, &["-C", "attach", "-t", "a"]);
    c.answer();
    c.send("run-shell 'sleep 0.3; echo slow'");
    assert_eq!(c.answer(), (vec!["slow".to_string()], false));
    c.send("display-message -p fast");
    assert_eq!(c.answer(), (vec!["fast".to_string()], false));
    // Closing standard input detaches it too.
    drop(c.stdin.take());
    c.until("%exit", |l| l == "%exit");
    assert!(c.child.wait().unwrap().success());
}

/// A control client that does not read: the server keeps at most its
/// bound for it, pauses the pane's output for it, and the pane runs on.
#[test]
fn control_mode_backlog_is_bounded() {
    let env = Env::new();
    env.ok(&["new-session", "-d", "-s", "a"]);
    let mut c = Ctl::start(&env, &["-C", "attach", "-t", "a"]);
    c.answer();
    // Stop the client: it no longer reads its socket.
    // SAFETY: signalling our own child.
    unsafe { libc::kill(c.child.id() as i32, libc::SIGSTOP) };
    env.ok(&["send-keys", "-t", "a", "head -c 30000000 /dev/zero | tr '\\0' y; echo D''ONE", "Enter"]);
    env.wait_capture("a", "DONE");
    let rss = rss_kib(env.server_pid());
    assert!(rss < 120 * 1024, "the server holds {rss} KiB");
    // SAFETY: as above.
    unsafe { libc::kill(c.child.id() as i32, libc::SIGCONT) };
    c.until("%pause", |l| l == "%pause %0");
    c.until("%continue", |l| l == "%continue %0");
    c.send("");
    c.until("%exit", |l| l == "%exit");
}

/// -CC wraps the whole exchange in tmux's DCS (for terminals that speak
/// it); a client whose input is all there at once still gets every answer.
#[test]
fn control_mode_cc_wraps_in_dcs() {
    let env = Env::new();
    env.ok(&["new-session", "-d", "-s", "a"]);
    let mut child = env.command().args(["-CC", "attach", "-t", "a"]).stdin(Stdio::piped()).stdout(Stdio::piped()).spawn().unwrap();
    child.stdin.take().unwrap().write_all(b"display -p hi\n\n").unwrap();
    let out = child.wait_with_output().unwrap();
    assert!(out.status.success(), "{:?}", out.status);
    let s = String::from_utf8_lossy(&out.stdout);
    assert!(s.starts_with("\x1bP1000p%begin ") && s.ends_with("%exit\n\x1b\\") && s.contains("\nhi\n%end "), "{s:?}");
}
