//! Key tables: which key runs which copy-mode command. Keys are data, so a
//! configuration can rebind or unbind anything.

use crate::command::Command;
use lif_core::KeyCode;
use lif_types::KeyEvent;
use std::collections::HashMap;

/// Which default key table applies (tmux's `mode-keys` option).
#[derive(Clone, Copy, Debug, Default, PartialEq, Eq, Hash)]
pub enum ModeKeys {
    Vi,
    #[default]
    Emacs,
}

impl ModeKeys {
    /// tmux's start-up rule: `$VISUAL`, or if unset `$EDITOR`, decides; if
    /// the program's base name contains "vi" the keys are vi, otherwise (or
    /// with neither set) emacs. Pass the variables' values; this crate never
    /// reads the environment itself.
    pub fn from_editor(visual: Option<&str>, editor: Option<&str>) -> ModeKeys {
        let Some(s) = visual.or(editor) else { return ModeKeys::Emacs };
        let base = s.rsplit('/').next().unwrap_or(s);
        if base.contains("vi") { ModeKeys::Vi } else { ModeKeys::Emacs }
    }

    /// The tmux name of the key table: `copy-mode-vi` or `copy-mode`.
    pub const fn table_name(self) -> &'static str {
        match self {
            ModeKeys::Vi => "copy-mode-vi",
            ModeKeys::Emacs => "copy-mode",
        }
    }
}

/// What a key does in copy mode.
#[derive(Clone, Copy, Debug, PartialEq, Eq, Hash)]
pub enum Action {
    /// Run the command now (`send-keys -X <command>`).
    Command(Command),
    /// Ask for the command's argument first, then run it: the next key for
    /// jumps (copy mode reads it itself), a prompt shown by the server for
    /// searches and goto-line (see [`Outcome::Prompt`](crate::Outcome)).
    Ask(Command),
    /// Search for the word under the cursor (vi `*` and `#`).
    SearchWord { forward: bool },
    /// Start a repeat count with this digit (vi 1-9, emacs M-1..M-9). While
    /// a count is being typed, plain digits extend it.
    Count(u8),
}

impl Action {
    /// Parse the textual form used by key tables and configuration:
    /// `<command>`, `ask <command>`, `search-word-forward`,
    /// `search-word-backward` or `count <digit>`.
    pub fn parse(s: &str) -> Option<Action> {
        let s = s.trim();
        if let Some(rest) = s.strip_prefix("ask ") {
            let c = Command::from_name(rest.trim())?;
            return c.takes_argument().then_some(Action::Ask(c));
        }
        if let Some(rest) = s.strip_prefix("count ") {
            let d: u8 = rest.trim().parse().ok()?;
            return (1..=9).contains(&d).then_some(Action::Count(d));
        }
        match s {
            "search-word-forward" => Some(Action::SearchWord { forward: true }),
            "search-word-backward" => Some(Action::SearchWord { forward: false }),
            _ => Command::from_name(s).map(Action::Command),
        }
    }
}

/// tmux 3.8's default `copy-mode-vi` table, as (key, action). Mouse
/// bindings are handled by [`CopyMode::handle_mouse`](crate::CopyMode).
pub const VI_DEFAULTS: &[(&str, &str)] = &[
    ("Enter", "copy-selection-and-cancel"),
    ("Escape", "clear-selection"),
    ("Space", "begin-selection"),
    ("#", "search-word-backward"),
    ("$", "end-of-line"),
    ("%", "next-matching-bracket"),
    ("*", "search-word-forward"),
    (",", "jump-reverse"),
    ("/", "ask search-forward"),
    ("0", "start-of-line"),
    ("1", "count 1"),
    ("2", "count 2"),
    ("3", "count 3"),
    ("4", "count 4"),
    ("5", "count 5"),
    ("6", "count 6"),
    ("7", "count 7"),
    ("8", "count 8"),
    ("9", "count 9"),
    (":", "ask goto-line"),
    (";", "jump-again"),
    ("?", "ask search-backward"),
    ("A", "append-selection-and-cancel"),
    ("B", "previous-space"),
    ("D", "copy-end-of-line-and-cancel"),
    ("E", "next-space-end"),
    ("F", "ask jump-backward"),
    ("G", "history-bottom"),
    ("H", "top-line"),
    ("J", "scroll-down"),
    ("K", "scroll-up"),
    ("L", "bottom-line"),
    ("M", "middle-line"),
    ("N", "search-reverse"),
    ("P", "toggle-position"),
    ("T", "ask jump-to-backward"),
    ("V", "select-line"),
    ("W", "next-space"),
    ("X", "set-mark"),
    ("^", "back-to-indentation"),
    ("b", "previous-word"),
    ("e", "next-word-end"),
    ("f", "ask jump-forward"),
    ("g", "history-top"),
    ("h", "cursor-left"),
    ("j", "cursor-down"),
    ("k", "cursor-up"),
    ("l", "cursor-right"),
    ("n", "search-again"),
    ("o", "other-end"),
    ("q", "cancel"),
    ("r", "refresh-now"),
    ("t", "ask jump-to-forward"),
    ("v", "rectangle-toggle"),
    ("w", "next-word"),
    ("z", "scroll-middle"),
    ("{", "previous-paragraph"),
    ("}", "next-paragraph"),
    ("BSpace", "cursor-left"),
    ("Home", "start-of-line"),
    ("End", "end-of-line"),
    ("NPage", "page-down"),
    ("PPage", "page-up"),
    ("Up", "cursor-up"),
    ("Down", "cursor-down"),
    ("Left", "cursor-left"),
    ("Right", "cursor-right"),
    ("M-x", "jump-to-mark"),
    ("C-[", "clear-selection"),
    ("C-b", "page-up"),
    ("C-c", "cancel"),
    ("C-d", "halfpage-down"),
    ("C-e", "scroll-down"),
    ("C-f", "page-down"),
    ("C-h", "cursor-left"),
    ("C-j", "copy-selection-and-cancel"),
    ("C-u", "halfpage-up"),
    ("C-v", "rectangle-toggle"),
    ("C-y", "scroll-up"),
    ("C-Up", "scroll-up"),
    ("C-Down", "scroll-down"),
];

/// tmux 3.8's default `copy-mode` (emacs) table.
pub const EMACS_DEFAULTS: &[(&str, &str)] = &[
    ("Escape", "cancel"),
    ("Space", "page-down"),
    (",", "jump-reverse"),
    (";", "jump-again"),
    ("F", "ask jump-backward"),
    ("N", "search-reverse"),
    ("P", "toggle-position"),
    ("R", "rectangle-toggle"),
    ("T", "ask jump-to-backward"),
    ("X", "set-mark"),
    ("f", "ask jump-forward"),
    ("g", "ask goto-line"),
    ("n", "search-again"),
    ("q", "cancel"),
    ("r", "refresh-now"),
    ("t", "ask jump-to-forward"),
    ("Home", "start-of-line"),
    ("End", "end-of-line"),
    ("NPage", "page-down"),
    ("PPage", "page-up"),
    ("Up", "cursor-up"),
    ("Down", "cursor-down"),
    ("Left", "cursor-left"),
    ("Right", "cursor-right"),
    ("M-1", "count 1"),
    ("M-2", "count 2"),
    ("M-3", "count 3"),
    ("M-4", "count 4"),
    ("M-5", "count 5"),
    ("M-6", "count 6"),
    ("M-7", "count 7"),
    ("M-8", "count 8"),
    ("M-9", "count 9"),
    ("M-<", "history-top"),
    ("M->", "history-bottom"),
    ("M-R", "top-line"),
    ("M-b", "previous-word"),
    ("M-f", "next-word-end"),
    ("M-l", "cursor-centre-horizontal"),
    ("M-m", "back-to-indentation"),
    ("M-r", "middle-line"),
    ("M-v", "page-up"),
    ("M-w", "copy-selection-and-cancel"),
    ("M-x", "jump-to-mark"),
    ("M-{", "previous-paragraph"),
    ("M-}", "next-paragraph"),
    ("M-Up", "halfpage-up"),
    ("M-Down", "halfpage-down"),
    ("C-Space", "begin-selection"),
    ("C-[", "cancel"),
    ("C-a", "start-of-line"),
    ("C-b", "cursor-left"),
    ("C-c", "cancel"),
    ("C-e", "end-of-line"),
    ("C-f", "cursor-right"),
    ("C-g", "clear-selection"),
    ("C-k", "copy-end-of-line-and-cancel"),
    ("C-l", "recentre-top-bottom"),
    ("C-n", "cursor-down"),
    ("C-p", "cursor-up"),
    ("C-r", "ask search-backward-incremental"),
    ("C-s", "ask search-forward-incremental"),
    ("C-v", "page-down"),
    ("C-w", "copy-selection-and-cancel"),
    ("C-Up", "scroll-up"),
    ("C-Down", "scroll-down"),
    ("C-M-b", "previous-matching-bracket"),
    ("C-M-f", "next-matching-bracket"),
    ("C-M-Up", "previous-prompt"),
    ("C-M-Down", "next-prompt"),
];

/// Default bindings of tmux 3.8 that lif's copy mode does not implement,
/// with the reason, so `list-keys`-style tooling can say so.
pub const LEFT_OUT: &[(&str, &str, &str)] = &[
    ("copy-mode", "L", "line-numbers-toggle: needs a line-number gutter in the renderer"),
];

/// A copy-mode key table.
#[derive(Clone, Debug, Default)]
pub struct KeyTable {
    map: HashMap<KeyCode, Action>,
}

impl KeyTable {
    /// An empty table.
    pub fn new() -> KeyTable {
        KeyTable::default()
    }

    /// tmux's default table for `keys`.
    pub fn defaults(keys: ModeKeys) -> KeyTable {
        let list = match keys {
            ModeKeys::Vi => VI_DEFAULTS,
            ModeKeys::Emacs => EMACS_DEFAULTS,
        };
        let mut t = KeyTable { map: HashMap::with_capacity(list.len()) };
        for (k, a) in list {
            // The defaults are checked by a test; a bad entry is skipped.
            if let (Some(k), Some(a)) = (KeyCode::parse(k), Action::parse(a)) {
                t.map.insert(k, a);
            }
        }
        t
    }

    pub fn bind(&mut self, key: KeyCode, action: Action) {
        self.map.insert(key, action);
    }

    /// Bind by tmux key name and action text (see [`Action::parse`]).
    pub fn bind_str(&mut self, key: &str, action: &str) -> Result<(), String> {
        let k = KeyCode::parse(key).ok_or_else(|| format!("unknown key: {key}"))?;
        let a = Action::parse(action).ok_or_else(|| format!("unknown copy-mode command: {action}"))?;
        self.map.insert(k, a);
        Ok(())
    }

    pub fn unbind(&mut self, key: &KeyCode) -> bool {
        self.map.remove(key).is_some()
    }

    pub fn get(&self, key: &KeyCode) -> Option<Action> {
        self.map.get(key).copied()
    }

    /// The action bound to a key event, if any.
    pub fn lookup(&self, ev: &KeyEvent) -> Option<Action> {
        self.get(&KeyCode::from_event(ev)?)
    }

    pub fn len(&self) -> usize {
        self.map.len()
    }

    pub fn is_empty(&self) -> bool {
        self.map.is_empty()
    }

    pub fn iter(&self) -> impl Iterator<Item = (&KeyCode, &Action)> {
        self.map.iter()
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn editor_rule() {
        assert_eq!(ModeKeys::from_editor(None, None), ModeKeys::Emacs);
        assert_eq!(ModeKeys::from_editor(None, Some("/usr/bin/nvim")), ModeKeys::Vi);
        assert_eq!(ModeKeys::from_editor(Some("emacs"), Some("vim")), ModeKeys::Emacs, "VISUAL wins");
        assert_eq!(ModeKeys::from_editor(Some("/opt/vi/bin/nano"), None), ModeKeys::Emacs, "base name only");
        assert_eq!(ModeKeys::from_editor(Some(""), Some("vi")), ModeKeys::Emacs, "set but empty VISUAL");
    }

    #[test]
    fn defaults_parse() {
        for (keys, list) in [(ModeKeys::Vi, VI_DEFAULTS), (ModeKeys::Emacs, EMACS_DEFAULTS)] {
            for (k, a) in list {
                assert!(KeyCode::parse(k).is_some(), "{k}");
                assert!(Action::parse(a).is_some(), "{a}");
            }
            let t = KeyTable::defaults(keys);
            // C-[ is Escape, so it shares an entry.
            assert_eq!(t.len(), list.len() - 1, "{keys:?}");
        }
    }
}
