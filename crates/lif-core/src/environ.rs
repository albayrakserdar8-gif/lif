//! The environment commands: `set-environment` and `show-environment`.
//!
//! A pane starts with the server's environment and the few variables the
//! server takes from the client that made its session (the server keeps
//! those). What is set here goes on top, for panes and shell commands
//! started afterwards: the global set first, then the session's, then `-e`
//! of the command creating the pane. An entry can also say "remove this
//! variable" (`-r`), shown as `-NAME` like tmux. Hidden variables (`-h`)
//! are listed with `show-environment -h` and never passed on.
//!
//! Unlike tmux's, the sets hold only what was set with these commands:
//! the server's own environment is not listed.

use crate::cmd::{Args, CmdError};
use crate::format;
use crate::host::Host;
use crate::ids::{ClientId, SessionId};
use crate::model::{CmdContext, CmdOutput, Model};
use crate::target::{Flags, Kind};
use std::collections::BTreeMap;

/// Most variables in one set (the global one, or a session's).
pub(crate) const MAX_ENV_VARS: usize = 1024;

/// Longest name plus value of one variable, in bytes.
pub(crate) const MAX_ENV_BYTES: usize = 64 * 1024;

#[derive(Clone, Debug, PartialEq, Eq)]
struct Var {
    /// `None`: remove the variable from the environment.
    value: Option<String>,
    hidden: bool,
}

/// One set of variables, by name.
#[derive(Clone, Debug, Default)]
pub(crate) struct Environ(BTreeMap<String, Var>);

impl Environ {
    /// Every variable but the hidden ones: its value, or `None` for one
    /// to be removed.
    pub(crate) fn entries(&self) -> Vec<(String, Option<String>)> {
        self.0.iter().filter(|(_, v)| !v.hidden).map(|(k, v)| (k.clone(), v.value.clone())).collect()
    }

    fn overrides(&self) -> impl Iterator<Item = (String, Option<String>)> + '_ {
        self.0.iter().filter(|(_, v)| !v.hidden).map(|(k, v)| (k.clone(), v.value.clone()))
    }
}

/// `value` inside double quotes for a POSIX shell.
fn shell_escape(value: &str) -> String {
    let mut out = String::with_capacity(value.len());
    for c in value.chars() {
        if matches!(c, '$' | '`' | '"' | '\\') {
            out.push('\\');
        }
        out.push(c);
    }
    out
}

/// `-e NAME=value` arguments as overrides; ones without `=` are ignored,
/// as tmux ignores them.
pub(crate) fn flag_env(a: &Args) -> Vec<(String, Option<String>)> {
    a.all('e').filter_map(|e| e.split_once('=')).filter(|(k, _)| !k.is_empty()).map(|(k, v)| (k.to_string(), Some(v.to_string()))).collect()
}

type R = Result<CmdOutput, CmdError>;

impl Model {
    /// What panes and shell commands of `session` get on top of their
    /// environment, in order: the global set, then the session's.
    pub(crate) fn environ_overrides(&self, session: Option<SessionId>) -> Vec<(String, Option<String>)> {
        let mut v: Vec<(String, Option<String>)> = self.environ.overrides().collect();
        if let Some(s) = session.and_then(|s| self.sessions.get(&s)) {
            v.extend(s.environ.overrides());
        }
        v
    }

    /// `update-environment`: each variable it names is copied from
    /// `client`'s environment into `session`'s, or marked to be removed
    /// when the client has none (tmux's `-NAME`). Run when a session is
    /// created or a client attaches or switches to it, unless `-E`.
    pub(crate) fn update_environment(&mut self, session: SessionId, client: ClientId, host: &dyn Host) {
        let names = self.sopt(Some(session), "update-environment").as_str().to_string();
        if names.is_empty() {
            return;
        }
        let from = host.client_env(client);
        let Some(s) = self.sessions.get_mut(&session) else { return };
        for name in names.split_whitespace() {
            if name.contains('=') {
                continue;
            }
            let value = from.iter().find(|(k, _)| k == name).map(|(_, v)| v.clone());
            if name.len() + value.as_ref().map_or(0, |v| v.len()) > MAX_ENV_BYTES {
                continue;
            }
            if !s.environ.0.contains_key(name) && s.environ.0.len() >= MAX_ENV_VARS {
                break;
            }
            s.environ.0.insert(name.to_string(), Var { value, hidden: false });
        }
    }

    fn environ_of(&mut self, a: &Args, ctx: &CmdContext) -> Result<&mut Environ, CmdError> {
        if a.has('g') {
            return Ok(&mut self.environ);
        }
        let f = self.find_target(ctx, a.get('t'), Kind::Session, Flags::default())?;
        Ok(&mut self.sessions.get_mut(&f.s).expect("session").environ)
    }

    pub(crate) fn cmd_set_environment(&mut self, a: &Args, ctx: &CmdContext, host: &mut dyn Host) -> R {
        let name = a.value(0).unwrap_or("");
        if name.is_empty() {
            return Err(CmdError::new("empty variable name"));
        }
        if name.contains('=') {
            return Err(CmdError::new("variable name contains ="));
        }
        let value = match a.value(1) {
            Some(v) if a.has('F') => {
                let f = self.find_target(ctx, a.get('t'), Kind::Session, Flags::default()).ok();
                let cx = self.fctx(&*host, f.map(|f| f.s), f.and_then(|f| f.w), f.and_then(|f| f.p), ctx.client, ctx.now);
                Some(format::expand(v, &cx))
            }
            v => v.map(str::to_string),
        };
        let (unset, remove, hidden) = (a.has('u'), a.has('r'), a.has('h'));
        if (unset || remove) && value.is_some() {
            return Err(CmdError::new(if unset { "can't specify a value with -u" } else { "can't specify a value with -r" }));
        }
        if !unset && !remove && value.is_none() {
            return Err(CmdError::new("no value specified"));
        }
        if name.len() + value.as_ref().map_or(0, |v| v.len()) > MAX_ENV_BYTES {
            return Err(CmdError(format!("variable too large (more than {MAX_ENV_BYTES} bytes)")));
        }
        let env = self.environ_of(a, ctx)?;
        if unset {
            env.0.remove(name);
            return Ok(CmdOutput::default());
        }
        if !env.0.contains_key(name) && env.0.len() >= MAX_ENV_VARS {
            return Err(CmdError(format!("too many variables ({MAX_ENV_VARS})")));
        }
        env.0.insert(name.to_string(), Var { value, hidden });
        Ok(CmdOutput::default())
    }

    pub(crate) fn cmd_show_environment(&mut self, a: &Args, ctx: &CmdContext) -> R {
        let (shell, hidden) = (a.has('s'), a.has('h'));
        let env = self.environ_of(a, ctx)?;
        let line = |name: &str, v: &Var| match (&v.value, shell) {
            (Some(value), false) => format!("{name}={value}\n"),
            (None, false) => format!("-{name}\n"),
            (Some(value), true) => format!("{name}=\"{}\"; export {name};\n", shell_escape(value)),
            (None, true) => format!("unset {name};\n"),
        };
        let mut out = String::new();
        match a.value(0) {
            Some(name) => match env.0.get(name) {
                Some(v) => out.push_str(&line(name, v)),
                None => return Err(CmdError(format!("unknown variable: {name}"))),
            },
            None => {
                for (name, v) in env.0.iter().filter(|(_, v)| v.hidden == hidden) {
                    out.push_str(&line(name, v));
                }
            }
        }
        Ok(CmdOutput { stdout: out, ..CmdOutput::default() })
    }
}
