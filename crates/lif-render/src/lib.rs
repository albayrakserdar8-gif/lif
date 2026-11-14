//! Compositor and diff renderer: draws lif onto a client's terminal.
//!
//! For every attached client the server builds a [`Scene`] (panes, borders,
//! status line, overlays, cursor) and a [`Renderer`] turns it into the bytes
//! that bring the client's terminal from what it shows to what it should
//! show. There is never a queue of bytes: a frame is the difference between
//! the screen the client last received and the screen now, so skipping
//! frames for a slow client is free and always correct (see
//! [`Renderer::skip`]).
//!
//! What it does to keep frames small:
//! - only rows marked dirty (by the engine, or overlays/chrome changes) are
//!   looked at, and only cells that differ are written;
//! - SGR changes are minimal (incremental or reset, whichever is shorter);
//! - cursor movement picks the cheapest of CR/LF/BS, relative, absolute and
//!   reprinting identical cells;
//! - EL/ECH erase blank runs (only in the default style, because of
//!   background-color-erase);
//! - content that moved up or down is scrolled with SU/SD inside a scroll
//!   region (DECSTBM, plus DECSLRM for panes narrower than the screen when
//!   enabled) instead of being repainted;
//! - frames are wrapped in synchronized output when available.
//!
//! REP (repeat character) is not used: Terminal.app, Alacritty and older
//! VTE do not implement it, and `OuterCaps` has no flag to tell.
#![forbid(unsafe_code)]

mod cell;
mod esc;
mod layout;
mod moves;
mod render;
mod scene;
mod sgr;
mod text;

pub use layout::border_glyph;
pub use render::Renderer;
pub use scene::{
    LinkLookup, Message, Scene, ScenePane, Selection, SelectionKind, StatusBar, StatusPosition,
    Theme, pane_cursor,
};
pub use sgr::rgb_to_256;
pub use text::char_width;
