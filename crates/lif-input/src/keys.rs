//! Key tables shared by the legacy, xterm and kitty decoders.

use lif_types::{Key, KeyAction, KeyEvent, KeypadKey, Mods};

use crate::InputEvent;

/// Key text kept inline so decoding a key does not allocate until the event
/// is built.
#[derive(Clone, Copy, PartialEq, Eq, Debug)]
pub(crate) struct Text {
    buf: [u8; 32],
    len: u8,
}

impl Text {
    pub const EMPTY: Text = Text { buf: [0; 32], len: 0 };

    pub fn push(&mut self, c: char) {
        let n = c.len_utf8();
        if self.len as usize + n <= self.buf.len() {
            c.encode_utf8(&mut self.buf[self.len as usize..]);
            self.len += n as u8;
        }
    }

    pub fn of(c: char) -> Text {
        let mut t = Text::EMPTY;
        t.push(c);
        t
    }

    pub fn as_str(&self) -> &str {
        // Only whole chars are ever pushed.
        core::str::from_utf8(&self.buf[..self.len as usize]).unwrap_or("")
    }

    pub fn is_empty(&self) -> bool {
        self.len == 0
    }
}

#[derive(Clone, Copy, PartialEq, Eq, Debug)]
pub(crate) enum Kind {
    Key(Key),
    Other(u32),
}

/// A decoded key before it becomes an [`InputEvent`].
#[derive(Clone, Copy, PartialEq, Eq, Debug)]
pub(crate) struct K {
    pub kind: Kind,
    pub mods: Mods,
    pub action: KeyAction,
    pub text: Text,
}

impl K {
    pub fn new(key: Key, mods: Mods) -> K {
        K { kind: Kind::Key(key), mods, action: KeyAction::Press, text: Text::EMPTY }
    }

    /// A key that types `c` (text implied).
    pub fn typed(c: char) -> K {
        let text = if c.is_control() { Text::EMPTY } else { Text::of(c) };
        K { kind: Kind::Key(Key::Char(c)), mods: Mods::NONE, action: KeyAction::Press, text }
    }

    /// The ESC-prefixed (legacy Alt) form of this key.
    pub fn with_alt(mut self) -> K {
        self.mods = self.mods.union(Mods::ALT);
        self.text = Text::EMPTY;
        self
    }

    pub fn into_event(self) -> InputEvent {
        match self.kind {
            Kind::Key(key) => InputEvent::Key(KeyEvent {
                key,
                mods: self.mods,
                action: self.action,
                text: if self.text.is_empty() { String::new() } else { String::from(self.text.as_str()) },
            }),
            Kind::Other(code) => InputEvent::OtherKey { code, mods: self.mods, action: self.action },
        }
    }
}

/// A single byte below 0x80 that is not ESC.
pub(crate) fn byte_key(c: u8) -> K {
    match c {
        // Ctrl+Space and Ctrl+@ both send NUL; Ctrl+Space is the key people press.
        0x00 => K::new(Key::Char(' '), Mods::CTRL),
        0x09 => K::new(Key::Tab, Mods::NONE),
        0x0d => K::new(Key::Enter, Mods::NONE),
        // 0x08 is Ctrl+H on every terminal that sends DEL (0x7f) for
        // Backspace, which is all current ones; terminals that send 0x08 for
        // Backspace leave Ctrl+H indistinguishable either way.
        0x01..=0x1a => K::new(Key::Char((c + 0x60) as char), Mods::CTRL),
        0x1c => K::new(Key::Char('\\'), Mods::CTRL),
        0x1d => K::new(Key::Char(']'), Mods::CTRL),
        0x1e => K::new(Key::Char('^'), Mods::CTRL),
        0x1f => K::new(Key::Char('_'), Mods::CTRL),
        0x7f => K::new(Key::Backspace, Mods::NONE),
        0x1b => K::new(Key::Escape, Mods::NONE),
        _ => K::typed(c as char),
    }
}

/// xterm / kitty modifier parameter (1 + bits) to [`Mods`]. Caps Lock and
/// Num Lock (64, 128) are dropped.
pub(crate) fn mods_param(p: u32) -> Mods {
    Mods((p.saturating_sub(1) & 0x3f) as u8)
}

/// Raw lock bits of a modifier parameter.
pub(crate) fn caps_lock(p: u32) -> bool {
    p.saturating_sub(1) & 64 != 0
}

pub(crate) fn action_param(p: u32) -> KeyAction {
    match p {
        2 => KeyAction::Repeat,
        3 => KeyAction::Release,
        _ => KeyAction::Press,
    }
}

/// Kitty / fixterms / modifyOtherKeys key number to a key.
pub(crate) fn code_key(code: u32) -> Option<Kind> {
    use KeypadKey as P;
    let k = match code {
        9 => Key::Tab,
        13 => Key::Enter,
        27 => Key::Escape,
        8 | 127 => Key::Backspace,
        57358 => Key::CapsLock,
        57359 => Key::ScrollLock,
        57360 => Key::NumLock,
        57361 => Key::PrintScreen,
        57362 => Key::Pause,
        57363 => Key::Menu,
        57376..=57398 => Key::F((code - 57376 + 13) as u8),
        57399..=57408 => Key::Keypad(P::Digit((code - 57399) as u8)),
        57409 => Key::Keypad(P::Decimal),
        57410 => Key::Keypad(P::Divide),
        57411 => Key::Keypad(P::Multiply),
        57412 => Key::Keypad(P::Subtract),
        57413 => Key::Keypad(P::Add),
        57414 => Key::Keypad(P::Enter),
        57415 => Key::Keypad(P::Equal),
        57416 => Key::Keypad(P::Separator),
        57417 => Key::Keypad(P::Left),
        57418 => Key::Keypad(P::Right),
        57419 => Key::Keypad(P::Up),
        57420 => Key::Keypad(P::Down),
        57421 => Key::Keypad(P::PageUp),
        57422 => Key::Keypad(P::PageDown),
        57423 => Key::Keypad(P::Home),
        57424 => Key::Keypad(P::End),
        57425 => Key::Keypad(P::Insert),
        57426 => Key::Keypad(P::Delete),
        57427 => Key::Keypad(P::Begin),
        0xe000..=0xf8ff => return Some(Kind::Other(code)),
        0..=31 => return None,
        _ => Key::Char(char::from_u32(code)?),
    };
    Some(Kind::Key(k))
}

/// Number of a `CSI n ~` key.
pub(crate) fn tilde_key(n: u32) -> Option<Key> {
    Some(match n {
        1 | 7 => Key::Home,
        2 => Key::Insert,
        3 => Key::Delete,
        4 | 8 => Key::End,
        5 => Key::PageUp,
        6 => Key::PageDown,
        11..=15 => Key::F((n - 10) as u8),
        17..=21 => Key::F((n - 11) as u8),
        23..=26 => Key::F((n - 12) as u8),
        28 => Key::F(15),
        // xterm's F16 and the PC Menu key share 29~ (foot, urxvt, VTE send it
        // for Menu); a keyboard with a Menu key is far more common than F16.
        29 => Key::Menu,
        31..=34 => Key::F((n - 14) as u8),
        57427 => Key::Keypad(KeypadKey::Begin),
        _ => return None,
    })
}

/// Final byte of `CSI [1;mods] X` / `SS3 X` cursor and function keys.
pub(crate) fn letter_key(f: u8) -> Option<Key> {
    Some(match f {
        b'A' => Key::Up,
        b'B' => Key::Down,
        b'C' => Key::Right,
        b'D' => Key::Left,
        b'E' => Key::Keypad(KeypadKey::Begin),
        b'F' => Key::End,
        b'H' => Key::Home,
        b'P' => Key::F(1),
        b'Q' => Key::F(2),
        b'R' => Key::F(3),
        b'S' => Key::F(4),
        _ => return None,
    })
}

/// SS3 application-keypad finals.
pub(crate) fn ss3_keypad(f: u8) -> Option<KeypadKey> {
    use KeypadKey as P;
    Some(match f {
        b'p'..=b'y' => P::Digit(f - b'p'),
        b'j' => P::Multiply,
        b'k' => P::Add,
        b'l' => P::Separator,
        b'm' => P::Subtract,
        b'n' => P::Decimal,
        b'o' => P::Divide,
        b'M' => P::Enter,
        b'X' => P::Equal,
        _ => return None,
    })
}

/// The text a keypad key types when Num Lock makes it a character key.
pub(crate) fn keypad_text(k: KeypadKey) -> Option<char> {
    use KeypadKey as P;
    Some(match k {
        P::Digit(d) => (b'0' + d) as char,
        P::Decimal => '.',
        P::Divide => '/',
        P::Multiply => '*',
        P::Subtract => '-',
        P::Add => '+',
        P::Equal => '=',
        P::Separator => ',',
        _ => return None,
    })
}

/// Text implied by a key when the terminal did not send any: the character
/// itself for unmodified (or shifted) character keys, nothing for releases
/// or when Ctrl / Alt / Super / Hyper / Meta are held.
pub(crate) fn implied_text(k: &K, shifted: Option<char>, caps: bool) -> Text {
    if k.action == KeyAction::Release || k.mods.0 & !Mods::SHIFT.0 != 0 {
        return Text::EMPTY;
    }
    let shift = k.mods.contains(Mods::SHIFT);
    match k.kind {
        Kind::Key(Key::Char(c)) if !c.is_control() => {
            let t = if shift {
                shifted.unwrap_or_else(|| upper(c))
            } else if caps {
                upper(c)
            } else {
                c
            };
            Text::of(t)
        }
        Kind::Key(Key::Keypad(p)) if !shift => keypad_text(p).map(Text::of).unwrap_or(Text::EMPTY),
        _ => Text::EMPTY,
    }
}

fn upper(c: char) -> char {
    let mut u = c.to_uppercase();
    match (u.next(), u.next()) {
        (Some(x), None) => x,
        _ => c,
    }
}
