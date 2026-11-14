//! Parser for the bytes the client's (outer) terminal sends while a client
//! is attached: keys in every common encoding (legacy, xterm modifiers,
//! modifyOtherKeys, fixterms, the full kitty keyboard protocol), mouse
//! reports (SGR, SGR-pixel, X10, urxvt), bracketed paste, focus changes and
//! the terminal's answers to lif's queries. Also writes the sequences that
//! set the outer terminal up and restore it ([`write_queries`],
//! [`write_enter`], [`write_mouse`], [`write_leave`]) and refines
//! [`lif_types::OuterCaps`] from the answers ([`CapsProbe`]).
//!
//! The parser is a hand-written state machine on the input-latency path: a
//! key costs no allocation beyond its event's `text` string, nothing is
//! buffered except an incomplete sequence (at most [`MAX_PENDING`] bytes) or
//! a terminal reply string (at most [`MAX_STRING`] bytes), and an idle parser
//! needs no timer. See [`InputParser`] for the lone-ESC rule.
//!
//! # Decoding choices
//!
//! - C0 bytes become Ctrl+letter (0x01 → Ctrl+a). 0x00 is Ctrl+Space,
//!   0x09 Tab, 0x0d Enter, 0x7f Backspace, 0x08 Ctrl+h (modern terminals send
//!   0x7f for Backspace), 0x0a Ctrl+j, 0x1c..=0x1f Ctrl+\\ ] ^ _.
//! - Legacy printable characters are reported as received (`A` is
//!   `Char('A')` without Shift); the kitty protocol reports the unshifted key
//!   with Shift. `text` is filled from the kitty text field when present and
//!   otherwise implied for character / keypad keys pressed without
//!   Ctrl/Alt/Super/Hyper/Meta.
//! - Kitty alternate keys fill `KeyEvent::shifted` / `base_layout`; kitty's
//!   media and modifier keys become `Key::Media` / `Key::Modifier`; codes in
//!   kitty's private-use block that it does not assign are `Unknown`.
//! - `CSI 29 ~` is Menu (foot, urxvt, VTE), not xterm's F16.
//! - `CSI 1 ; m R`: see [`InputParser::expect_cursor_position`].
//! - Mouse coordinates are converted to 0-based cells; SGR-pixel reports
//!   keep the pixels, see [`InputParser::set_sgr_pixels`]. UTF-8 mouse (1005)
//!   is not decoded: it cannot be told apart from X10 without knowing it was
//!   enabled, and lif never enables it.
//! - 8-bit C1 introducers (0x9b CSI, 0x8f SS3, 0x90 DCS, 0x9d OSC, 0x9e PM,
//!   0x9f APC, 0x98 SOS) are honored where a UTF-8 character would start
//!   (they are invalid there anyway). The C1 string terminator 0x9c is not:
//!   it is a UTF-8 continuation byte. Strings end with ESC \\ or BEL.
#![forbid(unsafe_code)]

mod csi;
mod event;
mod keys;
mod parser;
mod probe;
mod setup;
mod strings;

pub use event::*;
pub use parser::{
    DEFAULT_ESC_TIMEOUT, DEFAULT_INCOMPLETE_TIMEOUT, InputParser, MAX_PENDING, MAX_STRING, Pending,
};
pub use probe::CapsProbe;
pub use setup::*;
