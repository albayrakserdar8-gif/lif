//! The file format: which keys exist, what values they take, and the lif
//! commands they become.

use crate::check::Scratch;
use crate::style;
use crate::suggest::{did_you_mean, suggest};
use crate::{Diagnostic, Entry, Severity};
use lif_core::cmd::supported_entries;
use lif_core::{KeyCode, escape_arg, parse_string};
use std::borrow::Cow;
use std::ops::Range;
use toml::Spanned;
use toml::de::{DeTable, DeValue};

type V<'i> = Spanned<DeValue<'i>>;
type K<'i> = Spanned<Cow<'i, str>>;

/// What a setting accepts.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub(crate) enum Kind {
    /// `true` / `false` (tmux on / off).
    Bool,
    /// An integer.
    Int,
    /// Any string (formats, commands, paths).
    Str,
    /// A key name, or `false` / `"none"` for no key.
    Key,
    /// One of these strings; `true` / `false` too when the list has on and off.
    Choice(&'static [&'static str]),
    /// A pane size: cells (integer) or `"50%"`.
    Size,
    /// `"80x24"`.
    Dim,
    /// Bytes: an integer or a size with a unit (`"32MB"`).
    Bytes,
    /// A style table or tmux style string.
    Style,
}

/// One setting of the file and the lif option it sets.
#[derive(Debug)]
pub(crate) struct Opt {
    pub key: &'static str,
    pub option: &'static str,
    pub kind: Kind,
    pub server: bool,
}

const fn o(key: &'static str, option: &'static str, kind: Kind) -> Opt {
    Opt { key, option, kind, server: false }
}

const fn srv(key: &'static str, kind: Kind) -> Opt {
    Opt { key, option: key, kind, server: true }
}

use Kind::*;

const OFF_ON: &[&str] = &["off", "on"];

/// Top-level settings: every lif-core option except the status line ones
/// (`[status]`) and styles (`[theme]`). Named as in tmux, except
/// `scrollback`, which takes human sizes.
pub(crate) static TOP: &[Opt] = &[
    o("automatic-rename", "automatic-rename", Bool),
    o("automatic-rename-format", "automatic-rename-format", Str),
    o("base-index", "base-index", Int),
    o("bell-action", "bell-action", Choice(&["none", "any", "current", "other"])),
    srv("buffer-limit", Int),
    o("default-command", "default-command", Str),
    o("default-shell", "default-shell", Str),
    o("default-size", "default-size", Dim),
    o("detach-on-destroy", "detach-on-destroy", Choice(&["off", "on", "no-detached", "previous", "next"])),
    o("display-panes-time", "display-panes-time", Int),
    o("display-time", "display-time", Int),
    srv("escape-time", Int),
    srv("exit-empty", Bool),
    o("history-limit", "history-limit", Int),
    o("main-pane-height", "main-pane-height", Size),
    o("main-pane-width", "main-pane-width", Size),
    o("mode-keys", "mode-keys", Choice(&["emacs", "vi"])),
    o("monitor-activity", "monitor-activity", Bool),
    o("monitor-bell", "monitor-bell", Bool),
    o("mouse", "mouse", Bool),
    o("other-pane-height", "other-pane-height", Size),
    o("other-pane-width", "other-pane-width", Size),
    o("pane-base-index", "pane-base-index", Int),
    o("prefix", "prefix", Key),
    o("prefix2", "prefix2", Key),
    o("remain-on-exit", "remain-on-exit", Choice(&["off", "on", "failed"])),
    o("renumber-windows", "renumber-windows", Bool),
    o("repeat-time", "repeat-time", Int),
    o("scrollback", "scrollback-bytes", Bytes),
    srv("set-clipboard", Choice(&["off", "external", "on"])),
    o("tiled-layout-max-columns", "tiled-layout-max-columns", Int),
    o("visual-bell", "visual-bell", Choice(&["off", "on", "both"])),
    o("window-size", "window-size", Choice(&["largest", "smallest", "manual", "latest"])),
];

/// `[status]`: the status line.
pub(crate) static STATUS: &[Opt] = &[
    o("enabled", "status", Choice(OFF_ON)),
    o("position", "status-position", Choice(&["top", "bottom"])),
    o("justify", "status-justify", Choice(&["left", "centre", "right", "absolute-centre"])),
    o("interval", "status-interval", Int),
    o("left", "status-left", Str),
    o("left-length", "status-left-length", Int),
    o("right", "status-right", Str),
    o("right-length", "status-right-length", Int),
    o("window-format", "window-status-format", Str),
    o("current-window-format", "window-status-current-format", Str),
    o("separator", "window-status-separator", Str),
];

/// `[theme]`: styles.
pub(crate) static THEME: &[Opt] = &[
    o("status", "status-style", Style),
    o("status-left", "status-left-style", Style),
    o("status-right", "status-right-style", Style),
    o("window", "window-status-style", Style),
    o("current-window", "window-status-current-style", Style),
    o("last-window", "window-status-last-style", Style),
    o("activity", "window-status-activity-style", Style),
    o("bell", "window-status-bell-style", Style),
    o("border", "pane-border-style", Style),
    o("active-border", "pane-active-border-style", Style),
    o("selection", "mode-style", Style),
    o("message", "message-style", Style),
];

/// Key tables a `[keys.NAME]` section may name.
pub(crate) const TABLES: &[&str] = &["prefix", "root", "copy-mode", "copy-mode-vi"];

const SECTIONS: &[&str] = &["keys", "status", "theme"];

/// Named keys, for suggestions on misspelled key names.
const KEY_NAMES: &[&str] = &[
    "Enter", "Escape", "Space", "Tab", "BTab", "BSpace", "Up", "Down", "Left", "Right", "Home", "End", "PPage", "NPage",
    "PageUp", "PageDown", "IC", "DC", "Insert", "Delete", "F1", "F2", "F3", "F4", "F5", "F6", "F7", "F8", "F9", "F10", "F11",
    "F12", "KPEnter",
];

/// Where a tmux option lives in the file when it is not a top-level key.
pub(crate) fn location(option: &str) -> Option<(&'static str, &'static Opt)> {
    for (section, table) in [("", TOP), ("status", STATUS), ("theme", THEME)] {
        if let Some(opt) = table.iter().find(|o| o.option == option) {
            return Some((section, opt));
        }
    }
    None
}

struct Item {
    phase: u8,
    pos: usize,
    cmd: String,
    what: String,
}

struct Tr<'t> {
    lines: Lines<'t>,
    items: Vec<Item>,
    diags: Vec<(usize, Severity, String)>,
    scratch: Scratch,
    damaged: Vec<u32>,
}

/// Byte offsets to 1-based line and column.
pub(crate) struct Lines<'t> {
    text: &'t str,
    starts: Vec<usize>,
}

impl<'t> Lines<'t> {
    pub(crate) fn new(text: &'t str) -> Self {
        let mut starts = vec![0];
        starts.extend(text.match_indices('\n').map(|(i, _)| i + 1));
        Lines { text, starts }
    }

    pub(crate) fn pos(&self, off: usize) -> (u32, u32) {
        let mut off = off.min(self.text.len());
        while !self.text.is_char_boundary(off) {
            off -= 1;
        }
        let line = self.starts.partition_point(|&s| s <= off) - 1;
        let col = self.text[self.starts[line]..off].chars().count() + 1;
        (line as u32 + 1, col as u32)
    }
}

/// Quote a word for the command language so that it reads back exactly.
/// lif-core's `escape_arg` single-quotes strings whose only special
/// character is `"` but still backslash-escapes control characters and
/// backslashes inside the quotes, where the lexer reads them literally; such
/// strings get a double-quoted form instead.
pub(crate) fn quote(s: &str) -> String {
    let q = escape_arg(s);
    let back = parse_string(&format!("set-option -g status-left {q}"));
    if back.ok().and_then(|l| l.commands().first().and_then(|c| c.args().value(1).map(|v| v == s))) == Some(true) {
        return q;
    }
    let mut out = String::with_capacity(s.len() + 2);
    out.push('"');
    for c in s.chars() {
        match c {
            '\\' => out.push_str("\\\\"),
            '"' => out.push_str("\\\""),
            '$' => out.push_str("\\$"),
            '\n' => out.push_str("\\n"),
            '\t' => out.push_str("\\t"),
            '\r' => out.push_str("\\r"),
            c if c.is_control() => out.push_str(&format!("\\u{:04x}", c as u32)),
            c => out.push(c),
        }
    }
    out.push('"');
    out
}

/// Translate a file into commands and diagnostics.
pub(crate) fn translate(text: &str) -> (Vec<Entry>, Vec<Diagnostic>) {
    let (root, errors) = DeTable::parse_recoverable(text);
    let mut tr = Tr { lines: Lines::new(text), items: Vec::new(), diags: Vec::new(), scratch: Scratch::new(), damaged: Vec::new() };
    for e in &errors {
        let at = e.span().map(|s| s.start).unwrap_or(0);
        tr.damaged.push(tr.lines.pos(at).0);
        tr.diags.push((at, Severity::Error, format!("invalid TOML: {}", e.message())));
    }
    tr.root(root.get_ref());
    let Tr { lines, mut items, mut diags, mut scratch, .. } = tr;
    items.sort_by_key(|i| (i.phase, i.pos));
    let mut entries = Vec::with_capacity(items.len());
    for it in items {
        match scratch.run(&it.cmd) {
            Ok(()) => {
                let (line, column) = lines.pos(it.pos);
                entries.push(Entry { command: it.cmd, line, column });
            }
            Err(e) => diags.push((it.pos, Severity::Error, format!("{}: {e}", it.what))),
        }
    }
    diags.sort_by_key(|d| d.0);
    let diagnostics = diags
        .into_iter()
        .map(|(at, severity, message)| {
            let (line, column) = lines.pos(at);
            Diagnostic { severity, line, column, message }
        })
        .collect();
    (entries, diagnostics)
}

/// Table entries in file order.
fn entries<'a, 'i>(t: &'a DeTable<'i>) -> Vec<(&'a K<'i>, &'a V<'i>)> {
    let mut v: Vec<_> = t.iter().collect();
    v.sort_by_key(|(k, _)| k.span().start);
    v
}

/// A value's type for messages: `string "on"`, `integer 3`, `table`.
fn describe(v: &DeValue<'_>) -> String {
    match v {
        DeValue::String(s) => format!("string {:?}", s.as_ref()),
        DeValue::Integer(i) => format!("integer {}", i.as_str()),
        DeValue::Boolean(b) => format!("boolean {b}"),
        other => other.type_str().to_string(),
    }
}

fn int(v: &DeValue<'_>) -> Option<Result<i64, String>> {
    match v {
        DeValue::Integer(i) => Some(i64::from_str_radix(i.as_str(), i.radix()).map_err(|_| "integer out of range".to_string())),
        _ => None,
    }
}

/// `"32MB"`, `"512 KiB"`, `"1g"`, `"4096"` in bytes. Units are powers of 1024.
pub(crate) fn parse_size(s: &str) -> Option<i64> {
    let s = s.trim();
    let split = s.find(|c: char| !c.is_ascii_digit()).unwrap_or(s.len());
    let (num, unit) = s.split_at(split);
    let n: i64 = num.parse().ok()?;
    let mult: i64 = match unit.trim().to_ascii_lowercase().as_str() {
        "" | "b" => 1,
        "k" | "kb" | "kib" => 1 << 10,
        "m" | "mb" | "mib" => 1 << 20,
        "g" | "gb" | "gib" => 1 << 30,
        _ => return None,
    };
    n.checked_mul(mult)
}

fn validate_command(s: &str) -> Result<(), String> {
    match parse_string(s) {
        Ok(list) if list.is_empty() => Err("empty command".to_string()),
        Ok(_) => Ok(()),
        Err(e) => {
            let mut msg = e.0;
            if let Some(name) = msg.strip_prefix("unknown command: ") {
                let hint = did_you_mean(name, supported_entries().map(|e| e.name), ("`", "`"));
                msg = format!("unknown command `{name}`{hint}");
            }
            Err(msg)
        }
    }
}

/// Why a key name does not parse, with a suggestion.
fn bad_key_message(name: &str) -> String {
    if name.is_empty() {
        return "empty key name".to_string();
    }
    let lower = name.to_ascii_lowercase();
    for (long, short) in [("ctrl-", "C-"), ("control-", "C-"), ("alt-", "M-"), ("meta-", "M-"), ("shift-", "S-")] {
        if lower.starts_with(long) {
            return format!("unknown key name `{name}`: write modifiers as C- (Ctrl), M- (Alt/Meta) and S- (Shift), e.g. `{short}{}`", &name[long.len()..]);
        }
    }
    let mut rest = name;
    while rest.len() >= 3 && rest.as_bytes()[1] == b'-' && matches!(rest.as_bytes()[0], b'C' | b'c' | b'M' | b'm' | b'S' | b's') {
        rest = &rest[2..];
    }
    let mods = &name[..name.len() - rest.len()];
    match suggest(rest, KEY_NAMES.iter().copied()) {
        Some(s) => format!("unknown key name `{name}`, did you mean `{mods}{s}`?"),
        None => format!("unknown key name `{name}` (expected tmux key names such as `C-a`, `M-Left`, `F5`, `Enter` or a single character)"),
    }
}

enum Bind {
    Unbind,
    Run { cmd: String, repeat: bool, note: Option<String> },
}

impl Tr<'_> {
    fn err(&mut self, at: usize, msg: String) {
        self.diags.push((at, Severity::Error, msg));
    }

    /// Whether a value sits on a line where TOML recovery happened; such
    /// values may be truncated, so they are not applied.
    fn is_damaged(&self, span: Range<usize>) -> bool {
        if self.damaged.is_empty() {
            return false;
        }
        let (a, b) = (self.lines.pos(span.start).0, self.lines.pos(span.end).0);
        self.damaged.iter().any(|l| (a..=b).contains(l))
    }

    fn push(&mut self, phase: u8, pos: usize, cmd: String, what: &str) {
        self.items.push(Item { phase, pos, cmd, what: what.to_string() });
    }

    fn root(&mut self, root: &DeTable<'_>) {
        for (k, v) in entries(root) {
            let name = k.get_ref().as_ref();
            let at = k.span().start;
            match (name, v.get_ref()) {
                ("keys", DeValue::Table(t)) => self.keys(t),
                ("keys", other) => self.err(at, format!("`keys` must be a table of key bindings ([keys]), found {}", describe(other))),
                ("status", DeValue::Table(t)) => self.section(t, STATUS, "status"),
                ("status", DeValue::Boolean(_)) => self.option(&STATUS[0], "status", k, v),
                ("status", other) => {
                    self.err(at, format!("`status` must be true, false or a [status] table, found {}", describe(other)))
                }
                ("theme", DeValue::Table(t)) => self.section(t, THEME, "theme"),
                ("theme", other) => self.err(at, format!("`theme` must be a table of styles ([theme]), found {}", describe(other))),
                _ => match TOP.iter().find(|o| o.key == name) {
                    Some(opt) => self.option(opt, name, k, v),
                    None => {
                        let msg = self.unknown_top(name, v.get_ref());
                        self.err(at, msg)
                    }
                },
            }
        }
    }

    fn unknown_top(&self, name: &str, v: &DeValue<'_>) -> String {
        if let Some((section, opt)) = location(name)
            && !section.is_empty()
        {
            return format!("`{name}` is set as `{}` in the [{section}] section", opt.key);
        }
        if name == "scrollback-bytes" {
            return "`scrollback-bytes` is set as `scrollback` (bytes, or a size such as \"32MB\")".to_string();
        }
        if v.is_table() {
            let hint = did_you_mean(name, SECTIONS.iter().copied(), ("[", "]"));
            return format!("unknown section [{name}]{hint}");
        }
        let hint = did_you_mean(name, TOP.iter().map(|o| o.key).chain(SECTIONS.iter().copied()), ("`", "`"));
        format!("unknown option `{name}`{hint}")
    }

    fn section(&mut self, t: &DeTable<'_>, table: &'static [Opt], section: &str) {
        for (k, v) in entries(t) {
            let name = k.get_ref().as_ref();
            match table.iter().find(|o| o.key == name) {
                Some(opt) => self.option(opt, &format!("{section}.{name}"), k, v),
                None => {
                    let msg = match table.iter().find(|o| o.option == name) {
                        Some(o) => format!("`{name}` is called `{}` in [{section}]", o.key),
                        None => {
                            let hint = did_you_mean(name, table.iter().map(|o| o.key), ("`", "`"));
                            let all: Vec<&str> = table.iter().map(|o| o.key).collect();
                            if hint.is_empty() {
                                format!("unknown key `{name}` in [{section}] (expected one of: {})", all.join(", "))
                            } else {
                                format!("unknown key `{name}` in [{section}]{hint}")
                            }
                        }
                    };
                    self.err(k.span().start, msg);
                }
            }
        }
    }

    fn option(&mut self, opt: &Opt, label: &str, k: &K<'_>, v: &V<'_>) {
        if self.is_damaged(k.span().start..v.span().end) {
            return;
        }
        let at = v.span().start;
        let text = match self.value(opt, label, v) {
            Ok(t) => t,
            Err((pos, msg)) => return self.err(pos, format!("`{label}`: {msg}")),
        };
        if opt.kind == Style && !self.scratch.has_option(opt.option) {
            let msg = format!("`{label}` has no effect yet: this lif build has no `{}` option", opt.option);
            self.diags.push((k.span().start, Severity::Warning, msg));
            return;
        }
        let what = format!("`{label}`");
        let flag = if opt.server { "-s" } else { "-g" };
        self.push(0, at, format!("set-option {flag} {} {}", opt.option, quote(&text)), &what);
        if opt.option == "prefix"
            && let Some(code) = KeyCode::parse(&text)
        {
            // A new prefix replaces C-b completely: C-b no longer sends
            // itself, the new key does (tmux's documented recipe).
            if Some(code) != KeyCode::parse("C-b") {
                self.push(1, at, "unbind-key -T prefix C-b".to_string(), &what);
            }
            self.push(1, at, format!("bind-key -T prefix {} send-prefix", quote(&text)), &what);
        }
    }

    fn value(&self, opt: &Opt, label: &str, v: &V<'_>) -> Result<String, (usize, String)> {
        let at = v.span().start;
        let e = |m: String| Err((at, m));
        let got = describe(v.get_ref());
        match (opt.kind, v.get_ref()) {
            (Bool, DeValue::Boolean(b)) => Ok(if *b { "on" } else { "off" }.to_string()),
            (Bool, DeValue::String(s)) if matches!(s.to_ascii_lowercase().as_str(), "on" | "off" | "yes" | "no") => {
                let b = matches!(s.to_ascii_lowercase().as_str(), "on" | "yes");
                e(format!("expected true or false, found {got} (write `{} = {b}`)", label.rsplit('.').next().unwrap_or(label)))
            }
            (Bool, _) => e(format!("expected true or false, found {got}")),
            (Int | Size | Bytes, DeValue::Integer(_)) => match int(v.get_ref()) {
                Some(Ok(n)) if opt.kind == Bytes && n < 0 => e(format!("expected a size in bytes, found {got}")),
                Some(Ok(n)) => Ok(n.to_string()),
                _ => e("integer out of range".to_string()),
            },
            (Int, _) => e(format!("expected an integer, found {got}")),
            (Str | Dim | Size, DeValue::String(s)) => Ok(s.to_string()),
            (Str, _) => e(format!("expected a string, found {got}")),
            (Dim, _) => e(format!("expected a size such as \"80x24\", found {got}")),
            (Size, _) => e(format!("expected a number of cells or a percentage such as \"50%\", found {got}")),
            (Bytes, DeValue::String(s)) => match parse_size(s) {
                Some(n) => Ok(n.to_string()),
                None => e(format!("expected a size such as \"32MB\" (units B, KB, MB, GB; 1 KB = 1024 bytes), found {got}")),
            },
            (Bytes, _) => e(format!("expected a size in bytes or a string such as \"32MB\", found {got}")),
            (Key, DeValue::Boolean(false)) => Ok("None".to_string()),
            (Key, DeValue::String(s)) if s.eq_ignore_ascii_case("none") => Ok("None".to_string()),
            (Key, DeValue::String(s)) => KeyCode::parse(s).map(|k| k.to_string()).ok_or((at, bad_key_message(s))),
            (Key, _) => e(format!("expected a key name such as \"C-a\", or false for none, found {got}")),
            (Choice(list), DeValue::String(s)) => {
                if list.contains(&s.as_ref()) {
                    Ok(s.to_string())
                } else {
                    let hint = did_you_mean(s, list.iter().copied(), ("\"", "\""));
                    e(format!("unknown value {s:?}{hint} (expected one of: {})", list.join(", ")))
                }
            }
            (Choice(list), DeValue::Boolean(b)) if list.contains(&"off") && list.contains(&"on") => {
                Ok(if *b { "on" } else { "off" }.to_string())
            }
            (Choice(list), _) => {
                let bools = if list.contains(&"off") && list.contains(&"on") { "true, false or " } else { "" };
                e(format!("expected {bools}one of: {}, found {got}", list.join(", ")))
            }
            (Style, DeValue::String(s)) => style::check_string(s).map(|_| s.to_string()).map_err(|m| (at, m)),
            (Style, DeValue::Table(t)) => style_table(t),
            (Style, _) => e(format!("expected a style table such as {{ fg = \"black\", bg = \"green\" }} or a tmux style string, found {got}")),
        }
    }

    fn keys(&mut self, t: &DeTable<'_>) {
        // Few bindings per file: a list beats a hash map in code size.
        let mut seen: Vec<(String, KeyCode, usize)> = Vec::new();
        for (k, v) in entries(t) {
            let name = k.get_ref().as_ref();
            if let Some(code) = KeyCode::parse(name) {
                self.binding("prefix", code, k, v, &mut seen);
                continue;
            }
            if TABLES.contains(&name) {
                match v.get_ref() {
                    DeValue::Table(tt) => {
                        for (k2, v2) in entries(tt) {
                            match KeyCode::parse(k2.get_ref()) {
                                Some(code) => self.binding(name, code, k2, v2, &mut seen),
                                None => self.err(k2.span().start, format!("{} in [keys.{name}]", bad_key_message(k2.get_ref()))),
                            }
                        }
                    }
                    other => self.err(k.span().start, format!("`keys.{name}` must be a table of key bindings, found {}", describe(other))),
                }
                continue;
            }
            let looks_like_table = match v.get_ref() {
                DeValue::Table(tt) => !tt.iter().any(|(kk, _)| matches!(kk.get_ref().as_ref(), "command" | "repeat" | "note")),
                _ => false,
            };
            if looks_like_table {
                let hint = did_you_mean(name, TABLES.iter().copied(), ("[keys.", "]"));
                let msg = format!("unknown key table [keys.{name}]{hint} (tables are: {})", TABLES.join(", "));
                self.err(k.span().start, msg);
            } else {
                self.err(k.span().start, bad_key_message(name));
            }
        }
    }

    fn binding(&mut self, table: &str, code: KeyCode, k: &K<'_>, v: &V<'_>, seen: &mut Vec<(String, KeyCode, usize)>) {
        if self.is_damaged(k.span().start..v.span().end) {
            return;
        }
        let name = k.get_ref().as_ref();
        let section = if table == "prefix" { "[keys]".to_string() } else { format!("[keys.{table}]") };
        if let Some((_, _, prev)) = seen.iter().find(|(t, c, _)| t == table && *c == code) {
            let line = self.lines.pos(*prev).0;
            let msg = format!("key `{name}` is bound twice in {section} (it is the same key as the one on line {line})");
            return self.err(k.span().start, msg);
        }
        seen.push((table.to_string(), code, k.span().start));
        let key = quote(&code.to_string());
        let what = format!("key `{name}` in {section}");
        let at = v.span().start;
        match binding_value(v) {
            Ok(Bind::Unbind) => self.push(2, at, format!("unbind-key -T {table} {key}"), &what),
            Ok(Bind::Run { cmd, repeat, note }) => {
                let mut c = String::from("bind-key ");
                if repeat {
                    c.push_str("-r ");
                }
                if let Some(n) = note {
                    c.push_str("-N ");
                    c.push_str(&quote(&n));
                    c.push(' ');
                }
                c.push_str(&format!("-T {table} {key} {}", quote(&cmd)));
                self.push(2, at, c, &what);
            }
            Err((pos, msg)) => self.err(pos, format!("{what}: {msg}")),
        }
    }
}

fn commands(v: &V<'_>) -> Result<String, (usize, String)> {
    let at = v.span().start;
    match v.get_ref() {
        DeValue::String(s) => validate_command(s).map(|_| s.to_string()).map_err(|m| (at, m)),
        DeValue::Array(a) if a.is_empty() => Err((at, "empty list of commands".to_string())),
        DeValue::Array(a) => {
            let mut parts = Vec::new();
            for item in a.iter() {
                match item.get_ref() {
                    DeValue::String(s) => {
                        validate_command(s).map_err(|m| (item.span().start, m))?;
                        parts.push(s.to_string());
                    }
                    other => return Err((item.span().start, format!("expected a command string, found {}", describe(other)))),
                }
            }
            Ok(parts.join(" ; "))
        }
        other => Err((at, format!("expected a command string or a list of them, found {}", describe(other)))),
    }
}

fn binding_value(v: &V<'_>) -> Result<Bind, (usize, String)> {
    let at = v.span().start;
    match v.get_ref() {
        DeValue::Boolean(false) => Ok(Bind::Unbind),
        DeValue::String(s) if s.eq_ignore_ascii_case("none") => Ok(Bind::Unbind),
        DeValue::String(_) | DeValue::Array(_) => Ok(Bind::Run { cmd: commands(v)?, repeat: false, note: None }),
        DeValue::Table(t) => {
            let (mut cmd, mut repeat, mut note) = (None, false, None);
            for (k, val) in entries(t) {
                let pos = val.span().start;
                match (k.get_ref().as_ref(), val.get_ref()) {
                    ("command", _) => cmd = Some(commands(val)?),
                    ("repeat", DeValue::Boolean(b)) => repeat = *b,
                    ("repeat", other) => return Err((pos, format!("`repeat` expects true or false, found {}", describe(other)))),
                    ("note", DeValue::String(s)) => note = Some(s.to_string()),
                    ("note", other) => return Err((pos, format!("`note` expects a string, found {}", describe(other)))),
                    (name, _) => {
                        let hint = did_you_mean(name, ["command", "repeat", "note"], ("`", "`"));
                        return Err((k.span().start, format!("unknown binding field `{name}`{hint} (fields: command, repeat, note)")));
                    }
                }
            }
            match cmd {
                Some(cmd) => Ok(Bind::Run { cmd, repeat, note }),
                None => Err((at, "a binding table needs `command`, e.g. { command = \"resize-pane -L 5\", repeat = true }".to_string())),
            }
        }
        other => Err((
            at,
            format!(
                "expected a command string, a list of commands, false to unbind, or {{ command = ..., repeat = true }}, found {}",
                describe(other)
            ),
        )),
    }
}

fn style_table(t: &DeTable<'_>) -> Result<String, (usize, String)> {
    // Attributes apply in the order written, as in a tmux style string.
    let mut parts: Vec<String> = Vec::new();
    for (k, v) in entries(t) {
        let name = k.get_ref().as_ref();
        let pos = v.span().start;
        let got = describe(v.get_ref());
        let part = match (name, v.get_ref()) {
            ("fg" | "bg" | "underline-color", DeValue::String(c)) => {
                if !style::valid_color(c) {
                    let names = style::color_names();
                    let hint = did_you_mean(c, names.iter().map(|s| s.as_str()), ("\"", "\""));
                    return Err((pos, style::unknown_color(c, &hint)));
                }
                let k = if name == "underline-color" { "us" } else { name };
                format!("{k}={c}")
            }
            ("fg" | "bg" | "underline-color", _) => return Err((pos, format!("`{name}` expects {}, found {got}", style::COLOR_HELP))),
            ("underline", DeValue::Boolean(b)) => if *b { "underscore" } else { "nounderscore" }.to_string(),
            ("underline", DeValue::String(s)) => match style::UNDERLINES.iter().find(|(n, _)| *n == s.as_ref()) {
                Some((_, a)) => a.to_string(),
                None => {
                    return Err((pos, format!("`underline` expects true, false, \"single\", \"double\", \"curly\", \"dotted\" or \"dashed\", found {got}")));
                }
            },
            ("underline", _) => return Err((pos, format!("`underline` expects true, false or an underline kind, found {got}"))),
            (n, DeValue::Boolean(b)) if style::attr(n).is_some() => {
                let a = style::attr(n).unwrap_or_default();
                if *b { a.to_string() } else { format!("no{a}") }
            }
            (n, _) if style::attr(n).is_some() => return Err((pos, format!("`{n}` expects true or false, found {got}"))),
            (n, _) => {
                let hint = did_you_mean(n, style::TABLE_KEYS.iter().copied(), ("`", "`"));
                return Err((k.span().start, format!("unknown style key `{n}`{hint} (style keys: {})", style::TABLE_KEYS.join(", "))));
            }
        };
        parts.push(part);
    }
    if parts.is_empty() {
        return Ok("default".to_string());
    }
    Ok(parts.join(","))
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::check::NullHost;
    use lif_core::{CmdContext, Model, WallClock};

    fn lif_options() -> Vec<String> {
        let mut m = Model::new("t");
        let mut names = Vec::new();
        for cmd in ["show-options -g", "show-options -s", "show-options -gw"] {
            let list = parse_string(cmd).unwrap();
            let out = m.execute_list(&list, &CmdContext::none(WallClock::new(0, 0)), &mut NullHost).unwrap().stdout;
            names.extend(out.lines().filter_map(|l| l.split(' ').next()).map(|s| s.to_string()));
        }
        assert!(names.len() > 40, "{names:?}");
        names
    }

    #[test]
    fn every_lif_core_option_has_a_key() {
        for name in lif_options() {
            assert!(location(&name).is_some(), "lif-core option {name} has no config key");
        }
    }

    #[test]
    fn every_key_is_a_lif_core_option_except_known_theme_gaps() {
        let known = lif_options();
        let gaps = ["pane-border-style", "pane-active-border-style", "mode-style", "message-style"];
        for opt in TOP.iter().chain(STATUS).chain(THEME) {
            assert!(known.iter().any(|k| k == opt.option) || gaps.contains(&opt.option), "{} is not a lif-core option", opt.option);
        }
        let server = ["buffer-limit", "escape-time", "exit-empty", "set-clipboard"];
        for opt in TOP {
            assert_eq!(opt.server, server.contains(&opt.option), "{}", opt.option);
        }
    }

    #[test]
    fn sizes() {
        assert_eq!(parse_size("32MB"), Some(32 << 20));
        assert_eq!(parse_size("32 MiB"), Some(32 << 20));
        assert_eq!(parse_size("512k"), Some(512 << 10));
        assert_eq!(parse_size("1G"), Some(1 << 30));
        assert_eq!(parse_size("4096"), Some(4096));
        assert_eq!(parse_size("10b"), Some(10));
        for bad in ["", "MB", "1.5GB", "-1MB", "12 parsecs", "99999999999999999999", "9223372036854775807G"] {
            assert_eq!(parse_size(bad), None, "{bad}");
        }
    }

    #[test]
    fn line_columns() {
        let l = Lines::new("ab\nçd\n\nx");
        assert_eq!(l.pos(0), (1, 1));
        assert_eq!(l.pos(2), (1, 3));
        assert_eq!(l.pos(3), (2, 1));
        assert_eq!(l.pos(5), (2, 2), "columns count characters");
        assert_eq!(l.pos(4), (2, 1), "inside a character");
        assert_eq!(l.pos(7), (3, 1));
        assert_eq!(l.pos(8), (4, 1));
        assert_eq!(l.pos(999), (4, 2));
    }

    #[test]
    fn key_name_messages() {
        assert_eq!(bad_key_message("C-Lef"), "unknown key name `C-Lef`, did you mean `C-Left`?");
        assert_eq!(bad_key_message("Entr"), "unknown key name `Entr`, did you mean `Enter`?");
        assert!(bad_key_message("Ctrl-a").contains("`C-a`"));
        assert_eq!(bad_key_message(""), "empty key name");
    }
}
