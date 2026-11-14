#![allow(dead_code)]

use lif_input::{InputEvent, InputParser};
use lif_types::{Key, KeyAction, KeyEvent, Mods};

/// Adjacent PasteData chunks depend on read boundaries; merge them.
pub fn normalize(evs: Vec<InputEvent>) -> Vec<InputEvent> {
    let mut out: Vec<InputEvent> = Vec::new();
    for e in evs {
        if let InputEvent::PasteData(d) = &e
            && let Some(InputEvent::PasteData(prev)) = out.last_mut()
        {
            prev.extend_from_slice(d);
            continue;
        }
        out.push(e);
    }
    out
}

pub fn run_chunks(setup: &dyn Fn(&mut InputParser), chunks: &[&[u8]]) -> Vec<InputEvent> {
    let mut p = InputParser::new();
    setup(&mut p);
    let mut out = Vec::new();
    for c in chunks {
        p.feed(c, &mut out);
    }
    p.flush_pending(&mut out);
    assert!(!p.has_pending());
    normalize(out)
}

/// Decodes \`bytes\` whole, split at every boundary, and one byte at a time;
/// all must give \`expected\`.
pub fn check_with(setup: &dyn Fn(&mut InputParser), bytes: &[u8], expected: &[InputEvent]) {
    let whole = run_chunks(setup, &[bytes]);
    assert_eq!(whole, expected, "whole input {:?}", String::from_utf8_lossy(bytes));
    for i in 1..bytes.len() {
        let got = run_chunks(setup, &[&bytes[..i], &bytes[i..]]);
        assert_eq!(got, expected, "split at {i} of {:?}", String::from_utf8_lossy(bytes));
    }
    let singles: Vec<&[u8]> = bytes.chunks(1).collect();
    assert_eq!(run_chunks(setup, &singles), expected, "byte at a time {:?}", String::from_utf8_lossy(bytes));
}

pub fn check(bytes: &[u8], expected: &[InputEvent]) {
    check_with(&|_| {}, bytes, expected)
}

pub fn key(k: Key, mods: Mods) -> InputEvent {
    InputEvent::Key(KeyEvent::new(k, mods))
}

pub fn keyt(k: Key, mods: Mods, text: &str) -> InputEvent {
    InputEvent::Key(KeyEvent { key: k, mods, action: KeyAction::Press, text: text.into() })
}

pub fn keya(k: Key, mods: Mods, action: KeyAction, text: &str) -> InputEvent {
    InputEvent::Key(KeyEvent { key: k, mods, action, text: text.into() })
}

/// A plain typed character.
pub fn ch(c: char) -> InputEvent {
    keyt(Key::Char(c), Mods::NONE, &c.to_string())
}

pub fn m(bits: &[Mods]) -> Mods {
    bits.iter().fold(Mods::NONE, |a, &b| a.union(b))
}
