//! lif's configuration file: one small TOML file instead of tmux's command
//! language.
//!
//! The file is read once at server start and translated into ordinary lif
//! commands (`set-option -g ...`, `bind-key -T ...`, `unbind-key ...`), so a
//! config file means exactly what running those commands at startup means.
//! Every command is checked against a scratch [`lif_core::Model`] while loading,
//! so values lif-core would reject are reported here, with the file position
//! of the TOML key or value, instead of failing later.
//!
//! - [`Config::parse`] / [`Config::load_file`] / [`Config::load`] give a
//!   [`Config`]: the commands (each parseable by [`lif_core::parse_string`])
//!   plus every diagnostic. Problems never stop loading: whatever parsed is
//!   kept.
//! - [`Config::apply`] runs the commands on a model directly.
//! - [`resolve_path`] / [`default_path`] find the file.
//! - [`example`] is the documented starter file, [`migrate_tmux_conf`] a
//!   best-effort translator from `.tmux.conf`.
//!
//! The format is documented in `config.example.toml`.
//!
//! TOML is parsed by the `toml` crate with only its `parse` feature (no
//! serde, no proc macros); it adds about 83 KB to a release binary, measured
//! against an empty program with lif's release profile. The whole crate adds
//! about 195 KB to a program that already runs lif-core commands (about
//! 160 KB without the example text and the tmux.conf translator).
#![forbid(unsafe_code)]
#![warn(missing_docs)]

mod check;
mod schema;
mod style;
mod suggest;
mod tmux;

use lif_core::{CmdContext, Host, Model, WallClock, parse_string};
use std::ffi::OsStr;
use std::fmt;
use std::path::{Path, PathBuf};

pub use tmux::{Migration, Untranslated, looks_like_tmux_conf, migrate_tmux_conf};

/// The documented example configuration, every key explained.
pub const EXAMPLE: &str = include_str!("../config.example.toml");

/// The documented example configuration (for `lif` to print a starter file).
pub fn example() -> &'static str {
    EXAMPLE
}

/// How bad a diagnostic is.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum Severity {
    /// The setting was not applied.
    Error,
    /// The setting is valid but has no effect in this build.
    Warning,
}

/// A problem found in the file. `line` and `column` are 1-based (columns
/// count characters); both are 0 for problems with the file as a whole.
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct Diagnostic {
    /// Error or warning.
    pub severity: Severity,
    /// Line of the offending key or value.
    pub line: u32,
    /// Column of the offending key or value.
    pub column: u32,
    /// What is wrong, and what was expected.
    pub message: String,
}

impl fmt::Display for Diagnostic {
    /// `line:column: error: message`.
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        let sev = match self.severity {
            Severity::Error => "error",
            Severity::Warning => "warning",
        };
        if self.line == 0 {
            write!(f, "{sev}: {}", self.message)
        } else {
            write!(f, "{}:{}: {sev}: {}", self.line, self.column, self.message)
        }
    }
}

/// One command produced by the file, with the position of the setting it
/// came from.
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct Entry {
    /// A lif command string, parseable by [`lif_core::parse_string`].
    pub command: String,
    /// Line of the setting.
    pub line: u32,
    /// Column of the setting.
    pub column: u32,
}

/// A loaded configuration: the commands to run at startup and every
/// diagnostic.
#[derive(Clone, Debug, Default, PartialEq, Eq)]
pub struct Config {
    /// The file read, if any.
    pub path: Option<PathBuf>,
    /// Commands in the order they must run.
    pub commands: Vec<Entry>,
    /// Problems, in file order.
    pub diagnostics: Vec<Diagnostic>,
}

impl Config {
    /// Translate the text of a config file.
    pub fn parse(text: &str) -> Config {
        let (commands, diagnostics) = schema::translate(text);
        Config { path: None, commands, diagnostics }
    }

    /// Read and translate `path`. A missing file gives an empty config
    /// without diagnostics; an unreadable one a diagnostic.
    pub fn load_file(path: &Path) -> Config {
        let mut cfg = match std::fs::read(path) {
            Ok(bytes) => match String::from_utf8(bytes) {
                Ok(text) => Config::parse(&text),
                Err(e) => Config {
                    diagnostics: vec![file_error(format!("file is not valid UTF-8 (byte {})", e.utf8_error().valid_up_to()))],
                    ..Config::default()
                },
            },
            Err(e) if e.kind() == std::io::ErrorKind::NotFound => Config::default(),
            Err(e) => Config { diagnostics: vec![file_error(format!("cannot read file: {e}"))], ..Config::default() },
        };
        cfg.path = Some(path.to_path_buf());
        cfg
    }

    /// Find the file from the real environment ([`default_path`]) and load it.
    pub fn load() -> Config {
        match default_path() {
            Some(p) => Config::load_file(&p),
            None => Config::default(),
        }
    }

    /// The commands as plain strings (the server's startup commands).
    pub fn command_strings(&self) -> Vec<String> {
        self.commands.iter().map(|e| e.command.clone()).collect()
    }

    /// Whether any diagnostic is an error.
    pub fn has_errors(&self) -> bool {
        self.diagnostics.iter().any(|d| d.severity == Severity::Error)
    }

    /// Every diagnostic, one per line, prefixed with the file name
    /// (`~/.config/lif/config.toml:3:9: error: ...`); empty when there are none.
    pub fn report(&self) -> String {
        let name = self.path.as_ref().map(|p| p.display().to_string()).unwrap_or_else(|| "config.toml".to_string());
        let mut out = String::new();
        for d in &self.diagnostics {
            out.push_str(&name);
            out.push(':');
            out.push_str(&d.to_string());
            out.push('\n');
        }
        out
    }

    /// Run the commands on `model` (as the server would at startup). Commands
    /// that fail are reported at the position of their setting; the rest
    /// still run.
    pub fn apply(&self, model: &mut Model, host: &mut dyn Host) -> Vec<Diagnostic> {
        let ctx = CmdContext::none(WallClock::new(0, 0));
        let mut out = Vec::new();
        for e in &self.commands {
            let r = parse_string(&e.command).and_then(|list| model.execute_list(&list, &ctx, host));
            if let Err(err) = r {
                out.push(Diagnostic { severity: Severity::Error, line: e.line, column: e.column, message: err.0 });
            }
        }
        out
    }
}

fn file_error(message: String) -> Diagnostic {
    Diagnostic { severity: Severity::Error, line: 0, column: 0, message }
}

/// Where the config file is: `$LIF_CONFIG`, else
/// `$XDG_CONFIG_HOME/lif/config.toml`, else `$HOME/.config/lif/config.toml`.
/// Empty values count as unset, and a relative `XDG_CONFIG_HOME` is ignored
/// as the XDG spec requires. `None` when nothing usable is set.
pub fn resolve_path(lif_config: Option<&OsStr>, xdg_config_home: Option<&OsStr>, home: Option<&OsStr>) -> Option<PathBuf> {
    let set = |v: Option<&OsStr>| v.filter(|v| !v.is_empty()).map(PathBuf::from);
    if let Some(p) = set(lif_config) {
        return Some(p);
    }
    if let Some(x) = set(xdg_config_home).filter(|p| p.is_absolute()) {
        return Some(x.join("lif").join("config.toml"));
    }
    set(home).map(|h| h.join(".config").join("lif").join("config.toml"))
}

/// [`resolve_path`] with the process environment.
pub fn default_path() -> Option<PathBuf> {
    let var = std::env::var_os;
    resolve_path(var("LIF_CONFIG").as_deref(), var("XDG_CONFIG_HOME").as_deref(), var("HOME").as_deref())
}
