mod common;

use common::*;
use lif_input::*;
use lif_types::{Key, Mods, MouseAction as MA, MouseButton as MB, MouseEvent, OuterCaps, PixelSize, Pos, Size};

fn mouse(action: MA, button: Option<MB>, mods: Mods, x: u16, y: u16) -> InputEvent {
    InputEvent::Mouse(MouseEvent { action, button, mods, cell: Pos { x, y }, pixel: None })
}

fn resp(r: TermResponse) -> InputEvent {
    InputEvent::Response(r)
}

#[test]
fn sgr_mouse() {
    let n = Mods::NONE;
    let cases: Vec<(&[u8], InputEvent)> = vec![
        (b"\x1b[<0;10;5M", mouse(MA::Press, Some(MB::Left), n, 9, 4)),
        (b"\x1b[<0;10;5m", mouse(MA::Release, Some(MB::Left), n, 9, 4)),
        (b"\x1b[<1;1;1M", mouse(MA::Press, Some(MB::Middle), n, 0, 0)),
        (b"\x1b[<2;300;200M", mouse(MA::Press, Some(MB::Right), n, 299, 199)),
        (b"\x1b[<32;11;5M", mouse(MA::Motion, Some(MB::Left), n, 10, 4)),
        (b"\x1b[<35;11;5M", mouse(MA::Motion, None, n, 10, 4)),
        (b"\x1b[<64;1;1M", mouse(MA::Press, Some(MB::WheelUp), n, 0, 0)),
        (b"\x1b[<65;1;1M", mouse(MA::Press, Some(MB::WheelDown), n, 0, 0)),
        (b"\x1b[<66;1;1M", mouse(MA::Press, Some(MB::WheelLeft), n, 0, 0)),
        (b"\x1b[<67;1;1M", mouse(MA::Press, Some(MB::WheelRight), n, 0, 0)),
        (b"\x1b[<128;1;1M", mouse(MA::Press, Some(MB::Back), n, 0, 0)),
        (b"\x1b[<129;1;1m", mouse(MA::Release, Some(MB::Forward), n, 0, 0)),
        (b"\x1b[<4;1;1M", mouse(MA::Press, Some(MB::Left), Mods::SHIFT, 0, 0)),
        (b"\x1b[<8;1;1M", mouse(MA::Press, Some(MB::Left), Mods::ALT, 0, 0)),
        (b"\x1b[<16;1;1M", mouse(MA::Press, Some(MB::Left), Mods::CTRL, 0, 0)),
        (b"\x1b[<84;1;1M", mouse(MA::Press, Some(MB::WheelUp), Mods::SHIFT.union(Mods::CTRL), 0, 0)),
        (b"\x1b[<0;0;0M", mouse(MA::Press, Some(MB::Left), n, 0, 0)),
    ];
    for (b, e) in cases {
        check(b, &[e]);
    }
}

#[test]
fn sgr_pixel_mouse() {
    let setup = |p: &mut InputParser| {
        p.set_sgr_pixels(true);
        p.set_cell_size(PixelSize { width: 10, height: 20 });
    };
    let e = InputEvent::Mouse(MouseEvent {
        action: MA::Press,
        button: Some(MB::Left),
        mods: Mods::NONE,
        cell: Pos { x: 10, y: 2 },
        pixel: Some((105, 47)),
    });
    check_with(&setup, b"\x1b[<0;105;47M", &[e]);
    // Cell size learned from the terminal's own report.
    let e2 = InputEvent::Mouse(MouseEvent {
        action: MA::Motion,
        button: None,
        mods: Mods::NONE,
        cell: Pos { x: 5, y: 3 },
        pixel: Some((40, 50)),
    });
    check_with(
        &|p: &mut InputParser| p.set_sgr_pixels(true),
        b"\x1b[6;16;8t\x1b[<35;40;50M",
        &[resp(TermResponse::CellSize(PixelSize { width: 8, height: 16 })), e2],
    );
}

#[test]
fn legacy_mouse() {
    let n = Mods::NONE;
    let x10 = |b: u8, x: u8, y: u8| vec![0x1b, b'[', b'M', b + 32, x + 33, y + 33];
    let cases: Vec<(Vec<u8>, InputEvent)> = vec![
        (x10(0, 9, 4), mouse(MA::Press, Some(MB::Left), n, 9, 4)),
        (x10(2, 0, 0), mouse(MA::Press, Some(MB::Right), n, 0, 0)),
        (x10(3, 9, 4), mouse(MA::Release, None, n, 9, 4)),
        (x10(64, 1, 1), mouse(MA::Press, Some(MB::WheelUp), n, 1, 1)),
        (x10(32, 5, 5), mouse(MA::Motion, Some(MB::Left), n, 5, 5)),
        (x10(16, 200, 100), mouse(MA::Press, Some(MB::Left), Mods::CTRL, 200, 100)),
        (b"\x1b[32;10;5M".to_vec(), mouse(MA::Press, Some(MB::Left), n, 9, 4)),
        (b"\x1b[35;10;5M".to_vec(), mouse(MA::Release, None, n, 9, 4)),
    ];
    for (b, e) in cases {
        check(&b, &[e]);
    }
}

#[test]
fn focus() {
    check(b"\x1b[I\x1b[O", &[InputEvent::Focus(true), InputEvent::Focus(false)]);
}

#[test]
fn paste() {
    use InputEvent::{PasteData as D, PasteEnd as E, PasteStart as S};
    let cases: Vec<(&[u8], Vec<InputEvent>)> = vec![
        (b"\x1b[200~hello\x1b[201~", vec![S, D(b"hello".to_vec()), E]),
        (b"\x1b[200~\x1b[201~", vec![S, E]),
        (b"\x1b[200~a\x1b[Ab\x1bc\x1b[201~", vec![S, D(b"a\x1b[Ab\x1bc".to_vec()), E]),
        (b"\x1b[200~x\x1b[201y\x1b[20\x1b[201~", vec![S, D(b"x\x1b[201y\x1b[20".to_vec()), E]),
        (b"\x1b[200~\x1b\x1b[201~", vec![S, D(b"\x1b".to_vec()), E]),
        (b"\x1b[200~\x1b[200~\x1b[201~", vec![S, D(b"\x1b[200~".to_vec()), E]),
        ("\x1b[200~é\r\n\x1b[201~a".as_bytes(), vec![S, D("é\r\n".as_bytes().to_vec()), E, ch('a')]),
        (b"x\x1b[200~y\x1b[201~\x1b[A", vec![ch('x'), S, D(b"y".to_vec()), E, key(Key::Up, Mods::NONE)]),
    ];
    for (b, e) in cases {
        check(b, &e);
    }
}

#[test]
fn huge_paste_streams_with_bounded_memory() {
    let mut p = InputParser::new();
    let mut out = Vec::new();
    p.feed(b"\x1b[200~", &mut out);
    assert_eq!(out, vec![InputEvent::PasteStart]);
    let chunk: Vec<u8> = (0..4096u32).map(|i| b"abc\x1bdef\n"[i as usize % 8]).collect();
    let mut total = 0;
    for _ in 0..1024 {
        out.clear();
        p.feed(&chunk, &mut out);
        for e in &out {
            match e {
                InputEvent::PasteData(d) => {
                    assert!(d.len() <= chunk.len());
                    total += d.len();
                }
                other => panic!("unexpected {other:?}"),
            }
        }
        assert!(p.buffered_len() <= MAX_PENDING);
    }
    out.clear();
    p.feed(b"\x1b[201", &mut out);
    p.feed(b"~", &mut out);
    assert_eq!(out.last(), Some(&InputEvent::PasteEnd));
    for e in &out {
        if let InputEvent::PasteData(d) = e {
            total += d.len();
        }
    }
    assert_eq!(total, 4096 * 1024);
    assert!(!p.in_paste());
}

#[test]
fn responses() {
    use ColorSlot::*;
    use TermResponse as T;
    let cases: Vec<(&[u8], Vec<TermResponse>)> = vec![
        (b"\x1b[?62;22;52c", vec![T::PrimaryDa(vec![62, 22, 52])]),
        (b"\x1b[?1;2c", vec![T::PrimaryDa(vec![1, 2])]),
        (b"\x9b?65;1;9c", vec![T::PrimaryDa(vec![65, 1, 9])]),
        (b"\x1b[>1;4000;0c", vec![T::SecondaryDa { model: 1, version: 4000, rom: 0 }]),
        (b"\x1bP!|7E4B4954\x1b\\", vec![T::TertiaryDa("7E4B4954".into())]),
        (b"\x1bP>|kitty(0.39.1)\x1b\\", vec![T::XtVersion("kitty(0.39.1)".into())]),
        (b"\x1bP>|WezTerm 20240203\x07", vec![T::XtVersion("WezTerm 20240203".into())]),
        (b"\x90>|foot(1.20.0)\x1b\\", vec![T::XtVersion("foot(1.20.0)".into())]),
        (b"\x1b[?31u", vec![T::KittyKeyboard(31)]),
        (b"\x1b[?0u", vec![T::KittyKeyboard(0)]),
        (b"\x1b[?2026;2$y", vec![T::ModeReport { mode: 2026, private: true, state: ModeState::Reset }]),
        (b"\x1b[?2027;0$y", vec![T::ModeReport { mode: 2027, private: true, state: ModeState::NotRecognized }]),
        (b"\x1b[?1016;1$y", vec![T::ModeReport { mode: 1016, private: true, state: ModeState::Set }]),
        (b"\x1b[?2048;4$y", vec![T::ModeReport { mode: 2048, private: true, state: ModeState::PermanentlyReset }]),
        (b"\x1b[4;3$y", vec![T::ModeReport { mode: 4, private: false, state: ModeState::PermanentlySet }]),
        (b"\x1b[12;40R", vec![T::CursorPosition { row: 12, col: 40 }]),
        (b"\x1b[1;1R", vec![T::CursorPosition { row: 1, col: 1 }]),
        (b"\x1b[?1;5;1R", vec![T::CursorPosition { row: 1, col: 5 }]),
        (b"\x1b]11;rgb:1e1e/1e1e/2e2e\x1b\\", vec![T::Color { slot: Background, rgb: Rgb::new(0x1e, 0x1e, 0x2e) }]),
        (b"\x1b]10;rgb:ff/80/00\x07", vec![T::Color { slot: Foreground, rgb: Rgb::new(255, 128, 0) }]),
        (b"\x1b]10;rgb:f/8/0\x07", vec![T::Color { slot: Foreground, rgb: Rgb::new(255, 136, 0) }]),
        (b"\x1b]10;rgb:fff/000/800\x07", vec![T::Color { slot: Foreground, rgb: Rgb::new(255, 0, 128) }]),
        (b"\x1b]12;#ff8000\x07", vec![T::Color { slot: Cursor, rgb: Rgb::new(255, 128, 0) }]),
        (b"\x1b]12;#f80\x07", vec![T::Color { slot: Cursor, rgb: Rgb::new(0xf0, 0x80, 0) }]),
        (b"\x1b]4;1;rgb:cdcd/0000/0000\x1b\\", vec![T::Color { slot: Palette(1), rgb: Rgb::new(205, 0, 0) }]),
        (
            b"\x1b]4;1;rgb:00/00/00;255;rgb:ff/ff/ff\x07",
            vec![
                T::Color { slot: Palette(1), rgb: Rgb::new(0, 0, 0) },
                T::Color { slot: Palette(255), rgb: Rgb::new(255, 255, 255) },
            ],
        ),
        (
            b"\x1b]10;rgb:00/00/00;rgb:ff/ff/ff\x07",
            vec![
                T::Color { slot: Foreground, rgb: Rgb::new(0, 0, 0) },
                T::Color { slot: Background, rgb: Rgb::new(255, 255, 255) },
            ],
        ),
        (b"\x9d11;rgb:00/00/00\x07", vec![T::Color { slot: Background, rgb: Rgb::new(0, 0, 0) }]),
        (
            b"\x1b]52;c;aGVsbG8gd29ybGQ=\x07",
            vec![T::Clipboard { selection: "c".into(), data: b"hello world".to_vec(), truncated: false }],
        ),
        (b"\x1b]52;;\x1b\\", vec![T::Clipboard { selection: "".into(), data: vec![], truncated: false }]),
        (b"\x1bP1+r544e=787465726d\x1b\\", vec![T::TermCap(vec![("TN".into(), Some("xterm".into()))])]),
        (
            b"\x1bP1+r616d;436f=323536\x1b\\",
            vec![T::TermCap(vec![("am".into(), None), ("Co".into(), Some("256".into()))])],
        ),
        (b"\x1bP0+r544e\x1b\\", vec![T::TermCapFailed(vec!["TN".into()])]),
        (b"\x1bP0+r\x1b\\", vec![T::TermCapFailed(vec![])]),
        (b"\x1b[?997;1n", vec![T::ColorScheme(ColorScheme::Dark)]),
        (b"\x1b[?997;2n", vec![T::ColorScheme(ColorScheme::Light)]),
        (
            b"\x1b[48;24;80;480;800t",
            vec![T::InBandResize { cells: Size::new(80, 24), pixels: PixelSize { width: 800, height: 480 } }],
        ),
        (b"\x1b[4;600;800t", vec![T::TextAreaPixels(PixelSize { width: 800, height: 600 })]),
        (b"\x1b[8;24;80t", vec![T::TextAreaCells(Size::new(80, 24))]),
        (b"\x1b[6;20;10t", vec![T::CellSize(PixelSize { width: 10, height: 20 })]),
    ];
    for (b, e) in cases {
        let e: Vec<InputEvent> = e.into_iter().map(resp).collect();
        check(b, &e);
    }
}

#[test]
fn cursor_position_vs_f3() {
    // Unrequested: xterm's modified F3.
    check(b"\x1b[1;5R", &[key(Key::F(3), Mods::CTRL)]);
    check(b"\x1b[R", &[key(Key::F(3), Mods::NONE)]);
    // Requested: a report, once.
    let mut p = InputParser::new();
    let mut out = Vec::new();
    p.expect_cursor_position();
    p.feed(b"\x1b[1;5R\x1b[1;5R", &mut out);
    assert_eq!(
        out,
        vec![resp(TermResponse::CursorPosition { row: 1, col: 5 }), key(Key::F(3), Mods::CTRL)]
    );
}

#[test]
fn unknown_sequences_are_kept() {
    let cases: Vec<(&[u8], &[u8])> = vec![
        (b"\x1b[5x", b"\x1b[5x"),
        (b"\x1b[?1049h", b"\x1b[?1049h"),
        (b"\x1b[>4;2m", b"\x1b[>4;2m"),
        (b"\x1b[1;2;3;4A", b"\x1b[1;2;3;4A"),
        (b"\x1b]133;A\x07", b"\x1b]133;A\x1b\\"),
        (b"\x1b]52;c;?\x07", b"\x1b]52;c;?\x1b\\"),
        (b"\x1b]11;nonsense\x1b\\", b"\x1b]11;nonsense\x1b\\"),
        (b"\x1b_Gi=1;OK\x1b\\", b"\x1b_Gi=1;OK\x1b\\"),
        (b"\x1bP1$r0m\x1b\\", b"\x1bP1$r0m\x1b\\"),
        (b"\x9e privacy\x1b\\", b"\x1b^ privacy\x1b\\"),
        (b"\x1b[201~", b"\x1b[201~"),
        (b"\x1b[<0;1M", b"\x1b[<0;1M"),
    ];
    for (b, e) in cases {
        check(b, &[InputEvent::Unknown(e.to_vec())]);
    }
    // Strings cut short by ESC or a control byte give the byte back.
    check(b"\x1b]11;rgb\x1b[A", &[InputEvent::Unknown(b"\x1b]11;rgb".to_vec()), key(Key::Up, Mods::NONE)]);
    check(b"\x1bP>|abc\x03", &[InputEvent::Unknown(b"\x1bP>|abc".to_vec()), key(Key::Char('c'), Mods::CTRL)]);
    // Incomplete CSI flushed.
    check(b"\x1b[1;5", &[InputEvent::Unknown(b"\x1b[1;5".to_vec())]);
    check(b"\x1b[1;5\x1b[B", &[InputEvent::Unknown(b"\x1b[1;5".to_vec()), key(Key::Down, Mods::NONE)]);
}

#[test]
fn osc52_is_bounded() {
    let mut p = InputParser::new();
    let mut out = Vec::new();
    p.feed(b"\x1b]52;c;", &mut out);
    let block = [b'Q'; 4096];
    for _ in 0..(MAX_STRING / 4096 + 64) {
        p.feed(&block, &mut out);
        assert!(p.buffered_len() <= MAX_STRING + MAX_PENDING);
    }
    p.feed(b"\x07z", &mut out);
    assert_eq!(out.len(), 2);
    match &out[0] {
        InputEvent::Response(TermResponse::Clipboard { truncated, data, .. }) => {
            assert!(*truncated);
            assert!(data.len() <= MAX_STRING / 4 * 3);
            assert!(data.len() >= MAX_STRING / 2);
        }
        e => panic!("{e:?}"),
    }
    assert_eq!(out[1], ch('z'));
    assert_eq!(p.buffered_len(), 0);
}

#[test]
fn lone_esc_pending_and_flush() {
    let mut p = InputParser::new();
    let mut out = Vec::new();
    p.feed(b"\x1b", &mut out);
    assert!(out.is_empty());
    assert_eq!(p.pending(), Pending::Ambiguous);
    assert_eq!(p.pending_timeout(), Some(DEFAULT_ESC_TIMEOUT));
    p.flush_pending(&mut out);
    assert_eq!(out, vec![key(Key::Escape, Mods::NONE)]);
    assert_eq!(p.pending(), Pending::None);
    assert_eq!(p.pending_timeout(), None);

    // Continued by the next read before the timer: decided by those bytes.
    out.clear();
    p.feed(b"\x1b", &mut out);
    p.feed(b"a", &mut out);
    assert_eq!(out, vec![key(Key::Char('a'), Mods::ALT)]);
    assert!(!p.has_pending());

    // ESC followed by more bytes in the same read: no pending state at all.
    out.clear();
    p.feed(b"\x1bx\x1b[A", &mut out);
    assert_eq!(out, vec![key(Key::Char('x'), Mods::ALT), key(Key::Up, Mods::NONE)]);
    assert!(!p.has_pending());

    // Plain typing never leaves anything pending.
    out.clear();
    p.feed(b"hello", &mut out);
    assert_eq!(out.len(), 5);
    assert!(!p.has_pending());

    for (b, want) in [
        (&b"\x1b["[..], Pending::Ambiguous),
        (b"\x1bO", Pending::Ambiguous),
        (b"\x1b\x1b", Pending::Ambiguous),
        (b"\x1b[1;5", Pending::Incomplete),
        (b"\xe2\x82", Pending::Incomplete),
        (b"\x1b]11;rgb:", Pending::Incomplete),
        (b"\x1b[M ", Pending::Incomplete),
    ] {
        let mut p = InputParser::new();
        p.feed(b, &mut out);
        assert_eq!(p.pending(), want, "{b:?}");
        p.flush_pending(&mut out);
        assert!(!p.has_pending());
    }

    // Kitty protocol active: a trailing ESC is certainly a split sequence.
    let mut p = InputParser::new();
    p.set_kitty_keyboard(true);
    out.clear();
    p.feed(b"\x1b", &mut out);
    assert_eq!(p.pending(), Pending::Incomplete);
    assert_eq!(p.pending_timeout(), Some(DEFAULT_INCOMPLETE_TIMEOUT));
    p.feed(b"[27u", &mut out);
    assert_eq!(out, vec![key(Key::Escape, Mods::NONE)]);
}

#[test]
fn pending_is_capped() {
    let mut p = InputParser::new();
    let mut out = Vec::new();
    let mut seq = b"\x1b[".to_vec();
    seq.extend(std::iter::repeat_n(b'1', 1000));
    for b in seq.chunks(7) {
        p.feed(b, &mut out);
        assert!(p.buffered_len() < MAX_PENDING);
    }
    p.feed(b"a", &mut out);
    p.flush_pending(&mut out);
    assert!(matches!(out[0], InputEvent::Unknown(_)));
    assert_eq!(out.last(), Some(&ch('a')));
}

struct XorShift(u64);
impl XorShift {
    fn next(&mut self) -> u64 {
        self.0 ^= self.0 << 13;
        self.0 ^= self.0 >> 7;
        self.0 ^= self.0 << 17;
        self.0
    }
    fn below(&mut self, n: usize) -> usize {
        (self.next() % n as u64) as usize
    }
}

#[test]
fn garbage_never_panics_or_loses_the_next_key() {
    const ALPHABET: &[u8] = b"\x1b\x1b\x1b[[]]PO_^X<>?=;;:0123456789Mmu~$yRctn\\\x07\x9b\x9d\x90\x8f\x18 a\xc3\xa9\xe2\x82\xf0";
    let keys: Vec<(&[u8], InputEvent)> = vec![
        (b"a", ch('a')),
        (b"\x1b[A", key(Key::Up, Mods::NONE)),
        (b"\x1b[97;5u", key(Key::Char('a'), Mods::CTRL)),
        ("é".as_bytes(), ch('é')),
        (b"\x1b[3;5~", key(Key::Delete, Mods::CTRL)),
    ];
    let mut rng = XorShift(0x2545_f491_4f6c_dd1d);
    let mut out = Vec::new();
    for round in 0..50_000 {
        let mut p = InputParser::new();
        p.set_kitty_keyboard(round % 2 == 0);
        p.set_sgr_pixels(round % 3 == 0);
        let len = rng.below(80);
        let garbage: Vec<u8> = (0..len)
            .map(|_| if rng.below(5) == 0 { rng.next() as u8 } else { ALPHABET[rng.below(ALPHABET.len())] })
            .collect();
        let mut rest = &garbage[..];
        while !rest.is_empty() {
            let n = 1 + rng.below(rest.len());
            p.feed(&rest[..n], &mut out);
            rest = &rest[n..];
            assert!(p.buffered_len() <= MAX_PENDING + MAX_STRING);
            assert!(p.buffered_len() <= MAX_PENDING + garbage.len());
        }
        // The pending timer fires, and a paste left open is closed.
        p.flush_pending(&mut out);
        p.feed(b"\x1b[201~", &mut out);
        p.flush_pending(&mut out);
        assert!(!p.has_pending() && !p.in_paste(), "{garbage:?}");
        let (k, want) = &keys[rng.below(keys.len())];
        out.clear();
        p.feed(k, &mut out);
        p.flush_pending(&mut out);
        assert_eq!(&out, std::slice::from_ref(want), "after {garbage:?}");
        out.clear();
    }
}

#[test]
fn random_bytes_in_random_chunks_never_panic() {
    let mut rng = XorShift(0x9e37_79b9_7f4a_7c15);
    let mut p = InputParser::new();
    let mut out = Vec::new();
    let buf: Vec<u8> = (0..2_000_000).map(|_| rng.next() as u8).collect();
    let mut rest = &buf[..];
    while !rest.is_empty() {
        let n = (1 + rng.below(512)).min(rest.len());
        p.feed(&rest[..n], &mut out);
        rest = &rest[n..];
        if rng.below(4) == 0 {
            p.flush_pending(&mut out);
        }
        assert!(p.buffered_len() <= MAX_PENDING + MAX_STRING);
        out.clear();
    }
}

fn items(b: &[u8]) -> Vec<Vec<u8>> {
    let mut v: Vec<Vec<u8>> = Vec::new();
    for &c in b {
        if c == 0x1b {
            v.push(Vec::new());
        }
        v.last_mut().unwrap().push(c);
    }
    v
}

fn inverse(s: &[u8]) -> Vec<u8> {
    let t = String::from_utf8(s.to_vec()).unwrap();
    let r = match t.as_str() {
        "\x1b=" => "\x1b>".to_string(),
        "\x1b[>31u" => "\x1b[<u".to_string(),
        "\x1b[>4;2m" => "\x1b[>4m".to_string(),
        x if x.starts_with("\x1b[?") && x.ends_with('h') => format!("{}l", &x[..x.len() - 1]),
        x => panic!("no inverse known for {x:?}"),
    };
    r.into_bytes()
}

#[test]
fn enter_and_leave_are_inverses() {
    let all = OuterCaps { kitty_keyboard: true, sync_output: true, ..OuterCaps::default() };
    let none = OuterCaps { focus_events: false, bracketed_paste: false, sgr_mouse: false, ..OuterCaps::default() };
    for caps in [OuterCaps::default(), all, none] {
        let mut enter = Vec::new();
        write_enter(&caps, &mut enter);
        let mut leave = Vec::new();
        write_leave(&caps, &mut leave);
        let e = items(&enter);
        let l = items(&leave);
        // Every enter item is undone, in reverse order.
        let mut last = l.len();
        for item in &e {
            let inv = inverse(item);
            let pos = l.iter().position(|x| *x == inv).unwrap_or_else(|| panic!("{inv:?} missing"));
            assert!(pos < last, "{inv:?} out of order");
            last = pos;
        }
        assert_eq!(l.last().unwrap(), b"\x1b[?1049l");
        // The leave sequence parses as well-formed sequences only.
        let mut p = InputParser::new();
        let mut out = Vec::new();
        p.feed(&leave, &mut out);
        assert!(!p.has_pending());
        // (ESC > reads back as Alt+> since it is not a CSI sequence.)
        let alt_gt = key(Key::Char('>'), Mods::ALT);
        assert!(out.iter().all(|e| matches!(e, InputEvent::Unknown(_)) || *e == alt_gt), "{out:?}");
    }
}

#[test]
fn mouse_enable_disable() {
    let caps = OuterCaps::default();
    let mut on = Vec::new();
    write_mouse(true, true, &caps, &mut on);
    assert_eq!(on, b"\x1b[?1006h\x1b[?1000h\x1b[?1002h\x1b[?1003h");
    let mut off = Vec::new();
    write_mouse(false, false, &caps, &mut off);
    assert_eq!(off, b"\x1b[?1003l\x1b[?1002l\x1b[?1000l\x1b[?1016l\x1b[?1006l");
    let mut x10 = Vec::new();
    write_mouse(true, false, &OuterCaps { sgr_mouse: false, ..caps }, &mut x10);
    assert_eq!(x10, b"\x1b[?1000h\x1b[?1002h\x1b[?1003l");
}

#[test]
fn queries_end_with_da1() {
    let mut q = Vec::new();
    write_queries(&mut q);
    assert!(q.ends_with(b"\x1b[c"));
    assert_eq!(q.windows(3).filter(|w| *w == b"\x1b[c").count(), 1);
    for m in QUERIED_MODES {
        assert!(q.windows(10).any(|w| w.starts_with(format!("\x1b[?{m}$p").as_bytes())));
    }
}

#[test]
fn probe_from_replies() {
    let mut probe = CapsProbe::from_env("xterm-kitty", "truecolor", "");
    assert!(probe.caps().truecolor);
    assert!(!probe.caps().kitty_keyboard);
    let replies: &[u8] = b"\x1b[?0u\x1b[?2026;2$y\x1b[?2027;0$y\x1b[?1016;2$y\x1b[?2031;2$y\x1b[?2048;2$y\
        \x1bP>|kitty(0.39.1)\x1b\\\x1b]10;rgb:dddd/dddd/dddd\x1b\\\x1b]11;rgb:0000/0000/0000\x1b\\\
        \x1b[6;20;10t\x1b[?62;52c";
    let mut p = InputParser::new();
    let mut out = Vec::new();
    p.feed(replies, &mut out);
    assert!(!p.has_pending());
    let mut done = Vec::new();
    for e in &out {
        let InputEvent::Response(r) = e else { panic!("{e:?}") };
        done.push(probe.apply(r));
    }
    assert_eq!(done.iter().filter(|&&d| d).count(), 1);
    assert!(*done.last().unwrap());
    let c = probe.caps();
    assert!(c.kitty_keyboard && c.sync_output && c.sgr_pixel_mouse && c.osc52_write);
    assert!(!c.grapheme_clusters);
    assert!(probe.in_band_resize() && probe.color_scheme_reports());
    assert_eq!(probe.kitty_keyboard_flags(), Some(0));
    assert_eq!(probe.version(), Some("kitty(0.39.1)"));
    assert_eq!(probe.background(), Some(Rgb::new(0, 0, 0)));
    assert!(probe.background().unwrap().is_dark());
    assert_eq!(probe.cell_size(), PixelSize { width: 10, height: 20 });
    assert_eq!(p.cell_size(), PixelSize { width: 10, height: 20 });

    // A terminal that answers only DA1.
    let mut probe = CapsProbe::from_env("xterm-256color", "", "Apple_Terminal");
    assert!(probe.apply(&TermResponse::PrimaryDa(vec![1, 2])));
    assert_eq!(probe.caps(), OuterCaps::default());
}
