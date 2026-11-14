//! Key tables: which key runs which copy-mode command. Keys are data, so a
//! configuration can rebind or unbind anything. The bindings themselves
//! (tmux's defaults included) live in lif-core's model; a [`KeyTable`] is
//! compiled from them.

use crate::command::Command;
use lif_core::bindings::{COPY_MODE_TABLE, COPY_MODE_VI_TABLE};
use lif_core::{Arg, CommandList, KeyBindings, KeyCode};
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
            ModeKeys::Vi => COPY_MODE_VI_TABLE,
            ModeKeys::Emacs => COPY_MODE_TABLE,
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
    /// What a key binding's commands mean to copy mode, when they are one
    /// of the forms tmux's default tables use (as `list-keys` prints them):
    ///
    /// * `send-keys -X <command>`: run it;
    /// * `send-keys -X <command>` for a command that needs an argument
    ///   (a lif shorthand), or `command-prompt ... { send-keys -X <command>
    ///   [--] "%%" }`: ask for the argument, then run it;
    /// * `send-keys -FX search-forward|search-backward [--]
    ///   "#{copy_cursor_word}"`: search for the word under the cursor;
    /// * `command-prompt -N -I <digit> ... { send-keys -N "%%" }`: start a
    ///   repeat count.
    ///
    /// `None` for anything else: the server runs such bindings as ordinary
    /// commands (their `send-keys -X` reach copy mode through
    /// [`CopyMode::run_command`](crate::CopyMode::run_command)).
    pub fn from_commands(list: &CommandList) -> Option<Action> {
        let [cmd] = list.commands() else { return None };
        let a = cmd.args();
        match cmd.name() {
            "send-keys" if a.has('X') && !a.has('N') && a.get('t').is_none() => {
                let words = a.strings(0);
                let (name, rest) = words.split_first()?;
                let rest = rest.strip_prefix(&["--".to_string()]).unwrap_or(rest);
                let c = Command::from_name(name)?;
                match rest {
                    [] if c.takes_argument() => Some(Action::Ask(c)),
                    [] => Some(Action::Command(c)),
                    [w] if a.has('F') && w == "#{copy_cursor_word}" => match c {
                        Command::SearchForward | Command::SearchForwardText => Some(Action::SearchWord { forward: true }),
                        Command::SearchBackward | Command::SearchBackwardText => {
                            Some(Action::SearchWord { forward: false })
                        }
                        _ => None,
                    },
                    _ => None,
                }
            }
            "command-prompt" if a.get('t').is_none() => {
                let inner = match a.values().first()? {
                    Arg::Block(l) => l.clone(),
                    Arg::Str(s) => lif_core::parse_string(s).ok()?,
                };
                let [ic] = inner.commands() else { return None };
                let ia = ic.args();
                if ic.name() != "send-keys" {
                    return None;
                }
                let words = ia.strings(0);
                if a.has('N') && !ia.has('X') && ia.get('N') == Some("%%") && words.is_empty() {
                    let d: u8 = a.get('I')?.parse().ok()?;
                    return (1..=9).contains(&d).then_some(Action::Count(d));
                }
                if !ia.has('X') || ia.has('N') {
                    return None;
                }
                let (name, rest) = words.split_first()?;
                let rest = rest.strip_prefix(&["--".to_string()]).unwrap_or(rest);
                let c = Command::from_name(name)?;
                (rest == ["%%"] && c.takes_argument()).then_some(Action::Ask(c))
            }
            _ => None,
        }
    }

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

    /// tmux's default table for `keys` (lif-core's
    /// `COPY_MODE_VI_DEFAULTS` / `COPY_MODE_DEFAULTS`).
    pub fn defaults(keys: ModeKeys) -> KeyTable {
        KeyTable::from_bindings(&KeyBindings::with_defaults(), keys.table_name())
    }

    /// The table copy mode uses for the model's key table `table`: every
    /// binding [`Action::from_commands`] understands. Bindings it does not
    /// are left out (the server runs those as commands).
    pub fn from_bindings(bindings: &KeyBindings, table: &str) -> KeyTable {
        let list = bindings.bindings(table);
        let mut t = KeyTable { map: HashMap::with_capacity(list.len()) };
        for b in list {
            if let Some(a) = Action::from_commands(&b.commands) {
                t.map.insert(b.key, a);
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
    fn defaults_compile() {
        use lif_core::bindings::{COPY_MODE_DEFAULTS, COPY_MODE_VI_DEFAULTS};
        for (keys, list) in [(ModeKeys::Vi, COPY_MODE_VI_DEFAULTS), (ModeKeys::Emacs, COPY_MODE_DEFAULTS)] {
            for (k, c) in list {
                let l = lif_core::parse_string(c).unwrap();
                assert!(Action::from_commands(&l).is_some(), "{k}: {c}");
            }
            // Every default binding becomes a copy-mode action.
            assert_eq!(KeyTable::defaults(keys).len(), list.len(), "{keys:?}");
        }
    }

    #[test]
    fn command_forms() {
        let act = |s: &str| Action::from_commands(&lif_core::parse_string(s).unwrap());
        assert_eq!(act("send-keys -X cursor-up"), Some(Action::Command(Command::CursorUp)));
        assert_eq!(act("send -X search-forward"), Some(Action::Ask(Command::SearchForward)));
        assert_eq!(
            act("command-prompt -p x { send-keys -X jump-forward -- \"%%\" }"),
            Some(Action::Ask(Command::JumpForward))
        );
        assert_eq!(act("send-keys -FX search-backward \"#{copy_cursor_word}\""), Some(Action::SearchWord { forward: false }));
        assert_eq!(act("command-prompt -N -I 7 -p (repeat) { send-keys -N \"%%\" }"), Some(Action::Count(7)));
        // Anything else is run as commands by the server.
        assert_eq!(act("send-keys -X -N 5 scroll-up"), None);
        assert_eq!(act("send-keys -X copy-pipe-and-cancel pbcopy"), None);
        assert_eq!(act("copy-mode -q"), None);
        assert_eq!(act("send-keys -X cancel ; kill-pane"), None);
    }
}
