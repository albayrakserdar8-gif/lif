//! A best-effort translator from `.tmux.conf` to `config.toml`.
//!
//! Only the common core is translated: `set` / `set-option` / `setw` with
//! `-g` (or `-s`) for options lif has, `bind` / `bind-key` with `-n`, `-r`,
//! `-T` and `-N`, `unbind` / `unbind-key` with `-n` and `-T`, and comments
//! directly above a translated line. Everything else is reported, never
//! guessed.

use crate::schema::{Kind, TABLES, location};
use lif_core::{Arg, Command, KeyCode, escape_arg, parse_list, parse_string};

/// The result of [`migrate_tmux_conf`].
#[derive(Clone, Debug, Default, PartialEq, Eq)]
pub struct Migration {
    /// The translated file.
    pub toml: String,
    /// Lines (or commands within a line) left out, in file order.
    pub untranslated: Vec<Untranslated>,
}

/// A `.tmux.conf` command that was not translated.
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct Untranslated {
    /// 1-based line number (of the first line, for continued lines).
    pub line: usize,
    /// The line as written.
    pub text: String,
    /// Why it was left out.
    pub reason: String,
}

struct Item {
    key: String,
    value: String,
    comments: Vec<String>,
}

#[derive(Default)]
struct Section {
    items: Vec<Item>,
}

impl Section {
    fn set(&mut self, key: String, value: String, comments: Vec<String>) {
        match self.items.iter_mut().find(|i| i.key == key) {
            Some(i) => {
                i.value = value;
                i.comments.extend(comments);
            }
            None => self.items.push(Item { key, value, comments }),
        }
    }

    fn get(&self, key: &str) -> Option<&str> {
        self.items.iter().find(|i| i.key == key).map(|i| i.value.as_str())
    }

    fn remove(&mut self, key: &str) {
        self.items.retain(|i| i.key != key);
    }
}

#[derive(Default)]
struct Out {
    top: Section,
    status: Section,
    theme: Section,
    keys: [Section; 4],
    prefix: Option<KeyCode>,
}

/// Translate the text of a `.tmux.conf`.
pub fn migrate_tmux_conf(text: &str) -> Migration {
    let mut out = Out::default();
    let mut untranslated = Vec::new();
    let mut pending: Vec<String> = Vec::new();
    let raw: Vec<&str> = text.lines().collect();
    let mut i = 0;
    while i < raw.len() {
        let first = i + 1;
        let mut line = raw[i].to_string();
        while continues(&line) && i + 1 < raw.len() {
            line.pop();
            i += 1;
            line.push_str(raw[i]);
        }
        i += 1;
        let t = line.trim();
        if t.is_empty() {
            pending.clear();
            continue;
        }
        if let Some(c) = t.strip_prefix('#') {
            pending.push(c.strip_prefix(' ').unwrap_or(c).trim_end().to_string());
            continue;
        }
        let comments = std::mem::take(&mut pending);
        match parse_string(&line) {
            Err(e) => untranslated.push(Untranslated { line: first, text: t.to_string(), reason: e.0 }),
            Ok(list) => {
                let mut comments = Some(comments);
                for cmd in list.commands() {
                    match out.command(cmd, &mut comments) {
                        Ok(()) => {}
                        Err(reason) => untranslated.push(Untranslated { line: first, text: t.to_string(), reason }),
                    }
                }
            }
        }
    }
    // `prefix = ...` already rebinds the prefix completely; drop the usual
    // recipe lines that would only repeat it.
    if let Some(p) = out.prefix
        && Some(p) != KeyCode::parse("C-b")
    {
        if out.keys[0].get("C-b") == Some("false") {
            out.keys[0].remove("C-b");
        }
        let name = p.to_string();
        if out.keys[0].get(&name) == Some(toml_str("send-prefix").as_str()) {
            out.keys[0].remove(&name);
        }
    }
    Migration { toml: out.render(), untranslated }
}

/// tmux's mouse key names (`MouseDown1Pane`, `WheelUpStatus`, ...).
const MOUSE_KEYS: &[&str] = &["Mouse", "Wheel", "DoubleClick", "TripleClick", "SecondClick"];

/// Whether `path` names a tmux configuration file rather than a lif one:
/// `.tmux.conf`, `tmux.conf`, or anything ending in `.conf`.
pub fn looks_like_tmux_conf(path: &str) -> bool {
    path.ends_with(".conf")
}

/// Whether the text of a file that failed to parse as TOML reads as tmux
/// commands instead (its first line is a command and not `key = value`), so
/// the user can be pointed at [`migrate_tmux_conf`].
pub fn looks_like_tmux_commands(text: &str) -> bool {
    text.lines()
        .map(str::trim)
        .find(|l| !l.is_empty() && !l.starts_with('#'))
        .is_some_and(|first| !first.contains('=') && parse_string(first).is_ok_and(|l| !l.is_empty()))
}

/// Whether a line ends in an unescaped backslash (continued on the next).
fn continues(line: &str) -> bool {
    line.chars().rev().take_while(|c| *c == '\\').count() % 2 == 1
}

fn table_index(table: &str) -> Option<usize> {
    TABLES.iter().position(|t| *t == table)
}

impl Out {
    fn command(&mut self, cmd: &Command, comments: &mut Option<Vec<String>>) -> Result<(), String> {
        let a = cmd.args();
        match cmd.name() {
            "set-option" | "set-window-option" => {
                for f in ['a', 'F', 'o', 'u', 't'] {
                    if a.has(f) {
                        return Err(format!("{} -{f} has no config.toml equivalent", cmd.name()));
                    }
                }
                if !a.has('g') && !a.has('s') {
                    return Err("only global options (set -g) can be translated".to_string());
                }
                let name = a.value(0).unwrap_or("");
                if name.starts_with('@') {
                    return Err("user options (@name) have no config.toml equivalent".to_string());
                }
                let value = a.value(1).ok_or_else(|| format!("no value given for {name}"))?;
                let Some((section, opt)) = location(name) else {
                    return Err(format!("lif has no option {name}"));
                };
                let v = convert(opt.kind, opt.key, name, value)?;
                if name == "prefix" {
                    self.prefix = KeyCode::parse(value);
                }
                let sec = match section {
                    "status" => &mut self.status,
                    "theme" => &mut self.theme,
                    _ => &mut self.top,
                };
                sec.set(opt.key.to_string(), v, comments.take().unwrap_or_default());
                Ok(())
            }
            "bind-key" | "unbind-key" => {
                let bind = cmd.name() == "bind-key";
                if a.has('a') {
                    return Err("unbind-key -a has no config.toml equivalent".to_string());
                }
                let table = if a.has('n') { "root" } else { a.get('T').unwrap_or("prefix") };
                let Some(ti) = table_index(table) else {
                    return Err(format!("key table {table} is not supported (config.toml has {})", TABLES.join(", ")));
                };
                let name = a.value(0).ok_or("no key given")?;
                if name == "MouseDragEnd1Pane" && table.starts_with("copy-mode") {
                    return self.drag_end(bind, &a.values()[1..], comments);
                }
                if MOUSE_KEYS.iter().any(|p| name.starts_with(p)) {
                    return Err(format!(
                        "{name}: lif's mouse actions are built in, not bound \
                         (mouse-drag-end and copy-command configure the copy at the end of a selection)"
                    ));
                }
                let key = KeyCode::parse(name).ok_or_else(|| format!("unknown key: {name}"))?;
                let value = if !bind {
                    "false".to_string()
                } else {
                    let rest = &a.values()[1..];
                    if rest.is_empty() {
                        return Err("bind-key without a command".to_string());
                    }
                    let text = command_text(rest)?;
                    if let Ok(list) = parse_string(&text)
                        && list.commands().iter().any(|c| {
                            c.name() == "source-file" && c.args().strings(0).iter().any(|p| looks_like_tmux_conf(p))
                        })
                    {
                        return Err("source-file of a tmux configuration file: lif reloads its own config.toml \
                                    (source-file ~/.config/lif/config.toml)"
                            .to_string());
                    }
                    let c = toml_str(&text);
                    if a.has('r') || a.get('N').is_some() {
                        let mut v = format!("{{ command = {c}");
                        if a.has('r') {
                            v.push_str(", repeat = true");
                        }
                        if let Some(n) = a.get('N') {
                            v.push_str(&format!(", note = {}", toml_str(n)));
                        }
                        v.push_str(" }");
                        v
                    } else {
                        c
                    }
                };
                self.keys[ti].set(key.to_string(), value, comments.take().unwrap_or_default());
                Ok(())
            }
            other => Err(format!("{other} has no config.toml equivalent")),
        }
    }

    /// `MouseDragEnd1Pane` in a copy-mode table: lif's `mouse-drag-end`.
    fn drag_end(&mut self, bind: bool, rest: &[Arg], comments: &mut Option<Vec<String>>) -> Result<(), String> {
        let value = if !bind {
            "none"
        } else {
            let text = command_text(rest)?;
            let list = parse_string(&text).map_err(|e| e.0)?;
            let words = match list.commands() {
                [c] if c.name() == "send-keys" && c.args().has('X') => c.args().strings(0),
                _ => Vec::new(),
            };
            let value = match words.first().map(String::as_str) {
                Some("copy-pipe-and-cancel" | "copy-selection-and-cancel") => "copy-pipe-and-cancel",
                Some("copy-pipe-no-clear" | "copy-selection-no-clear") => "copy-pipe-no-clear",
                _ => {
                    return Err(format!(
                        "MouseDragEnd1Pane runs {text}: lif's mouse-drag-end is copy-pipe-and-cancel, \
                         copy-pipe-no-clear or none"
                    ));
                }
            };
            if let Some(pipe) = words.get(1).filter(|_| words[0].starts_with("copy-pipe")) {
                // Translated as far as it goes; the command needs a choice.
                self.top.set("mouse-drag-end".into(), toml_str(value), comments.take().unwrap_or_default());
                return Err(format!(
                    "lif pipes mouse copies to the copy-command option, which also serves copy-pipe keys \
                     without a command of their own (Enter in vi, M-w in emacs): add copy-command = {} \
                     if that is what you want",
                    toml_str(pipe)
                ));
            }
            value
        };
        self.top.set("mouse-drag-end".into(), toml_str(value), comments.take().unwrap_or_default());
        Ok(())
    }

    fn render(&self) -> String {
        let mut s = String::from("# lif config.toml, translated from .tmux.conf.\n");
        let section = |s: &mut String, header: &str, sec: &Section| {
            if sec.items.is_empty() {
                return;
            }
            s.push('\n');
            if !header.is_empty() {
                s.push_str(header);
                s.push('\n');
            }
            for it in &sec.items {
                for c in &it.comments {
                    s.push_str("# ");
                    s.push_str(c);
                    s.push('\n');
                }
                s.push_str(&format!("{} = {}\n", toml_key(&it.key), it.value));
            }
        };
        section(&mut s, "", &self.top);
        section(&mut s, "[status]", &self.status);
        section(&mut s, "[theme]", &self.theme);
        for (t, sec) in TABLES.iter().zip(&self.keys) {
            let header = if *t == "prefix" { "[keys]".to_string() } else { format!("[keys.{t}]") };
            section(&mut s, &header, sec);
        }
        s
    }
}

/// The command string of a binding from `bind-key`'s arguments: one string
/// as written, or argv words (with `\;` separators) quoted back into one.
fn command_text(values: &[Arg]) -> Result<String, String> {
    if let [Arg::Str(s)] = values {
        parse_string(s).map_err(|e| e.0)?;
        return Ok(s.clone());
    }
    let argv: Vec<String> = values
        .iter()
        .map(|a| match a {
            Arg::Str(s) => s.clone(),
            Arg::Block(b) => b.to_string(),
        })
        .collect();
    if let [Arg::Block(b)] = values {
        return Ok(b.to_string());
    }
    let list = parse_list(&argv).map_err(|e| e.0)?;
    let text = argv.iter().map(|w| if w == ";" { ";".to_string() } else { escape_arg(w) }).collect::<Vec<_>>().join(" ");
    match parse_string(&text) {
        Ok(l) if l == list => Ok(text),
        _ => Ok(list.to_string()),
    }
}

fn convert(kind: Kind, key: &str, name: &str, v: &str) -> Result<String, String> {
    let bad = || Err(format!("value {v} is not valid for {name}"));
    let flag = |v: &str| match v.to_ascii_lowercase().as_str() {
        "on" | "yes" | "1" => Some(true),
        "off" | "no" | "0" => Some(false),
        _ => None,
    };
    match kind {
        Kind::Bool => flag(v).map(|b| b.to_string()).map_or_else(bad, Ok),
        Kind::Choice(_) if key == "enabled" => flag(v).map(|b| b.to_string()).map_or_else(bad, Ok),
        Kind::Choice(list) if list.contains(&v) => Ok(toml_str(v)),
        Kind::Choice(_) => bad(),
        Kind::Int | Kind::Bytes => v.parse::<i64>().map(|n| n.to_string()).or_else(|_| bad()),
        Kind::Size => Ok(v.parse::<i64>().map(|n| n.to_string()).unwrap_or_else(|_| toml_str(v))),
        Kind::Key if v.eq_ignore_ascii_case("none") || KeyCode::parse(v).is_some() => Ok(toml_str(v)),
        Kind::Key => bad(),
        Kind::Str | Kind::Dim | Kind::Style => Ok(toml_str(v)),
    }
}

/// A TOML string: literal (`'...'`) when that avoids escapes, else basic.
pub(crate) fn toml_str(s: &str) -> String {
    let control = s.chars().any(|c| c.is_control());
    if !control && !s.contains('\'') && (s.contains('"') || s.contains('\\')) {
        return format!("'{s}'");
    }
    let mut out = String::with_capacity(s.len() + 2);
    out.push('"');
    for c in s.chars() {
        match c {
            '"' => out.push_str("\\\""),
            '\\' => out.push_str("\\\\"),
            '\n' => out.push_str("\\n"),
            '\t' => out.push_str("\\t"),
            '\r' => out.push_str("\\r"),
            c if c.is_control() => out.push_str(&format!("\\u{:04X}", c as u32)),
            c => out.push(c),
        }
    }
    out.push('"');
    out
}

/// A TOML key: bare when possible, else quoted.
pub(crate) fn toml_key(s: &str) -> String {
    // Bare only when it starts like a word, so that keys such as "-" stay
    // recognizable.
    if s.bytes().next().is_some_and(|b| b.is_ascii_alphanumeric()) && s.bytes().all(|b| b.is_ascii_alphanumeric() || b == b'-' || b == b'_') {
        s.to_string()
    } else {
        toml_str(s)
    }
}
