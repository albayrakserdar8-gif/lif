use crate::Pos;

/// Keyboard modifiers, as a bit set.
#[derive(Clone, Copy, Debug, Default, PartialEq, Eq, Hash)]
pub struct Mods(pub u8);

impl Mods {
    pub const NONE: Mods = Mods(0);
    pub const SHIFT: Mods = Mods(1 << 0);
    pub const ALT: Mods = Mods(1 << 1);
    pub const CTRL: Mods = Mods(1 << 2);
    pub const SUPER: Mods = Mods(1 << 3);
    pub const HYPER: Mods = Mods(1 << 4);
    pub const META: Mods = Mods(1 << 5);

    pub const fn contains(self, other: Mods) -> bool {
        self.0 & other.0 == other.0
    }

    pub const fn union(self, other: Mods) -> Mods {
        Mods(self.0 | other.0)
    }

    pub const fn is_empty(self) -> bool {
        self.0 == 0
    }
}

/// A key, independent of how the outer terminal encoded it.
#[derive(Clone, Copy, Debug, PartialEq, Eq, Hash)]
pub enum Key {
    /// A key that produces a character. Always the unshifted, lowercase
    /// codepoint when the outer terminal tells us (kitty protocol); otherwise
    /// the character as received.
    Char(char),
    Enter,
    Tab,
    Backspace,
    Escape,
    Up,
    Down,
    Left,
    Right,
    Home,
    End,
    PageUp,
    PageDown,
    Insert,
    Delete,
    /// F1..=F35.
    F(u8),
    /// Keypad keys, when the outer terminal distinguishes them.
    Keypad(KeypadKey),
    CapsLock,
    ScrollLock,
    NumLock,
    PrintScreen,
    Pause,
    Menu,
}

#[derive(Clone, Copy, Debug, PartialEq, Eq, Hash)]
pub enum KeypadKey {
    Digit(u8),
    Decimal,
    Divide,
    Multiply,
    Subtract,
    Add,
    Enter,
    Equal,
    Separator,
    Up,
    Down,
    Left,
    Right,
    Home,
    End,
    PageUp,
    PageDown,
    Insert,
    Delete,
    Begin,
}

#[derive(Clone, Copy, Debug, Default, PartialEq, Eq, Hash)]
pub enum KeyAction {
    #[default]
    Press,
    Repeat,
    Release,
}

/// One key event from the outer terminal.
#[derive(Clone, Debug, PartialEq, Eq, Hash)]
pub struct KeyEvent {
    pub key: Key,
    pub mods: Mods,
    pub action: KeyAction,
    /// The text the key produces with the current modifiers and layout, if
    /// any (e.g. `"A"` for Shift+a). Empty when unknown or none.
    pub text: String,
}

impl KeyEvent {
    pub fn new(key: Key, mods: Mods) -> Self {
        Self { key, mods, action: KeyAction::Press, text: String::new() }
    }
}

#[derive(Clone, Copy, Debug, PartialEq, Eq, Hash)]
pub enum MouseButton {
    Left,
    Middle,
    Right,
    WheelUp,
    WheelDown,
    WheelLeft,
    WheelRight,
    Back,
    Forward,
}

#[derive(Clone, Copy, Debug, PartialEq, Eq, Hash)]
pub enum MouseAction {
    Press,
    Release,
    /// Movement; `button` is the held button, if any.
    Motion,
}

/// One mouse event. Coordinates are relative to whatever the event is being
/// delivered to (the client screen from the input parser; the pane when
/// passed to `VtEngine::encode_mouse`).
#[derive(Clone, Copy, Debug, PartialEq, Eq, Hash)]
pub struct MouseEvent {
    pub action: MouseAction,
    pub button: Option<MouseButton>,
    pub mods: Mods,
    pub cell: Pos,
    /// Pixel position, when the outer terminal reports SGR-pixel coordinates.
    pub pixel: Option<(u16, u16)>,
}
