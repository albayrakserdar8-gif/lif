//! Key tables, tmux's default bindings, and the per-client prefix state
//! machine.

use crate::cmd::{CommandList, escape_arg, parse_string};
use crate::keys::KeyCode;
use lif_types::{Key, KeyAction, KeyEvent};
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
    /// Bumped by every change, so a copy compiled from a table (copy mode's
    /// key table) knows when it is stale.
    generation: u64,
    /// tmux's copy-mode tables (`copy-mode-vi`, `copy-mode`) are part of
    /// the defaults but parsed only when something reads them: a server
    /// nobody uses copy mode on never needs them, and parsed they cost about
    /// 250 KB of its memory. `copy_pending[i]`: table `i` is still the
    /// default (not yet moved into `tables` by a change).
    copy_pending: [bool; 2],
    copy_defaults: std::cell::OnceCell<[HashMap<KeyCode, Binding>; 2]>,
    /// The default mouse bindings ([`ROOT_DEFAULTS`]) and whether each is
    /// built in, to tell a default from a rebinding.
    mouse_defaults: HashMap<KeyCode, (CommandList, bool)>,
}

/// Index of a copy-mode table in `KeyBindings::copy_pending`.
fn copy_index(table: &str) -> Option<usize> {
    match table {
        COPY_MODE_VI_TABLE => Some(0),
        COPY_MODE_TABLE => Some(1),
        _ => None,
    }
}

fn parse_defaults(list: &[(&str, &str)]) -> HashMap<KeyCode, Binding> {
    let mut t = HashMap::with_capacity(list.len());
    for (key, cmd) in list {
        let key = KeyCode::parse(key).unwrap_or_else(|| panic!("bad default key {key}"));
        let commands = parse_string(cmd).unwrap_or_else(|e| panic!("bad default binding {cmd}: {e}"));
        t.insert(key, Binding { key, repeat: false, note: None, commands });
    }
    t
}

/// tmux 3.8's default prefix table, restricted to commands lif implements
/// (choose-tree, clock-mode, menus, popups, marks and floating panes are
/// left out). `(key, repeat, command)`.
const DEFAULTS: &[(&str, bool, &str)] = &[
    ("C-b", false, "send-prefix"),
    ("C-o", false, "rotate-window"),
    ("C-z", false, "suspend-client"),
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
    ("/", false, "command-prompt -k -p key { list-keys -1N \"%%\" }"),
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
    (
        "<",
        false,
        "display-menu -T \"#[align=centre]#{window_index}:#{window_name}\" -x W -y W \"#{?#{>:#{session_windows},1},,-}Swap Left\" l { swap-window -t :-1 } \"#{?#{>:#{session_windows},1},,-}Swap Right\" r { swap-window -t :+1 } \"#{?pane_marked_set,,-}Swap Marked\" s { swap-window } '' Kill X { kill-window } Respawn R { respawn-window -k } \"#{?pane_marked,Unmark,Mark}\" m { select-pane -m } Rename n { command-prompt -F -I \"#W\" { rename-window -t \"#{window_id}\" \"%%\" } } '' \"New After\" w { new-window -a } \"New At End\" W { new-window }",
    ),
    ("=", false, "choose-buffer -Z"),
    (
        ">",
        false,
        "display-menu -T \"#[align=centre]#{pane_index} (#{pane_id})\" -x P -y P \"#{?pane_in_mode,Go To Top,}\" < { send-keys -X history-top } \"#{?pane_in_mode,Go To Bottom,}\" > { send-keys -X history-bottom } '' \"#{?pane_in_mode,,Paste}\" p { paste-buffer } '' \"Horizontal Split\" h { split-window -h } \"Vertical Split\" v { split-window -v } '' \"#{?#{>:#{window_panes},1},,-}Swap Up\" u { swap-pane -U } \"#{?#{>:#{window_panes},1},,-}Swap Down\" d { swap-pane -D } \"#{?pane_marked_set,,-}Swap Marked\" s { swap-pane } '' Kill X { kill-pane } Respawn R { respawn-pane -k } \"#{?pane_marked,Unmark,Mark}\" m { select-pane -m } \"#{?#{>:#{window_panes},1},,-}#{?window_zoomed_flag,Unzoom,Zoom}\" z { resize-pane -Z }",
    ),
    ("?", false, "list-keys"),
    ("C", false, "customize-mode -Z"),
    ("D", false, "choose-client -Z"),
    ("E", false, "select-layout -E"),
    ("L", false, "switch-client -l"),
    ("M", false, "select-pane -M"),
    ("T", false, "command-prompt -I \"#T\" { select-pane -T \"%%\" }"),
    ("[", false, "copy-mode"),
    ("]", false, "paste-buffer -p"),
    ("c", false, "new-window"),
    ("d", false, "detach-client"),
    ("f", false, "command-prompt { find-window -Z \"%%\" }"),
    ("i", false, "display-message"),
    ("l", false, "last-window"),
    ("m", false, "select-pane -m"),
    ("n", false, "next-window"),
    ("o", false, "select-pane -t :.+"),
    ("p", false, "previous-window"),
    ("q", false, "display-panes"),
    ("r", false, "refresh-client"),
    ("s", false, "choose-tree -Zs"),
    ("t", false, "clock-mode"),
    ("w", false, "choose-tree -Zw"),
    ("x", false, "confirm-before -p \"kill-pane #P? (y/n)\" kill-pane"),
    ("z", false, "resize-pane -Z"),
    ("{", false, "swap-pane -U"),
    ("~", false, "show-messages"),
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

/// Name of copy mode's key table with `mode-keys emacs`.
pub const COPY_MODE_TABLE: &str = "copy-mode";
/// Name of copy mode's key table with `mode-keys vi`.
pub const COPY_MODE_VI_TABLE: &str = "copy-mode-vi";

/// tmux 3.8's default `copy-mode-vi` table, `(key, command)`, exactly as
/// `tmux list-keys -T copy-mode-vi` prints it, without the mouse bindings
/// (copy mode handles the mouse itself) and `C-[` (the same key as Escape).
///
/// tmux 3.8's default root-table mouse bindings, for what lif has (no
/// floating panes, scrollbars or marks). `(key, built in, command)`: the
/// ones marked built in are what the server's mouse handling does by
/// itself, and it does so for as long as the binding is this one; rebound,
/// the key runs its commands, and unbound it does nothing. The others
/// always run as commands.
const ROOT_DEFAULTS: &[(&str, bool, &str)] = &[
    ("MouseDown1Pane", true, "select-pane -t = ; send-keys -M"),
    ("MouseDown1Status", true, "switch-client -t ="),
    ("MouseDown1Border", true, "select-pane -t ="),
    ("MouseDown2Pane", false, "select-pane -t = ; if-shell -F \"#{||:#{pane_in_mode},#{mouse_any_flag}}\" { send-keys -M } { paste-buffer -p }"),
    (
        "MouseDown3Pane",
        false,
        "if-shell -F -t = \"#{||:#{mouse_any_flag},#{pane_in_mode}}\" { select-pane -t = ; send-keys -M } { display-menu -T \"#[align=centre]#{pane_index} (#{pane_id})\" -t = -x M -y M \"#{?pane_in_mode,Go To Top,}\" < { send-keys -X history-top } \"#{?pane_in_mode,Go To Bottom,}\" > { send-keys -X history-bottom } '' \"#{?pane_in_mode,,Paste}\" p { paste-buffer } '' \"Horizontal Split\" h { split-window -h } \"Vertical Split\" v { split-window -v } '' \"#{?#{>:#{window_panes},1},,-}Swap Up\" u { swap-pane -U } \"#{?#{>:#{window_panes},1},,-}Swap Down\" d { swap-pane -D } \"#{?pane_marked_set,,-}Swap Marked\" s { swap-pane } '' Kill X { kill-pane } Respawn R { respawn-pane -k } \"#{?pane_marked,Unmark,Mark}\" m { select-pane -m } \"#{?#{>:#{window_panes},1},,-}#{?window_zoomed_flag,Unzoom,Zoom}\" z { resize-pane -Z } }",
    ),
    (
        "MouseDown3Status",
        false,
        "display-menu -T \"#[align=centre]#{window_index}:#{window_name}\" -t = -x W -y W \"#{?#{>:#{session_windows},1},,-}Swap Left\" l { swap-window -t :-1 } \"#{?#{>:#{session_windows},1},,-}Swap Right\" r { swap-window -t :+1 } \"#{?pane_marked_set,,-}Swap Marked\" s { swap-window } '' Kill X { kill-window } Respawn R { respawn-window -k } \"#{?pane_marked,Unmark,Mark}\" m { select-pane -m } Rename n { command-prompt -F -I \"#W\" { rename-window -t \"#{window_id}\" \"%%\" } } '' \"New After\" w { new-window -a } \"New At End\" W { new-window }",
    ),
    (
        "MouseDown3StatusLeft",
        false,
        "display-menu -T \"#[align=centre]#{session_name}\" -t = -x M -y W Renumber N { move-window -r } Rename r { command-prompt -I \"#S\" { rename-session -- \"%%\" } } Detach d { detach-client } '' \"New Session\" s { new-session } \"New Window\" w { new-window }",
    ),
    ("MouseDrag1Pane", true, "if-shell -F \"#{||:#{pane_in_mode},#{mouse_any_flag}}\" { send-keys -M } { copy-mode -M }"),
    ("MouseDrag1Border", true, "resize-pane -M"),
    ("WheelDownStatus", false, "next-window"),
    ("WheelUpPane", true, "if-shell -F \"#{||:#{alternate_on},#{pane_in_mode},#{mouse_any_flag}}\" { send-keys -M } { copy-mode -e }"),
    ("WheelUpStatus", false, "previous-window"),
    (
        "DoubleClick1Pane",
        true,
        "select-pane -t = ; if-shell -F \"#{||:#{pane_in_mode},#{mouse_any_flag}}\" { send-keys -M } { copy-mode -H ; send-keys -X select-word ; run-shell -d 0.3 ; send-keys -X copy-pipe-and-cancel }",
    ),
    (
        "TripleClick1Pane",
        true,
        "select-pane -t = ; if-shell -F \"#{||:#{pane_in_mode},#{mouse_any_flag}}\" { send-keys -M } { copy-mode -H ; send-keys -X select-line ; run-shell -d 0.3 ; send-keys -X copy-pipe-and-cancel }",
    ),
];

/// What the root table says about a mouse key.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum MouseBinding<'a> {
    /// Nothing is bound to it.
    Unbound,
    /// Its default binding, which the server's built-in handling stands
    /// for.
    BuiltIn,
    /// Its default binding, to run as commands.
    Default(&'a CommandList),
    /// Something the user bound.
    Custom(&'a CommandList),
}

/// tmux 3.8's notes for the default prefix bindings (`list-keys -N`), by
/// key.
const NOTES: &[(&str, &str)] = &[
    ("<", "Display window menu"),
    (">", "Display pane menu"),
    ("Space", "Select next layout"),
    ("!", "Break pane to a new window"),
    ("\"", "Split window vertically"),
    ("#", "List all paste buffers"),
    ("$", "Rename current session"),
    ("%", "Split window horizontally"),
    ("&", "Kill current window"),
    ("'", "Prompt for window index to select"),
    ("(", "Switch to previous client"),
    (")", "Switch to next client"),
    (",", "Rename current window"),
    ("-", "Delete the most recent paste buffer"),
    (".", "Move the current window"),
    ("/", "Describe key binding"),
    ("0", "Select window 0"),
    ("1", "Select window 1"),
    ("2", "Select window 2"),
    ("3", "Select window 3"),
    ("4", "Select window 4"),
    ("5", "Select window 5"),
    ("6", "Select window 6"),
    ("7", "Select window 7"),
    ("8", "Select window 8"),
    ("9", "Select window 9"),
    (":", "Prompt for a command"),
    (";", "Move to the previously active pane"),
    ("=", "Choose a paste buffer from a list"),
    ("?", "List key bindings"),
    ("D", "Choose and detach a client from a list"),
    ("E", "Spread panes out evenly"),
    ("L", "Switch to the last client"),
    ("M", "Clear the marked pane"),
    ("T", "Change the pane title"),
    ("[", "Enter copy mode"),
    ("]", "Paste the most recent paste buffer"),
    ("c", "Create a new window"),
    ("d", "Detach the current client"),
    ("f", "Search for a pane"),
    ("i", "Display window information"),
    ("l", "Select the previously current window"),
    ("m", "Toggle the marked pane"),
    ("n", "Select the next window"),
    ("o", "Select the next pane"),
    ("p", "Select the previous window"),
    ("q", "Display pane numbers"),
    ("r", "Redraw the current client"),
    ("s", "Choose a session from a list"),
    ("t", "Show a clock"),
    ("w", "Choose a window from a list"),
    ("x", "Kill the active pane"),
    ("z", "Zoom the active pane"),
    ("{", "Swap the active pane with the pane above"),
    ("}", "Swap the active pane with the pane below"),
    ("~", "Show messages"),
    ("PPage", "Enter copy mode and scroll up"),
    ("Up", "Select the pane above the active pane"),
    ("Down", "Select the pane below the active pane"),
    ("Left", "Select the pane to the left of the active pane"),
    ("Right", "Select the pane to the right of the active pane"),
    ("M-1", "Set the even-horizontal layout"),
    ("M-2", "Set the even-vertical layout"),
    ("M-3", "Set the main-horizontal layout"),
    ("M-4", "Set the main-vertical layout"),
    ("M-5", "Select the tiled layout"),
    ("M-6", "Set the main-horizontal-mirrored layout"),
    ("M-7", "Set the main-vertical-mirrored layout"),
    ("M-n", "Select the next window with an alert"),
    ("M-o", "Rotate through the panes in reverse"),
    ("M-p", "Select the previous window with an alert"),
    ("M-Up", "Resize the pane up by 5"),
    ("M-Down", "Resize the pane down by 5"),
    ("M-Left", "Resize the pane left by 5"),
    ("M-Right", "Resize the pane right by 5"),
    ("C-b", "Send the prefix key"),
    ("C-o", "Rotate through the panes"),
    ("C-z", "Suspend the current client"),
    ("C-Up", "Resize the pane up"),
    ("C-Down", "Resize the pane down"),
    ("C-Left", "Resize the pane left"),
    ("C-Right", "Resize the pane right"),
];

/// This and [`COPY_MODE_DEFAULTS`] are the only definition of copy mode's
/// default keys: [`KeyBindings::with_defaults`] loads them into the model,
/// where `bind-key -T`, `unbind-key -T`, `list-keys -T` and the
/// configuration file change and show them, and copy mode compiles its key
/// table from the model's (`lif_copy::KeyTable::from_bindings`).
pub const COPY_MODE_VI_DEFAULTS: &[(&str, &str)] = &[
    (r#"Enter"#, r#"send-keys -X copy-pipe-and-cancel"#),
    (r#"Escape"#, r#"send-keys -X clear-selection"#),
    (r#"Space"#, r#"send-keys -X begin-selection"#),
    (r#"#"#, r##"send-keys -FX search-backward -- "#{copy_cursor_word}""##),
    (r#"$"#, r#"send-keys -X end-of-line"#),
    (r#"%"#, r#"send-keys -X next-matching-bracket"#),
    (r#"*"#, r##"send-keys -FX search-forward -- "#{copy_cursor_word}""##),
    (r#","#, r#"send-keys -X jump-reverse"#),
    (r#"/"#, r#"command-prompt -P -T search -p "(search down)" { send-keys -X search-forward -- "%%" }"#),
    (r#"0"#, r#"send-keys -X start-of-line"#),
    (r#"1"#, r#"command-prompt -NP -I 1 -p (repeat) { send-keys -N "%%" }"#),
    (r#"2"#, r#"command-prompt -NP -I 2 -p (repeat) { send-keys -N "%%" }"#),
    (r#"3"#, r#"command-prompt -NP -I 3 -p (repeat) { send-keys -N "%%" }"#),
    (r#"4"#, r#"command-prompt -NP -I 4 -p (repeat) { send-keys -N "%%" }"#),
    (r#"5"#, r#"command-prompt -NP -I 5 -p (repeat) { send-keys -N "%%" }"#),
    (r#"6"#, r#"command-prompt -NP -I 6 -p (repeat) { send-keys -N "%%" }"#),
    (r#"7"#, r#"command-prompt -NP -I 7 -p (repeat) { send-keys -N "%%" }"#),
    (r#"8"#, r#"command-prompt -NP -I 8 -p (repeat) { send-keys -N "%%" }"#),
    (r#"9"#, r#"command-prompt -NP -I 9 -p (repeat) { send-keys -N "%%" }"#),
    (r#":"#, r#"command-prompt -P -p "(goto line)" { send-keys -X goto-line -- "%%" }"#),
    (r#";"#, r#"send-keys -X jump-again"#),
    (r#"?"#, r#"command-prompt -P -T search -p "(search up)" { send-keys -X search-backward -- "%%" }"#),
    (r#"A"#, r#"send-keys -X append-selection-and-cancel"#),
    (r#"B"#, r#"send-keys -X previous-space"#),
    (r#"D"#, r#"send-keys -X copy-pipe-end-of-line-and-cancel"#),
    (r#"E"#, r#"send-keys -X next-space-end"#),
    (r#"F"#, r#"command-prompt -1P -p "(jump backward)" { send-keys -X jump-backward -- "%%" }"#),
    (r#"G"#, r#"send-keys -X history-bottom"#),
    (r#"H"#, r#"send-keys -X top-line"#),
    (r#"J"#, r#"send-keys -X scroll-down"#),
    (r#"K"#, r#"send-keys -X scroll-up"#),
    (r#"L"#, r#"send-keys -X bottom-line"#),
    (r#"M"#, r#"send-keys -X middle-line"#),
    (r#"N"#, r#"send-keys -X search-reverse"#),
    (r#"P"#, r#"send-keys -X toggle-position"#),
    (r#"T"#, r#"command-prompt -1P -p "(jump to backward)" { send-keys -X jump-to-backward -- "%%" }"#),
    (r#"V"#, r#"send-keys -X select-line"#),
    (r#"W"#, r#"send-keys -X next-space"#),
    (r#"X"#, r#"send-keys -X set-mark"#),
    (r#"^"#, r#"send-keys -X back-to-indentation"#),
    (r#"b"#, r#"send-keys -X previous-word"#),
    (r#"e"#, r#"send-keys -X next-word-end"#),
    (r#"f"#, r#"command-prompt -1P -p "(jump forward)" { send-keys -X jump-forward -- "%%" }"#),
    (r#"g"#, r#"send-keys -X history-top"#),
    (r#"h"#, r#"send-keys -X cursor-left"#),
    (r#"j"#, r#"send-keys -X cursor-down"#),
    (r#"k"#, r#"send-keys -X cursor-up"#),
    (r#"l"#, r#"send-keys -X cursor-right"#),
    (r#"n"#, r#"send-keys -X search-again"#),
    (r#"o"#, r#"send-keys -X other-end"#),
    (r#"q"#, r#"send-keys -X cancel"#),
    (r#"r"#, r#"send-keys -X refresh-now"#),
    (r#"t"#, r#"command-prompt -1P -p "(jump to forward)" { send-keys -X jump-to-forward -- "%%" }"#),
    (r#"v"#, r#"send-keys -X rectangle-toggle"#),
    (r#"w"#, r#"send-keys -X next-word"#),
    (r#"z"#, r#"send-keys -X scroll-middle"#),
    (r#"{"#, r#"send-keys -X previous-paragraph"#),
    (r#"}"#, r#"send-keys -X next-paragraph"#),
    (r#"BSpace"#, r#"send-keys -X cursor-left"#),
    (r#"Home"#, r#"send-keys -X start-of-line"#),
    (r#"End"#, r#"send-keys -X end-of-line"#),
    (r#"NPage"#, r#"send-keys -X page-down"#),
    (r#"PPage"#, r#"send-keys -X page-up"#),
    (r#"Up"#, r#"send-keys -X cursor-up"#),
    (r#"Down"#, r#"send-keys -X cursor-down"#),
    (r#"Left"#, r#"send-keys -X cursor-left"#),
    (r#"Right"#, r#"send-keys -X cursor-right"#),
    (r#"M-x"#, r#"send-keys -X jump-to-mark"#),
    (r#"C-b"#, r#"send-keys -X page-up"#),
    (r#"C-c"#, r#"send-keys -X cancel"#),
    (r#"C-d"#, r#"send-keys -X halfpage-down"#),
    (r#"C-e"#, r#"send-keys -X scroll-down"#),
    (r#"C-f"#, r#"send-keys -X page-down"#),
    (r#"C-h"#, r#"send-keys -X cursor-left"#),
    (r#"C-j"#, r#"send-keys -X copy-pipe-and-cancel"#),
    (r#"C-u"#, r#"send-keys -X halfpage-up"#),
    (r#"C-v"#, r#"send-keys -X rectangle-toggle"#),
    (r#"C-y"#, r#"send-keys -X scroll-up"#),
    (r#"C-Up"#, r#"send-keys -X scroll-up"#),
    (r#"C-Down"#, r#"send-keys -X scroll-down"#),
];

/// tmux 3.8's default `copy-mode` (emacs) table; see
/// [`COPY_MODE_VI_DEFAULTS`]. tmux's `L` (line-numbers-toggle) is left out:
/// lif has no line-number gutter.
pub const COPY_MODE_DEFAULTS: &[(&str, &str)] = &[
    (r#"Escape"#, r#"send-keys -X cancel"#),
    (r#"Space"#, r#"send-keys -X page-down"#),
    (r#","#, r#"send-keys -X jump-reverse"#),
    (r#";"#, r#"send-keys -X jump-again"#),
    (r#"F"#, r#"command-prompt -1P -p "(jump backward)" { send-keys -X jump-backward -- "%%" }"#),
    (r#"N"#, r#"send-keys -X search-reverse"#),
    (r#"P"#, r#"send-keys -X toggle-position"#),
    (r#"R"#, r#"send-keys -X rectangle-toggle"#),
    (r#"T"#, r#"command-prompt -1P -p "(jump to backward)" { send-keys -X jump-to-backward -- "%%" }"#),
    (r#"X"#, r#"send-keys -X set-mark"#),
    (r#"f"#, r#"command-prompt -1P -p "(jump forward)" { send-keys -X jump-forward -- "%%" }"#),
    (r#"g"#, r#"command-prompt -P -p "(goto line)" { send-keys -X goto-line -- "%%" }"#),
    (r#"n"#, r#"send-keys -X search-again"#),
    (r#"q"#, r#"send-keys -X cancel"#),
    (r#"r"#, r#"send-keys -X refresh-now"#),
    (r#"t"#, r#"command-prompt -1P -p "(jump to forward)" { send-keys -X jump-to-forward -- "%%" }"#),
    (r#"Home"#, r#"send-keys -X start-of-line"#),
    (r#"End"#, r#"send-keys -X end-of-line"#),
    (r#"NPage"#, r#"send-keys -X page-down"#),
    (r#"PPage"#, r#"send-keys -X page-up"#),
    (r#"Up"#, r#"send-keys -X cursor-up"#),
    (r#"Down"#, r#"send-keys -X cursor-down"#),
    (r#"Left"#, r#"send-keys -X cursor-left"#),
    (r#"Right"#, r#"send-keys -X cursor-right"#),
    (r#"M-1"#, r#"command-prompt -NP -I 1 -p (repeat) { send-keys -N "%%" }"#),
    (r#"M-2"#, r#"command-prompt -NP -I 2 -p (repeat) { send-keys -N "%%" }"#),
    (r#"M-3"#, r#"command-prompt -NP -I 3 -p (repeat) { send-keys -N "%%" }"#),
    (r#"M-4"#, r#"command-prompt -NP -I 4 -p (repeat) { send-keys -N "%%" }"#),
    (r#"M-5"#, r#"command-prompt -NP -I 5 -p (repeat) { send-keys -N "%%" }"#),
    (r#"M-6"#, r#"command-prompt -NP -I 6 -p (repeat) { send-keys -N "%%" }"#),
    (r#"M-7"#, r#"command-prompt -NP -I 7 -p (repeat) { send-keys -N "%%" }"#),
    (r#"M-8"#, r#"command-prompt -NP -I 8 -p (repeat) { send-keys -N "%%" }"#),
    (r#"M-9"#, r#"command-prompt -NP -I 9 -p (repeat) { send-keys -N "%%" }"#),
    (r#"M-<"#, r#"send-keys -X history-top"#),
    (r#"M->"#, r#"send-keys -X history-bottom"#),
    (r#"M-R"#, r#"send-keys -X top-line"#),
    (r#"M-b"#, r#"send-keys -X previous-word"#),
    (r#"M-f"#, r#"send-keys -X next-word-end"#),
    (r#"M-l"#, r#"send-keys -X cursor-centre-horizontal"#),
    (r#"M-m"#, r#"send-keys -X back-to-indentation"#),
    (r#"M-r"#, r#"send-keys -X middle-line"#),
    (r#"M-v"#, r#"send-keys -X page-up"#),
    (r#"M-w"#, r#"send-keys -X copy-pipe-and-cancel"#),
    (r#"M-x"#, r#"send-keys -X jump-to-mark"#),
    (r#"M-{"#, r#"send-keys -X previous-paragraph"#),
    (r#"M-}"#, r#"send-keys -X next-paragraph"#),
    (r#"M-Up"#, r#"send-keys -X halfpage-up"#),
    (r#"M-Down"#, r#"send-keys -X halfpage-down"#),
    (r#"C-Space"#, r#"send-keys -X begin-selection"#),
    (r#"C-a"#, r#"send-keys -X start-of-line"#),
    (r#"C-b"#, r#"send-keys -X cursor-left"#),
    (r#"C-c"#, r#"send-keys -X cancel"#),
    (r#"C-e"#, r#"send-keys -X end-of-line"#),
    (r#"C-f"#, r#"send-keys -X cursor-right"#),
    (r#"C-g"#, r#"send-keys -X clear-selection"#),
    (r#"C-k"#, r#"send-keys -X copy-pipe-end-of-line-and-cancel"#),
    (r#"C-l"#, r#"send-keys -X recentre-top-bottom"#),
    (r#"C-n"#, r#"send-keys -X cursor-down"#),
    (r#"C-p"#, r#"send-keys -X cursor-up"#),
    (r#"C-r"#, r##"command-prompt -Pi -I "#{pane_search_string}" -T search -p "(search up)" { send-keys -X search-backward-incremental -- "%%" }"##),
    (r#"C-s"#, r##"command-prompt -Pi -I "#{pane_search_string}" -T search -p "(search down)" { send-keys -X search-forward-incremental -- "%%" }"##),
    (r#"C-v"#, r#"send-keys -X page-down"#),
    (r#"C-w"#, r#"send-keys -X copy-pipe-and-cancel"#),
    (r#"C-Up"#, r#"send-keys -X scroll-up"#),
    (r#"C-Down"#, r#"send-keys -X scroll-down"#),
    (r#"C-M-b"#, r#"send-keys -X previous-matching-bracket"#),
    (r#"C-M-f"#, r#"send-keys -X next-matching-bracket"#),
    (r#"C-M-Up"#, r#"send-keys -X previous-prompt"#),
    (r#"C-M-Down"#, r#"send-keys -X next-prompt"#),
];

impl KeyBindings {
    /// No bindings at all.
    pub fn new() -> Self {
        Self::default()
    }

    /// tmux's default prefix table (see the source for the list) and copy
    /// mode's tables ([`COPY_MODE_VI_DEFAULTS`], [`COPY_MODE_DEFAULTS`]).
    pub fn with_defaults() -> Self {
        let mut b = Self::new();
        for (key, repeat, cmd) in DEFAULTS {
            KeyCode::parse(key).unwrap_or_else(|| panic!("bad default key {key}"));
            let commands = parse_string(cmd).unwrap_or_else(|e| panic!("bad default binding {cmd}: {e}"));
            let raw = *key;
            let note = NOTES.iter().find(|(k, _)| *k == raw).map(|(_, n)| n.to_string());
            b.bind(PREFIX_TABLE, Binding { key: KeyCode::parse(raw).expect("checked"), repeat: *repeat, note, commands });
        }
        for (key, built_in, cmd) in ROOT_DEFAULTS {
            let key = KeyCode::parse(key).unwrap_or_else(|| panic!("bad default key {key}"));
            let commands = parse_string(cmd).unwrap_or_else(|e| panic!("bad default binding {cmd}: {e}"));
            b.mouse_defaults.insert(key, (commands.clone(), *built_in));
            b.bind(ROOT_TABLE, Binding { key, repeat: false, note: None, commands });
        }
        b.copy_pending = [true, true];
        b
    }

    /// What the root table has for mouse key `key`.
    pub fn mouse_binding(&self, key: &KeyCode) -> MouseBinding<'_> {
        let Some(b) = self.tables.get(ROOT_TABLE).and_then(|t| t.get(key)) else { return MouseBinding::Unbound };
        match self.mouse_defaults.get(key) {
            Some((d, built_in)) if *d == b.commands => {
                if *built_in {
                    MouseBinding::BuiltIn
                } else {
                    MouseBinding::Default(&b.commands)
                }
            }
            _ => MouseBinding::Custom(&b.commands),
        }
    }

    /// A table's bindings, the copy-mode defaults included.
    fn table(&self, table: &str) -> Option<&HashMap<KeyCode, Binding>> {
        if let Some(t) = self.tables.get(table) {
            return Some(t);
        }
        let i = copy_index(table).filter(|&i| self.copy_pending[i])?;
        let d = self
            .copy_defaults
            .get_or_init(|| [parse_defaults(COPY_MODE_VI_DEFAULTS), parse_defaults(COPY_MODE_DEFAULTS)]);
        Some(&d[i])
    }

    /// Before changing `table`: a pending copy-mode default becomes an
    /// ordinary table.
    fn own(&mut self, table: &str) {
        if let Some(i) = copy_index(table).filter(|&i| self.copy_pending[i]) {
            self.copy_pending[i] = false;
            let t = self.copy_defaults.get().map(|d| d[i].clone()).unwrap_or_else(|| {
                parse_defaults(if i == 0 { COPY_MODE_VI_DEFAULTS } else { COPY_MODE_DEFAULTS })
            });
            self.tables.insert(table.to_string(), t);
        }
    }

    /// The binding for an input event in `table`.
    pub fn lookup(&self, table: &str, key: &KeyEvent) -> Option<&Binding> {
        self.lookup_code(table, &KeyCode::from_event(key)?)
    }

    /// The binding for a key code in `table`.
    pub fn lookup_code(&self, table: &str, key: &KeyCode) -> Option<&Binding> {
        self.table(table)?.get(key)
    }

    /// Add or replace a binding.
    pub fn bind(&mut self, table: &str, binding: Binding) {
        self.generation += 1;
        self.own(table);
        self.tables.entry(table.to_string()).or_default().insert(binding.key, binding);
    }

    /// A number that changes whenever any binding does.
    pub fn generation(&self) -> u64 {
        self.generation
    }

    /// Remove a binding; returns whether it existed.
    pub fn unbind(&mut self, table: &str, key: &KeyCode) -> bool {
        self.own(table);
        let Some(t) = self.tables.get_mut(table) else { return false };
        self.generation += 1;
        let removed = t.remove(key).is_some();
        if t.is_empty() {
            self.tables.remove(table);
        }
        removed
    }

    /// Remove every binding in `table`.
    pub fn unbind_all(&mut self, table: &str) {
        self.generation += 1;
        if let Some(i) = copy_index(table) {
            self.copy_pending[i] = false;
        }
        self.tables.remove(table);
    }

    /// Whether a table exists (has at least one binding).
    pub fn has_table(&self, table: &str) -> bool {
        self.tables.contains_key(table) || copy_index(table).is_some_and(|i| self.copy_pending[i])
    }

    /// Table names in order.
    pub fn table_names(&self) -> impl Iterator<Item = &str> {
        let mut names: Vec<&str> = self.tables.keys().map(|s| s.as_str()).collect();
        for (i, n) in [COPY_MODE_VI_TABLE, COPY_MODE_TABLE].into_iter().enumerate() {
            if self.copy_pending[i] {
                names.push(n);
            }
        }
        names.sort_unstable();
        names.into_iter()
    }

    /// Bindings of a table in tmux's `list-keys` order.
    pub fn bindings(&self, table: &str) -> Vec<&Binding> {
        let mut v: Vec<&Binding> = self.table(table).map(|t| t.values().collect()).unwrap_or_default();
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
            // Releases and lone modifiers (Shift on the way to `%`) belong to
            // the key that follows the prefix, so they neither reach the pane
            // nor end the prefix.
            if (ev.action == KeyAction::Release || matches!(ev.key, Key::Modifier(_))) && self.in_prefix() {
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
            "bind-key    -T prefix T       command-prompt -I \"#T\" { select-pane -T \"%%\" }",
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
    fn copy_mode_defaults_are_parsed_on_first_use() {
        let mut b = KeyBindings::with_defaults();
        assert!(b.copy_defaults.get().is_none(), "not parsed at start-up");
        let names: Vec<&str> = b.table_names().collect();
        assert_eq!(names, ["copy-mode", "copy-mode-vi", "prefix", "root"]);
        assert!(b.has_table(COPY_MODE_VI_TABLE));
        let q = KeyCode::parse("q").unwrap();
        assert_eq!(run(&KeyDecision::Run(b.lookup_code(COPY_MODE_VI_TABLE, &q).unwrap().commands.clone())), "send-keys -X cancel");
        // A change makes the table an ordinary one, defaults kept.
        let g = b.generation();
        b.bind(COPY_MODE_VI_TABLE, Binding { key: KeyCode::parse("y").unwrap(), repeat: false, note: None, commands: parse_string("send -X copy-selection").unwrap() });
        assert!(b.generation() > g);
        assert!(b.lookup_code(COPY_MODE_VI_TABLE, &q).is_some());
        assert_eq!(b.bindings(COPY_MODE_VI_TABLE).len(), COPY_MODE_VI_DEFAULTS.len() + 1, "y is new");
        assert!(b.unbind(COPY_MODE_TABLE, &q));
        assert!(b.lookup_code(COPY_MODE_TABLE, &q).is_none());
        assert_eq!(b.bindings(COPY_MODE_TABLE).len(), COPY_MODE_DEFAULTS.len() - 1);
        b.unbind_all(COPY_MODE_TABLE);
        assert!(!b.has_table(COPY_MODE_TABLE));
        assert!(b.lookup_code(COPY_MODE_TABLE, &q).is_none());
        // An empty set of bindings has no copy tables.
        assert!(!KeyBindings::new().has_table(COPY_MODE_TABLE));
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
        // The mouse keys' defaults keep the root table.
        assert!(b.lookup(ROOT_TABLE, &ev("M-h")).is_none());
        assert!(b.has_table(ROOT_TABLE));
        assert!(b.lookup(PREFIX_TABLE, &ev("c")).is_some());
    }

    #[test]
    fn media_and_modifier_keys() {
        use lif_types::{MediaKey, ModifierKey};
        let b = KeyBindings::with_defaults();
        let prefix = [KeyCode::parse("C-b").unwrap()];
        let media = KeyEvent::new(Key::Media(MediaKey::PlayPause), Mods::NONE);
        let shift = KeyEvent::new(Key::Modifier(ModifierKey::LeftShift), Mods::SHIFT);
        assert_eq!(KeyCode::from_event(&media), None);
        assert_eq!(KeyCode::from_event(&shift), None);
        assert!(b.lookup(ROOT_TABLE, &media).is_none());
        let mut s = KeyState::default();
        assert_eq!(s.handle(&b, &prefix, 500, &media, 0), KeyDecision::SendToPane);
        assert_eq!(s.handle(&b, &prefix, 500, &shift, 0), KeyDecision::SendToPane);
        // Prefix, Shift, %: the lone Shift neither reaches the pane nor ends
        // the prefix.
        assert_eq!(s.handle(&b, &prefix, 500, &ev("C-b"), 0), KeyDecision::Swallow);
        assert_eq!(s.handle(&b, &prefix, 500, &shift, 0), KeyDecision::Swallow);
        assert!(s.in_prefix());
        assert_eq!(run(&s.handle(&b, &prefix, 500, &ev("%"), 0)), "split-window -h");
        // A hand-built code still has a name.
        assert_eq!(KeyCode::new(Key::Media(MediaKey::PlayPause), Mods::NONE).to_string(), "PlayPause");
    }
}
