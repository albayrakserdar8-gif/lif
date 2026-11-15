//! Options: the subset of tmux's options the model uses, with tmux's names,
//! scopes, defaults and error messages, plus user options (`@name`).
//!
//! Like tmux there are three scopes. Server options have one value. Session
//! options have a global value (`set -g`) that a session can override (`set`
//! with a session target). Window options likewise have a global value and
//! per-window overrides. `set-option` works out the scope from the name.

use crate::keys::KeyCode;
use crate::layout::SizeSpec;
use std::collections::BTreeMap;

/// Which table an option lives in.
#[derive(Clone, Copy, Debug, PartialEq, Eq, Hash)]
pub enum Scope {
    /// One value for the server (`set -s`).
    Server,
    /// Global value plus per-session overrides.
    Session,
    /// Global value plus per-window overrides.
    Window,
}

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub(crate) enum Kind {
    Flag,
    Number(i64, i64),
    Str,
    /// A string that must parse as a layout size (`80` or `50%`).
    SizeStr,
    /// A string of the form `WxH`.
    DimStr,
    Key,
    Choice(&'static [&'static str]),
}

/// One option's definition.
#[derive(Clone, Copy, Debug)]
pub(crate) struct Def {
    pub name: &'static str,
    pub scope: Scope,
    pub kind: Kind,
    pub default: &'static str,
}

const fn d(name: &'static str, scope: Scope, kind: Kind, default: &'static str) -> Def {
    Def { name, scope, kind, default }
}

const MAXI: i64 = i32::MAX as i64;
use Kind::*;
use Scope::*;

/// Every option lif knows, sorted by name.
///
/// Notes on lif-specific meanings:
/// - `escape-time` (milliseconds) is how long a lone Escape from a legacy
///   terminal waits for the rest of a key sequence (Alt+key, arrows) in the
///   same read; 0 never waits (then an Alt+key split across reads arrives
///   as Escape and the key, as in tmux). Terminals speaking the kitty
///   keyboard protocol, which lif turns on when available, send Escape
///   unambiguously and never wait.
/// - Scrollback has two limits and both hold: `history-limit` in lines
///   (tmux's) and `scrollback-bytes`, a memory budget. At 200 columns a
///   line of history costs about 1.7 KB until the pane is quiet, when it is
///   compressed (to about 50-400 bytes); the default 4000 lines keep twice
///   tmux's history in about tmux's memory. Changing either applies to
///   existing panes too.
/// - `tmux-compat` (lif's own) makes panes started from then on look like
///   tmux panes to programs that only know tmux: `TMUX` and `TMUX_PANE`
///   are set (pointing at lif's socket and pane), and `lif` invoked as
///   `tmux` serves them. Off by default because programs that see `TMUX`
///   change behavior (passthrough wrapping, fewer colors).
/// - Window options can also be set per pane (`set -p`), as in tmux; the
///   pane value wins over the window's.
/// - `focus-events` is on by default (tmux: off): a program that asks
///   for focus reports gets them without configuration.
/// - The popup and menu styles default to the terminal's colors and a
///   yellow selected line; tmux's defaults are theme colors, which lif
///   does not have.
/// - `destroy-unattached` takes tmux's `keep-last` and `keep-group`,
///   which are about session groups: without groups they keep every
///   session, as in tmux.
/// - `mouse-drag-end` (lif's own) is what tmux's `MouseDragEnd1Pane`
///   binding sets, since lif's mouse actions are built in rather than
///   bound: copy and leave copy mode (tmux's default), copy and stay with
///   the selection shown, or only select. It applies to double- and
///   triple-click selections too.
pub(crate) static DEFS: &[Def] = &[
    d("activity-action", Session, Choice(&["none", "any", "current", "other"]), "other"),
    d("aggressive-resize", Window, Flag, "off"),
    d("allow-passthrough", Window, Choice(&["off", "on", "all"]), "off"),
    d("automatic-rename", Window, Flag, "on"),
    d("automatic-rename-format", Window, Str, "#{?pane_in_mode,[lif],#{pane_current_command}}#{?pane_dead,[dead],}"),
    d("base-index", Session, Number(0, MAXI), "0"),
    d("bell-action", Session, Choice(&["none", "any", "current", "other"]), "any"),
    d("buffer-limit", Server, Number(1, MAXI), "50"),
    d("clock-mode-colour", Window, Str, "blue"),
    d("clock-mode-style", Window, Choice(&["12", "24"]), "24"),
    d("copy-command", Server, Str, ""),
    d("copy-mode-current-match-style", Window, Str, "bg=magenta,fg=black"),
    d("copy-mode-match-style", Window, Str, "bg=cyan,fg=black"),
    d("default-command", Session, Str, ""),
    d("default-shell", Session, Str, "/bin/sh"),
    d("default-size", Session, DimStr, "80x24"),
    d("destroy-unattached", Session, Choice(&["off", "on", "keep-last", "keep-group"]), "off"),
    d("detach-on-destroy", Session, Choice(&["off", "on", "no-detached", "previous", "next"]), "on"),
    d("display-panes-time", Window, Number(1, MAXI), "1000"),
    d("display-time", Session, Number(0, MAXI), "750"),
    d("escape-time", Server, Number(0, MAXI), "5"),
    d("exit-empty", Server, Flag, "on"),
    d("focus-events", Server, Flag, "on"),
    d("history-limit", Session, Number(0, MAXI), "4000"),
    d("main-pane-height", Window, SizeStr, "24"),
    d("main-pane-width", Window, SizeStr, "80"),
    d("menu-border-lines", Window, Choice(&["single", "rounded", "double", "heavy", "simple", "padded", "none"]), "single"),
    d("menu-border-style", Window, Str, "default"),
    d("menu-selected-style", Window, Str, "bg=yellow,fg=black"),
    d("menu-style", Window, Str, "default"),
    d("message-style", Session, Str, "bg=yellow,fg=black"),
    d("mode-keys", Window, Choice(&["emacs", "vi"]), "emacs"),
    d("mode-style", Window, Str, "bg=yellow,fg=black"),
    d("monitor-activity", Window, Flag, "off"),
    d("monitor-bell", Window, Flag, "on"),
    d("monitor-silence", Window, Number(0, MAXI), "0"),
    d("mouse", Session, Flag, "on"),
    d("mouse-drag-end", Session, Choice(&["copy-pipe-and-cancel", "copy-pipe-no-clear", "none"]), "copy-pipe-and-cancel"),
    d("other-pane-height", Window, SizeStr, "0"),
    d("other-pane-width", Window, SizeStr, "0"),
    d("pane-active-border-style", Window, Str, "fg=green"),
    d("pane-base-index", Window, Number(0, u16::MAX as i64), "0"),
    d("pane-border-format", Window, Str, "#{?pane_active,#[reverse],}#{pane_index}#[default] \"#{pane_title}\""),
    d("pane-border-status", Window, Choice(&["off", "top", "bottom"]), "off"),
    d("pane-border-style", Window, Str, "default"),
    d("popup-border-lines", Window, Choice(&["single", "rounded", "double", "heavy", "simple", "padded", "none"]), "single"),
    d("popup-border-style", Window, Str, "default"),
    d("popup-style", Window, Str, "default"),
    d("prefix", Session, Key, "C-b"),
    d("prefix2", Session, Key, "None"),
    d("remain-on-exit", Window, Choice(&["off", "on", "failed"]), "off"),
    d("renumber-windows", Session, Flag, "off"),
    d("repeat-time", Session, Number(0, i16::MAX as i64), "500"),
    d("scrollback-bytes", Session, Number(0, i64::MAX), "10485760"),
    d("set-clipboard", Server, Choice(&["off", "external", "on"]), "external"),
    d("set-titles", Session, Flag, "off"),
    d("set-titles-string", Session, Str, "#S:#I:#W - \"#T\""),
    d("silence-action", Session, Choice(&["none", "any", "current", "other"]), "other"),
    d("status", Session, Choice(&["off", "on"]), "on"),
    d("status-interval", Session, Number(0, MAXI), "15"),
    d("status-justify", Session, Choice(&["left", "centre", "right", "absolute-centre"]), "left"),
    d("status-left", Session, Str, "[#{session_name}] "),
    d("status-left-length", Session, Number(0, i16::MAX as i64), "10"),
    d("status-left-style", Session, Str, "default"),
    d("status-position", Session, Choice(&["top", "bottom"]), "bottom"),
    d(
        "status-right",
        Session,
        Str,
        "#{?window_bigger,[#{window_offset_x}#,#{window_offset_y}] ,}\"#{=21:pane_title}\" %H:%M %d-%b-%y",
    ),
    d("status-right-length", Session, Number(0, i16::MAX as i64), "40"),
    d("status-right-style", Session, Str, "default"),
    d("status-style", Session, Str, "bg=green,fg=black"),
    d("synchronize-panes", Window, Flag, "off"),
    d("tiled-layout-max-columns", Window, Number(0, MAXI), "0"),
    d("tmux-compat", Server, Flag, "off"),
    d("visual-activity", Session, Choice(&["off", "on", "both"]), "off"),
    d("visual-bell", Session, Choice(&["off", "on", "both"]), "off"),
    d("visual-silence", Session, Choice(&["off", "on", "both"]), "off"),
    d("window-active-style", Window, Str, "default"),
    d("window-size", Window, Choice(&["largest", "smallest", "manual", "latest"]), "latest"),
    d("window-status-activity-style", Window, Str, "reverse"),
    d("window-status-bell-style", Window, Str, "reverse"),
    d("window-status-current-format", Window, Str, "#I:#W#{?window_flags,#{window_flags}, }"),
    d("window-status-current-style", Window, Str, "underscore"),
    d("window-status-format", Window, Str, "#I:#W#{?window_flags,#{window_flags}, }"),
    d("window-status-last-style", Window, Str, "default"),
    d("window-status-separator", Window, Str, " "),
    d("window-status-style", Window, Str, "default"),
    d("window-style", Window, Str, "default"),
    d("word-separators", Session, Str, "!\"#$%&'()*+,-./:;<=>?@[\\]^`{|}~"),
    d("wrap-search", Window, Flag, "on"),
];

/// A typed option value.
#[derive(Clone, Debug, PartialEq, Eq)]
pub enum Value {
    /// on / off.
    Flag(bool),
    /// An integer.
    Number(i64),
    /// A string (formats, styles, sizes).
    Str(String),
    /// `None` is tmux's "None" (no key).
    Key(Option<KeyCode>),
    /// Index into the option's list of choices.
    Choice(usize),
}

impl Value {
    pub(crate) fn as_num(&self) -> i64 {
        match self {
            Value::Number(n) => *n,
            Value::Flag(b) => *b as i64,
            Value::Choice(c) => *c as i64,
            _ => 0,
        }
    }

    pub(crate) fn as_flag(&self) -> bool {
        self.as_num() != 0
    }

    pub(crate) fn as_str(&self) -> &str {
        match self {
            Value::Str(s) => s,
            _ => "",
        }
    }
}

/// Look an option up by exact name or unique prefix, with tmux's errors.
pub(crate) fn find(name: &str) -> Result<&'static Def, String> {
    if let Some(def) = DEFS.iter().find(|d| d.name == name) {
        return Ok(def);
    }
    let mut found = None;
    for def in DEFS {
        if !name.is_empty() && def.name.starts_with(name) {
            if found.is_some() {
                return Err(format!("ambiguous option: {name}"));
            }
            found = Some(def);
        }
    }
    found.ok_or_else(|| format!("invalid option: {name}"))
}

/// Parse `text` (or toggle / take the default when it is `None`) for `def`.
/// `append` appends to the current string value (`set -a`).
pub(crate) fn parse_value(def: &Def, current: &Value, text: Option<&str>, append: bool) -> Result<Value, String> {
    match def.kind {
        Flag => match text {
            None => Ok(Value::Flag(!current.as_flag())),
            Some(t) => match t.to_ascii_lowercase().as_str() {
                "on" | "yes" | "1" => Ok(Value::Flag(true)),
                "off" | "no" | "0" => Ok(Value::Flag(false)),
                _ => Err(format!("bad value: {t}")),
            },
        },
        Number(min, max) => {
            let t = text.ok_or_else(|| "empty value".to_string())?;
            let n: i64 = t.parse().map_err(|_| format!("value is invalid: {t}"))?;
            if n < min {
                Err(format!("value is too small: {t}"))
            } else if n > max {
                Err(format!("value is too large: {t}"))
            } else {
                Ok(Value::Number(n))
            }
        }
        Str | SizeStr | DimStr => {
            let t = text.unwrap_or("");
            let s = if append { format!("{}{t}", current.as_str()) } else { t.to_string() };
            match def.kind {
                SizeStr if SizeSpec::parse(&s).is_none() => Err(format!("invalid value: {s}")),
                DimStr if parse_dim(&s).is_none() => Err(format!("invalid value: {s}")),
                _ => Ok(Value::Str(s)),
            }
        }
        Key => {
            let t = text.ok_or_else(|| "empty value".to_string())?;
            if t.eq_ignore_ascii_case("none") {
                return Ok(Value::Key(None));
            }
            KeyCode::parse(t).map(|k| Value::Key(Some(k))).ok_or_else(|| format!("bad key: {t}"))
        }
        Choice(names) => match text {
            None if names.len() == 2 && names[0] == "off" => Ok(Value::Choice(1 - current.as_num().clamp(0, 1) as usize)),
            None => Err("empty value".to_string()),
            Some(t) => {
                if let Some(i) = names.iter().position(|n| *n == t) {
                    return Ok(Value::Choice(i));
                }
                if names[0] == "off" {
                    match t {
                        "no" | "0" => return Ok(Value::Choice(0)),
                        "yes" | "1" => return Ok(Value::Choice(1)),
                        _ => {}
                    }
                }
                Err(format!("unknown value: {t}"))
            }
        },
    }
}

/// Parse `"80x24"`.
pub(crate) fn parse_dim(s: &str) -> Option<(u16, u16)> {
    let (w, h) = s.split_once('x')?;
    let (w, h) = (w.parse::<u16>().ok()?, h.parse::<u16>().ok()?);
    (w > 0 && h > 0 && crate::model::size_allowed(w, h)).then_some((w, h))
}

/// Text form of a value, unescaped (`show-options -v`).
pub(crate) fn value_text(def: &Def, v: &Value) -> String {
    match (def.kind, v) {
        (_, Value::Flag(b)) => if *b { "on" } else { "off" }.to_string(),
        (_, Value::Number(n)) => n.to_string(),
        (_, Value::Str(s)) => s.clone(),
        (_, Value::Key(None)) => "None".to_string(),
        (_, Value::Key(Some(k))) => k.to_string(),
        (Choice(names), Value::Choice(i)) => names.get(*i).copied().unwrap_or("").to_string(),
        (_, Value::Choice(i)) => i.to_string(),
    }
}

/// Option values at one level (a global table, or one session's or
/// window's overrides).
#[derive(Clone, Debug, Default, PartialEq, Eq)]
pub struct OptionSet {
    values: BTreeMap<&'static str, Value>,
    user: BTreeMap<String, String>,
}

impl OptionSet {
    /// Every option of `scope` at its default value.
    pub(crate) fn defaults(scope: Scope) -> OptionSet {
        let mut set = OptionSet::default();
        for def in DEFS.iter().filter(|d| d.scope == scope) {
            let v = parse_value(def, &Value::Flag(false), Some(def.default), false)
                .unwrap_or_else(|e| panic!("bad default for {}: {e}", def.name));
            set.values.insert(def.name, v);
        }
        set
    }

    /// The value of a built-in option, if set at this level.
    pub fn get(&self, name: &str) -> Option<&Value> {
        self.values.get(name)
    }

    /// The value of a user option (`@name`), if set at this level.
    pub fn user(&self, name: &str) -> Option<&str> {
        self.user.get(name).map(|s| s.as_str())
    }

    pub(crate) fn set(&mut self, name: &'static str, v: Value) {
        self.values.insert(name, v);
    }

    pub(crate) fn unset(&mut self, name: &str) -> bool {
        self.values.remove(name).is_some()
    }

    pub(crate) fn set_user(&mut self, name: &str, v: String) {
        self.user.insert(name.to_string(), v);
    }

    pub(crate) fn unset_user(&mut self, name: &str) -> bool {
        self.user.remove(name).is_some()
    }

    pub(crate) fn iter(&self) -> impl Iterator<Item = (&'static str, &Value)> {
        self.values.iter().map(|(k, v)| (*k, v))
    }

    pub(crate) fn user_iter(&self) -> impl Iterator<Item = (&str, &str)> {
        self.user.iter().map(|(k, v)| (k.as_str(), v.as_str()))
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn table_is_sorted_and_defaults_parse() {
        for w in DEFS.windows(2) {
            assert!(w[0].name < w[1].name, "{} / {}", w[0].name, w[1].name);
        }
        for scope in [Scope::Server, Scope::Session, Scope::Window] {
            OptionSet::defaults(scope);
        }
    }

    #[test]
    fn lookup_and_errors_match_tmux() {
        // tmux 3.8: "set -g foo bar" -> "invalid option: foo".
        assert_eq!(find("foo").unwrap_err(), "invalid option: foo");
        assert_eq!(find("status-l").unwrap_err(), "ambiguous option: status-l");
        assert_eq!(find("renumber").unwrap().name, "renumber-windows");
        assert_eq!(find("status").unwrap().name, "status");
        let def = find("base-index").unwrap();
        let cur = Value::Number(0);
        assert_eq!(parse_value(def, &cur, Some("x"), false).unwrap_err(), "value is invalid: x");
        assert_eq!(parse_value(def, &cur, Some("-1"), false).unwrap_err(), "value is too small: -1");
        let def = find("mouse").unwrap();
        assert_eq!(parse_value(def, &Value::Flag(true), None, false), Ok(Value::Flag(false)));
        assert_eq!(parse_value(def, &Value::Flag(true), Some("maybe"), false).unwrap_err(), "bad value: maybe");
        let def = find("status-position").unwrap();
        assert_eq!(parse_value(def, &Value::Choice(1), Some("up"), false).unwrap_err(), "unknown value: up");
        let def = find("status").unwrap();
        assert_eq!(parse_value(def, &Value::Choice(1), None, false), Ok(Value::Choice(0)));
        let def = find("prefix").unwrap();
        assert_eq!(parse_value(def, &Value::Key(None), Some("C-a"), false), Ok(Value::Key(KeyCode::parse("C-a"))));
        assert_eq!(parse_value(def, &Value::Key(None), Some("C-"), false).unwrap_err(), "bad key: C-");
        let def = find("status-left").unwrap();
        assert_eq!(parse_value(def, &Value::Str("a".into()), Some("b"), true), Ok(Value::Str("ab".into())));
        let def = find("main-pane-width").unwrap();
        assert!(parse_value(def, &Value::Str("80".into()), Some("50%"), false).is_ok());
        assert!(parse_value(def, &Value::Str("80".into()), Some("wide"), false).is_err());
    }
}
