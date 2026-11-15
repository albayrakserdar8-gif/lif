//! Copy mode end to end: keys, mouse, search, clipboard, paste, pager.
mod common;

use std::time::{Duration, Instant};

use common::*;
use lif_types::{Color, Size, Style};
use lif_vt::{VtEngine, VtEvent};

fn session(env: &Env, name: &str) -> Term {
    let mut t = Term::spawn(env, &["new-session", "-s", name], Size::new(80, 24));
    t.wait_text("$");
    t
}

/// The outer terminal's style of the cell at (x, y).
fn style_at(t: &mut Term, x: usize, y: usize) -> Style {
    t.vt.refresh(&mut t.frame);
    t.frame.rows[y].cells[x].style
}

fn row_of(t: &Term, needle: &str) -> Option<usize> {
    t.lines().iter().position(|l| l.contains(needle))
}

fn in_mode(t: &mut Term) -> bool {
    t.lines()[0].trim_end().ends_with(']')
}

#[test]
fn vi_select_copy_osc52_and_paste() {
    let env = Env::new();
    let mut t = session(&env, "c");
    env.ok(&["set", "-g", "mode-keys", "vi"]);
    t.type_str("echo alpha beta''gamma\r");
    t.wait_text("alpha betagamma");
    t.wait_for("prompt", secs(10), |t| t.lines()[2].starts_with('$'));
    t.write(b"\x02[");
    t.wait_for("copy mode", secs(10), in_mode);
    // From the prompt line: up to the output, start of line, next word,
    // select to its end, copy and leave.
    t.write(b"k0w");
    t.write(b" ");
    t.write(b"e");
    t.write(b"\r");
    t.wait_for("copy mode left", secs(10), |t| !in_mode(t));
    assert_eq!(env.ok(&["show-buffer"]), "betagamma\n");
    // The plain xterm-256color terminal got the OSC 52 copy too.
    t.wait_for("OSC 52 on the outer terminal", secs(10), |t| {
        t.events.iter().any(|e| matches!(e, VtEvent::ClipboardWrite { data, .. } if data == b"betagamma"))
    });
    // prefix ] pastes it into the shell.
    t.type_str("echo X");
    t.write(b"\x02]");
    t.type_str("Y\r");
    t.wait_text("XbetagammaY");
}

#[test]
fn emacs_keys_select_and_copy() {
    let env = Env::new();
    let mut t = session(&env, "e");
    assert_eq!(env.ok(&["show", "-gv", "mode-keys"]), "emacs\n");
    t.type_str("echo one two''three\r");
    t.wait_text("one twothree");
    t.wait_for("prompt", secs(10), |t| t.lines()[2].starts_with('$'));
    t.write(b"\x02[");
    t.wait_for("copy mode", secs(10), in_mode);
    // C-p up, C-a start, M-f to the end of "one", C-f past the space,
    // C-Space to start selecting, C-e to the end of the line, M-w to copy.
    t.write(b"\x10\x01\x1bf\x06");
    t.write(b"\x00");
    t.write(b"\x05");
    t.write(b"\x1bw");
    t.wait_for("copy mode left", secs(10), |t| !in_mode(t));
    assert_eq!(env.ok(&["show-buffer"]), "twothree\n");
}

#[test]
fn search_backward_next_previous_and_highlight() {
    let env = Env::new();
    let mut t = session(&env, "s");
    env.ok(&["set", "-g", "mode-keys", "vi"]);
    t.type_str("i=0; while [ $i -lt 120 ]; do if [ $i = 30 ] || [ $i = 90 ]; then echo needle$i; else echo hay$i; fi; i=$((i+1)); done\r");
    t.wait_text("hay119");
    t.write(b"\x02[");
    t.wait_for("copy mode", secs(10), in_mode);
    t.write(b"?");
    t.wait_for("search prompt", secs(10), |t| t.lines()[23].starts_with("(search up)"));
    t.type_str("needle\r");
    t.wait_for("needle90 found", secs(10), |t| row_of(t, "needle90").is_some());
    // The current match is drawn in copy-mode-current-match-style
    // (magenta), on the outer terminal's cells.
    let y = row_of(&t, "needle90").unwrap();
    t.wait_for("current match highlighted", secs(10), |t| style_at(t, 0, y).bg == Color::Indexed(5));
    assert_eq!(style_at(&mut t, 5, y).bg, Color::Indexed(5), "the whole match");
    assert_eq!(style_at(&mut t, 6, y).bg, Color::Default, "and no more");
    // n: the next match up; N: back down.
    t.write(b"n");
    t.wait_for("needle30", secs(10), |t| row_of(t, "needle30").is_some_and(|y| style_at(t, 0, y).bg == Color::Indexed(5)));
    t.write(b"N");
    t.wait_for("needle90 again", secs(10), |t| row_of(t, "needle90").is_some_and(|y| style_at(t, 0, y).bg == Color::Indexed(5)));
    // Other matches in view are drawn in copy-mode-match-style (cyan).
    env.ok(&["set", "-g", "copy-mode-match-style", "bg=blue"]);
    t.write(b"q");
    t.wait_for("left", secs(10), |t| !in_mode(t));
}

#[test]
fn mouse_drag_selects_and_copies() {
    let env = Env::new();
    let mut t = session(&env, "m");
    t.type_str("echo drag''me please\r");
    t.wait_text("dragme please");
    t.wait_for("mouse on", secs(10), |t| t.modes().mouse_tracking);
    let y = row_of(&t, "dragme please").unwrap() + 1;
    // Press on "d", drag past the "e" of "dragme" (with emacs keys the
    // end under the pointer is exclusive, as in tmux), release (SGR,
    // 1-based columns).
    t.write(format!("\x1b[<0;1;{y}M\x1b[<32;3;{y}M\x1b[<32;7;{y}M\x1b[<0;7;{y}m").as_bytes());
    t.wait_for("selection copied", secs(10), |_| env.ok(&["show-buffer"]) == "dragme\n");
    // tmux's default MouseDragEnd1Pane (copy-pipe-and-cancel): copy mode
    // is left once the drag ends.
    t.wait_for("copy mode left", secs(10), |t| !in_mode(t));
    assert_eq!(env.ok(&["display", "-p", "#{pane_in_mode}"]), "0\n");
    // mouse-drag-end copy-pipe-no-clear: copy and stay.
    env.ok(&["set", "-g", "mouse-drag-end", "copy-pipe-no-clear"]);
    t.write(format!("\x1b[<0;8;{y}M\x1b[<32;10;{y}M\x1b[<32;14;{y}M\x1b[<0;14;{y}m").as_bytes());
    t.wait_for("second selection copied", secs(10), |_| env.ok(&["show-buffer"]) == "please\n");
    t.wait_for("copy mode shown", secs(10), in_mode);
    t.write(b"q");
    t.wait_for("left", secs(10), |t| !in_mode(t));
    // none: select only.
    env.ok(&["set", "-g", "mouse-drag-end", "none"]);
    t.write(format!("\x1b[<0;1;{y}M\x1b[<32;3;{y}M\x1b[<32;7;{y}M\x1b[<0;7;{y}m").as_bytes());
    t.wait_for("copy mode shown", secs(10), in_mode);
    t.settle(Duration::from_millis(100));
    assert_eq!(env.ok(&["show-buffer"]), "please\n", "nothing new copied");
    t.write(b"q");
    t.wait_for("left", secs(10), |t| !in_mode(t));
}

/// tmux's DoubleClick1Pane / TripleClick1Pane outside copy mode: select the
/// word / line, copy it, leave copy mode.
#[test]
fn double_and_triple_click_copy_word_and_line() {
    let env = Env::new();
    let mut t = session(&env, "d");
    t.type_str("echo one two''x three\r");
    t.wait_text("one twox three");
    t.wait_for("mouse on", secs(10), |t| t.modes().mouse_tracking);
    let y = row_of(&t, "one twox three").unwrap() + 1;
    // Double click on "twox" (column 6, 1-based).
    t.write(format!("\x1b[<0;6;{y}M\x1b[<0;6;{y}m\x1b[<0;6;{y}M\x1b[<0;6;{y}m").as_bytes());
    t.wait_for("word copied", secs(10), |_| env.run(&["show-buffer"]).stdout == b"twox\n");
    t.wait_for("copy mode left", secs(10), |t| !in_mode(t));
    assert_eq!(env.ok(&["display", "-p", "#{pane_in_mode}"]), "0\n");
    // A third click soon after: the whole line.
    t.write(format!("\x1b[<0;6;{y}M\x1b[<0;6;{y}m").as_bytes());
    t.wait_for("line copied", secs(10), |_| env.run(&["show-buffer"]).stdout == b"one twox three\n");
    assert_eq!(env.ok(&["display", "-p", "#{pane_in_mode}"]), "0\n");
    // Clicks far apart in time are single clicks: nothing copied.
    let buffers = || env.ok(&["list-buffers"]).lines().count();
    let before = buffers();
    t.write(format!("\x1b[<0;2;{y}M\x1b[<0;2;{y}m").as_bytes());
    std::thread::sleep(Duration::from_millis(400));
    t.write(format!("\x1b[<0;2;{y}M\x1b[<0;2;{y}m").as_bytes());
    t.settle(Duration::from_millis(150));
    assert_eq!(buffers(), before, "no new buffer");
    // A program that takes the mouse gets the clicks instead.
    t.type_str("printf '\\033[?1000h'; sleep 2; printf '\\033[?1000l'\r");
    t.wait_for("program has the mouse", secs(10), |t| t.modes().mouse_tracking);
    std::thread::sleep(Duration::from_millis(300));
    t.write(format!("\x1b[<0;6;{y}M\x1b[<0;6;{y}m\x1b[<0;6;{y}M\x1b[<0;6;{y}m").as_bytes());
    t.settle(Duration::from_millis(150));
    assert_eq!(env.ok(&["display", "-p", "#{pane_in_mode}"]), "0\n");
    assert_eq!(buffers(), before, "no new buffer");
}

/// copy-pipe runs its command with the copied text on stdin, without
/// blocking the server, and the copy-command option serves copy-pipe
/// without a command (the default Enter binding, mouse copies).
#[test]
fn copy_pipe_runs_the_command() {
    let env = Env::new();
    env.ok(&["new-session", "-d", "-s", "P", "-x", "80", "-y", "24"]);
    env.ok(&["set", "-g", "mode-keys", "vi"]);
    env.wait_capture("P", "$");
    env.ok(&["send-keys", "-t", "P", "echo pipe''d text", "Enter"]);
    env.wait_capture("P", "piped text");
    let read = |name: &str| std::fs::read_to_string(env.home.join(name)).unwrap_or_default();
    let wait_file = |name: &str, want: &str| {
        let mut got = String::new();
        let ok = env.wait(secs(10), || {
            got = read(name);
            got == want
        });
        assert!(ok, "{name}: {got:?}, want {want:?}");
    };
    let to = |name: &str| format!("cat > '{}'", env.home.join(name).display());
    // send-keys -X copy-pipe-and-cancel with a command.
    env.ok(&["copy-mode", "-t", "P"]);
    env.ok(&["send-keys", "-t", "P", "-X", "cursor-up"]);
    env.ok(&["send-keys", "-t", "P", "-X", "select-line"]);
    env.ok(&["send-keys", "-t", "P", "-X", "copy-pipe-and-cancel", &to("a.out")]);
    wait_file("a.out", "piped text\n");
    assert_eq!(env.ok(&["show-buffer"]), "piped text\n", "the buffer gets it too");
    assert_eq!(env.ok(&["display", "-p", "-t", "P", "#{pane_in_mode}"]), "0\n");
    // A key bound to copy-pipe with a command, pressed through send-keys.
    env.ok(&["bind", "-T", "copy-mode-vi", "y", "send-keys", "-X", "copy-pipe-and-cancel", &to("b.out")]);
    env.ok(&["copy-mode", "-t", "P"]);
    env.ok(&["send-keys", "-t", "P", "k", "0", "Space", "e"]);
    env.ok(&["send-keys", "-t", "P", "y"]);
    wait_file("b.out", "piped");
    assert_eq!(env.ok(&["display", "-p", "-t", "P", "#{pane_in_mode}"]), "0\n");
    // copy-command for the default Enter (copy-pipe-and-cancel).
    env.ok(&["set", "-s", "copy-command", &to("c.out")]);
    env.ok(&["copy-mode", "-t", "P"]);
    env.ok(&["send-keys", "-t", "P", "k", "0", "w", "Space", "$", "Enter"]);
    wait_file("c.out", "text");
    // copy-selection does not pipe.
    env.ok(&["set", "-s", "copy-command", &to("d.out")]);
    env.ok(&["copy-mode", "-t", "P"]);
    env.ok(&["send-keys", "-t", "P", "-X", "cursor-up"]);
    env.ok(&["send-keys", "-t", "P", "-X", "select-line"]);
    env.ok(&["send-keys", "-t", "P", "-X", "copy-selection-and-cancel"]);
    std::thread::sleep(Duration::from_millis(300));
    assert!(!env.home.join("d.out").exists());
    // A big copy into a command that does not read for a while: the server
    // keeps answering, and the command gets every byte in the end.
    env.ok(&["set", "-s", "copy-command", ""]);
    env.ok(&["send-keys", "-t", "P", "clear; yes 'a line of text that pads the copy out' | head -3000; echo big''done", "Enter"]);
    env.wait_capture("P", "bigdone");
    env.ok(&["copy-mode", "-t", "P"]);
    env.ok(&["send-keys", "-t", "P", "-X", "history-top"]);
    env.ok(&["send-keys", "-t", "P", "-X", "begin-selection"]);
    env.ok(&["send-keys", "-t", "P", "-X", "history-bottom"]);
    env.ok(&["send-keys", "-t", "P", "-X", "end-of-line"]);
    let t0 = Instant::now();
    env.ok(&["send-keys", "-t", "P", "-X", "copy-pipe-and-cancel", &format!("sleep 1; wc -c > '{}'", env.home.join("e.out").display())]);
    env.ok(&["display", "-p", "x"]);
    assert!(t0.elapsed() < Duration::from_millis(700), "the server waited for the pipe: {:?}", t0.elapsed());
    let len = env.ok(&["show-buffer"]).len();
    assert!(len > 64 * 1024, "a copy bigger than a pipe buffer: {len}");
    let mut got = String::new();
    let ok = env.wait(secs(10), || {
        got = read("e.out");
        got.trim().parse::<usize>().ok().is_some_and(|n| n + 1 >= len)
    });
    assert!(ok, "e.out: {got:?}, buffer {len} bytes");
}

#[test]
fn wheel_scrolls_and_bottom_exits() {
    let env = Env::new();
    let mut t = session(&env, "w");
    t.type_str("i=0; while [ $i -lt 60 ]; do echo w$i; i=$((i+1)); done\r");
    t.wait_text("w59");
    t.wait_for("mouse on", secs(10), |t| t.modes().mouse_tracking);
    for _ in 0..3 {
        t.write(b"\x1b[<64;10;10M");
    }
    t.wait_for("scrolled 15 lines", secs(10), |t| t.lines()[0].contains("[15/"));
    t.write(b"\x1b[<65;10;10M");
    t.wait_for("down 5", secs(10), |t| t.lines()[0].contains("[10/"));
    t.write(b"\x1b[<65;10;10M\x1b[<65;10;10M");
    t.wait_for("bottom leaves copy mode", secs(10), |t| !in_mode(t) && t.screen().contains("w59"));
}

#[test]
fn output_during_copy_mode_does_not_move_the_view() {
    let env = Env::new();
    let mut t = session(&env, "o");
    t.type_str("i=0; while [ $i -lt 60 ]; do echo v$i; i=$((i+1)); done\r");
    t.wait_text("v59");
    // Output that starts once copy mode is open.
    t.type_str("sleep 3; i=0; while [ $i -lt 40 ]; do echo late$i; i=$((i+1)); done\r");
    t.write(b"\x02[");
    t.wait_for("copy mode", secs(10), in_mode);
    t.settle(Duration::from_millis(100));
    let offset = |t: &Term| -> usize {
        let l = &t.lines()[0];
        let i = l.rfind('[').unwrap();
        l[i + 1..].split('/').next().unwrap().parse().unwrap()
    };
    let start = offset(&t);
    t.write(b"\x1b[5~");
    t.wait_for("paged up", secs(10), |t| offset(t) == start + 21);
    // The last row is the status line, whose clock may tick between the two
    // looks; compare the pane's rows only.
    let pane_rows = |t: &Term| -> Vec<String> {
        let l = t.lines();
        l[1..l.len() - 1].to_vec()
    };
    let before = pane_rows(&t);
    env.wait_capture("o", "late39");
    t.settle(Duration::from_millis(200));
    assert_eq!(pane_rows(&t), before, "the view stayed on its rows");
    // The rows that arrived are counted below the view.
    assert!(offset(&t) >= start + 21 + 40, "{}", t.lines()[0]);
    t.write(b"q");
    t.wait_for("live again", secs(10), |t| t.screen().contains("late39"));
}

#[test]
fn send_keys_x_from_the_command_line() {
    let env = Env::new();
    let mut t = session(&env, "x");
    t.type_str("i=0; while [ $i -lt 40 ]; do echo s$i; i=$((i+1)); done\r");
    t.wait_text("s39");
    let o = env.run(&["send-keys", "-X", "history-top"]);
    assert!(!o.status.success());
    assert_eq!(String::from_utf8_lossy(&o.stderr), "not in a mode\n");
    env.ok(&["copy-mode", "-t", "x"]);
    env.ok(&["send-keys", "-t", "x", "-X", "history-top"]);
    t.wait_for("top", secs(10), |t| t.lines()[0].starts_with("s0") || t.lines()[1].starts_with("s0"));
    env.ok(&["send-keys", "-t", "x", "-X", "-N", "2", "cursor-down"]);
    env.ok(&["send-keys", "-t", "x", "-X", "select-line"]);
    env.ok(&["send-keys", "-t", "x", "-X", "copy-selection-and-cancel"]);
    t.wait_for("left", secs(10), |t| !in_mode(t));
    let buf = env.ok(&["show-buffer"]);
    assert!(buf.starts_with('s') && buf.ends_with('\n'), "{buf:?}");
    let o = env.run(&["send-keys", "-X", "no-such-command"]);
    assert!(!o.status.success());
}

#[test]
fn long_command_output_opens_a_pager() {
    let env = Env::new();
    let mut t = session(&env, "p");
    t.write(b"\x02?");
    t.wait_for("list-keys in view mode", secs(10), |t| {
        t.lines()[0].starts_with("bind-key") && in_mode(t) && t.lines()[22].starts_with("bind-key")
    });
    // It scrolls like copy mode, and q leaves.
    t.write(b"\x1b[6~");
    t.wait_for("paged", secs(10), |t| !t.lines()[0].contains("[0/"));
    t.write(b"q");
    t.wait_for("pane back", secs(10), |t| t.lines()[0].starts_with('$') && !in_mode(t));
    // A one-line output is a message.
    t.write(b"\x02:");
    t.type_str("display -p hello\r");
    t.wait_for("message", secs(10), |t| t.lines()[23].contains("hello"));
}

#[test]
fn search_through_a_huge_history_keeps_everything_live() {
    let env = Env::new();
    // 300k rows of history (both limits are raised before the pane exists).
    let mut t = Term::spawn(
        &env,
        &["set", "-g", "scrollback-bytes", "300000000", ";", "set", "-g", "history-limit", "400000", ";", "new-session", "-s", "h"],
        Size::new(80, 24),
    );
    t.wait_text("$");
    env.ok(&["set", "-g", "mode-keys", "vi"]);
    t.type_str("seq 1 300000; echo seq''done\r");
    let end = Instant::now() + secs(60);
    while !env.run(&["capture-pane", "-p", "-t", "h"]).stdout.windows(7).any(|w| w == b"seqdone") {
        assert!(Instant::now() < end, "seq did not finish");
        t.pump(Duration::from_millis(50));
    }
    t.settle(Duration::from_millis(100));
    env.ok(&["split-window", "-d", "-t", "h"]);
    t.write(b"\x02[");
    t.wait_for("copy mode", secs(10), in_mode);
    t.write(b"?");
    t.wait_for("search prompt", secs(10), |t| t.lines()[23].contains("(search up)"));
    // A pattern that is nowhere: the search reads the whole history.
    t.type_str("zzz\r");
    let started = Instant::now();
    // While it runs, the server answers commands and the other pane works.
    let (mut slowest, mut answered) = (Duration::ZERO, 0);
    let mut done = None;
    while done.is_none() && started.elapsed() < secs(120) {
        let t0 = Instant::now();
        env.ok(&["send-keys", "-t", "h.1", &format!("echo other{answered}"), "Enter"]);
        slowest = slowest.max(t0.elapsed());
        answered += 1;
        t.pump(Duration::from_millis(20));
        if t.screen().contains("No match") || t.lines()[23].contains("zzz") {
            done = Some(started.elapsed());
        }
    }
    let took = done.expect("the search never finished");
    env.wait_capture("h.1", &format!("other{}", answered - 1));
    eprintln!("searched 300k rows in {took:?}; {answered} commands answered meanwhile, slowest {slowest:?}");
    assert!(answered >= 2, "commands must run during the search");
    assert!(slowest < Duration::from_millis(500), "a command took {slowest:?} during the search");
    // A key press cancels a search: start another and leave at once.
    t.write(b"n");
    t.write(b"q");
    t.wait_for("copy mode left", secs(20), |t| !in_mode(t));
}

#[test]
fn copy_mode_flags_and_prefix_page_up() {
    let env = Env::new();
    let mut t = session(&env, "f");
    t.type_str("i=0; while [ $i -lt 80 ]; do echo f$i; i=$((i+1)); done\r");
    t.wait_text("f79");
    // prefix PPage is copy-mode -u: a page up at once.
    t.write(b"\x02\x1b[5~");
    t.wait_for("a page up", secs(10), |t| t.lines()[0].contains("[21/"));
    // copy-mode -q leaves.
    env.ok(&["copy-mode", "-q"]);
    t.wait_for("left", secs(10), |t| !in_mode(t) && t.screen().contains("f79"));
    // -H: no position indicator.
    env.ok(&["copy-mode", "-H"]);
    env.ok(&["send-keys", "-X", "halfpage-up"]);
    t.wait_for("scrolled without indicator", secs(10), |t| !t.screen().contains("f79") && !in_mode(t));
    assert_eq!(env.ok(&["display", "-p", "#{pane_in_mode}"]), "1\n");
    env.ok(&["copy-mode", "-q"]);
    // -e: scrolling back to the bottom leaves.
    env.ok(&["copy-mode", "-e"]);
    env.ok(&["send-keys", "-X", "-N", "3", "scroll-up"]);
    t.wait_for("scrolled", secs(10), |t| t.lines()[0].contains("[3/"));
    env.ok(&["send-keys", "-X", "-N", "3", "scroll-down"]);
    t.wait_for("bottom leaves", secs(10), |t| !in_mode(t));
    assert_eq!(env.ok(&["display", "-p", "#{pane_in_mode}"]), "0\n");
    // -d: a page down (tmux's PageDown recipe is copy-mode -ed).
    env.ok(&["copy-mode", "-u"]);
    env.ok(&["copy-mode", "-u"]);
    t.wait_for("two pages up", secs(10), |t| t.lines()[0].contains("[42/"));
    env.ok(&["copy-mode", "-d"]);
    t.wait_for("a page down", secs(10), |t| t.lines()[0].contains("[21/"));
    env.ok(&["copy-mode", "-ed"]);
    t.wait_for("bottom leaves", secs(10), |t| !in_mode(t));
    assert_eq!(env.ok(&["display", "-p", "#{pane_in_mode}"]), "0\n");
}

/// send-keys with key names drives a pane in copy mode through the mode's
/// key table (tmux 3.8), from the command line with no client attached.
#[test]
fn send_keys_drives_copy_mode_from_the_command_line() {
    let env = Env::new();
    env.ok(&["new-session", "-d", "-s", "S", "-x", "80", "-y", "24"]);
    env.ok(&["set", "-g", "mode-keys", "vi"]);
    env.wait_capture("S:0", "$");
    env.ok(&["send-keys", "-t", "S:0", "echo copy''me", "Enter"]);
    env.wait_capture("S:0", "copyme");
    let in_mode = || env.ok(&["display", "-p", "-t", "S:0", "#{pane_in_mode}"]);
    env.ok(&["copy-mode", "-t", "S:0"]);
    assert_eq!(in_mode(), "1\n");
    env.ok(&["send-keys", "-t", "S:0", "-X", "cursor-up"]);
    env.ok(&["send-keys", "-t", "S:0", "V"]);
    env.ok(&["send-keys", "-t", "S:0", "Enter"]);
    assert_eq!(in_mode(), "0\n", "Enter (copy-pipe-and-cancel) leaves copy mode");
    assert_eq!(env.ok(&["show-buffer"]), "copyme\n");
    // Entering and driving the mode in one command list.
    env.ok(&["copy-mode", "-t", "S:0", ";", "send-keys", "-t", "S:0", "k", "0", "Space", "$", "Enter"]);
    assert_eq!(in_mode(), "0\n");
    assert_eq!(env.ok(&["show-buffer"]), "copyme\n");
    // A later change of mode-keys reaches a mode entered before it.
    env.ok(&["copy-mode", "-t", "S:0"]);
    env.ok(&["set", "-g", "mode-keys", "emacs"]);
    // Escape cancels in emacs copy mode (in vi it only clears the selection).
    env.ok(&["send-keys", "-t", "S:0", "Escape"]);
    assert_eq!(in_mode(), "0\n");
}
