//! Terminal features passed through between programs and the outer terminal.
mod common;

use common::*;
use lif_types::Size;
use lif_vt::VtEvent;

fn attach(env: &Env, extra: &[(&str, &str)]) -> Term {
    let mut t = Term::spawn_with(env, &["new-session", "-s", "f"], Size::new(80, 24), extra);
    t.wait_text("$");
    t
}

#[test]
fn kitty_keyboard_shift_enter_reaches_program() {
    let env = Env::new();
    let mut t = attach(&env, &[]);
    // The virtual terminal answers the kitty query, so lif pushes flags.
    t.wait_for("kitty flags pushed on the outer terminal", secs(10), |t| t.modes().kitty_keyboard == 31);
    // A program that enables kitty disambiguation and dumps 7 raw bytes.
    t.type_str("printf '\\033[>1u'; stty raw -echo; head -c 7 | od -An -tx1; stty sane; printf '\\033[<u'\r");
    std::thread::sleep(std::time::Duration::from_millis(300));
    t.settle(std::time::Duration::from_millis(100));
    // Shift+Enter as the outer terminal encodes it with all flags pushed.
    t.write(b"\x1b[13;2u");
    t.wait_for("Shift+Enter in kitty encoding", secs(10), |t| {
        t.screen().split_whitespace().collect::<Vec<_>>().join(" ").contains("1b 5b 31 33 3b 32 75")
    });
}

#[test]
fn legacy_terminal_keys_still_work() {
    let env = Env::new();
    let mut t = Term::spawn(&env, &["new-session", "-s", "f"], Size::new(80, 24));
    t.answer = false;
    t.wait_text("$");
    t.type_str("echo plain''keys\r");
    t.wait_text("plainkeys");
    assert_eq!(t.modes().kitty_keyboard, 0, "no kitty flags without an answer");
}

#[test]
fn osc52_from_pane_reaches_outer_terminal() {
    let env = Env::new();
    let mut t = attach(&env, &[("TERM_PROGRAM", "ghostty")]);
    t.type_str("printf '\\033]52;c;aGVsbG8=\\a'\r");
    t.wait_for("OSC 52 on the outer terminal", secs(10), |t| {
        t.events.iter().any(|e| matches!(e, VtEvent::ClipboardWrite { data, .. } if data == b"hello"))
    });
}

#[test]
fn notification_reaches_outer_terminal() {
    let env = Env::new();
    let mut t = attach(&env, &[]);
    t.type_str("printf '\\033]9;build finished\\a'\r");
    t.wait_for("OSC 9 on the outer terminal", secs(10), |t| {
        t.events.iter().any(|e| matches!(e, VtEvent::Notification { body, .. } if body.contains("build finished")))
    });
}

#[test]
fn color_queries_answered_with_outer_colors() {
    let env = Env::new();
    let mut t = attach(&env, &[]);
    // Probe complete (kitty push happens on the DA1 answer).
    t.wait_for("probe complete", secs(10), |t| t.modes().kitty_keyboard == 31);
    t.type_str("stty raw -echo min 0 time 10; printf '\\033]11;?\\033\\\\'; r=$(dd bs=64 count=1 2>/dev/null); stty sane; printf '%s\\n' \"$r\" | tr -d '\\033'\r");
    t.wait_text("rgb:1111/2222/3333");
    t.type_str("stty raw -echo min 0 time 10; printf '\\033]10;?\\033\\\\'; r=$(dd bs=64 count=1 2>/dev/null); stty sane; printf '%s\\n' \"$r\" | tr -d '\\033'\r");
    t.wait_text("rgb:dddd/dddd/dddd");
}

#[test]
fn bracketed_paste_streams_once() {
    let env = Env::new();
    let mut t = attach(&env, &[]);
    t.wait_for("probe complete", secs(10), |t| t.modes().kitty_keyboard == 31);
    // A program with bracketed paste on that dumps what it gets.
    t.type_str("printf '\\033[?2004h'; stty raw -echo; head -c 16 | od -An -c; stty sane; printf '\\033[?2004l'\r");
    std::thread::sleep(std::time::Duration::from_millis(300));
    t.settle(std::time::Duration::from_millis(100));
    // The paste arrives in two reads; the program must see one bracket pair.
    t.write(b"\x1b[200~ab");
    std::thread::sleep(std::time::Duration::from_millis(50));
    t.write(b"cd\x1b[201~");
    t.wait_for("one bracket pair around the paste", secs(10), |t| {
        t.screen().split_whitespace().collect::<Vec<_>>().join(" ").contains("033 [ 2 0 0 ~ a b c d 033 [ 2 0 1 ~")
    });
}

/// set-clipboard, end to end. Copies from copy mode always become a paste
/// buffer and reach the outer terminal unless the option is off. OSC 52
/// from a program in a pane: `on` forwards it and keeps it as a buffer,
/// `external` forwards it only, `off` drops it.
#[test]
fn set_clipboard_on_external_off() {
    for (mode, pane_forwarded, pane_buffer, copy_forwarded) in
        [("on", true, true, true), ("external", true, false, true), ("off", false, false, false)]
    {
        let env = Env::new();
        let mut t = attach(&env, &[]);
        env.ok(&["set", "-s", "set-clipboard", mode]);
        let clip = |t: &Term, want: &[u8]| t.events.iter().any(|e| matches!(e, VtEvent::ClipboardWrite { data, .. } if data == want));
        // A program in the pane sets the clipboard ("hello").
        t.type_str("printf '\\033]52;c;aGVsbG8=\\a'; echo osc''sent\r");
        t.wait_text("oscsent");
        t.settle(std::time::Duration::from_millis(200));
        assert_eq!(clip(&t, b"hello"), pane_forwarded, "{mode}: pane OSC 52 forwarded");
        let buffers = env.ok(&["list-buffers"]);
        assert_eq!(buffers.contains("hello"), pane_buffer, "{mode}: pane OSC 52 kept as a buffer: {buffers:?}");
        // A copy from copy mode ("oscsent").
        env.ok(&["copy-mode"]);
        env.ok(&["send-keys", "-X", "cursor-up"]);
        env.ok(&["send-keys", "-X", "select-line"]);
        env.ok(&["send-keys", "-X", "copy-selection-and-cancel"]);
        assert_eq!(env.ok(&["show-buffer"]), "oscsent\n", "{mode}: copies are always buffers");
        t.settle(std::time::Duration::from_millis(200));
        assert_eq!(clip(&t, b"oscsent\n") || clip(&t, b"oscsent"), copy_forwarded, "{mode}: copy forwarded");
    }
}
