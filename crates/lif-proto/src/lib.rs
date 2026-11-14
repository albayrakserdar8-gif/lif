//! The lif client/server wire protocol.
//!
//! # Framing
//!
//! Every message is one frame on a Unix stream socket:
//!
//! ```text
//! u32 LE  payload length (not counting this 6-byte header), at most MAX_PAYLOAD
//! u16 LE  message type (see the `type_id` module)
//! ...     payload
//! ```
//!
//! # Payload encoding
//!
//! Fields are written in declaration order with no padding:
//!
//! - integers: little endian, fixed width
//! - `bool`: one byte, 0 or 1
//! - byte strings and `String`: u32 LE length, then the bytes (strings are UTF-8)
//! - `Vec<T>`: u32 LE element count, then the elements
//! - `Size`: cols u16, rows u16. `PixelSize`: width u16, height u16
//! - enums: u8 discriminant as documented on the type
//!
//! # Compatibility rules (these are what keep old and new builds talking)
//!
//! 1. A decoder ignores payload bytes after the last field it knows. New
//!    fields are only ever appended to the end of a message.
//! 2. A decoder skips frames whose type it does not know (it knows their
//!    length) and reports them as `Unknown`, never as an error.
//! 3. A field missing from the end of a payload (an older peer) decodes to
//!    its `Default`. Only a field that is cut off in the middle is an error.
//! 4. Existing fields and type ids are never removed, reordered or resized.
//! 5. Optional behavior is negotiated through [`Caps`], never by comparing
//!    version numbers. `PROTOCOL_VERSION` only changes if the framing itself
//!    changes.
#![forbid(unsafe_code)]

mod codec;
mod msg;

pub use codec::*;
pub use msg::*;

/// Version of the framing. See compatibility rule 5.
pub const PROTOCOL_VERSION: u16 = 1;

/// Largest accepted payload. A peer announcing more is a protocol error.
pub const MAX_PAYLOAD: usize = 16 * 1024 * 1024;

/// Bytes in a frame header.
pub const HEADER_LEN: usize = 6;
