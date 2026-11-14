//! The command language: tmux command names (with aliases and unambiguous
//! prefixes), getopt-style flags, argv-form command lists (`;` separated)
//! and the command-string syntax typed at the `:` prompt or used in
//! `bind-key` (quoting, escapes, `;`, `{ ... }` blocks, comments).

mod lexer;
mod table;

use std::fmt;
pub use table::CmdEntry;
pub(crate) use table::lookup_entry;

/// Every command lif implements, in name order.
pub fn supported_entries() -> impl Iterator<Item = &'static CmdEntry> {
    table::supported()
}

/// A command parse or execution error, with tmux's message text.
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct CmdError(pub String);

impl CmdError {
    pub(crate) fn new(s: impl Into<String>) -> CmdError {
        CmdError(s.into())
    }
}

impl fmt::Display for CmdError {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        f.write_str(&self.0)
    }
}

impl std::error::Error for CmdError {}

impl From<String> for CmdError {
    fn from(s: String) -> Self {
        CmdError(s)
    }
}

/// One positional argument: a string, or a `{ ... }` block of commands.
#[derive(Clone, Debug, PartialEq, Eq)]
pub enum Arg {
    /// A plain word.
    Str(String),
    /// A `{ ... }` block, parsed.
    Block(CommandList),
}

/// Parsed flags and positional arguments of one command.
#[derive(Clone, Debug, Default, PartialEq, Eq)]
pub struct Args {
    flags: Vec<(char, Option<String>)>,
    values: Vec<Arg>,
}

impl Args {
    /// Whether flag `f` was given.
    pub fn has(&self, f: char) -> bool {
        self.flags.iter().any(|(c, _)| *c == f)
    }

    /// How many times flag `f` was given.
    pub fn count(&self, f: char) -> usize {
        self.flags.iter().filter(|(c, _)| *c == f).count()
    }

    /// The value of flag `f` (the last one if repeated).
    pub fn get(&self, f: char) -> Option<&str> {
        self.flags.iter().rev().find(|(c, _)| *c == f).and_then(|(_, v)| v.as_deref())
    }

    /// Positional arguments.
    pub fn values(&self) -> &[Arg] {
        &self.values
    }

    /// Positional argument `i` if it is a string.
    pub fn value(&self, i: usize) -> Option<&str> {
        match self.values.get(i) {
            Some(Arg::Str(s)) => Some(s),
            _ => None,
        }
    }

    /// Positional arguments from `from` on, as strings (blocks print as
    /// their command text).
    pub fn strings(&self, from: usize) -> Vec<String> {
        self.values
            .iter()
            .skip(from)
            .map(|a| match a {
                Arg::Str(s) => s.clone(),
                Arg::Block(b) => b.to_string(),
            })
            .collect()
    }

    fn parse(entry: &CmdEntry, input: Vec<Arg>) -> Result<Args, String> {
        let mut args = Args::default();
        let mut it = input.into_iter().peekable();
        while let Some(Arg::Str(s)) = it.peek() {
            let Some(body) = s.strip_prefix('-') else { break };
            if body.is_empty() {
                break;
            }
            let body = body.to_string();
            it.next();
            if body == "-" {
                break;
            }
            for (i, f) in body.char_indices() {
                if !f.is_ascii_alphanumeric() {
                    return Err(format!("invalid flag -{f}"));
                }
                let Some(pos) = entry.template.find(f) else {
                    return Err(format!("unknown flag -{f}"));
                };
                if entry.template[pos + 1..].starts_with(':') {
                    let rest = &body[i + f.len_utf8()..];
                    let v = if !rest.is_empty() {
                        rest.to_string()
                    } else {
                        match it.next() {
                            Some(Arg::Str(v)) => v,
                            Some(Arg::Block(b)) => b.to_string(),
                            None => return Err(format!("-{f} expects an argument")),
                        }
                    };
                    args.flags.push((f, Some(v)));
                    break;
                }
                args.flags.push((f, None));
            }
        }
        args.values = it.collect();
        let n = args.values.len();
        if n < entry.lower {
            return Err(format!("too few arguments (need at least {})", entry.lower));
        }
        if let Some(upper) = entry.upper
            && n > upper
        {
            return Err(format!("too many arguments (need at most {upper})"));
        }
        Ok(args)
    }
}

/// One parsed command.
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct Command {
    entry: &'static CmdEntry,
    args: Args,
}

impl Command {
    /// The canonical command name (`split-window` for `splitw`).
    pub fn name(&self) -> &'static str {
        self.entry.name
    }

    /// The command's table entry.
    pub fn entry(&self) -> &'static CmdEntry {
        self.entry
    }

    /// Its arguments.
    pub fn args(&self) -> &Args {
        &self.args
    }

    fn build(words: Vec<Arg>) -> Result<Command, CmdError> {
        let mut it = words.into_iter();
        let name = match it.next() {
            Some(Arg::Str(s)) => s,
            Some(Arg::Block(_)) => return Err(CmdError::new("syntax error")),
            None => return Err(CmdError::new("no command")),
        };
        let entry = lookup_entry(&name)?;
        if !entry.supported {
            return Err(CmdError(format!("unsupported command: {}", entry.name)));
        }
        let args = Args::parse(entry, it.collect()).map_err(|e| CmdError(format!("command {}: {e}", entry.name)))?;
        Ok(Command { entry, args })
    }
}

/// A sequence of commands run one after another (`a ; b`).
#[derive(Clone, Debug, Default, PartialEq, Eq)]
pub struct CommandList(pub Vec<Command>);

impl CommandList {
    /// The commands.
    pub fn commands(&self) -> &[Command] {
        &self.0
    }

    /// Whether the list has no commands.
    pub fn is_empty(&self) -> bool {
        self.0.is_empty()
    }

    /// Print the list; top-level separators are `\;` when `escaped` (how
    /// `list-keys` shows a binding), `;` otherwise.
    pub fn print(&self, escaped: bool) -> String {
        let sep = if escaped { " \\; " } else { " ; " };
        self.0.iter().map(|c| c.to_string()).collect::<Vec<_>>().join(sep)
    }
}

/// Parse one command from argv form (`["split-window", "-h"]`). The command
/// name may be an alias or unique prefix. Errors if `argv` holds several
/// `;`-separated commands; use [`parse_list`] for those.
pub fn parse(argv: &[String]) -> Result<Command, CmdError> {
    let mut list = parse_list(argv)?;
    match list.0.len() {
        0 => Err(CmdError::new("no command")),
        1 => Ok(list.0.pop().expect("one")),
        _ => Err(CmdError::new("more than one command")),
    }
}

/// Parse argv form with tmux's `;` rules: an argument that is `;`, or ends in
/// an unescaped `;`, ends a command; a trailing `\;` is a literal `;`.
pub fn parse_list(argv: &[String]) -> Result<CommandList, CmdError> {
    let mut list = Vec::new();
    let mut cur: Vec<Arg> = Vec::new();
    for a in argv {
        if let Some(stripped) = a.strip_suffix(';') {
            if let Some(lit) = stripped.strip_suffix('\\') {
                cur.push(Arg::Str(format!("{lit};")));
                continue;
            }
            if !stripped.is_empty() {
                cur.push(Arg::Str(stripped.to_string()));
            }
            if !cur.is_empty() {
                list.push(Command::build(std::mem::take(&mut cur))?);
            }
            continue;
        }
        cur.push(Arg::Str(a.clone()));
    }
    if !cur.is_empty() {
        list.push(Command::build(cur)?);
    }
    Ok(CommandList(list))
}

/// Parse a command string as typed at the `:` prompt or found in a
/// configuration line: words separated by blanks; `'...'` and `"..."`
/// quoting; backslash escapes; `;` or a newline between commands; `{ ... }`
/// blocks; `#` comments (but `#{...}` formats are words).
///
/// Not supported (left literal): `$VAR` and `~` expansion, `%if` directives.
pub fn parse_string(s: &str) -> Result<CommandList, CmdError> {
    let words = lexer::lex(s).map_err(CmdError)?;
    to_list(words)
}

fn to_list(cmds: Vec<Vec<lexer::Word>>) -> Result<CommandList, CmdError> {
    let mut list = Vec::new();
    for words in cmds {
        let mut args = Vec::with_capacity(words.len());
        for w in words {
            args.push(match w {
                lexer::Word::Str(s) => Arg::Str(s),
                lexer::Word::Block(inner) => Arg::Block(to_list(inner)?),
            });
        }
        if !args.is_empty() {
            list.push(Command::build(args)?);
        }
    }
    Ok(CommandList(list))
}

/// Build the command list stored by `bind-key` / run by `confirm-before`
/// from positional arguments: one block, one string (parsed as a command
/// string), or several strings (argv form).
pub(crate) fn commands_from_args(values: &[Arg]) -> Result<CommandList, CmdError> {
    match values {
        [Arg::Block(b)] => Ok(b.clone()),
        [Arg::Str(s)] => parse_string(s),
        _ => {
            let argv: Vec<String> = values
                .iter()
                .map(|a| match a {
                    Arg::Str(s) => s.clone(),
                    Arg::Block(b) => b.to_string(),
                })
                .collect();
            parse_list(&argv)
        }
    }
}

/// Quote a string the way tmux prints arguments (`args_escape`): an empty
/// string becomes `''`, a single special character gets a backslash,
/// strings with blanks or any of `#';${}%` (or `$`) are double-quoted,
/// strings with only `"` single-quoted (unless they also hold a backslash
/// or a control character: single quotes take those literally, so such
/// strings are double-quoted with escapes instead). The result always reads
/// back as `s` (see the round-trip test).
pub fn escape_arg(s: &str) -> String {
    const DQUOTED: &str = " #';${}%";
    const SQUOTED: &str = " \"";
    if s.is_empty() {
        return "''".to_string();
    }
    let needs_escape = s.contains(|c: char| c == '\\' || (c as u32) < 0x20 || c == '\x7f');
    let quotes = if s.contains(|c| DQUOTED.contains(c)) {
        '"'
    } else if s.contains(|c| SQUOTED.contains(c)) {
        if needs_escape { '"' } else { '\'' }
    } else {
        '\0'
    };
    let mut chars = s.chars();
    let first = chars.next().unwrap_or(' ');
    if first != ' ' && chars.next().is_none() && (quotes != '\0' || first == '~') {
        return format!("\\{first}");
    }
    let mut body = String::with_capacity(s.len() + 2);
    for c in s.chars() {
        match c {
            '\n' => body.push_str("\\n"),
            '\t' => body.push_str("\\t"),
            '\r' => body.push_str("\\r"),
            '\\' => body.push_str("\\\\"),
            '"' if quotes == '"' => body.push_str("\\\""),
            '$' if quotes == '"' => body.push_str("\\$"),
            c if (c as u32) < 0x20 || c == '\x7f' => body.push_str(&format!("\\{:03o}", c as u32)),
            c => body.push(c),
        }
    }
    let tilde = body.starts_with('~');
    match quotes {
        '\'' => format!("'{body}'"),
        '"' if tilde => format!("\"\\{body}\""),
        '"' => format!("\"{body}\""),
        _ if tilde => format!("\\{body}"),
        _ => body,
    }
}

impl fmt::Display for Args {
    /// tmux's `args_print`: flags without values grouped (`-dh`), then flags
    /// with values, then positional arguments, each escaped.
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        let mut flags: Vec<&(char, Option<String>)> = self.flags.iter().collect();
        flags.sort_by_key(|(c, _)| *c);
        let mut out = String::new();
        for (c, v) in &flags {
            if v.is_none() {
                if out.is_empty() {
                    out.push('-');
                }
                out.push(*c);
            }
        }
        for (c, v) in &flags {
            if let Some(v) = v {
                if !out.is_empty() {
                    out.push(' ');
                }
                out.push('-');
                out.push(*c);
                out.push(' ');
                out.push_str(&escape_arg(v));
            }
        }
        for v in &self.values {
            if !out.is_empty() {
                out.push(' ');
            }
            match v {
                Arg::Str(s) => out.push_str(&escape_arg(s)),
                Arg::Block(b) => {
                    out.push_str("{ ");
                    out.push_str(&b.print(false));
                    out.push_str(" }");
                }
            }
        }
        f.write_str(&out)
    }
}

impl fmt::Display for Command {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        let args = self.args.to_string();
        if args.is_empty() { f.write_str(self.entry.name) } else { write!(f, "{} {args}", self.entry.name) }
    }
}

impl fmt::Display for CommandList {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        f.write_str(&self.print(false))
    }
}

#[cfg(test)]
mod tests;
