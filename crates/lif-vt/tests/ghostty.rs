//! Integration tests for the libghostty-vt backend, against the real library.

use std::collections::HashMap;

use lif_types::{
    Attrs, CellWidth, Color, Key, KeyEvent, Mods, MouseAction, MouseButton, MouseEvent,
    PaneFrame, PixelSize, Pos, Size, Underline,
};
use lif_vt::{GhosttyVt, Scroll, TextOptions, TextRange, VtEngine, VtEvent, VtOptions};

fn vt(cols: u16, rows: u16) -> GhosttyVt {
    GhosttyVt::new(&VtOptions { size: Size::new(cols, rows), ..Default::default() }).unwrap()
}

fn frame(v: &mut GhosttyVt) -> PaneFrame {
    let mut f = PaneFrame::new(v.size());
    v.refresh(&mut f);
    f
}

fn row_text(f: &PaneFrame, y: usize) -> String {
    let mut s = String::new();
    for x in 0..f.rows[y].cells.len() {
        f.rows[y].push_cell_text(x, &mut s);
    }
    s.trim_end().to_string()
}

fn replies(v: &mut GhosttyVt) -> Vec<u8> {
    let mut out = Vec::new();
    v.drain_replies(&mut out);
    out
}

fn events(v: &mut GhosttyVt) -> Vec<VtEvent> {
    let mut out = Vec::new();
    v.drain_events(&mut out);
    out
}

fn key(k: Key, mods: Mods) -> KeyEvent {
    KeyEvent::new(k, mods)
}

/// The key event lif-input makes of `bytes` from an outer terminal that
/// speaks the kitty protocol.
fn parse_key(bytes: &[u8]) -> KeyEvent {
    let mut p = lif_input::InputParser::new();
    p.set_kitty_keyboard(true);
    let mut out = Vec::new();
    p.feed(bytes, &mut out);
    match out.as_slice() {
        [lif_input::InputEvent::Key(k)] => k.clone(),
        other => panic!("{:?} parsed as {other:?}", String::from_utf8_lossy(bytes)),
    }
}

/// Outer-terminal bytes -> KeyEvent -> bytes for a pane whose program pushed
/// kitty `flags`.
fn round_trip(flags: u8, bytes: &[u8]) -> Vec<u8> {
    let mut v = vt(20, 3);
    v.feed(format!("\x1b[>{flags}u").as_bytes());
    assert_eq!(v.modes().kitty_keyboard, flags);
    enc_key(&mut v, parse_key(bytes))
}

fn enc_key(v: &mut GhosttyVt, k: KeyEvent) -> Vec<u8> {
    let mut out = Vec::new();
    v.encode_key(&k, &mut out);
    out
}

#[test]
fn plain_text_and_frame() {
    let mut v = vt(20, 5);
    v.feed(b"hello\r\nworld");
    let f = frame(&mut v);
    assert_eq!(row_text(&f, 0), "hello");
    assert_eq!(row_text(&f, 1), "world");
    assert_eq!(f.cursor.pos, Some(Pos { x: 5, y: 1 }));
    assert!(f.cursor.visible);
    assert_eq!(v.text(TextRange::Screen, false), "hello\nworld");
}

#[test]
fn sgr_styles() {
    let mut v = vt(40, 3);
    v.feed(b"\x1b[1;3;38;2;255;100;0;48;5;123mA\x1b[0m");
    v.feed(b"\x1b[4:3;58;2;1;2;3mB\x1b[0m");
    v.feed(b"\x1b[2;5;7;8;9;53;31;102mC\x1b[0m");
    v.feed(b"\x1b[21mD\x1b[4:4mE\x1b[4:5mF\x1b[4mG\x1b[58;5;9mH\x1b[0mI");
    let f = frame(&mut v);
    let c = &f.rows[0].cells;
    assert_eq!(c[0].ch, 'A');
    assert_eq!(c[0].style.fg, Color::Rgb(255, 100, 0));
    assert_eq!(c[0].style.bg, Color::Indexed(123));
    assert_eq!(c[0].style.attrs, Attrs::BOLD.union(Attrs::ITALIC));
    assert_eq!(c[1].style.underline, Underline::Curly);
    assert_eq!(c[1].style.underline_color, Color::Rgb(1, 2, 3));
    let want = [
        Attrs::FAINT,
        Attrs::BLINK,
        Attrs::INVERSE,
        Attrs::INVISIBLE,
        Attrs::STRIKETHROUGH,
        Attrs::OVERLINE,
    ]
    .into_iter()
    .fold(Attrs::NONE, Attrs::union);
    assert_eq!(c[2].style.attrs, want);
    assert_eq!(c[2].style.fg, Color::Indexed(1));
    assert_eq!(c[2].style.bg, Color::Indexed(10));
    assert_eq!(c[3].style.underline, Underline::Double);
    assert_eq!(c[4].style.underline, Underline::Dotted);
    assert_eq!(c[5].style.underline, Underline::Dashed);
    assert_eq!(c[6].style.underline, Underline::Single);
    assert_eq!(c[7].style.underline_color, Color::Indexed(9));
    assert_eq!(c[8].style, Default::default());
    assert_eq!(c[9].style, Default::default());
}

#[test]
fn background_only_cells() {
    let mut v = vt(10, 2);
    // Erase with a background color set: cells get a bg but no text.
    v.feed(b"\x1b[48;2;9;8;7m\x1b[K\x1b[0m");
    let f = frame(&mut v);
    assert_eq!(f.rows[0].cells[5].ch, ' ');
    assert_eq!(f.rows[0].cells[5].style.bg, Color::Rgb(9, 8, 7));
}

#[test]
fn wide_and_graphemes() {
    let mut v = vt(20, 3);
    v.feed("中x\r\n".as_bytes());
    v.feed("e\u{301}!\r\n".as_bytes());
    v.feed("\u{1F469}\u{200D}\u{1F4BB}|".as_bytes());
    let f = frame(&mut v);
    let r0 = &f.rows[0].cells;
    assert_eq!((r0[0].ch, r0[0].width), ('中', CellWidth::Wide));
    assert_eq!(r0[1].width, CellWidth::WideTail);
    assert_eq!(r0[2].ch, 'x');
    assert_eq!(row_text(&f, 0), "中x");

    let r1 = &f.rows[1];
    assert_eq!(r1.cells[0].ch, 'e');
    assert_eq!(r1.cells[0].extra, 1);
    assert_eq!(&*r1.clusters[0], "e\u{301}");
    assert_eq!(r1.cells[1].ch, '!');
    assert_eq!(row_text(&f, 1), "e\u{301}!");

    let r2 = &f.rows[2];
    assert_eq!(r2.cells[0].ch, '\u{1F469}');
    assert_eq!(r2.cells[0].width, CellWidth::Wide);
    assert_ne!(r2.cells[0].extra, 0);
    assert_eq!(&*r2.clusters[r2.cells[0].extra as usize - 1], "\u{1F469}\u{200D}\u{1F4BB}");
    assert_eq!(r2.cells[1].width, CellWidth::WideTail);
    assert_eq!(row_text(&f, 2), "\u{1F469}\u{200D}\u{1F4BB}|");
}

#[test]
fn line_wrap_flag() {
    let mut v = vt(10, 3);
    v.feed(b"0123456789abcde\r\nnext");
    let f = frame(&mut v);
    assert!(f.rows[0].wrapped);
    assert!(!f.rows[1].wrapped);
    assert_eq!(row_text(&f, 1), "abcde");
    assert_eq!(v.text(TextRange::Screen, true), "0123456789abcde\nnext");
    assert_eq!(v.text(TextRange::Screen, false), "0123456789\nabcde\nnext");
}

#[test]
fn dirty_tracking() {
    let mut v = vt(30, 10);
    v.feed(b"some text\r\nmore");
    let mut f = PaneFrame::new(v.size());
    assert!(v.refresh(&mut f));
    f.clear_dirty();
    assert!(!v.refresh(&mut f), "idle refresh must report no change");
    assert!(!f.any_dirty());

    v.feed(b"\x1b[5;3Hx");
    assert!(v.is_dirty());
    assert!(v.refresh(&mut f));
    // Row 4 changed; row 1 is dirty too because the cursor left it.
    let dirty: Vec<usize> = (0..10).filter(|&y| f.dirty[y]).collect();
    assert_eq!(dirty, [1, 4]);
    f.clear_dirty();
    // With the cursor already on the row, only that row.
    v.feed(b"y");
    assert!(v.refresh(&mut f));
    let dirty: Vec<usize> = (0..10).filter(|&y| f.dirty[y]).collect();
    assert_eq!(dirty, [4]);
    assert_eq!(f.rows[4].cells[2].ch, 'x');
    assert!(!v.is_dirty());
    f.clear_dirty();

    // Cursor-only move: no rows, but the cursor changed.
    v.feed(b"\x1b[1;1H");
    assert!(v.refresh(&mut f));
    assert_eq!(f.cursor.pos, Some(Pos { x: 0, y: 0 }));
    f.clear_dirty();
    assert!(!v.refresh(&mut f));

    // Resize resets the frame and rewrites everything.
    v.resize(Size::new(40, 12), PixelSize::default());
    assert!(v.refresh(&mut f));
    assert_eq!(f.size, Size::new(40, 12));
    assert!(f.dirty.iter().all(|d| *d));
    assert_eq!(row_text(&f, 0), "some text");
    f.clear_dirty();
    assert!(!v.refresh(&mut f));

    // A fresh frame with the same size is fully written too.
    let mut g = PaneFrame::new(Size::new(40, 12));
    g.clear_dirty();
    g.size = Size::new(1, 1);
    assert!(v.refresh(&mut g));
    assert_eq!(row_text(&g, 0), "some text");
}

#[test]
fn reverse_video_mode() {
    let mut v = vt(10, 3);
    v.feed(b"ab[7mc");
    let mut f = frame(&mut v);
    f.clear_dirty();
    v.feed(b"[?5h");
    assert!(v.refresh(&mut f));
    assert!(f.dirty.iter().all(|d| *d));
    let c = &f.rows[0].cells;
    assert!(c[0].style.attrs.contains(Attrs::INVERSE));
    assert!(!c[2].style.attrs.contains(Attrs::INVERSE));
    assert!(f.rows[2].cells[9].style.attrs.contains(Attrs::INVERSE));
    f.clear_dirty();
    v.feed(b"[?5l");
    assert!(v.refresh(&mut f));
    assert!(!f.rows[0].cells[0].style.attrs.contains(Attrs::INVERSE));
    assert!(f.rows[0].cells[2].style.attrs.contains(Attrs::INVERSE));
}

#[test]
fn query_replies() {
    let mut v = vt(80, 24);
    v.feed(b"\x1b[c");
    let r = replies(&mut v);
    assert!(r.starts_with(b"\x1b[?") && r.ends_with(b"c"), "{:?}", String::from_utf8_lossy(&r));

    v.feed(b"\x1b[5;7H\x1b[6n");
    assert_eq!(replies(&mut v), b"\x1b[5;7R");

    v.feed(b"\x1b[?2026$p");
    assert_eq!(replies(&mut v), b"\x1b[?2026;2$y");
    v.feed(b"\x1b[?2026h\x1b[?2026$p");
    assert_eq!(replies(&mut v), b"\x1b[?2026;1$y");
    assert!(v.modes().synchronized_output);

    v.feed(b"\x1b[>q");
    let r = replies(&mut v);
    assert!(r.windows(3).any(|w| w == b"lif"), "{:?}", String::from_utf8_lossy(&r));
    assert!(replies(&mut v).is_empty());
}

#[test]
fn modes_follow_program() {
    let mut v = vt(80, 24);
    let m = v.modes();
    assert!(!m.alt_screen && !m.bracketed_paste && !m.mouse_tracking && !m.focus_events);
    assert_eq!(m.kitty_keyboard, 0);

    v.feed(b"\x1b[?1049h\x1b[?2004h\x1b[?1000h\x1b[?1004h\x1b[?1h\x1b=");
    let m = v.modes();
    assert!(m.alt_screen);
    assert!(m.bracketed_paste);
    assert!(m.mouse_tracking);
    assert!(!m.mouse_motion);
    assert!(m.focus_events);
    assert!(m.app_cursor_keys);
    assert!(m.app_keypad);

    v.feed(b"\x1b[?1003h");
    assert!(v.modes().mouse_motion);

    v.feed(b"\x1b[>1u");
    assert_eq!(v.modes().kitty_keyboard, 1);
    v.feed(b"\x1b[>3u");
    assert_eq!(v.modes().kitty_keyboard, 3);
    v.feed(b"\x1b[<u");
    assert_eq!(v.modes().kitty_keyboard, 1);
    v.feed(b"\x1b[<u");
    assert_eq!(v.modes().kitty_keyboard, 0);

    v.feed(b"\x1b[?1049l\x1b[?2004l\x1b[?1000l\x1b[?1003l");
    let m = v.modes();
    assert!(!m.alt_screen && !m.bracketed_paste && !m.mouse_tracking);
}

#[test]
fn key_encoding() {
    let mut v = vt(80, 24);
    assert_eq!(enc_key(&mut v, key(Key::Char('a'), Mods::NONE)), b"a");
    assert_eq!(enc_key(&mut v, key(Key::Char('a'), Mods::SHIFT)), b"A");
    assert_eq!(enc_key(&mut v, key(Key::Char('c'), Mods::CTRL)), b"\x03");
    assert_eq!(enc_key(&mut v, key(Key::Char('x'), Mods::ALT)), b"\x1bx");
    assert_eq!(enc_key(&mut v, key(Key::Up, Mods::NONE)), b"\x1b[A");
    assert_eq!(enc_key(&mut v, key(Key::Right, Mods::CTRL)), b"\x1b[1;5C");
    assert_eq!(enc_key(&mut v, key(Key::Enter, Mods::NONE)), b"\r");
    assert_eq!(enc_key(&mut v, key(Key::F(5), Mods::NONE)), b"\x1b[15~");
    let mut text = key(Key::Char('e'), Mods::NONE);
    text.text = "é".into();
    assert_eq!(enc_key(&mut v, text), "é".as_bytes());

    v.feed(b"\x1b[?1h");
    assert_eq!(enc_key(&mut v, key(Key::Up, Mods::NONE)), b"\x1bOA");
    v.feed(b"\x1b[?1l");

    // Kitty: disambiguate.
    v.feed(b"\x1b[>1u");
    assert_eq!(enc_key(&mut v, key(Key::Enter, Mods::SHIFT)), b"\x1b[13;2u");
    assert_eq!(enc_key(&mut v, key(Key::Char('c'), Mods::CTRL)), b"\x1b[99;5u");
    assert_eq!(enc_key(&mut v, key(Key::Escape, Mods::NONE)), b"\x1b[27u");
    assert_eq!(enc_key(&mut v, key(Key::Char('a'), Mods::NONE)), b"a");
    // Release events are only reported with flag 2.
    let mut rel = key(Key::Char('a'), Mods::NONE);
    rel.action = lif_types::KeyAction::Release;
    assert!(enc_key(&mut v, rel.clone()).is_empty());
    v.feed(b"\x1b[>3u");
    assert_eq!(enc_key(&mut v, rel), b"\x1b[97;1:3u");
}

fn mouse(action: MouseAction, button: Option<MouseButton>, x: u16, y: u16) -> MouseEvent {
    MouseEvent { action, button, mods: Mods::NONE, cell: Pos { x, y }, pixel: None }
}

fn enc_mouse(v: &mut GhosttyVt, ev: MouseEvent) -> Vec<u8> {
    let mut out = Vec::new();
    v.encode_mouse(&ev, &mut out);
    out
}

#[test]
fn mouse_encoding() {
    let mut v = vt(80, 24);
    let press = mouse(MouseAction::Press, Some(MouseButton::Left), 4, 2);
    assert!(enc_mouse(&mut v, press).is_empty(), "no tracking requested");

    v.feed(b"\x1b[?1000h\x1b[?1006h");
    assert_eq!(enc_mouse(&mut v, press), b"\x1b[<0;5;3M");
    assert_eq!(
        enc_mouse(&mut v, mouse(MouseAction::Release, Some(MouseButton::Left), 4, 2)),
        b"\x1b[<0;5;3m"
    );
    assert_eq!(
        enc_mouse(&mut v, mouse(MouseAction::Press, Some(MouseButton::WheelUp), 0, 0)),
        b"\x1b[<64;1;1M"
    );
    // Normal tracking does not report motion.
    assert!(enc_mouse(&mut v, mouse(MouseAction::Motion, None, 9, 9)).is_empty());
    v.feed(b"\x1b[?1003h");
    assert_eq!(enc_mouse(&mut v, mouse(MouseAction::Motion, None, 9, 9)), b"\x1b[<35;10;10M");

    // X10 encoding without SGR.
    v.feed(b"\x1b[?1006l\x1b[?1000h");
    assert_eq!(enc_mouse(&mut v, press), b"\x1b[M %#");

    // SGR-pixels reports the pixel position.
    v.feed(b"\x1b[?1016h");
    let mut px = press;
    px.pixel = Some((45, 37));
    assert_eq!(enc_mouse(&mut v, px), b"\x1b[<0;45;37M");
}

#[test]
fn paste_and_focus() {
    let mut v = vt(80, 24);
    let mut out = Vec::new();
    v.encode_paste(b"a\nb", &mut out);
    assert_eq!(out, b"a\rb");
    out.clear();
    v.feed(b"\x1b[?2004h");
    v.encode_paste(b"hi\x1b[201~x", &mut out);
    assert!(out.starts_with(b"\x1b[200~") && out.ends_with(b"\x1b[201~"));
    assert_eq!(out.windows(6).filter(|w| w == b"\x1b[201~").count(), 1, "{out:?}");
    assert!(replies(&mut v).is_empty(), "paste must not leak into replies");

    out.clear();
    v.encode_focus(true, &mut out);
    assert!(out.is_empty());
    v.feed(b"\x1b[?1004h");
    v.encode_focus(true, &mut out);
    v.encode_focus(false, &mut out);
    assert_eq!(out, b"\x1b[I\x1b[O");
}

#[test]
fn scrollback_and_text() {
    let mut v = vt(20, 5);
    for i in 0..100 {
        v.feed(format!("line {i}\r\n").as_bytes());
    }
    assert!(v.scrollback_rows() >= 95, "{}", v.scrollback_rows());
    assert_eq!(v.viewport_offset(), 0);
    let mut f = PaneFrame::new(v.size());
    v.refresh(&mut f);
    assert_eq!(row_text(&f, 0), "line 96");
    f.clear_dirty();

    v.scroll(Scroll::Top);
    assert!(v.viewport_offset() > 90);
    assert!(v.refresh(&mut f));
    assert_eq!(row_text(&f, 0), "line 0");
    assert_eq!(f.cursor.pos, None);

    v.scroll(Scroll::Lines(2));
    v.refresh(&mut f);
    assert_eq!(row_text(&f, 0), "line 2");
    v.scroll(Scroll::Bottom);
    assert_eq!(v.viewport_offset(), 0);
    v.refresh(&mut f);
    assert_eq!(row_text(&f, 0), "line 96");

    let all = v.text(TextRange::All, false);
    assert!(all.starts_with("line 0\nline 1\n"), "{all:.40}");
    assert!(all.trim_end().ends_with("line 99"));
    assert_eq!(v.text(TextRange::Rows { start: 3, end: 5 }, false), "line 3\nline 4");
    assert_eq!(v.text(TextRange::Screen, false), "line 96\nline 97\nline 98\nline 99");
}

#[test]
fn scrollback_byte_cap() {
    // 50 MB in release. A dev build links libghostty compiled by Zig in
    // Debug mode, where pruning scrollback is thousands of times slower
    // (about 25 s per MB here), so it feeds less there.
    let (cap, megabytes) = if cfg!(debug_assertions) { (512 << 10, 2) } else { (1 << 20, 50) };
    let mut v = GhosttyVt::new(&VtOptions {
        size: Size::new(120, 40),
        pixels: PixelSize::default(),
        scrollback_bytes: cap,
        ..Default::default()
    })
    .unwrap();
    let mut chunk = Vec::new();
    let mut i = 0u32;
    while chunk.len() < 1 << 20 {
        chunk.extend_from_slice(
            format!("\x1b[3{}m{i:08} the quick brown fox jumps over the lazy dog\x1b[0m\r\n", i % 8)
                .as_bytes(),
        );
        i += 1;
    }
    let mut peak = 0;
    for _ in 0..megabytes {
        v.feed(&chunk);
        peak = peak.max(v.memory().resident_bytes);
    }
    let mem = v.memory();
    eprintln!("cap {cap} peak {peak} final {mem:?} rows {}", v.scrollback_rows());
    // libghostty prunes whole pages (~0.5 MB), so allow a few pages of slack.
    assert!(peak < cap + (4 << 20), "peak {peak}");
    assert!(v.scrollback_rows() > 100);

    // No scrollback at all.
    let mut z = GhosttyVt::new(&VtOptions { scrollback_bytes: 0, ..Default::default() }).unwrap();
    z.feed(&chunk);
    assert_eq!(z.scrollback_rows(), 0);
    z.compact();
    v.compact();
    v.compact();
}

#[test]
fn effects_become_events() {
    let mut v = vt(80, 24);
    v.feed(b"\x07");
    v.feed(b"\x1b]2;hello title\x07");
    v.feed(b"\x1b]7;file://host/tmp\x1b\\");
    v.feed(b"\x1b]52;c;aGk=\x07");
    v.feed(b"\x1b]52;p;?\x07");
    v.feed(b"\x1b]133;C\x07\x1b]133;D;3\x07");
    v.feed(b"\x1b]9;build done\x07");
    v.feed(b"\x1b]777;notify;T;B\x07");
    v.feed(b"\x1b]99;;kitty says\x1b\\");
    let ev = events(&mut v);
    assert_eq!(
        ev,
        vec![
            VtEvent::Bell,
            VtEvent::TitleChanged,
            VtEvent::PwdChanged,
            VtEvent::ClipboardWrite { selection: b'c', data: b"hi".to_vec() },
            VtEvent::ClipboardRead { selection: b'p' },
            VtEvent::CommandStarted,
            VtEvent::CommandFinished { exit_code: Some(3) },
            VtEvent::Notification { title: String::new(), body: "build done".into() },
            VtEvent::Notification { title: "T".into(), body: "B".into() },
            VtEvent::Notification { title: "kitty says".into(), body: String::new() },
        ]
    );
    assert_eq!(v.title(), "hello title");
    assert_eq!(v.pwd(), "file://host/tmp");
    assert!(replies(&mut v).is_empty(), "OSC 52 read must wait for clipboard_reply");
    assert!(events(&mut v).is_empty());

    // OSC 52 read split across feeds.
    v.feed(b"\x1b]5");
    v.feed(b"2;c;");
    v.feed(b"?\x1b");
    v.feed(b"\\");
    assert_eq!(events(&mut v), vec![VtEvent::ClipboardRead { selection: b'c' }]);

    let mut out = Vec::new();
    v.clipboard_reply(b'c', b"hi", &mut out);
    assert_eq!(out, b"\x1b]52;c;aGk=\x1b\\");

    // RIS clears the title.
    v.feed(b"\x1bc");
    assert_eq!(v.title(), "");
}

#[test]
fn hyperlinks() {
    let mut v = vt(40, 3);
    v.feed(b"a\x1b]8;;https://example.com/x\x1b\\link\x1b]8;;\x1b\\b");
    let f = frame(&mut v);
    let c = &f.rows[0].cells;
    assert_eq!(c[0].link, 0);
    assert_ne!(c[1].link, 0);
    assert_eq!(c[1].link, c[4].link);
    assert_eq!(c[5].link, 0);
    assert_eq!(v.hyperlink(c[1].link), Some("https://example.com/x"));
    assert_eq!(v.hyperlink(9999), None);
}

#[test]
fn engine_is_owned_and_static() {
    fn assert_static<T: 'static>() {}
    assert_static::<GhosttyVt>();
    let mut panes: HashMap<u32, GhosttyVt> = HashMap::new();
    for id in 0..4 {
        panes.insert(id, vt(10, 2));
    }
    // Rehash moves every engine; callbacks must still land in the right one.
    for id in 4..64 {
        panes.insert(id, vt(10, 2));
    }
    let p = panes.get_mut(&3).unwrap();
    p.feed(b"\x1b[6n\x07");
    assert_eq!(replies(p), b"\x1b[1;1R");
    assert_eq!(events(p), vec![VtEvent::Bell]);
    let moved = panes.remove(&3).unwrap();
    let mut boxed = Box::new(moved);
    boxed.feed(b"\x1b[c");
    assert!(!replies(&mut boxed).is_empty());
    assert!(replies(panes.get_mut(&2).unwrap()).is_empty());
}

#[test]
fn kitty_alternate_keys_round_trip() {
    // lif pushes all kitty flags (31) to the outer terminal; a program that
    // pushed the same flags gets the outer terminal's bytes back unchanged.
    for b in [
        &b"\x1b[1089:1057:99;2;1057u"[..], // Shift+с on a Russian layout
        b"\x1b[49:33;2;33u",               // Shift+1 -> !
        b"\x1b[1089::99;5u",               // Ctrl+с
        b"\x1b[97:65;6u",                  // Ctrl+Shift+a
        b"\x1b[1089:1057:99;2:3u",         // release
    ] {
        assert_eq!(round_trip(31, b), b, "{}", String::from_utf8_lossy(b));
    }
    // Alternates without the other flags (disambiguate + alternates + all keys).
    assert_eq!(round_trip(13, b"\x1b[1089:1057:99;2;1057u"), b"\x1b[1089:1057:99;2u");
    assert_eq!(round_trip(13, b"\x1b[49:33;2;33u"), b"\x1b[49:33;2u");
    assert_eq!(round_trip(5, b"\x1b[1089::99;5u"), b"\x1b[1089::99;5u");
    // Without flag 4 the alternates are dropped; without "report all keys"
    // shifted text is plain text.
    assert_eq!(round_trip(1, b"\x1b[1089::99;5u"), b"\x1b[1089;5u");
    assert_eq!(round_trip(5, b"\x1b[1089:1057:99;2;1057u"), "С".as_bytes());
    // Legacy: the base-layout key makes Ctrl+с the Ctrl+c a user expects.
    assert_eq!(round_trip(0, b"\x1b[1089::99;5u"), b"\x03");
    assert_eq!(round_trip(0, b"\x1b[49:33;2;33u"), b"!");
}

#[test]
fn kitty_media_and_modifier_keys_round_trip() {
    let media = b"\x1b[57430u"; // play/pause
    let media_ctrl = b"\x1b[57430;5u";
    let media_release = b"\x1b[57430;1:3u";
    let shift = b"\x1b[57441;2u"; // left shift press
    let shift_release = b"\x1b[57441;1:3u";
    let iso3 = b"\x1b[57453;1:2u"; // ISO level 3 shift, repeat
    for b in [&media[..], media_ctrl, media_release, shift, shift_release, iso3] {
        assert_eq!(round_trip(31, b), b, "{}", String::from_utf8_lossy(b));
    }
    // Disambiguate only: media keys yes (presses), lone modifiers never.
    assert_eq!(round_trip(1, media), media);
    assert_eq!(round_trip(1, media_ctrl), media_ctrl);
    assert_eq!(round_trip(1, media_release), b"");
    assert_eq!(round_trip(1, shift), b"");
    // Report all keys without event types: modifier presses, no releases,
    // a repeat is a press.
    assert_eq!(round_trip(9, shift), shift);
    assert_eq!(round_trip(9, shift_release), b"");
    assert_eq!(round_trip(9, iso3), b"\x1b[57453u");
    // Legacy has no encoding for either.
    assert_eq!(round_trip(0, media), b"");
    assert_eq!(round_trip(0, shift), b"");
}

#[test]
fn default_color_queries() {
    let q = b"\x1b]10;?\x1b\\\x1b]11;?\x1b\\\x1b]12;?\x07";
    // Never set: answered with the fallbacks (light gray on black), so a
    // program never waits for a timeout.
    let mut v = vt(10, 2);
    v.feed(q);
    assert_eq!(
        replies(&mut v),
        b"\x1b]10;rgb:c0c0/c0c0/c0c0\x1b\\\x1b]11;rgb:0000/0000/0000\x1b\\\x1b]12;rgb:c0c0/c0c0/c0c0\x07"
    );

    // From the options (the outer terminal's colors, a light theme).
    let mut v = GhosttyVt::new(&VtOptions {
        size: Size::new(10, 2),
        default_fg: Some((0x10, 0x20, 0x30)),
        default_bg: Some((0xfa, 0xfb, 0xfc)),
        default_cursor: Some((0xff, 0x00, 0x80)),
        ..Default::default()
    })
    .unwrap();
    v.feed(q);
    assert_eq!(
        replies(&mut v),
        b"\x1b]10;rgb:1010/2020/3030\x1b\\\x1b]11;rgb:fafa/fbfb/fcfc\x1b\\\x1b]12;rgb:ffff/0000/8080\x07"
    );

    // Changed on the live engine; the cursor follows the foreground.
    v.set_default_colors(Some((1, 2, 3)), Some((4, 5, 6)), None);
    v.feed(q);
    assert_eq!(
        replies(&mut v),
        b"\x1b]10;rgb:0101/0202/0303\x1b\\\x1b]11;rgb:0404/0505/0606\x1b\\\x1b]12;rgb:0101/0202/0303\x07"
    );

    // A color the program set wins until it resets it (OSC 111).
    v.feed(b"\x1b]11;rgb:ff/00/00\x1b\\");
    v.set_default_colors(Some((1, 2, 3)), Some((7, 7, 7)), None);
    v.feed(b"\x1b]11;?\x1b\\");
    assert_eq!(replies(&mut v), b"\x1b]11;rgb:ffff/0000/0000\x1b\\");
    v.feed(b"\x1b]111\x1b\\\x1b]11;?\x1b\\");
    assert_eq!(replies(&mut v), b"\x1b]11;rgb:0707/0707/0707\x1b\\");

    // Back to unset: the fallbacks again.
    v.set_default_colors(None, None, None);
    v.feed(b"\x1b]11;?\x1b\\");
    assert_eq!(replies(&mut v), b"\x1b]11;rgb:0000/0000/0000\x1b\\");
}

#[test]
fn paste_chunks_carry_no_markers() {
    let mut v = vt(20, 4);
    let mut out = Vec::new();
    // Bracketed: the text, with ESC removed, and no markers of its own.
    v.feed(b"\x1b[?2004h");
    v.encode_paste_chunk(b"ab\x1b[201~cd\n", &mut out);
    assert!(!out.windows(6).any(|w| w == b"\x1b[200~" || w == b"\x1b[201~"), "{out:?}");
    assert!(!out.contains(&0x1b), "{out:?}");
    assert!(out.starts_with(b"ab"), "{out:?}");
    // Not bracketed: same as encode_paste.
    v.feed(b"\x1b[?2004l");
    let (mut a, mut b) = (Vec::new(), Vec::new());
    v.encode_paste_chunk(b"x\ny", &mut a);
    v.encode_paste(b"x\ny", &mut b);
    assert_eq!(a, b);
}

// ----- rows for copy mode ---------------------------------------------------

fn abs_text(v: &GhosttyVt, y: usize) -> String {
    let mut r = lif_types::FrameRow::default();
    v.row(y, &mut r);
    assert_eq!(r.cells.len(), v.size().cols as usize);
    let mut s = String::new();
    for x in 0..r.cells.len() {
        r.push_cell_text(x, &mut s);
    }
    s.trim_end().to_string()
}

#[test]
fn rows_of_scrollback_and_screen() {
    let mut v = vt(10, 3);
    v.feed(b"\x1b[1;31mred\x1b[0m plain\r\nab\xe4\xb8\xad\xe6\x96\x87cd\r\ne\xcc\x81x\r\n0123456789wrap\r\nlast");
    // 5 lines, the 4th wrapped: 6 rows, 3 of them history.
    assert_eq!(v.scrollback_rows(), 3);
    let mut r = lif_types::FrameRow::default();
    v.row(0, &mut r);
    assert_eq!(abs_text(&v, 0), "red plain");
    assert_eq!(r.cells[0].style.fg, Color::Indexed(1));
    assert!(r.cells[0].style.attrs.contains(Attrs::BOLD));
    assert_eq!(r.cells[4].style.fg, Color::Default);
    assert!(!r.wrapped);
    v.row(1, &mut r);
    assert_eq!(abs_text(&v, 1), "ab\u{4e2d}\u{6587}cd");
    assert_eq!(r.cells[2].width, CellWidth::Wide);
    assert_eq!(r.cells[3].width, CellWidth::WideTail);
    v.row(2, &mut r);
    assert_eq!(abs_text(&v, 2), "e\u{301}x");
    assert_eq!(r.cells[0].extra, 1);
    assert_eq!(&*r.clusters[0], "e\u{301}");
    v.row(3, &mut r);
    assert!(r.wrapped, "soft-wrapped row");
    assert_eq!(abs_text(&v, 3), "0123456789");
    assert_eq!(abs_text(&v, 4), "wrap");
    assert_eq!(abs_text(&v, 5), "last");
    // Past the end: blank, the right width, no stale clusters.
    v.row(2, &mut r);
    v.row(99, &mut r);
    assert!(r.clusters.is_empty() && !r.wrapped);
    assert!(r.cells.iter().all(|c| *c == lif_types::Cell::BLANK));
    // The screen rows read the same as the frame.
    let f = frame(&mut v);
    for y in 0..3 {
        let mut r = lif_types::FrameRow::default();
        v.row(3 + y, &mut r);
        assert_eq!(r.cells, f.rows[y].cells, "row {y}");
    }
}

#[test]
fn prompts_from_osc_133() {
    let mut v = vt(20, 3);
    for i in 0..5 {
        v.feed(format!("\x1b]133;A\x07$ cmd{i}\r\n\x1b]133;C\x07out{i}\r\nmore{i}\r\n").as_bytes());
    }
    // Rows: 0 "$ cmd0", 1 out0, 2 more0, 3 "$ cmd1", ...
    assert_eq!(abs_text(&v, 3), "$ cmd1");
    assert_eq!(v.find_prompt(0, true), Some(0));
    assert_eq!(v.find_prompt(1, true), Some(3));
    assert_eq!(v.find_prompt(5, false), Some(3));
    assert_eq!(v.find_prompt(2, false), Some(0));
    assert_eq!(v.find_prompt(13, true), None);
    assert_eq!(v.find_prompt(1000, false), Some(12));
}

#[test]
fn trimmed_rows_count_evictions() {
    // libghostty evicts whole storage pages (about 1170 rows of 40 columns,
    // about 512 KiB); 4 MiB holds several.
    let mut v = GhosttyVt::new(&VtOptions { size: Size::new(40, 5), scrollback_bytes: 4 << 20, ..Default::default() })
        .unwrap();
    assert_eq!(v.trimmed_rows(), 0);
    v.track_trimmed(true);
    let mut n = 0u64;
    let mut line = |v: &mut GhosttyVt| {
        v.feed(format!("line {n:08}\r\n").as_bytes());
        n += 1;
    };
    for _ in 0..100 {
        line(&mut v);
    }
    assert_eq!(v.trimmed_rows(), 0, "nothing evicted yet");
    // Absolute row of a line = its number - trimmed: always consistent.
    for _ in 0..40_000 {
        line(&mut v);
    }
    let t = v.trimmed_rows();
    assert!(t > 0, "the 4 MiB scrollback must have evicted rows");
    let check = |v: &GhosttyVt| {
        let t = v.trimmed_rows() as usize;
        for y in [0usize, 7, v.scrollback_rows() / 2] {
            assert_eq!(abs_text(v, y), format!("line {:08}", y + t), "row {y} after {t} evicted");
        }
    };
    check(&v);
    // Many lines in one write (less than the scrollback holds).
    let mut big = String::new();
    for _ in 0..3000 {
        big.push_str(&format!("line {n:08}\r\n"));
        n += 1;
    }
    v.feed(big.as_bytes());
    assert!(v.trimmed_rows() > t);
    check(&v);
    // One write bigger than the whole scrollback: every older row is gone,
    // and the count says at least that many went.
    let (t, h) = (v.trimmed_rows(), v.scrollback_rows() as u64);
    let mut huge = String::new();
    for _ in 0..100_000 {
        huge.push_str(&format!("line {n:08}\r\n"));
        n += 1;
    }
    v.feed(huge.as_bytes());
    assert!(v.trimmed_rows() >= t + h, "{} < {t} + {h}", v.trimmed_rows());
    // Clearing the scrollback evicts everything above the screen.
    let (t, h) = (v.trimmed_rows(), v.scrollback_rows() as u64);
    v.feed(b"\x1b[3J");
    assert_eq!(v.scrollback_rows(), 0);
    assert_eq!(v.trimmed_rows(), t + h);
    // A resize reflows without counting evictions.
    let t = v.trimmed_rows();
    v.resize(Size::new(20, 5), PixelSize::default());
    v.resize(Size::new(40, 5), PixelSize::default());
    assert_eq!(v.trimmed_rows(), t);
    // Off, nothing is counted.
    v.track_trimmed(false);
    let t = v.trimmed_rows();
    for i in 0..20_000 {
        v.feed(format!("more {i:08}\r\n").as_bytes());
    }
    assert_eq!(v.trimmed_rows(), t);
}

#[test]
fn viewport_top_is_exact_and_holds_while_output_arrives() {
    let mut v = vt(20, 4);
    for i in 0..50 {
        v.feed(format!("l{i}\r\n").as_bytes());
    }
    let h = v.scrollback_rows();
    for top in [0, 1, 17, h - 1] {
        v.set_viewport_top(top);
        let f = frame(&mut v);
        assert_eq!(row_text(&f, 0), abs_text(&v, top), "top {top}");
        assert_eq!(v.viewport_offset(), h - top);
    }
    v.set_viewport_top(10);
    v.feed(b"more\r\nand more\r\n");
    let f = frame(&mut v);
    assert_eq!(row_text(&f, 0), "l10", "the viewport stays on its rows");
    // At the bottom it follows output again.
    v.set_viewport_top(v.scrollback_rows());
    v.feed(b"x\r\ny\r\n");
    let f = frame(&mut v);
    assert_eq!(v.viewport_offset(), 0);
    assert_eq!(row_text(&f, 2), "y");
}

#[test]
fn reading_a_row_restores_at_most_its_own_page() {
    let mut v = GhosttyVt::new(&VtOptions { size: Size::new(80, 24), scrollback_bytes: 64 << 20, ..Default::default() })
        .unwrap();
    let mut s = String::new();
    for i in 0..200_000u32 {
        s.push_str(&format!("{i:08} the quick brown fox jumps over the lazy dog\r\n"));
    }
    // In read-sized pieces, as the server feeds it.
    v.track_trimmed(true);
    for chunk in s.as_bytes().chunks(64 * 1024) {
        v.feed(chunk);
    }
    for _ in 0..10_000 {
        v.compact();
    }
    let before = v.memory().compressed_pages;
    if before < 3 {
        // This build of libghostty does not compress (unsupported platform).
        return;
    }
    let (h, t) = (v.scrollback_rows(), v.trimmed_rows() as usize);
    for y in [5usize, h / 2, h - 30] {
        assert!(abs_text(&v, y).starts_with(&format!("{:08}", y + t)), "row {y}: {}", abs_text(&v, y));
    }
    let after = v.memory().compressed_pages;
    assert!(before - after <= 3, "three rows read, {} pages restored", before - after);
}

#[test]
fn capture_keeps_spaces_and_styles_on_request() {
    let mut v = vt(20, 3);
    v.feed(b"a  \x1b[31mred\x1b[0m   \r\nb");
    let plain = v.capture(TextRange::Screen, TextOptions::default());
    assert_eq!(plain.lines().next(), Some("a  red"));
    let keep = v.capture(TextRange::Screen, TextOptions { keep_trailing: true, ..Default::default() });
    assert!(keep.lines().next().unwrap().starts_with("a  red   "), "{keep:?}");
    let styled = v.capture(TextRange::Screen, TextOptions { styled: true, ..Default::default() });
    assert!(styled.contains("\x1b[") && styled.contains("red"), "{styled:?}");
    assert!(!styled.contains("\x1b]"), "only styles, no terminal state: {styled:?}");
    assert!(!plain.contains('\x1b'));
}

#[test]
fn row_scan_reads_the_same_rows_and_moves_the_viewport() {
    let mut v = vt(20, 4);
    for i in 0..300 {
        v.feed(format!("r{i}\r\n").as_bytes());
    }
    let h = v.scrollback_rows();
    assert_eq!(v.viewport_top(), h);
    let (mut a, mut b) = (lif_types::FrameRow::default(), lif_types::FrameRow::default());
    for y in (0..h + 4).rev().chain(0..h + 4) {
        v.row_scan(y, &mut a);
        v.row(y, &mut b);
        assert_eq!(a, b, "row {y}");
        let top = v.viewport_top();
        assert!(top <= y && y < top + 4 || y >= h, "row {y} not in the viewport at {top}");
    }
    v.set_viewport_top(h);
    assert_eq!(v.viewport_offset(), 0);
}

/// Everything a program or user can observe of the primary and alternate
/// screen, the modes, the strings and the query answers.
fn observable(v: &mut GhosttyVt) -> String {
    let f = frame(v);
    let rows: Vec<String> = (0..f.rows.len()).map(|y| row_text(&f, y)).collect();
    v.feed(b"\x1b[c\x1b]11;?\x1b\\\x1b[6n");
    let r = String::from_utf8_lossy(&replies(v)).into_owned();
    let up = enc_key(v, key(Key::Up, Mods::NONE));
    format!(
        "{:?}\n{:?}\n{}\n{}\n{:?}\n{:?}\n{}\n{r:?}\n{:?}",
        v.modes(),
        f.cursor,
        v.title(),
        v.pwd(),
        rows,
        v.scrollback_rows(),
        v.capture(TextRange::All, TextOptions { styled: true, ..Default::default() }),
        up,
    )
}

/// After a resize, `compact` rebuilds a small terminal from a snapshot of
/// itself to give back storage the resize left behind. Nothing observable
/// may change, and the rebuilt terminal must keep working (replies, events,
/// scrollback limits).
#[test]
fn compact_after_resize_keeps_every_observable() {
    let mut v = GhosttyVt::new(&VtOptions { size: Size::new(60, 10), scrollback_bytes: 1 << 20, ..Default::default() }).unwrap();
    for i in 0..30 {
        v.feed(format!("line {i} \x1b[1;31mred\x1b[0m \x1b]8;;http://x/{i}\x1b\\link\x1b]8;;\x1b\\\r\n").as_bytes());
    }
    v.feed(b"\x1b]0;the title\x07\x1b]7;file://host/tmp\x07");
    v.feed(b"\x1b[?1h\x1b[?2004h\x1b[?1002h\x1b[?1006h\x1b[>5u\x1b]11;rgb:12/34/56\x1b\\");
    v.feed(b"\x1b[3;8r\x1b[5;7H\x1b[4 q\xc3\xa9\xe4\xb8\xad");
    v.feed(b"\x1b[?1049h\x1b[2;2Halt screen text");
    v.resize(Size::new(40, 8), PixelSize::default());
    events(&mut v);
    let before = observable(&mut v);
    v.compact();
    assert_eq!(v.repacks(), 1, "the resize was followed by a rebuild");
    let after = observable(&mut v);
    assert_eq!(before, after);
    // Still alive: replies, events, output, back to the primary screen.
    v.feed(b"\x07\x1b[?1049l");
    assert_eq!(events(&mut v), vec![VtEvent::Bell]);
    v.feed(b"\x1b[5n");
    assert_eq!(replies(&mut v), b"\x1b[0n");
    let f = frame(&mut v);
    assert!(f.rows.iter().any(|r| r.cells.iter().any(|c| c.link != 0)), "hyperlinks survive");
    // The byte limit still holds after the rebuild.
    for i in 0..20_000 {
        v.feed(format!("filler {i:06} {}\r\n", "x".repeat(30)).as_bytes());
    }
    v.compact();
    assert!(v.memory().resident_bytes <= 2 << 20, "{:?}", v.memory());
}

/// A terminal in the middle of an escape sequence keeps its parser state
/// (no rebuild then); the sequence completes as usual.
#[test]
fn compact_mid_sequence_is_harmless() {
    let mut v = vt(30, 5);
    v.resize(Size::new(20, 5), PixelSize::default());
    v.feed(b"ab\x1b[3");
    v.compact();
    assert_eq!(v.repacks(), 0);
    v.feed(b"1mred");
    let f = frame(&mut v);
    assert_eq!(row_text(&f, 0), "abred");
    assert_eq!(f.rows[0].cells[2].style.fg, Color::Indexed(1));
}
