//! lif's model: sessions, windows, panes, layouts, the tmux command
//! language, formats, options, key bindings and the status line.
//!
//! The crate is pure logic. The server owns PTYs, terminal engines and
//! client connections and drives the model:
//!
//! - commands: [`parse`] / [`parse_list`] / [`parse_string`] turn argv or a
//!   command string into [`Command`]s; [`Model::execute`] runs them with a
//!   [`CmdContext`] (who is asking) and a [`Host`] (how to touch the outside
//!   world);
//! - events: [`Model::pane_exited`], [`Model::pane_bell`],
//!   [`Model::pane_activity`], [`Model::pane_changed`],
//!   [`Model::client_connect`], [`Model::client_disconnect`],
//!   [`Model::client_resized`];
//! - keys: [`Model::handle_key`] runs the prefix state machine and says
//!   whether a key goes to the pane, runs commands, or is swallowed;
//! - rendering: [`Model::client_view`] gives pane rectangles and borders,
//!   [`Model::status_line`] the styled status line.
//!
//! tmux 3.8 is the behavioural reference: names, flags, targets, defaults
//! and messages match it for everything implemented. The crate never reads a
//! clock or the environment: time and host name come from the caller.
#![forbid(unsafe_code)]
#![warn(missing_docs)]

pub mod bindings;
pub mod cmd;
mod exec;
pub mod format;
pub mod host;
pub mod ids;
pub mod keys;
pub mod layout;
mod model;
pub mod options;
pub(crate) mod status;
mod target;
pub mod time;

pub use bindings::{Binding, KeyBindings, KeyDecision, KeyState};
pub use cmd::{Arg, Args, CmdError, Command, CommandList, escape_arg, parse, parse_list, parse_string};
pub use exec::expand_prompt_template;
pub use host::{
    CaptureRange, ConfigError, ConfigFile, CopyModeOptions, DetachReason, Host, LinePos, PaneInfo, PaneInput, Prompt,
    Redraw, SpawnCommand, SpawnSpec,
};
pub use ids::{ClientId, PaneId, SessionId, WindowId};
pub use keys::KeyCode;
pub use layout::{Axis, Direction, Geometry, Layout, LayoutError, Preset, PresetParams, SizeSpec, SplitSize};
pub use model::{
    Client, ClientInfo, ClientView, CmdContext, CmdOutput, Model, Pane, PasteBuffer, Session, Window,
};
pub use time::WallClock;

/// A tmux style string (`fg=black,bg=green,bold`) applied to the default
/// style, as the status line applies them; unknown words are ignored.
pub fn parse_style(spec: &str) -> lif_types::Style {
    status::parse_style(spec)
}

#[cfg(test)]
mod tests;
