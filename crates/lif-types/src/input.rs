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
    /// Media and volume keys (kitty protocol only).
    Media(MediaKey),
    /// A modifier pressed or released on its own (kitty protocol with
    /// "report all keys as escape codes" only). `mods` is kept exactly as the
    /// outer terminal reported it (kitty sets the key's own bit on press).
    Modifier(ModifierKey),
}

/// Media keys of the kitty keyboard protocol, which numbers them 57428..=57440.
#[derive(Clone, Copy, Debug, PartialEq, Eq, Hash)]
pub enum MediaKey {
    Play,
    Pause,
    PlayPause,
    Reverse,
    Stop,
    FastForward,
    Rewind,
    TrackNext,
    TrackPrevious,
    Record,
    LowerVolume,
    RaiseVolume,
    MuteVolume,
}

impl MediaKey {
    const ALL: [MediaKey; 13] = [
        MediaKey::Play,
        MediaKey::Pause,
        MediaKey::PlayPause,
        MediaKey::Reverse,
        MediaKey::Stop,
        MediaKey::FastForward,
        MediaKey::Rewind,
        MediaKey::TrackNext,
        MediaKey::TrackPrevious,
        MediaKey::Record,
        MediaKey::LowerVolume,
        MediaKey::RaiseVolume,
        MediaKey::MuteVolume,
    ];
    const FIRST: u32 = 57428;

    /// The kitty functional key number.
    pub const fn kitty_code(self) -> u32 {
        Self::FIRST + self as u32
    }

    pub fn from_kitty_code(code: u32) -> Option<MediaKey> {
        Self::ALL.get(code.checked_sub(Self::FIRST)? as usize).copied()
    }
}

/// Modifier keys of the kitty keyboard protocol, which numbers them
/// 57441..=57454. Super is the Command / Windows key; Hyper and Meta are the
/// X11 modifiers of those names; the ISO level shifts are AltGr-like keys.
#[derive(Clone, Copy, Debug, PartialEq, Eq, Hash)]
pub enum ModifierKey {
    LeftShift,
    LeftControl,
    LeftAlt,
    LeftSuper,
    LeftHyper,
    LeftMeta,
    RightShift,
    RightControl,
    RightAlt,
    RightSuper,
    RightHyper,
    RightMeta,
    IsoLevel3Shift,
    IsoLevel5Shift,
}

impl ModifierKey {
    const ALL: [ModifierKey; 14] = [
        ModifierKey::LeftShift,
        ModifierKey::LeftControl,
        ModifierKey::LeftAlt,
        ModifierKey::LeftSuper,
        ModifierKey::LeftHyper,
        ModifierKey::LeftMeta,
        ModifierKey::RightShift,
        ModifierKey::RightControl,
        ModifierKey::RightAlt,
        ModifierKey::RightSuper,
        ModifierKey::RightHyper,
        ModifierKey::RightMeta,
        ModifierKey::IsoLevel3Shift,
        ModifierKey::IsoLevel5Shift,
    ];
    const FIRST: u32 = 57441;

    /// The kitty functional key number.
    pub const fn kitty_code(self) -> u32 {
        Self::FIRST + self as u32
    }

    pub fn from_kitty_code(code: u32) -> Option<ModifierKey> {
        Self::ALL.get(code.checked_sub(Self::FIRST)? as usize).copied()
    }
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
    /// For a `Key::Char` pressed with Shift: the character the key produces
    /// with Shift under the current layout (`'!'` for Shift+1 on a US
    /// layout). Kitty "report alternate keys" (flag 4); `None` when the outer
    /// terminal did not say.
    pub shifted: Option<char>,
    /// For a `Key::Char`: the key at the same position on the standard PC-101
    /// (US) layout, when it differs from `key` (`'c'` for the Cyrillic `'с'`
    /// key). Kitty "report alternate keys" (flag 4); `None` when the same or
    /// unknown.
    pub base_layout: Option<char>,
}

impl KeyEvent {
    pub fn new(key: Key, mods: Mods) -> Self {
        Self { key, mods, action: KeyAction::Press, text: String::new(), shifted: None, base_layout: None }
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
