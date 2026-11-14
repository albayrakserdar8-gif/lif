//! lif input events -> libghostty key / mouse encoders.

use libghostty_vt::Terminal;
use std::io::Write;

use libghostty_vt::key::{self, Action, Key as GKey, KittyKeyFlags, Mods as GMods, OptionAsAlt};
use libghostty_vt::mouse::{self, Button, EncoderSize, Position};
use lif_types::{
    Key, KeyAction, KeyEvent, KeypadKey, Mods, MouseAction, MouseButton, MouseEvent, Size,
};

fn mods(m: Mods) -> GMods {
    let mut g = GMods::empty();
    if m.contains(Mods::SHIFT) {
        g |= GMods::SHIFT;
    }
    if m.contains(Mods::ALT) || m.contains(Mods::META) {
        g |= GMods::ALT;
    }
    if m.contains(Mods::CTRL) {
        g |= GMods::CTRL;
    }
    if m.contains(Mods::SUPER) || m.contains(Mods::HYPER) {
        g |= GMods::SUPER;
    }
    g
}

/// The physical (US layout) key that produces `c` unshifted, if any.
fn char_key(c: char) -> GKey {
    const LETTERS: [GKey; 26] = [
        GKey::A, GKey::B, GKey::C, GKey::D, GKey::E, GKey::F, GKey::G, GKey::H, GKey::I,
        GKey::J, GKey::K, GKey::L, GKey::M, GKey::N, GKey::O, GKey::P, GKey::Q, GKey::R,
        GKey::S, GKey::T, GKey::U, GKey::V, GKey::W, GKey::X, GKey::Y, GKey::Z,
    ];
    const DIGITS: [GKey; 10] = [
        GKey::Digit0, GKey::Digit1, GKey::Digit2, GKey::Digit3, GKey::Digit4,
        GKey::Digit5, GKey::Digit6, GKey::Digit7, GKey::Digit8, GKey::Digit9,
    ];
    match c {
        'a'..='z' => LETTERS[(c as u8 - b'a') as usize],
        'A'..='Z' => LETTERS[(c as u8 - b'A') as usize],
        '0'..='9' => DIGITS[(c as u8 - b'0') as usize],
        ' ' => GKey::Space,
        '\x60' => GKey::Backquote,
        '\\' => GKey::Backslash,
        '[' => GKey::BracketLeft,
        ']' => GKey::BracketRight,
        ',' => GKey::Comma,
        '=' => GKey::Equal,
        '-' => GKey::Minus,
        '.' => GKey::Period,
        '\'' => GKey::Quote,
        ';' => GKey::Semicolon,
        '/' => GKey::Slash,
        _ => GKey::Unidentified,
    }
}

fn keypad_key(k: KeypadKey) -> GKey {
    const DIGITS: [GKey; 10] = [
        GKey::Numpad0, GKey::Numpad1, GKey::Numpad2, GKey::Numpad3, GKey::Numpad4,
        GKey::Numpad5, GKey::Numpad6, GKey::Numpad7, GKey::Numpad8, GKey::Numpad9,
    ];
    match k {
        KeypadKey::Digit(d) => DIGITS.get(d as usize).copied().unwrap_or(GKey::Unidentified),
        KeypadKey::Decimal => GKey::NumpadDecimal,
        KeypadKey::Divide => GKey::NumpadDivide,
        KeypadKey::Multiply => GKey::NumpadMultiply,
        KeypadKey::Subtract => GKey::NumpadSubtract,
        KeypadKey::Add => GKey::NumpadAdd,
        KeypadKey::Enter => GKey::NumpadEnter,
        KeypadKey::Equal => GKey::NumpadEqual,
        KeypadKey::Separator => GKey::NumpadSeparator,
        KeypadKey::Up => GKey::NumpadUp,
        KeypadKey::Down => GKey::NumpadDown,
        KeypadKey::Left => GKey::NumpadLeft,
        KeypadKey::Right => GKey::NumpadRight,
        KeypadKey::Home => GKey::NumpadHome,
        KeypadKey::End => GKey::NumpadEnd,
        KeypadKey::PageUp => GKey::NumpadPageUp,
        KeypadKey::PageDown => GKey::NumpadPageDown,
        KeypadKey::Insert => GKey::NumpadInsert,
        KeypadKey::Delete => GKey::NumpadDelete,
        KeypadKey::Begin => GKey::NumpadBegin,
    }
}

fn function_key(n: u8) -> Option<GKey> {
    const F: [GKey; 25] = [
        GKey::F1, GKey::F2, GKey::F3, GKey::F4, GKey::F5, GKey::F6, GKey::F7, GKey::F8,
        GKey::F9, GKey::F10, GKey::F11, GKey::F12, GKey::F13, GKey::F14, GKey::F15, GKey::F16,
        GKey::F17, GKey::F18, GKey::F19, GKey::F20, GKey::F21, GKey::F22, GKey::F23, GKey::F24,
        GKey::F25,
    ];
    F.get((n as usize).checked_sub(1)?).copied()
}

/// Media and modifier keys. libghostty's kitty table has no media keys and
/// lacks Hyper, Meta and the ISO level shifts, so all of them are encoded
/// here, by the rules libghostty applies to the functional keys it knows:
/// nothing without kitty flags (there is no legacy encoding), releases only
/// with "report event types", lone modifiers only with "report all keys as
/// escape codes". `mods` is passed on as the outer terminal reported it.
fn encode_kitty_functional(flags: KittyKeyFlags, code: u32, modifier: bool, k: &KeyEvent, out: &mut Vec<u8>) {
    let events = flags.contains(KittyKeyFlags::REPORT_EVENTS);
    if flags.is_empty()
        || (k.action == KeyAction::Release && !events)
        || (modifier && !flags.contains(KittyKeyFlags::REPORT_ALL))
    {
        return;
    }
    let mods = 1 + u32::from(k.mods.0 & 0x3f);
    let event = match k.action {
        _ if !events => 1,
        KeyAction::Press => 1,
        KeyAction::Repeat => 2,
        KeyAction::Release => 3,
    };
    // Writing to a Vec cannot fail.
    let _ = if event != 1 {
        write!(out, "\x1b[{code};{mods}:{event}u")
    } else if mods > 1 {
        write!(out, "\x1b[{code};{mods}u")
    } else {
        write!(out, "\x1b[{code}u")
    };
}

/// Text libghostty may use for a key: never a control character.
fn usable_text(s: &str) -> bool {
    !s.is_empty() && !s.chars().any(|c| c.is_control() || ('\u{f700}'..='\u{f8ff}').contains(&c))
}

pub(super) fn encode_key(
    term: &Terminal<'static, 'static>,
    enc: &mut key::Encoder<'static>,
    ev: &mut key::Event<'static>,
    k: &KeyEvent,
    out: &mut Vec<u8>,
) {
    let (gkey, unshifted) = match k.key {
        // libghostty derives the kitty base-layout alternate from the
        // physical key, so the base-layout key (when the outer terminal
        // reported one) is the physical key.
        Key::Char(c) => (char_key(k.base_layout.unwrap_or(c)), Some(c)),
        Key::Enter => (GKey::Enter, None),
        Key::Tab => (GKey::Tab, None),
        Key::Backspace => (GKey::Backspace, None),
        Key::Escape => (GKey::Escape, None),
        Key::Up => (GKey::ArrowUp, None),
        Key::Down => (GKey::ArrowDown, None),
        Key::Left => (GKey::ArrowLeft, None),
        Key::Right => (GKey::ArrowRight, None),
        Key::Home => (GKey::Home, None),
        Key::End => (GKey::End, None),
        Key::PageUp => (GKey::PageUp, None),
        Key::PageDown => (GKey::PageDown, None),
        Key::Insert => (GKey::Insert, None),
        Key::Delete => (GKey::Delete, None),
        Key::F(n) => match function_key(n) {
            Some(g) => (g, None),
            None => return,
        },
        Key::Keypad(kp) => (keypad_key(kp), None),
        Key::CapsLock => (GKey::CapsLock, None),
        Key::ScrollLock => (GKey::ScrollLock, None),
        Key::NumLock => (GKey::NumLock, None),
        Key::PrintScreen => (GKey::PrintScreen, None),
        Key::Pause => (GKey::Pause, None),
        Key::Menu => (GKey::ContextMenu, None),
        Key::Media(m) => {
            let flags = term.kitty_keyboard_flags().unwrap_or(KittyKeyFlags::DISABLED);
            return encode_kitty_functional(flags, m.kitty_code(), false, k, out);
        }
        Key::Modifier(m) => {
            let flags = term.kitty_keyboard_flags().unwrap_or(KittyKeyFlags::DISABLED);
            return encode_kitty_functional(flags, m.kitty_code(), true, k, out);
        }
    };
    let gm = mods(k.mods);
    // libghostty wants the layout text *before* Ctrl/Alt transforms; for a
    // char key that is the reported text, else the reported shifted key
    // when Shift is held, else the char itself (uppercased under Shift). It
    // also derives the kitty shifted-key alternate from this text.
    let shifted = k.shifted.filter(|s| k.mods.contains(Mods::SHIFT) && !s.is_control());
    let text: Option<String> = if usable_text(&k.text) {
        Some(k.text.clone())
    } else if let (Some(s), Some(_)) = (shifted, unshifted) {
        Some(s.to_string())
    } else if let Some(c) = unshifted.filter(|c| !c.is_control()) {
        if k.mods.contains(Mods::SHIFT) && c.is_lowercase() {
            Some(c.to_uppercase().collect())
        } else {
            Some(c.to_string())
        }
    } else {
        None
    };
    let mut consumed = GMods::empty();
    if let (Some(t), Some(c)) = (&text, unshifted)
        && k.mods.contains(Mods::SHIFT)
        && !t.starts_with(c)
    {
        consumed |= GMods::SHIFT;
    }
    ev.set_action(match k.action {
        KeyAction::Press => Action::Press,
        KeyAction::Repeat => Action::Repeat,
        KeyAction::Release => Action::Release,
    })
    .set_key(gkey)
    .set_mods(gm)
    .set_consumed_mods(consumed)
    .set_composing(false)
    .set_utf8(text)
    .set_unshifted_codepoint(unshifted.map_or('\0', |c| c.to_lowercase().next().unwrap_or(c)));
    // Our ALT already means "the outer terminal reported Alt", so the macOS
    // option-key heuristics must not reinterpret it.
    enc.set_options_from_terminal(term).set_macos_option_as_alt(OptionAsAlt::True);
    let _ = enc.encode_to_vec(ev, out);
}

pub(super) fn encode_mouse(
    term: &Terminal<'static, 'static>,
    enc: &mut mouse::Encoder<'static>,
    ev: &mut mouse::Event<'static>,
    size: Size,
    (cw, ch): (u32, u32),
    m: &MouseEvent,
    out: &mut Vec<u8>,
) {
    let button = m.button.map(|b| match b {
        MouseButton::Left => Button::Left,
        MouseButton::Middle => Button::Middle,
        MouseButton::Right => Button::Right,
        MouseButton::WheelUp => Button::Four,
        MouseButton::WheelDown => Button::Five,
        MouseButton::WheelLeft => Button::Six,
        MouseButton::WheelRight => Button::Seven,
        MouseButton::Back => Button::Eight,
        MouseButton::Forward => Button::Nine,
    });
    let action = match m.action {
        MouseAction::Press => mouse::Action::Press,
        MouseAction::Release => mouse::Action::Release,
        MouseAction::Motion => mouse::Action::Motion,
    };
    // libghostty works in surface pixels. Without real pixels, aim at the
    // middle of the cell so rounding can never land in a neighbor.
    let (x, y) = match m.pixel {
        Some((px, py)) => (f64::from(px), f64::from(py)),
        None => (
            f64::from(m.cell.x) * f64::from(cw) + f64::from(cw) / 2.0,
            f64::from(m.cell.y) * f64::from(ch) + f64::from(ch) / 2.0,
        ),
    };
    let held = m.action == MouseAction::Motion && m.button.is_some();
    enc.set_options_from_terminal(term).set_any_button_pressed(held);
    let _ = enc.set_size(EncoderSize {
        screen_width: u32::from(size.cols) * cw,
        screen_height: u32::from(size.rows) * ch,
        cell_width: cw,
        cell_height: ch,
        padding_top: 0,
        padding_bottom: 0,
        padding_right: 0,
        padding_left: 0,
    });
    ev.set_action(action)
        .set_button(button)
        .set_mods(mods(m.mods))
        .set_position(Position { x: x as f32, y: y as f32 });
    let _ = enc.encode_to_vec(ev, out);
}
