//! Copy mode: scrolling back through a pane's history, searching it,
//! selecting text with the keyboard or the mouse and copying it, with tmux's
//! commands and default key tables.
//!
//! The crate is pure logic. The server owns one [`CopyMode`] per pane in
//! copy mode and reaches the pane's contents through [`PaneView`], which
//! reads rows lazily: nothing here ever materializes the whole history.
//!
//! # Driving it from the server
//!
//! * Enter: [`CopyMode::new`] with the pane's cursor and the `copy-mode`
//!   flags ([`Enter`]). Leave: [`CopyMode::leave`] puts the viewport back.
//! * Keys: [`CopyMode::handle_key`]; mouse: [`CopyMode::handle_mouse`];
//!   `send-keys -X`: [`CopyMode::run_command`] with [`Command::from_name`].
//!   Each returns an [`Outcome`].
//! * Key tables: the model's `copy-mode` / `copy-mode-vi` tables (lif-core's
//!   `KeyBindings`, which also hold tmux's defaults) are the only source of
//!   bindings, so the configuration file, `bind-key -T` and `list-keys -T`
//!   all see the same keys. [`KeyTable::from_bindings`] compiles a table
//!   for [`Config::table`]; bindings it cannot express ([`Action::from_commands`]
//!   returns `None`) are run by the server as commands.
//! * Prompts ([`Outcome::Prompt`]): the server shows the prompt and
//!   reports back with [`CopyMode::prompt_changed`] (incremental search,
//!   on every edit), [`CopyMode::prompt_again`] (C-s / C-r inside an
//!   incremental prompt), [`CopyMode::prompt_accept`] and
//!   [`CopyMode::prompt_cancel`].
//! * Searches run in bounded steps. When an outcome is
//!   [`Outcome::Searching`], redraw, keep polling the other descriptors
//!   with a zero timeout, and call [`CopyMode::search_step`] with a row
//!   budget once per loop turn until the outcome is something else. A key
//!   press cancels the search. No timer is involved, so an idle server still
//!   makes no wakeups.
//! * Drawing: [`CopyMode::overlays`] gives the selection, the cursor, the
//!   `[offset/history]` indicator and the search matches of the viewport.
//! * Output arriving while in copy mode, or a resize: call
//!   [`CopyMode::refresh`] (every entry point also re-reads the view).
#![forbid(unsafe_code)]

mod command;
mod grid;
mod keys;
mod mode;
pub mod regex;
mod search;
mod view;

pub use command::Command;
pub use keys::{Action, KeyTable, LEFT_OUT, ModeKeys};
pub use mode::{
    Config, CopyMode, DEFAULT_SEARCH_STEP_ROWS, DEFAULT_WORD_SEPARATORS, DragEnd, Enter, MouseConfig, Outcome, Overlays,
    SelectionOverlay, SelectionShape,
};
pub use search::MatchSpan;
pub use view::{MemoryView, PaneView, char_width};

/// An absolute position: a row of the [`PaneView`] (0 = the oldest row
/// held) and a column.
#[derive(Clone, Copy, Debug, Default, PartialEq, Eq, PartialOrd, Ord, Hash)]
pub struct Point {
    pub row: usize,
    pub col: usize,
}
