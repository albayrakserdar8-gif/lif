//! Engine-neutral types shared by every lif crate.
//!
//! Nothing here depends on the VT engine, the OS or the wire protocol. The
//! VT engine fills [`PaneFrame`]s, the renderer diffs them, and the input
//! layer produces [`KeyEvent`]s / [`MouseEvent`]s that the engine encodes for
//! the program running in a pane.
#![forbid(unsafe_code)]

mod frame;
mod geom;
mod input;

pub use frame::*;
pub use geom::*;
pub use input::*;
