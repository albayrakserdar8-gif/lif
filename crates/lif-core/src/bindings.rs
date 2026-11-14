//! Key tables, tmux's default bindings, and the per-client prefix state
//! machine.

use crate::cmd::{CommandList, escape_arg, parse_string};
use crate::keys::KeyCode;
use lif_types::{KeyAction, KeyEvent};
use std::collections::{BTreeMap, HashMap};

/// Name of the table keys are looked up in when no prefix is pending.
pub const ROOT_TABLE: &str = "root";
/// Name of the table entered by the prefix key.
pub const PREFIX_TABLE: &str = "prefix";

/// One key binding.
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct Binding {
    /// The key.
    pub key: KeyCode,
    /// `bind-key -r`: the key may be repeated without the prefix within
    /// `repeat-time`.
    pub repeat: bool,
    /// `bind-key -N` note.
    pub note: Option<String>,
    /// What the key runs.
    pub commands: CommandList,
}

/// All key tables. Tables are created on first bind and removed when their
/// last binding goes; copy-mode tables can be added by name the same way.
#[derive(Clone, Debug, Default)]
pub struct KeyBindings {
    tables: BTreeMap<String, HashMap<KeyCode, Binding>>,
}

/// tmux 3.8's default prefix table, restricted to commands lif implements
/// (choose-tree, clock-mode, menus, popups, marks and floating panes are
/// left out). `(key, repeat, command)`.
const DEFAULTS: &[(&str, bool, &str)] = &[
    ("C-b", false, "send-prefix"),
    ("C-o", false, "rotate-window"),
    ("Space", false, "next-layout"),
    ("!", false, "break-pane"),
    ("\"", false, "split-window"),
    ("#", false, "list-buffers"),
    ("$", false, "command-prompt -I \"#S\" { rename-session \"%%\" }"),
    ("%", false, "split-window -h"),
    ("&", false, "confirm-before -p \"kill-window #W? (y/n)\" kill-window"),
    ("'", false, "command-prompt -p index { select-window -t \":%%\" }"),
    ("(", false, "switch-client -p"),
    (")", false, "switch-client -n"),
    (",", false, "command-prompt -I \"#W\" { rename-window \"%%\" }"),
    ("-", false, "delete-buffer"),
    (".", false, "command-prompt { move-window -t \"%%\" }"),
    ("0", false, "select-window -t :=0"),
    ("1", false, "select-window -t :=1"),
    ("2", false, "select-window -t :=2"),
    ("3", false, "select-window -t :=3"),
    ("4", false, "select-window -t :=4"),
    ("5", false, "select-window -t :=5"),
    ("6", false, "select-window -t :=6"),
    ("7", false, "select-window -t :=7"),
    ("8", false, "select-window -t :=8"),
    ("9", false, "select-window -t :=9"),
    (":", false, "command-prompt"),
    (";", false, "last-pane"),
    ("?", false, "list-keys"),
    ("E", false, "select-layout -E"),
    ("L", false, "switch-client -l"),
    ("[", false, "copy-mode"),
    ("]", false, "paste-buffer -p"),
    ("c", false, "new-window"),
    ("d", false, "detach-client"),
    ("i", false, "display-message"),
    ("l", false, "last-window"),
    ("n", false, "next-window"),
    ("o", false, "select-pane -t :.+"),
    ("p", false, "previous-window"),
    ("q", false, "display-panes"),
    ("r", false, "refresh-client"),
    ("x", false, "confirm-before -p \"kill-pane #P? (y/n)\" kill-pane"),
    ("z", false, "resize-pane -Z"),
    ("{", false, "swap-pane -U"),
    ("}", false, "swap-pane -D"),
    ("PPage", false, "copy-mode -u"),
    ("Up", true, "select-pane -U"),
    ("Down", true, "select-pane -D"),
    ("Left", true, "select-pane -L"),
    ("Right", true, "select-pane -R"),
    ("M-1", false, "select-layout even-horizontal"),
    ("M-2", false, "select-layout even-vertical"),
    ("M-3", false, "select-layout main-horizontal"),
    ("M-4", false, "select-layout main-vertical"),
    ("M-5", false, "select-layout tiled"),
    ("M-6", false, "select-layout main-horizontal-mirrored"),
    ("M-7", false, "select-layout main-vertical-mirrored"),
    ("M-n", false, "next-window -a"),
    ("M-o", false, "rotate-window -D"),
    ("M-p", false, "previous-window -a"),
    ("M-Up", true, "resize-pane -U 5"),
    ("M-Down", true, "resize-pane -D 5"),
    ("M-Left", true, "resize-pane -L 5"),
    ("M-Right", true, "resize-pane -R 5"),
    ("C-Up", true, "resize-pane -U"),
    ("C-Down", true, "resize-pane -D"),
    ("C-Left", true, "resize-pane -L"),
    ("C-Right", true, "resize-pane -R"),
];

impl KeyBindings {
    /// No bindings at all.
    pub fn new() -> Self {
        Self::default()
    }

    /// tmux's default prefix table (see the source for the list).
    pub fn with_defaults() -> Self {
        let mut b = Self::new();
        for (key, repeat, cmd) in DEFAULTS {
            let key = KeyCode::parse(key).unwrap_or_else(|| panic!("bad default key {key}"));
            let commands = parse_string(cmd).unwrap_or_else(|e| panic!("bad default binding {cmd}: {e}"));
            b.bind(PREFIX_TABLE, Binding { key, repeat: *repeat, note: None, commands });
        }
        b
    }

    /// The binding for an input event in `table`.
    pub fn lookup(&self, table: &str, key: &KeyEvent) -> Option<&Binding> {
        self.lookup_code(table, &KeyCode::from_event(key)?)
    }

    /// The binding for a key code in `table`.
    pub fn lookup_code(&self, table: &str, key: &KeyCode) -> Option<&Binding> {
        self.tables.get(table)?.get(key)
    }

    /// Add or replace a binding.
    pub fn bind(&mut self, table: &str, binding: Binding) {
        self.tables.entry(table.to_string()).or_default().insert(binding.key, binding);
    }

    /// Remove a binding; returns whether it existed.
    pub fn unbind(&mut self, table: &str, key: &KeyCode) -> bool {
        let Some(t) = self.tables.get_mut(table) else { return false };
        let removed = t.remove(key).is_some();
        if t.is_empty() {
            self.tables.remove(table);
        }
        removed
    }

    /// Remove every binding in `table`.
    pub fn unbind_all(&mut self, table: &str) {
        self.tables.remove(table);
    }

    /// Whether a table exists (has at least one binding).
    pub fn has_table(&self, table: &str) -> bool {
        self.tables.contains_key(table)
    }

    /// Table names in order.
    pub fn table_names(&self) -> impl Iterator<Item = &str> {
        self.tables.keys().map(|s| s.as_str())
    }

    /// Bindings of a table in tmux's `list-keys` order.
    pub fn bindings(&self, table: &str) -> Vec<&Binding> {
        let mut v: Vec<&Binding> = self.tables.get(table).map(|t| t.values().collect()).unwrap_or_default();
        v.sort_by_key(|b| b.key.sort_key());
        v
    }

    /// `list-keys` output for `table` (all tables when `None`), one
    /// `bind-key ...` line per binding, aligned as tmux aligns them.
    pub fn list(&self, table: Option<&str>, key: Option<&KeyCode>) -> String {
        let tables: Vec<&str> = match table {
            Some(t) => vec![t],
            None => self.table_names().collect(),
        };
        let mut rows: Vec<(&str, &Binding, String)> = Vec::new();
        for t in &tables {
            for b in self.bindings(t) {
                if key.is_none_or(|k| *k == b.key) {
                    rows.push((t, b, escape_arg(&b.key.to_string())));
                }
            }
        }
        let tw = rows.iter().map(|(t, _, _)| t.len()).max().unwrap_or(0);
        let kw = rows.iter().map(|(_, _, k)| crate::status::text_width(k)).max().unwrap_or(0);
        let any_repeat = rows.iter().any(|(_, b, _)| b.repeat);
        let mut out = String::new();
        for (t, b, k) in rows {
            let r = match (any_repeat, b.repeat) {
                (true, true) => "-r ",
                (true, false) => "   ",
                _ => " ",
            };
            let pad = kw - crate::status::text_width(&k);
            out.push_str(&format!("bind-key {r}-T {t:tw$} {k}{} {}\n", " ".repeat(pad), b.commands.print(true)));
        }
        out
    }
}

/// What to do with a key press.
#[derive(Clone, Debug, PartialEq, Eq)]
pub enum KeyDecision {
    /// Send the key to the active pane.
    SendToPane,
    /// Run these commands with the client as context.
    Run(CommandList),
    /// Drop the key (it was the prefix, or an unbound key after the prefix).
    Swallow,
}

/// One client's key table state: which table the next key is looked up
/// in, and the repeat window after a `-r` binding. Time is passed in by the
/// caller in milliseconds from any fixed origin.
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct KeyState {
    table: String,
    repeat_until: Option<u64>,
}

impl Default for KeyState {
    fn default() -> Self {
        Self { table: ROOT_TABLE.to_string(), repeat_until: None }
    }
}

impl KeyState {
    /// The table the next key will be looked up in.
    pub fn table(&self) -> &str {
        &self.table
    }

    /// Whether the prefix (or another non-root table) is pending; drives
    /// `#{client_prefix}`.
    pub fn in_prefix(&self) -> bool {
        self.table != ROOT_TABLE
    }

    /// Switch table (`switch-client -T`).
    pub fn set_table(&mut self, table: &str) {
        self.table = table.to_string();
        self.repeat_until = None;
    }

    /// When the repeat window ends, if one is open. The server may arm a
    /// timer for it to redraw the status line; [`KeyState::expire`] then
    /// resets the state.
    pub fn repeat_deadline(&self) -> Option<u64> {
        self.repeat_until
    }

    /// Leave repeat mode if its time is up. Returns whether the state
    /// changed.
    pub fn expire(&mut self, now_ms: u64) -> bool {
        match self.repeat_until {
            Some(d) if now_ms >= d => {
                self.reset();
                true
            }
            _ => false,
        }
    }

    fn reset(&mut self) {
        self.table.clear();
        self.table.push_str(ROOT_TABLE);
        self.repeat_until = None;
    }

    /// Decide what a key press does, following tmux's
    /// `server_client_key_callback`: the prefix keys switch to the prefix
    /// table; a binding found in the current table runs (and with `-r` opens
    /// a `repeat_time_ms` window in which the table stays); a key not bound
    /// in a non-root table is retried in the root table and otherwise
    /// swallowed; in the repeat window an unbound key leaves repeat mode and
    /// goes to the pane.
    pub fn handle(
        &mut self,
        bindings: &KeyBindings,
        prefixes: &[KeyCode],
        repeat_time_ms: u64,
        ev: &KeyEvent,
        now_ms: u64,
    ) -> KeyDecision {
        self.expire(now_ms);
        let Some(key) = KeyCode::from_event(ev) else {
            if ev.action == KeyAction::Release && self.in_prefix() {
                return KeyDecision::Swallow;
            }
            return KeyDecision::SendToPane;
        };
        // Whether the key arrived in the root table; tmux compares the first
        // table tried with the current one (a bool avoids a String per key).
        let mut first_root = self.table == ROOT_TABLE;
        loop {
            if prefixes.contains(&key) && self.table != PREFIX_TABLE {
                self.set_table(PREFIX_TABLE);
                return KeyDecision::Swallow;
            }
            let was_repeat = self.repeat_until.is_some();
            if let Some(b) = bindings.lookup_code(&self.table, &key) {
                if was_repeat && !b.repeat {
                    self.reset();
                    first_root = true;
                    continue;
                }
                if repeat_time_ms > 0 && b.repeat {
                    self.repeat_until = Some(now_ms + repeat_time_ms);
                } else {
                    self.reset();
                }
                return KeyDecision::Run(b.commands.clone());
            }
            if self.table != ROOT_TABLE || was_repeat {
                self.reset();
                if was_repeat {
                    first_root = true;
                }
                continue;
            }
            if !first_root && !was_repeat {
                self.reset();
                return KeyDecision::Swallow;
            }
            return KeyDecision::SendToPane;
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use lif_types::{Key, Mods};

    fn ev(name: &str) -> KeyEvent {
        KeyCode::parse(name).unwrap().to_event()
    }

    fn run(d: &KeyDecision) -> String {
        match d {
            KeyDecision::Run(c) => c.to_string(),
            other => format!("{other:?}"),
        }
    }

    #[test]
    fn defaults_parse_and_list_like_tmux() {
        let b = KeyBindings::with_defaults();
        let list = b.list(Some(PREFIX_TABLE), None);
        // Lines as printed by tmux 3.8 "list-keys -T prefix" (same key width
        // since M-Right / C-Right are the longest keys in both).
        for line in [
            "bind-key    -T prefix Space   next-layout",
            "bind-key    -T prefix \\\"      split-window",
            "bind-key    -T prefix \\$      command-prompt -I \"#S\" { rename-session \"%%\" }",
            "bind-key    -T prefix \\%      split-window -h",
            "bind-key    -T prefix &       confirm-before -p \"kill-window #W? (y/n)\" kill-window",
            "bind-key    -T prefix \\'      command-prompt -p index { select-window -t \":%%\" }",
            "bind-key    -T prefix 0       select-window -t :=0",
            "bind-key    -T prefix \\;      last-pane",
            "bind-key    -T prefix o       select-pane -t :.+",
            "bind-key    -T prefix \\{      swap-pane -U",
            "bind-key -r -T prefix Up      select-pane -U",
            "bind-key    -T prefix M-1     select-layout even-horizontal",
            "bind-key -r -T prefix M-Right resize-pane -R 5",
            "bind-key    -T prefix C-b     send-prefix",
        ] {
            assert!(list.lines().any(|l| l == line), "missing {line:?} in\n{list}");
        }
        let first: Vec<&str> = list.lines().take(3).collect();
        assert_eq!(first[0], "bind-key    -T prefix Space   next-layout");
        assert!(list.lines().last().unwrap().contains("C-Right"));
    }

    #[test]
    fn prefix_state_machine() {
        let b = KeyBindings::with_defaults();
        let prefix = [KeyCode::parse("C-b").unwrap()];
        let mut s = KeyState::default();
        assert_eq!(s.handle(&b, &prefix, 500, &ev("a"), 0), KeyDecision::SendToPane);
        assert_eq!(s.handle(&b, &prefix, 500, &ev("C-b"), 0), KeyDecision::Swallow);
        assert!(s.in_prefix());
        assert_eq!(run(&s.handle(&b, &prefix, 500, &ev("%"), 0)), "split-window -h");
        assert!(!s.in_prefix());
        // C-b C-b sends the prefix through.
        s.handle(&b, &prefix, 500, &ev("C-b"), 0);
        assert_eq!(run(&s.handle(&b, &prefix, 500, &ev("C-b"), 0)), "send-prefix");
        // Unbound key after the prefix is swallowed.
        s.handle(&b, &prefix, 500, &ev("C-b"), 0);
        assert_eq!(s.handle(&b, &prefix, 500, &ev("F12"), 0), KeyDecision::Swallow);
        assert_eq!(s.table(), ROOT_TABLE);
    }

    #[test]
    fn repeat_window() {
        let b = KeyBindings::with_defaults();
        let prefix = [KeyCode::parse("C-b").unwrap()];
        let mut s = KeyState::default();
        s.handle(&b, &prefix, 500, &ev("C-b"), 1000);
        assert_eq!(run(&s.handle(&b, &prefix, 500, &ev("Up"), 1000)), "select-pane -U");
        assert_eq!(s.repeat_deadline(), Some(1500));
        // Repeats without the prefix inside the window...
        assert_eq!(run(&s.handle(&b, &prefix, 500, &ev("Left"), 1400)), "select-pane -L");
        assert_eq!(s.repeat_deadline(), Some(1900));
        // ...a non-repeating binding in the window goes to the pane, as tmux
        // does (it is retried in the root table).
        assert_eq!(s.handle(&b, &prefix, 500, &ev("c"), 1500), KeyDecision::SendToPane);
        assert!(!s.in_prefix());
        // After the window closes, arrows go to the pane.
        s.handle(&b, &prefix, 500, &ev("C-b"), 2000);
        s.handle(&b, &prefix, 500, &ev("Up"), 2000);
        assert_eq!(s.handle(&b, &prefix, 500, &ev("Up"), 2600), KeyDecision::SendToPane);
        // repeat-time 0 disables repeating.
        s.handle(&b, &prefix, 0, &ev("C-b"), 3000);
        s.handle(&b, &prefix, 0, &ev("Up"), 3000);
        assert_eq!(s.repeat_deadline(), None);
        assert_eq!(s.handle(&b, &prefix, 0, &ev("Up"), 3001), KeyDecision::SendToPane);
        // The prefix itself is honoured in the repeat window.
        s.handle(&b, &prefix, 500, &ev("C-b"), 4000);
        s.handle(&b, &prefix, 500, &ev("Up"), 4000);
        // ...and starts a new prefix sequence (tmux retries it in root).
        assert_eq!(s.handle(&b, &prefix, 500, &ev("C-b"), 4100), KeyDecision::Swallow);
        assert!(s.in_prefix());
        assert_eq!(s.repeat_deadline(), None);
    }

    #[test]
    fn root_bindings_custom_tables_and_releases() {
        let mut b = KeyBindings::with_defaults();
        let prefix = [KeyCode::parse("C-a").unwrap(), KeyCode::parse("C-b").unwrap()];
        b.bind(ROOT_TABLE, Binding { key: KeyCode::parse("M-h").unwrap(), repeat: false, note: None, commands: parse_string("select-pane -L").unwrap() });
        b.bind("move", Binding { key: KeyCode::parse("h").unwrap(), repeat: false, note: None, commands: parse_string("select-pane -L").unwrap() });
        let mut s = KeyState::default();
        assert_eq!(run(&s.handle(&b, &prefix, 500, &ev("M-h"), 0)), "select-pane -L");
        assert_eq!(s.handle(&b, &prefix, 500, &ev("C-a"), 0), KeyDecision::Swallow, "prefix2");
        assert_eq!(run(&s.handle(&b, &prefix, 500, &ev("M-h"), 0)), "select-pane -L", "root binding after prefix");
        s.set_table("move");
        assert_eq!(run(&s.handle(&b, &prefix, 500, &ev("h"), 0)), "select-pane -L");
        s.set_table("move");
        assert_eq!(s.handle(&b, &prefix, 500, &ev("x"), 0), KeyDecision::Swallow);
        let mut rel = KeyEvent::new(Key::Char('x'), Mods::NONE);
        rel.action = KeyAction::Release;
        assert_eq!(s.handle(&b, &prefix, 500, &rel, 0), KeyDecision::SendToPane);
        s.set_table(PREFIX_TABLE);
        assert_eq!(s.handle(&b, &prefix, 500, &rel, 0), KeyDecision::Swallow);
        assert!(b.unbind(ROOT_TABLE, &KeyCode::parse("M-h").unwrap()));
        assert!(!b.has_table(ROOT_TABLE));
        assert!(b.lookup(PREFIX_TABLE, &ev("c")).is_some());
    }
}
