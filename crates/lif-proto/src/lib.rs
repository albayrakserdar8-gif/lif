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
//!
//! # Details the rules leave open
//!
//! - `ClientMsg::encode` / `ServerMsg::encode` return a `Result`: a
//!   message whose payload would exceed `MAX_PAYLOAD` is
//!   `ProtoError::TooLarge`, and an `Unknown` message (it only describes a
//!   frame that was skipped) is `ProtoError::NotEncodable`. On error nothing
//!   is appended.
//! - An `ExitReason` byte this build does not know decodes as
//!   `ExitReason::Error` rather than failing: the client only needs to know
//!   the attachment is over, and the message says why.
//! - `Size` and `PixelSize` each count as one field for rule 3: missing
//!   entirely they decode to zero, but a payload that ends after `cols` (or
//!   `width`) is an error. Rule 3 applies to top-level fields only; an
//!   element of a `Vec` (e.g. one `env` pair) cut short is always an error.
//! - A `bool` byte other than 0 or 1 is an error (`ProtoError::InvalidValue`).
//! - Terminal data longer than one frame can hold (`MAX_BYTES_CHUNK`) is
//!   sent as several consecutive `Input` / `Output` frames by
//!   [`encode_input`] / [`encode_output`]; frame boundaries carry no
//!   meaning and the receiver just writes the chunks in order. Encoding one
//!   oversized `ClientMsg::Input` / `ServerMsg::Output` message directly
//!   fails with `TooLarge` instead.
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
