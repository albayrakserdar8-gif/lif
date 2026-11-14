mod common;

use common::*;
use lif_input::InputEvent;
use lif_types::{Key, KeyAction as A, KeypadKey as P, Mods};

const S: Mods = Mods::SHIFT;
const AL: Mods = Mods::ALT;
const C: Mods = Mods::CTRL;
const N: Mods = Mods::NONE;

#[test]
fn text_and_utf8() {
    let r = char::REPLACEMENT_CHARACTER;
    let cases: Vec<(&[u8], Vec<InputEvent>)> = vec![
        (b"a", vec![ch('a')]),
        (b"A", vec![ch('A')]),
        (b" ", vec![ch(' ')]),
        (b"hi!", vec![ch('h'), ch('i'), ch('!')]),
        ("é".as_bytes(), vec![ch('é')]),
        ("€".as_bytes(), vec![ch('€')]),
        ("😀".as_bytes(), vec![ch('😀')]),
        ("aé€😀z".as_bytes(), vec![ch('a'), ch('é'), ch('€'), ch('😀'), ch('z')]),
        (b"\xff", vec![ch(r)]),
        (b"\xc3\x28", vec![ch(r), ch('(')]),
        (b"\xe2\x82", vec![ch(r)]),
        (b"\xe2\x82x", vec![ch(r), ch('x')]),
        (b"\xed\xa0\x80", vec![ch(r), ch(r), ch(r)]),
        (b"\xc0\xaf", vec![ch(r), ch(r)]),
        (b"\x80", vec![ch(r)]),
    ];
    for (b, e) in cases {
        check(b, &e);
    }
}

#[test]
fn c0_controls() {
    let cases: Vec<(&[u8], InputEvent)> = vec![
        (b"\x00", key(Key::Char(' '), C)),
        (b"\x01", key(Key::Char('a'), C)),
        (b"\x03", key(Key::Char('c'), C)),
        (b"\x08", key(Key::Char('h'), C)),
        (b"\x09", key(Key::Tab, N)),
        (b"\x0a", key(Key::Char('j'), C)),
        (b"\x0d", key(Key::Enter, N)),
        (b"\x1a", key(Key::Char('z'), C)),
        (b"\x1c", key(Key::Char('\\'), C)),
        (b"\x1d", key(Key::Char(']'), C)),
        (b"\x1e", key(Key::Char('^'), C)),
        (b"\x1f", key(Key::Char('_'), C)),
        (b"\x7f", key(Key::Backspace, N)),
    ];
    for (b, e) in cases {
        check(b, &[e]);
    }
}

#[test]
fn esc_prefixed_alt() {
    let cases: Vec<(&[u8], Vec<InputEvent>)> = vec![
        (b"\x1b", vec![key(Key::Escape, N)]),
        (b"\x1ba", vec![key(Key::Char('a'), AL)]),
        (b"\x1bA", vec![key(Key::Char('A'), AL)]),
        (b"\x1b\x01", vec![key(Key::Char('a'), m(&[AL, C]))]),
        (b"\x1b\x7f", vec![key(Key::Backspace, AL)]),
        (b"\x1b\x0d", vec![key(Key::Enter, AL)]),
        (b"\x1b ", vec![key(Key::Char(' '), AL)]),
        ("\x1bé".as_bytes(), vec![key(Key::Char('é'), AL)]),
        (b"\x1b\x1b", vec![key(Key::Escape, AL)]),
        (b"\x1b\x1b\x1b", vec![key(Key::Escape, N), key(Key::Escape, AL)]),
        (b"\x1b\x1b[A", vec![key(Key::Up, AL)]),
        (b"\x1b\x1bOB", vec![key(Key::Down, AL)]),
        (b"\x1b\x1bx", vec![key(Key::Char('x'), AL)]),
        (b"\x1b[", vec![key(Key::Char('['), AL)]),
        (b"\x1bO", vec![key(Key::Char('O'), AL)]),
        (b"\x1bP", vec![key(Key::Char('P'), AL)]),
        (b"\x1bPx", vec![key(Key::Char('P'), AL), ch('x')]),
        (b"\x1b]", vec![key(Key::Char(']'), AL)]),
        (b"\x1b]a", vec![key(Key::Char(']'), AL), ch('a')]),
        (b"\x1b_", vec![key(Key::Char('_'), AL)]),
        (b"\x1b[\x03", vec![key(Key::Char('['), AL), key(Key::Char('c'), C)]),
        (b"a\x1bb", vec![ch('a'), key(Key::Char('b'), AL)]),
    ];
    for (b, e) in cases {
        check(b, &e);
    }
}

#[test]
fn cursor_and_function_keys() {
    let cases: Vec<(&[u8], InputEvent)> = vec![
        (b"\x1b[A", key(Key::Up, N)),
        (b"\x1b[B", key(Key::Down, N)),
        (b"\x1b[C", key(Key::Right, N)),
        (b"\x1b[D", key(Key::Left, N)),
        (b"\x1bOA", key(Key::Up, N)),
        (b"\x1bOD", key(Key::Left, N)),
        (b"\x1b[H", key(Key::Home, N)),
        (b"\x1b[F", key(Key::End, N)),
        (b"\x1bOH", key(Key::Home, N)),
        (b"\x1bOF", key(Key::End, N)),
        (b"\x1b[E", key(Key::Keypad(P::Begin), N)),
        (b"\x1b[1;5A", key(Key::Up, C)),
        (b"\x1b[1;2B", key(Key::Down, S)),
        (b"\x1b[1;3C", key(Key::Right, AL)),
        (b"\x1b[1;9D", key(Key::Left, Mods::SUPER)),
        (b"\x1b[1;8H", key(Key::Home, m(&[S, AL, C]))),
        (b"\x1b[1;17F", key(Key::End, Mods::HYPER)),
        (b"\x1b[1;33A", key(Key::Up, Mods::META)),
        (b"\x1b[1;64A", key(Key::Up, Mods(63))),
        (b"\x1b[1;65A", key(Key::Up, N)),
        (b"\x1bOP", key(Key::F(1), N)),
        (b"\x1bOQ", key(Key::F(2), N)),
        (b"\x1bOR", key(Key::F(3), N)),
        (b"\x1bOS", key(Key::F(4), N)),
        (b"\x1b[P", key(Key::F(1), N)),
        (b"\x1b[1;2P", key(Key::F(1), S)),
        (b"\x1b[1;5S", key(Key::F(4), C)),
        (b"\x1bO5P", key(Key::F(1), C)),
        (b"\x1bO1;2Q", key(Key::F(2), S)),
        (b"\x1b[Z", key(Key::Tab, S)),
        (b"\x1b[[A", key(Key::F(1), N)),
        (b"\x1b[[E", key(Key::F(5), N)),
        (b"\x9bA", key(Key::Up, N)),
        (b"\x8fP", key(Key::F(1), N)),
        (b"\x1b[2~", key(Key::Insert, N)),
        (b"\x1b[3~", key(Key::Delete, N)),
        (b"\x1b[3;5~", key(Key::Delete, C)),
        (b"\x1b[1~", key(Key::Home, N)),
        (b"\x1b[4~", key(Key::End, N)),
        (b"\x1b[5~", key(Key::PageUp, N)),
        (b"\x1b[6;3~", key(Key::PageDown, AL)),
        (b"\x1b[7~", key(Key::Home, N)),
        (b"\x1b[8~", key(Key::End, N)),
        (b"\x1b[11~", key(Key::F(1), N)),
        (b"\x1b[12~", key(Key::F(2), N)),
        (b"\x1b[13~", key(Key::F(3), N)),
        (b"\x1b[14~", key(Key::F(4), N)),
        (b"\x1b[15~", key(Key::F(5), N)),
        (b"\x1b[17~", key(Key::F(6), N)),
        (b"\x1b[18~", key(Key::F(7), N)),
        (b"\x1b[19~", key(Key::F(8), N)),
        (b"\x1b[20~", key(Key::F(9), N)),
        (b"\x1b[21~", key(Key::F(10), N)),
        (b"\x1b[23~", key(Key::F(11), N)),
        (b"\x1b[24;2~", key(Key::F(12), S)),
        (b"\x1b[25~", key(Key::F(13), N)),
        (b"\x1b[26~", key(Key::F(14), N)),
        (b"\x1b[28~", key(Key::F(15), N)),
        (b"\x1b[29~", key(Key::Menu, N)),
        (b"\x1b[31~", key(Key::F(17), N)),
        (b"\x1b[34~", key(Key::F(20), N)),
        (b"\x1b[57427~", key(Key::Keypad(P::Begin), N)),
    ];
    for (b, e) in cases {
        check(b, &[e]);
    }
}

#[test]
fn application_keypad() {
    let cases: Vec<(&[u8], InputEvent)> = vec![
        (b"\x1bOp", keyt(Key::Keypad(P::Digit(0)), N, "0")),
        (b"\x1bOy", keyt(Key::Keypad(P::Digit(9)), N, "9")),
        (b"\x1bOj", keyt(Key::Keypad(P::Multiply), N, "*")),
        (b"\x1bOk", keyt(Key::Keypad(P::Add), N, "+")),
        (b"\x1bOl", keyt(Key::Keypad(P::Separator), N, ",")),
        (b"\x1bOm", keyt(Key::Keypad(P::Subtract), N, "-")),
        (b"\x1bOn", keyt(Key::Keypad(P::Decimal), N, ".")),
        (b"\x1bOo", keyt(Key::Keypad(P::Divide), N, "/")),
        (b"\x1bOX", keyt(Key::Keypad(P::Equal), N, "=")),
        (b"\x1bOM", key(Key::Keypad(P::Enter), N)),
        (b"\x1bO5M", key(Key::Keypad(P::Enter), C)),
    ];
    for (b, e) in cases {
        check(b, &[e]);
    }
}

#[test]
fn modify_other_keys_and_fixterms() {
    let cases: Vec<(&[u8], InputEvent)> = vec![
        (b"\x1b[27;5;105~", key(Key::Char('i'), C)),
        (b"\x1b[27;2;65~", keyt(Key::Char('A'), S, "A")),
        (b"\x1b[27;5;13~", key(Key::Enter, C)),
        (b"\x1b[27;6;9~", key(Key::Tab, m(&[S, C]))),
        (b"\x1b[27;3;127~", key(Key::Backspace, AL)),
        (b"\x1b[27;5;27~", key(Key::Escape, C)),
        (b"\x1b[105;5u", key(Key::Char('i'), C)),
        (b"\x1b[13;2u", key(Key::Enter, S)),
        (b"\x1b[59;5u", key(Key::Char(';'), C)),
    ];
    for (b, e) in cases {
        check(b, &[e]);
    }
}

#[test]
fn kitty_protocol() {
    let cases: Vec<(&[u8], InputEvent)> = vec![
        (b"\x1b[27u", key(Key::Escape, N)),
        (b"\x1b[13u", key(Key::Enter, N)),
        (b"\x1b[9u", key(Key::Tab, N)),
        (b"\x1b[127u", key(Key::Backspace, N)),
        (b"\x1b[9;2u", key(Key::Tab, S)),
        (b"\x1b[97u", keyt(Key::Char('a'), N, "a")),
        (b"\x1b[32u", keyt(Key::Char(' '), N, " ")),
        (b"\x1b[97;2u", keyt(Key::Char('a'), S, "A")),
        (b"\x1b[97:65;2u", keyt(Key::Char('a'), S, "A")),
        (b"\x1b[49:33;2u", keyt(Key::Char('1'), S, "!")),
        (b"\x1b[97:65:97;2;65u", keyt(Key::Char('a'), S, "A")),
        (b"\x1b[1092::97;1;1092u", keyt(Key::Char('ф'), N, "ф")),
        (b"\x1b[97;5u", key(Key::Char('a'), C)),
        (b"\x1b[97;3u", key(Key::Char('a'), AL)),
        (b"\x1b[97;9u", key(Key::Char('a'), Mods::SUPER)),
        (b"\x1b[97;17u", key(Key::Char('a'), Mods::HYPER)),
        (b"\x1b[97;33u", key(Key::Char('a'), Mods::META)),
        (b"\x1b[97;64u", key(Key::Char('a'), Mods(63))),
        (b"\x1b[97;65u", keyt(Key::Char('a'), N, "A")),
        (b"\x1b[97;129u", keyt(Key::Char('a'), N, "a")),
        (b"\x1b[97;197u", key(Key::Char('a'), C)),
        (b"\x1b[97;1:1u", keyt(Key::Char('a'), N, "a")),
        (b"\x1b[97;1:2u", keya(Key::Char('a'), N, A::Repeat, "a")),
        (b"\x1b[97;1:3u", keya(Key::Char('a'), N, A::Release, "")),
        (b"\x1b[97;;97u", keyt(Key::Char('a'), N, "a")),
        (b"\x1b[0;;229u", keyt(Key::Char('å'), N, "å")),
        (b"\x1b[0;;104:105u", keyt(Key::Char('h'), N, "hi")),
        (b"\x1b[57358u", key(Key::CapsLock, N)),
        (b"\x1b[57359u", key(Key::ScrollLock, N)),
        (b"\x1b[57360u", key(Key::NumLock, N)),
        (b"\x1b[57361u", key(Key::PrintScreen, N)),
        (b"\x1b[57362u", key(Key::Pause, N)),
        (b"\x1b[57363u", key(Key::Menu, N)),
        (b"\x1b[57376u", key(Key::F(13), N)),
        (b"\x1b[57398;5u", key(Key::F(35), C)),
        (b"\x1b[57399u", keyt(Key::Keypad(P::Digit(0)), N, "0")),
        (b"\x1b[57408;1;57u", keyt(Key::Keypad(P::Digit(9)), N, "9")),
        (b"\x1b[57409u", keyt(Key::Keypad(P::Decimal), N, ".")),
        (b"\x1b[57413u", keyt(Key::Keypad(P::Add), N, "+")),
        (b"\x1b[57414u", key(Key::Keypad(P::Enter), N)),
        (b"\x1b[57417u", key(Key::Keypad(P::Left), N)),
        (b"\x1b[57426u", key(Key::Keypad(P::Delete), N)),
        (b"\x1b[57427u", key(Key::Keypad(P::Begin), N)),
        (b"\x1b[57428u", InputEvent::OtherKey { code: 57428, mods: N, action: A::Press }),
        (b"\x1b[57440u", InputEvent::OtherKey { code: 57440, mods: N, action: A::Press }),
        (b"\x1b[57441;2u", InputEvent::OtherKey { code: 57441, mods: S, action: A::Press }),
        (b"\x1b[57442;1:3u", InputEvent::OtherKey { code: 57442, mods: N, action: A::Release }),
        (b"\x1b[57454u", InputEvent::OtherKey { code: 57454, mods: N, action: A::Press }),
        // Legacy-form functional keys with event types.
        (b"\x1b[1;5:3A", keya(Key::Up, C, A::Release, "")),
        (b"\x1b[1;1:2B", keya(Key::Down, N, A::Repeat, "")),
        (b"\x1b[3;1:2~", keya(Key::Delete, N, A::Repeat, "")),
        (b"\x1b[1;2:1P", key(Key::F(1), S)),
        (b"\x1b[1;1:3R", keya(Key::F(3), N, A::Release, "")),
        (b"\x1b[1;5:3E", keya(Key::Keypad(P::Begin), C, A::Release, "")),
        (b"\x1b[13;1:3u", keya(Key::Enter, N, A::Release, "")),
    ];
    for (b, e) in cases {
        check(b, &[e]);
    }
}

#[test]
fn kitty_mode_flag_changes_no_decoding() {
    let b: &[u8] = b"a\x1b[97;5u\x1b\x1b[A\x1b[27u";
    let legacy = run_chunks(&|_| {}, &[b]);
    let kitty = run_chunks(&|p| p.set_kitty_keyboard(true), &[b]);
    assert_eq!(legacy, kitty);
}
