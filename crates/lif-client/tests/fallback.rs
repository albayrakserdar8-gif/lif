//! FALLBACK_RESTORE against the authoritative list of what lif turns on
//! (lif-input's setup.rs) and against a real terminal emulator.

use lif_client::FALLBACK_RESTORE;
use lif_input::*;
use lif_types::{OuterCaps, Size};
use lif_vt::{GhosttyVt, VtEngine, VtOptions};

fn all_caps() -> OuterCaps {
    OuterCaps {
        truecolor: true,
        sync_output: true,
        kitty_keyboard: true,
        styled_underline: true,
        underline_color: true,
        hyperlinks: true,
        osc52_write: true,
        osc52_read: true,
        focus_events: true,
        sgr_mouse: true,
        sgr_pixel_mouse: true,
        bracketed_paste: true,
        grapheme_clusters: true,
        overline: true,
        lr_margins: true,
        in_band_resize: true,
        color_scheme_reports: true,
    }
}

fn none_caps() -> OuterCaps {
    let mut c = all_caps();
    c.sync_output = false;
    c.kitty_keyboard = false;
    c.focus_events = false;
    c.sgr_mouse = false;
    c.sgr_pixel_mouse = false;
    c.bracketed_paste = false;
    c.in_band_resize = false;
    c.color_scheme_reports = false;
    c
}

/// Every mode switch lif-input can emit to turn something on.
fn enable_everything(caps: &OuterCaps) -> Vec<u8> {
    let mut v = Vec::new();
    write_enter(caps, &mut v);
    write_mouse(true, true, caps, &mut v);
    write_mouse_pixels(true, caps, &mut v);
    write_in_band_resize(true, caps, &mut v);
    write_color_scheme_reports(true, caps, &mut v);
    if !caps.kitty_keyboard {
        write_kitty_upgrade(&mut v);
    }
    v
}

/// Splits into escape sequences (CSI ... final, or ESC + one byte).
fn escapes(mut b: &[u8]) -> Vec<&[u8]> {
    let mut out = Vec::new();
    while let Some(i) = b.iter().position(|&c| c == 0x1b) {
        b = &b[i..];
        let len = if b.get(1) == Some(&b'[') {
            2 + b[2..].iter().position(|c| (0x40..=0x7e).contains(c)).expect("unterminated CSI") + 1
        } else {
            2
        };
        out.push(&b[..len]);
        b = &b[len..];
    }
    out
}

fn contains(hay: &[u8], needle: &[u8]) -> bool {
    hay.windows(needle.len()).any(|w| w == needle)
}

/// What undoes one enabling sequence.
fn inverse(seq: &[u8]) -> Vec<u8> {
    match seq {
        b"\x1b=" => b"\x1b>".to_vec(),
        b"\x1b[>4;2m" => b"\x1b[>4m".to_vec(),
        s if s.starts_with(b"\x1b[>") && s.ends_with(b"u") => b"\x1b[<u".to_vec(),
        s if s.starts_with(b"\x1b[?") && s.ends_with(b"h") => [&s[..s.len() - 1], b"l"].concat(),
        // Turning something off needs no undoing.
        s if s.ends_with(b"l") || s == b"\x1b[>4m" => Vec::new(),
        s => panic!("no known inverse for {:?}: extend FALLBACK_RESTORE", String::from_utf8_lossy(s)),
    }
}

#[test]
fn fallback_covers_everything_setup_emits() {
    for caps in [all_caps(), none_caps(), OuterCaps::default()] {
        let mut leave = Vec::new();
        write_leave(&caps, &mut leave);
        for seq in escapes(&leave) {
            assert!(contains(FALLBACK_RESTORE, seq), "write_leave emits {:?}, FALLBACK_RESTORE lacks it", String::from_utf8_lossy(seq));
        }
        for seq in escapes(&enable_everything(&caps)) {
            let inv = inverse(seq);
            assert!(inv.is_empty() || contains(FALLBACK_RESTORE, &inv), "{:?} is not undone", String::from_utf8_lossy(seq));
        }
    }
}

#[test]
fn fallback_leaves_a_virtual_terminal_clean() {
    let mut vt = GhosttyVt::new(&VtOptions { size: Size::new(80, 24), scrollback_bytes: 0, ..Default::default() }).unwrap();
    vt.feed(b"shell prompt$ ");
    vt.feed(&enable_everything(&all_caps()));
    // Also what the renderer changes, and a sequence cut off in the middle.
    vt.feed(b"\x1b[?2026h\x1b[?25l\x1b[5 q\x1b[1;31mframe\x1b[38;2;1");
    let m = vt.modes();
    assert!(m.alt_screen && m.mouse_tracking && m.bracketed_paste && m.focus_events && m.kitty_keyboard == 31 && m.app_keypad);
    vt.feed(FALLBACK_RESTORE);
    let m = vt.modes();
    assert!(!m.alt_screen, "still on the alternate screen");
    assert!(!m.mouse_tracking, "mouse tracking still on");
    assert!(!m.mouse_motion);
    assert!(!m.bracketed_paste, "bracketed paste still on");
    assert!(!m.focus_events, "focus events still on");
    assert_eq!(m.kitty_keyboard, 0, "kitty flags still pushed");
    assert!(!m.app_keypad);
    assert!(!m.synchronized_output);
    assert_eq!(vt.text(lif_vt::TextRange::Screen, false).trim_end(), "shell prompt$");
}
