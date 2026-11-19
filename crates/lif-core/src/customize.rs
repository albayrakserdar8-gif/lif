//! The lines of `customize-mode` (tmux's customize mode): the options of
//! the server, of the client's session and of its window, the global hooks,
//! the global and the session environment, and every key table. Each line's
//! target is the command that edits it (a `command-prompt` holding the
//! value), and its `kill` command puts the default back; the chooser
//! (`choose.rs`) runs them and stays up, and the list is built again from
//! the model after each change.
//!
//! The headings and their order are tmux 3.8's. Unlike tmux, lif shows no
//! description of each option, only its scope and value.

use crate::choose::{ChooseItem, Preview};
use crate::cmd::escape_arg;
use crate::ids::ClientId;
use crate::model::Model;
use crate::options::{self, Scope};

/// A heading line.
fn heading(id: &str, name: &str) -> ChooseItem {
    ChooseItem { id: id.to_string(), target: String::new(), depth: 0, name: name.to_string(), text: String::new(), preview: Preview::None, kill: None }
}

/// A setting line: `edit` is the command that sets `what` to the
/// answer of a prompt that starts with `value`.
fn setting(id: String, name: String, value: String, edit: &str, reset: String, preview: String) -> ChooseItem {
    let prompt = format!("({name})");
    let target = format!("command-prompt -I {} -p {} {{ {edit} }}", escape_arg(&value), escape_arg(&prompt));
    ChooseItem { id, target, depth: 1, name, text: value, preview: Preview::Text(preview), kill: Some((String::new(), reset)) }
}

impl Model {
    pub(crate) fn customize_items(&self, client: Option<ClientId>) -> Vec<ChooseItem> {
        let sid = client.and_then(|c| self.clients.get(&c)).and_then(|c| c.session).filter(|s| self.sessions.contains_key(s));
        let wid = sid.map(|s| self.sessions[&s].current).filter(|w| self.windows.contains_key(w));
        let mut out = Vec::new();
        let groups: [(&str, &str, Scope); 3] =
            [("g:server", "Server Options", Scope::Server), ("g:session", "Session Options", Scope::Session), ("g:window", "Window & Pane Options", Scope::Window)];
        for (id, title, scope) in groups {
            out.push(heading(id, title));
            for def in options::DEFS.iter().filter(|d| d.scope == scope) {
                // The value the client's session and window see, and where
                // it is set: locally (tmux's "(session a)") or globally.
                let (local, set_flags, target) = match scope {
                    Scope::Server => (None, "-s".to_string(), String::new()),
                    Scope::Session => match sid.filter(|s| self.sessions[s].options.get(def.name).is_some()) {
                        Some(s) => (Some(format!("(session {}) ", self.sessions[&s].name)), String::new(), format!(" -t {}", escape_arg(&s.to_string()))),
                        None => (None, "-g".to_string(), String::new()),
                    },
                    Scope::Window => match wid.filter(|w| self.windows[w].options.get(def.name).is_some()) {
                        Some(w) => (Some(format!("(window {}) ", w)), "-w".to_string(), format!(" -t {}", escape_arg(&w.to_string()))),
                        None => (None, "-gw".to_string(), String::new()),
                    },
                };
                let value = self.option_text(def.name, sid, wid).unwrap_or_default();
                let scope_name = match scope {
                    Scope::Server => "server",
                    Scope::Session => "session",
                    Scope::Window => "window",
                };
                let flags = if set_flags.is_empty() { String::new() } else { format!("{set_flags} ") };
                let edit = format!("set-option {flags}{}{} \"%%%\"", def.name, target);
                let reset = format!("set-option {}u{target} {}", if flags.is_empty() { "-".to_string() } else { flags.trim_end().to_string() }, def.name);
                let preview = format!("This is a {scope_name} option.\n\nOption value: {value}");
                out.push(setting(format!("o:{}", def.name), def.name.to_string(), format!("{}{value}", local.unwrap_or_default()), &edit, reset, preview));
            }
        }
        for (id, title, window) in [("g:shooks", "Session Hooks", false), ("g:whooks", "Window & Pane Hooks", true)] {
            out.push(heading(id, title));
            for (name, command) in self.hooks.global_lines(window) {
                let flag = if window { "-gw" } else { "-g" };
                let edit = format!("set-hook {flag} {name} \"%%%\"");
                let preview = format!("This is a {} hook.\n\nHook value: {command}", if window { "window" } else { "session" });
                out.push(setting(format!("h:{name}"), name.clone(), command, &edit, format!("set-hook {flag}u {name}"), preview));
            }
        }
        let envs = [("g:genv", "Global Environment", None), ("g:senv", "Session Environment", sid)];
        for (id, title, session) in envs {
            out.push(heading(id, title));
            let set = match session {
                Some(s) => &self.sessions[&s].environ,
                None => &self.environ,
            };
            let flag = match session {
                Some(s) => format!("-t {}", escape_arg(&s.to_string())),
                None => "-g".to_string(),
            };
            for (name, value) in set.entries() {
                let shown = value.clone().unwrap_or_default();
                let edit = format!("set-environment {flag} {} \"%%%\"", escape_arg(&name));
                let preview = match &value {
                    Some(v) => format!("{name}={v}"),
                    None => format!("{name} is removed from the environment."),
                };
                let text = if value.is_some() { shown.clone() } else { "(removed)".to_string() };
                let mut item = setting(format!("e:{id}:{name}"), name.clone(), shown, &edit, format!("set-environment {flag} -u {}", escape_arg(&name)), preview);
                item.text = text;
                out.push(item);
            }
        }
        let mut tables: Vec<String> = self.bindings.table_names().map(str::to_string).collect();
        tables.sort();
        for t in tables {
            let id = format!("g:keys:{t}");
            out.push(heading(&id, &format!("Key Table - {t}")));
            for b in self.bindings.bindings(&t) {
                let key = b.key.to_string();
                let command = b.commands.print(false);
                let edit = format!("bind-key -T {} {} \"%%%\"", escape_arg(&t), escape_arg(&key));
                let preview = format!("This is a key binding of table {t}.\n\nCommand: {command}");
                out.push(setting(format!("k:{t}:{key}"), key.clone(), command, &edit, format!("unbind-key -T {} {}", escape_arg(&t), escape_arg(&key)), preview));
            }
        }
        out
    }
}
