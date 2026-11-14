//! Key names in tmux syntax and the normalized key form bindings use.
//!
//! A [`KeyCode`] is a [`lif_types::Key`] plus the three modifiers tmux
//! knows (Ctrl, Meta/Alt, Shift), normalized so that the same physical key
//! compares equal however the outer terminal encoded it: `Shift+a` is `A`,
//! `Ctrl+B` is `C-b`, a raw `0x02` is `C-b`, `Ctrl+i` is `Tab`, `Shift+Tab` is
//! `BTab`.

use lif_types::{Key, KeyAction, KeyEvent, KeypadKey, Mods};
use std::fmt;

const MASK: u8 = Mods::SHIFT.0 | Mods::ALT.0 | Mods::CTRL.0;

/// A normalized key with modifiers, as stored in key tables.
#[derive(Clone, Copy, Debug, PartialEq, Eq, Hash)]
pub struct KeyCode {
    /// The key. `Key::Char` holds the character the key produces (`'A'` for
    /// Shift+a; Shift is then not in `mods`).
    pub key: Key,
    /// Only `Mods::CTRL`, `Mods::ALT` (tmux's M-) and `Mods::SHIFT`.
    pub mods: Mods,
}

impl KeyCode {
    /// Build a normalized key code.
    pub fn new(key: Key, mods: Mods) -> KeyCode {
        normalize(key, mods)
    }

    /// The binding key for an input event, or `None` for events that can never
    /// match a binding (key releases, Super/Hyper chords).
    pub fn from_event(ev: &KeyEvent) -> Option<KeyCode> {
        if ev.action == KeyAction::Release {
            return None;
        }
        if ev.mods.contains(Mods::SUPER) || ev.mods.contains(Mods::HYPER) {
            return None;
        }
        let mut mods = Mods(ev.mods.0 & MASK);
        if ev.mods.contains(Mods::META) {
            mods = mods.union(Mods::ALT);
        }
        let mut key = ev.key;
        // Shift+1 reported as '1' with text "!" (kitty protocol) binds as "!".
        if let Key::Char(c) = key
            && mods.contains(Mods::SHIFT)
            && !mods.contains(Mods::CTRL)
            && !c.is_alphabetic()
        {
            let mut it = ev.text.chars();
            if let (Some(t), None) = (it.next(), it.next())
                && !t.is_control()
            {
                key = Key::Char(t);
                mods = Mods(mods.0 & !Mods::SHIFT.0);
            }
        }
        Some(normalize(key, mods))
    }

    /// The event to deliver to a pane for this key (`send-keys`, `send-prefix`).
    pub fn to_event(self) -> KeyEvent {
        let mut ev = KeyEvent::new(self.key, self.mods);
        match self.key {
            Key::Char(c) if !self.mods.contains(Mods::CTRL) && !self.mods.contains(Mods::ALT) => {
                ev.text.push(c);
            }
            Key::Char(' ') if self.mods.is_empty() => ev.text.push(' '),
            _ => {}
        }
        ev
    }

    /// Parse a tmux key name: `C-b`, `M-Left`, `S-F5`, `^A`, `Enter`, `Space`, `DC`,
    /// `PPage`, `KP5`, a single character, ... Modifier prefixes and names
    /// are case-insensitive as in tmux. Returns `None` for an unknown name.
    pub fn parse(name: &str) -> Option<KeyCode> {
        let mut s = name;
        let mut mods = Mods::NONE;
        if let Some(rest) = s.strip_prefix('^')
            && !rest.is_empty()
        {
            mods = mods.union(Mods::CTRL);
            s = rest;
        }
        loop {
            let b = s.as_bytes();
            if b.len() >= 3 && b[1] == b'-' {
                match b[0] {
                    b'C' | b'c' => mods = mods.union(Mods::CTRL),
                    b'M' | b'm' => mods = mods.union(Mods::ALT),
                    b'S' | b's' => mods = mods.union(Mods::SHIFT),
                    _ => return None,
                }
                s = &s[2..];
            } else {
                break;
            }
        }
        if s.is_empty() {
            return None;
        }
        let mut chars = s.chars();
        let first = chars.next()?;
        if chars.next().is_none() {
            if first.is_control() {
                return None;
            }
            return Some(normalize(Key::Char(first), mods));
        }
        let (key, extra) = lookup_name(s)?;
        Some(normalize(key, mods.union(extra)))
    }

    /// Order of `list-keys` (tmux sorts by its numeric key code: plain keys,
    /// then M-, C-, S- combinations; characters before special keys).
    pub(crate) fn sort_key(self) -> (u8, u64) {
        let mut m = 0u8;
        let mut base_shift = false;
        if self.key == Key::Tab && self.mods.contains(Mods::SHIFT) {
            base_shift = true; // BTab is a key of its own in tmux
        }
        if self.mods.contains(Mods::ALT) {
            m |= 1;
        }
        if self.mods.contains(Mods::CTRL) {
            m |= 2;
        }
        if self.mods.contains(Mods::SHIFT) && !base_shift {
            m |= 4;
        }
        let special = |i: u64| (1u64 << 32) + i;
        let base = match self.key {
            Key::Char(c) => c as u64,
            Key::Tab if base_shift => special(46),
            Key::Tab => 9,
            Key::Enter => 13,
            Key::Escape => 27,
            Key::Backspace => special(0),
            Key::F(n) => special(1 + n as u64),
            Key::Insert => special(40),
            Key::Delete => special(41),
            Key::Home => special(42),
            Key::End => special(43),
            Key::PageDown => special(44),
            Key::PageUp => special(45),
            Key::Up => special(47),
            Key::Down => special(48),
            Key::Left => special(49),
            Key::Right => special(50),
            Key::Keypad(k) => special(60 + keypad_index(k)),
            Key::CapsLock => special(80),
            Key::ScrollLock => special(81),
            Key::NumLock => special(82),
            Key::PrintScreen => special(83),
            Key::Pause => special(84),
            Key::Menu => special(85),
        };
        (m, base)
    }
}

fn keypad_index(k: KeypadKey) -> u64 {
    match k {
        KeypadKey::Divide => 0,
        KeypadKey::Multiply => 1,
        KeypadKey::Subtract => 2,
        KeypadKey::Digit(7) => 3,
        KeypadKey::Digit(8) => 4,
        KeypadKey::Digit(9) => 5,
        KeypadKey::Add => 6,
        KeypadKey::Digit(4) => 7,
        KeypadKey::Digit(5) => 8,
        KeypadKey::Digit(6) => 9,
        KeypadKey::Digit(1) => 10,
        KeypadKey::Digit(2) => 11,
        KeypadKey::Digit(3) => 12,
        KeypadKey::Enter => 13,
        KeypadKey::Digit(0) => 14,
        KeypadKey::Decimal => 15,
        KeypadKey::Digit(_) => 16,
        _ => 17,
    }
}

fn normalize(key: Key, mods: Mods) -> KeyCode {
    let mut m = Mods(mods.0 & MASK);
    let ctrl = Mods::CTRL.0;
    let key = match key {
        Key::Char(mut c) => {
            if (c as u32) < 0x20 {
                // A raw C0 byte: the terminal already applied Ctrl.
                c = match c as u8 {
                    0 => {
                        m.0 |= ctrl;
                        ' '
                    }
                    9 => return KeyCode { key: Key::Tab, mods: m },
                    13 => return KeyCode { key: Key::Enter, mods: m },
                    27 => return KeyCode { key: Key::Escape, mods: m },
                    b @ 1..=26 => {
                        m.0 |= ctrl;
                        (b'a' + b - 1) as char
                    }
                    b => {
                        m.0 |= ctrl;
                        (b + 64) as char
                    }
                };
            }
            if c == '\x7f' {
                return KeyCode { key: Key::Backspace, mods: m };
            }
            if m.0 & ctrl != 0 {
                if c.is_ascii_uppercase() {
                    c = c.to_ascii_lowercase();
                }
                let plain = Mods(m.0 & !ctrl);
                match c {
                    'i' => return KeyCode { key: Key::Tab, mods: plain },
                    'm' => return KeyCode { key: Key::Enter, mods: plain },
                    '[' => return KeyCode { key: Key::Escape, mods: plain },
                    '@' => c = ' ',
                    _ => {}
                }
            } else if m.contains(Mods::SHIFT) {
                let mut up = c.to_uppercase();
                if let (Some(u), None) = (up.next(), up.next())
                    && u != c
                {
                    c = u;
                    m.0 &= !Mods::SHIFT.0;
                } else if c.is_uppercase() {
                    m.0 &= !Mods::SHIFT.0;
                }
            }
            Key::Char(c)
        }
        k => k,
    };
    KeyCode { key, mods: m }
}

fn lookup_name(s: &str) -> Option<(Key, Mods)> {
    let eq = |n: &str| s.eq_ignore_ascii_case(n);
    let none = Mods::NONE;
    let k = |k: Key| Some((k, Mods::NONE));
    if let Some(n) = s.strip_prefix(['F', 'f'])
        && let Ok(n) = n.parse::<u8>()
        && (1..=35).contains(&n)
        && !s[1..].starts_with('0')
    {
        return k(Key::F(n));
    }
    if eq("IC") || eq("Insert") {
        return k(Key::Insert);
    }
    if eq("DC") || eq("Delete") {
        return k(Key::Delete);
    }
    if eq("Home") {
        return k(Key::Home);
    }
    if eq("End") {
        return k(Key::End);
    }
    if eq("NPage") || eq("PageDown") || eq("PgDn") {
        return k(Key::PageDown);
    }
    if eq("PPage") || eq("PageUp") || eq("PgUp") {
        return k(Key::PageUp);
    }
    if eq("BTab") {
        return Some((Key::Tab, Mods::SHIFT));
    }
    if eq("Space") {
        return k(Key::Char(' '));
    }
    if eq("BSpace") {
        return k(Key::Backspace);
    }
    if eq("Enter") {
        return k(Key::Enter);
    }
    if eq("Escape") {
        return k(Key::Escape);
    }
    if eq("Tab") {
        return k(Key::Tab);
    }
    if eq("Up") {
        return k(Key::Up);
    }
    if eq("Down") {
        return k(Key::Down);
    }
    if eq("Left") {
        return k(Key::Left);
    }
    if eq("Right") {
        return k(Key::Right);
    }
    if let Some(rest) = s.strip_prefix("KP").or_else(|| s.strip_prefix("kp")) {
        let kp = match rest {
            "/" => KeypadKey::Divide,
            "*" => KeypadKey::Multiply,
            "-" => KeypadKey::Subtract,
            "+" => KeypadKey::Add,
            "." => KeypadKey::Decimal,
            r if r.eq_ignore_ascii_case("Enter") => KeypadKey::Enter,
            r if r.len() == 1 && r.as_bytes()[0].is_ascii_digit() => KeypadKey::Digit(r.as_bytes()[0] - b'0'),
            _ => return None,
        };
        return Some((Key::Keypad(kp), none));
    }
    None
}

impl fmt::Display for KeyCode {
    /// The tmux name: modifiers as `C-`, `M-`, `S-` in that order, then the key.
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        let btab = self.key == Key::Tab && self.mods.contains(Mods::SHIFT);
        if self.mods.contains(Mods::CTRL) {
            f.write_str("C-")?;
        }
        if self.mods.contains(Mods::ALT) {
            f.write_str("M-")?;
        }
        if self.mods.contains(Mods::SHIFT) && !btab {
            f.write_str("S-")?;
        }
        let name = match self.key {
            Key::Char(' ') => "Space",
            Key::Char(c) => return write!(f, "{c}"),
            Key::Tab if btab => "BTab",
            Key::Tab => "Tab",
            Key::Enter => "Enter",
            Key::Backspace => "BSpace",
            Key::Escape => "Escape",
            Key::Up => "Up",
            Key::Down => "Down",
            Key::Left => "Left",
            Key::Right => "Right",
            Key::Home => "Home",
            Key::End => "End",
            Key::PageUp => "PPage",
            Key::PageDown => "NPage",
            Key::Insert => "IC",
            Key::Delete => "DC",
            Key::F(n) => return write!(f, "F{n}"),
            Key::Keypad(k) => {
                return match k {
                    KeypadKey::Digit(d) => write!(f, "KP{d}"),
                    KeypadKey::Divide => f.write_str("KP/"),
                    KeypadKey::Multiply => f.write_str("KP*"),
                    KeypadKey::Subtract => f.write_str("KP-"),
                    KeypadKey::Add => f.write_str("KP+"),
                    KeypadKey::Decimal => f.write_str("KP."),
                    KeypadKey::Enter => f.write_str("KPEnter"),
                    other => write!(f, "KP{other:?}"),
                };
            }
            Key::CapsLock => "CapsLock",
            Key::ScrollLock => "ScrollLock",
            Key::NumLock => "NumLock",
            Key::PrintScreen => "PrintScreen",
            Key::Pause => "Pause",
            Key::Menu => "Menu",
        };
        f.write_str(name)
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn p(s: &str) -> KeyCode {
        KeyCode::parse(s).unwrap_or_else(|| panic!("{s} did not parse"))
    }

    #[test]
    fn names_round_trip() {
        for name in [
            "C-b", "M-Left", "S-F5", "Enter", "Space", "DC", "IC", "PPage", "NPage", "BTab", "BSpace", "Escape", "Tab",
            "a", "A", "%", "\"", "C-Up", "M-1", "C-M-x", "KP5", "KPEnter", "KP/", "F12", "Home", "End", "M-Space",
            "C-Space", "é", "\\", "S-Up",
        ] {
            assert_eq!(p(name).to_string(), name, "{name}");
        }
    }

    #[test]
    fn aliases_and_case() {
        assert_eq!(p("C-B"), p("C-b"));
        assert_eq!(p("c-b"), p("C-b"));
        assert_eq!(p("^b"), p("C-b"));
        assert_eq!(p("^B"), p("C-b"));
        assert_eq!(p("Delete"), p("DC"));
        assert_eq!(p("PageUp"), p("PPage"));
        assert_eq!(p("PgDn"), p("NPage"));
        assert_eq!(p("enter"), p("Enter"));
        assert_eq!(p("C-m"), p("Enter"));
        assert_eq!(p("C-i"), p("Tab"));
        assert_eq!(p("C-["), p("Escape"));
        assert_eq!(p("S-a"), p("A"));
        assert_eq!(p("S-Tab"), p("BTab"));
        assert_eq!(p("C-@"), p("C-Space"));
        assert_eq!(p("Insert").to_string(), "IC");
        assert!(KeyCode::parse("").is_none());
        assert!(KeyCode::parse("C-").is_none());
        assert!(KeyCode::parse("Bogus").is_none());
        assert!(KeyCode::parse("X-a").is_none());
        assert!(KeyCode::parse("F0").is_none());
        assert!(KeyCode::parse("F36").is_none());
        assert!(KeyCode::parse("\x01").is_none());
        // "-" alone and "C--" are keys.
        assert_eq!(p("-").to_string(), "-");
        assert_eq!(p("C--").to_string(), "C--");
    }

    #[test]
    fn events_normalize() {
        let ev = |k, m| KeyCode::from_event(&KeyEvent::new(k, m)).unwrap();
        assert_eq!(ev(Key::Char('\x02'), Mods::NONE), p("C-b"));
        assert_eq!(ev(Key::Char('b'), Mods::CTRL), p("C-b"));
        assert_eq!(ev(Key::Char('B'), Mods::CTRL), p("C-b"));
        assert_eq!(ev(Key::Char('a'), Mods::SHIFT), p("A"));
        assert_eq!(ev(Key::Char('A'), Mods::SHIFT), p("A"));
        assert_eq!(ev(Key::Char('a'), Mods::META), p("M-a"));
        assert_eq!(ev(Key::Char('\x7f'), Mods::NONE), p("BSpace"));
        assert_eq!(ev(Key::Char('\r'), Mods::NONE), p("Enter"));
        assert_eq!(ev(Key::Char('\x1b'), Mods::NONE), p("Escape"));
        assert_eq!(ev(Key::Tab, Mods::SHIFT), p("BTab"));
        assert_eq!(ev(Key::Char('\0'), Mods::NONE), p("C-Space"));
        let mut kitty = KeyEvent::new(Key::Char('5'), Mods::SHIFT);
        kitty.text = "%".into();
        assert_eq!(KeyCode::from_event(&kitty), Some(p("%")));
        let mut rel = KeyEvent::new(Key::Char('a'), Mods::NONE);
        rel.action = KeyAction::Release;
        assert_eq!(KeyCode::from_event(&rel), None);
        assert_eq!(KeyCode::from_event(&KeyEvent::new(Key::Char('a'), Mods::SUPER)), None);
    }

    #[test]
    fn to_event_text() {
        assert_eq!(p("a").to_event().text, "a");
        assert_eq!(p("C-b").to_event().text, "");
        assert_eq!(p("C-b").to_event().mods, Mods::CTRL);
        assert_eq!(p("Space").to_event().text, " ");
    }

    #[test]
    fn sort_order_matches_tmux() {
        // Order of tmux 3.8 "list-keys -T prefix" for these keys.
        let order = ["Space", "!", "%", "z", "~", "DC", "PPage", "BTab", "Up", "Right", "M-1", "M-n", "M-Up", "C-b", "C-Up", "S-Up"];
        let mut v: Vec<KeyCode> = order.iter().map(|s| p(s)).collect();
        v.sort_by_key(|k| k.sort_key());
        let got: Vec<String> = v.iter().map(|k| k.to_string()).collect();
        assert_eq!(got, order);
    }
}
