//! tmux format expansion: `#{variable}`, the short aliases (`#S` `#I` `#W`
//! `#P` `#T` `#F` `#H` `#h` `#D` `##`), conditionals `#{?cond,a,b}`,
//! comparisons (`#{==:a,b}` `!=` `<` `>` `<=` `>=` `||` `&&` `!`), and the
//! modifiers `=N` / `=-N` (truncate), `pN` (pad), `t:` (time), `b:` `d:`
//! (basename / dirname), `l:` (literal), `E:` / `T:` (expand an option),
//! `q:` (shell quote), `n:` / `w:` (length / width), `s/a/b/:` (literal
//! substitution), `m:` (glob match) and `e|op|:` (integer arithmetic).
//!
//! `#[style]` directives are copied through unchanged for the status line
//! renderer. `#(shell command)` expands to what [`Vars::shell`] gives for
//! the command (itself expanded first): the last line of its previous run.

use crate::time::{Tm, WallClock, ctime, strftime};

/// Where a format gets its values.
pub trait Vars {
    /// The value of a variable, or `None` if it is unknown (expands to "").
    fn var(&self, name: &str) -> Option<String>;
    /// The current wall-clock time, for `t:`.
    fn now(&self) -> WallClock;
    /// `#(command)`: the last line `command` printed when it last ran
    /// (it is run again in the background), or `None` where formats do
    /// not run commands.
    fn shell(&self, command: &str) -> Option<String> {
        let _ = command;
        None
    }
}

/// How deeply `#{...}` (and `E:`/`T:` option expansions) may nest; deeper
/// parts expand to nothing. Bounds the recursion, so the stack.
const MAX_DEPTH: u32 = 32;

/// Most `#{...}` expansions one call of [`expand`] performs; the rest of the
/// format expands to nothing. A status line uses a few dozen. Without it a
/// self-referencing option (`set @x '#{E:@x}#{E:@x}'`) does 2^32 expansions
/// within the depth limit, on every redraw if it is in the status line.
pub const MAX_STEPS: u32 = 100_000;

/// Most bytes one call of [`expand`] produces or copies out of variables,
/// beyond the length of the format itself; output past it is cut off.
/// Real formats (a status line, a list line) produce well under a kilobyte;
/// 256 KiB still shows a large user option whole, and bounds what
/// padding (`#{p99999999999:x}`), substitution and option expansion can
/// allocate, also when a list command expands a format for each of
/// thousands of items or a status line once per window.
pub const MAX_BYTES: usize = 256 << 10;

/// Most comparison steps of one [`glob`] match. The match backtracks to the
/// last `*`, so it costs up to pattern length times string length; real
/// patterns against names take a few hundred steps. A match that would take
/// longer fails.
const MAX_GLOB_STEPS: u64 = 10_000_000;

/// Expand `fmt`.
pub fn expand(fmt: &str, vars: &dyn Vars) -> String {
    let mut out = String::with_capacity(fmt.len().min(MAX_BYTES));
    let mut cx = Cx::new(vars, fmt.len());
    expand_into(fmt, &mut cx, &mut out, 0);
    out
}

/// Expand strftime conversions, then the format (`display-message` and the
/// status line do this).
pub fn expand_time(fmt: &str, vars: &dyn Vars) -> String {
    if fmt.contains('%') {
        let tm = Tm::from_clock(vars.now());
        expand(&strftime(fmt, &tm), vars)
    } else {
        expand(fmt, vars)
    }
}

/// tmux's truth test: non-empty and not "0".
pub fn is_true(s: &str) -> bool {
    !s.is_empty() && s != "0"
}

/// One expansion's variables and what is left of its budgets.
struct Cx<'a> {
    vars: &'a dyn Vars,
    steps: u32,
    bytes: usize,
}

impl Cx<'_> {
    fn new(vars: &dyn Vars, fmt_len: usize) -> Cx<'_> {
        Cx { vars, steps: MAX_STEPS, bytes: MAX_BYTES.saturating_add(fmt_len) }
    }

    fn exhausted(&self) -> bool {
        self.steps == 0 || self.bytes == 0
    }

    /// Take one expansion step; false once the budget is spent.
    fn step(&mut self) -> bool {
        if self.exhausted() {
            return false;
        }
        self.steps -= 1;
        true
    }

    /// Append `s` to `out`, cut (at a character boundary) to what is left
    /// of the byte budget.
    fn push(&mut self, out: &mut String, s: &str) {
        let s = cut(s, self.bytes);
        self.bytes -= s.len();
        out.push_str(s);
    }

    fn push_char(&mut self, out: &mut String, c: char) {
        let mut b = [0u8; 4];
        self.push(out, c.encode_utf8(&mut b));
    }

    /// A variable's value, charged to the byte budget: fetching it copies
    /// it, however little of it ends up in the output (`#{n:@big}`).
    fn var(&mut self, name: &str) -> String {
        let v = self.vars.var(name).unwrap_or_default();
        self.bytes = self.bytes.saturating_sub(v.len());
        v
    }
}

/// The longest prefix of `s` of at most `max` bytes that ends on a
/// character boundary.
fn cut(s: &str, max: usize) -> &str {
    if s.len() <= max {
        return s;
    }
    let mut end = max;
    while !s.is_char_boundary(end) {
        end -= 1;
    }
    &s[..end]
}

fn short_alias(c: char) -> Option<&'static str> {
    Some(match c {
        'D' => "pane_id",
        'F' => "window_flags",
        'H' => "host",
        'h' => "host_short",
        'I' => "window_index",
        'P' => "pane_index",
        'S' => "session_name",
        'T' => "pane_title",
        'W' => "window_name",
        _ => return None,
    })
}

/// Index just past the `}` matching a `#{` that starts at `start`, honouring
/// `#{` nesting and `#,` `##` `#}` `#:` escapes (tmux's format_skip).
fn skip_to(s: &[u8], start: usize, ends: &[u8]) -> Option<usize> {
    let mut depth = 0i32;
    let mut i = start;
    while i < s.len() {
        if s[i] == b'#' && i + 1 < s.len() {
            if s[i + 1] == b'{' {
                depth += 1;
            }
            if b",#{}:".contains(&s[i + 1]) {
                i += 2;
                continue;
            }
        }
        if s[i] == b'}' {
            depth -= 1;
        }
        if ends.contains(&s[i]) && depth == 0 {
            return Some(i);
        }
        i += 1;
    }
    None
}

/// Split at top-level commas.
fn split_args(s: &str) -> Vec<&str> {
    let b = s.as_bytes();
    let mut parts = Vec::new();
    let mut start = 0;
    let mut i = 0;
    while let Some(j) = skip_to(b, i, b",") {
        parts.push(&s[start..j]);
        start = j + 1;
        i = j + 1;
    }
    parts.push(&s[start..]);
    parts
}

fn expand_into(fmt: &str, cx: &mut Cx, out: &mut String, depth: u32) {
    if depth > MAX_DEPTH {
        return;
    }
    let b = fmt.as_bytes();
    let mut i = 0;
    let mut lit = 0;
    while i < b.len() {
        if cx.exhausted() {
            return;
        }
        if b[i] != b'#' || i + 1 >= b.len() {
            i += 1;
            continue;
        }
        cx.push(out, &fmt[lit..i]);
        let c = b[i + 1];
        match c {
            b'#' | b',' | b'}' | b':' => {
                cx.push_char(out, c as char);
                i += 2;
            }
            b'{' => match skip_to(b, i, b"}") {
                Some(end) => {
                    if cx.step() {
                        expand_var(&fmt[i + 2..end], cx, out, depth + 1);
                    }
                    i = end + 1;
                }
                None => {
                    cx.push(out, &fmt[i..]);
                    i = b.len();
                }
            },
            b'[' => {
                let end = fmt[i..].find(']').map(|e| i + e + 1).unwrap_or(b.len());
                cx.push(out, &fmt[i..end]);
                i = end;
            }
            b'(' => {
                let mut nest = 0;
                let mut j = i + 1;
                while j < b.len() {
                    match b[j] {
                        b'(' => nest += 1,
                        b')' => {
                            nest -= 1;
                            if nest == 0 {
                                break;
                            }
                        }
                        _ => {}
                    }
                    j += 1;
                }
                if j < b.len() && cx.step() {
                    let command = sub(&fmt[i + 2..j], cx, depth + 1);
                    if let Some(v) = cx.vars.shell(&command) {
                        cx.push(out, &v);
                    }
                }
                i = (j + 1).min(b.len());
            }
            _ => {
                let ch = fmt[i + 1..].chars().next().unwrap_or('#');
                match short_alias(ch) {
                    Some(name) => {
                        if cx.step() {
                            let v = cx.var(name);
                            cx.push(out, &v);
                        }
                    }
                    None => {
                        cx.push_char(out, '#');
                        cx.push_char(out, ch);
                    }
                }
                i += 1 + ch.len_utf8();
            }
        }
        lit = i;
    }
    cx.push(out, &fmt[lit..]);
}

fn sub(s: &str, cx: &mut Cx, depth: u32) -> String {
    let mut out = String::new();
    expand_into(s, cx, &mut out, depth);
    out
}

/// Value of a condition or bare name: a format if it contains `#`,
/// otherwise a variable.
fn value_of(s: &str, cx: &mut Cx, depth: u32) -> String {
    if s.contains('#') { sub(s, cx, depth) } else { cx.var(s) }
}

enum Modifier {
    Truncate(i64, String),
    Pad(i64),
    Time,
    Basename,
    Dirname,
    Literal,
    Expand,
    ExpandTime,
    Quote,
    Length,
    Width,
    Subst(String, String),
}

/// `#{e|op|:a,b}`. Overflow saturates (`i64::MIN / -1` is `i64::MAX`) and
/// division by zero gives 0, as before: a format has no way to report an
/// error, and a panic would take the server down.
fn arithmetic(op: &str, a: i64, b: i64) -> i64 {
    match op {
        "+" => a.saturating_add(b),
        "-" => a.saturating_sub(b),
        "*" => a.saturating_mul(b),
        "/" if b != 0 => a.checked_div(b).unwrap_or(i64::MAX),
        // The only overflowing case, i64::MIN % -1, is 0 mathematically.
        "m" | "%" if b != 0 => a.checked_rem(b).unwrap_or(0),
        "==" => (a == b) as i64,
        "!=" => (a != b) as i64,
        "<" => (a < b) as i64,
        ">" => (a > b) as i64,
        "<=" => (a <= b) as i64,
        ">=" => (a >= b) as i64,
        _ => 0,
    }
}

fn expand_var(content: &str, cx: &mut Cx, out: &mut String, depth: u32) {
    // Conditional: #{?c1,a,c2,b,...,else}
    if let Some(rest) = content.strip_prefix('?') {
        let parts = split_args(rest);
        let mut k = 0;
        while k < parts.len() {
            if k + 1 >= parts.len() {
                expand_into(parts[k], cx, out, depth);
                return;
            }
            if is_true(&value_of(parts[k], cx, depth)) {
                expand_into(parts[k + 1], cx, out, depth);
                return;
            }
            k += 2;
        }
        return;
    }
    for (op, n) in [("==:", 3), ("!=:", 3), ("<=:", 3), (">=:", 3), ("||:", 3), ("&&:", 3), ("<:", 2), (">:", 2)] {
        if let Some(rest) = content.strip_prefix(op) {
            let parts = split_args(rest);
            if parts.len() != 2 {
                return;
            }
            let (a, b) = (sub(parts[0], cx, depth), sub(parts[1], cx, depth));
            let r = match &op[..n - 1] {
                "==" => a == b,
                "!=" => a != b,
                "<=" => a <= b,
                ">=" => a >= b,
                "||" => is_true(&a) || is_true(&b),
                "&&" => is_true(&a) && is_true(&b),
                "<" => a < b,
                _ => a > b,
            };
            cx.push_char(out, if r { '1' } else { '0' });
            return;
        }
    }
    if let Some(rest) = content.strip_prefix("!:") {
        let v = sub(rest, cx, depth);
        cx.push_char(out, if is_true(&v) { '0' } else { '1' });
        return;
    }
    if let Some(rest) = content.strip_prefix("m:") {
        let parts = split_args(rest);
        if parts.len() == 2 {
            let (pat, s) = (sub(parts[0], cx, depth), sub(parts[1], cx, depth));
            cx.push_char(out, if glob(pat.as_bytes(), s.as_bytes()) { '1' } else { '0' });
        }
        return;
    }
    if let Some(rest) = content.strip_prefix("e|") {
        let Some((op, args)) = rest.split_once("|:") else { return };
        let parts = split_args(args);
        if parts.len() != 2 {
            return;
        }
        let a: i64 = sub(parts[0], cx, depth).trim().parse().unwrap_or(0);
        let b: i64 = sub(parts[1], cx, depth).trim().parse().unwrap_or(0);
        cx.push(out, &arithmetic(op, a, b).to_string());
        return;
    }
    // Modifier chain "m1;m2:name".
    let (mods, name) = match parse_modifiers(content, cx, depth) {
        Some(x) => x,
        None => (Vec::new(), content),
    };
    let literal = mods.iter().any(|m| matches!(m, Modifier::Literal));
    let mut value = if literal { name.to_string() } else { String::new() };
    if !literal {
        let raw = cx.var(name);
        value = if mods.iter().any(|m| matches!(m, Modifier::ExpandTime)) {
            // Expanded here rather than with expand_time so the depth and
            // the budgets carry on: `set @x '#{T:@x}'` must not recurse
            // without end.
            let timed = if raw.contains('%') { strftime(&raw, &Tm::from_clock(cx.vars.now())) } else { raw };
            sub(&timed, cx, depth)
        } else if mods.iter().any(|m| matches!(m, Modifier::Expand)) {
            sub(&raw, cx, depth)
        } else {
            raw
        };
    }
    for m in &mods {
        match m {
            Modifier::Time => {
                if let Ok(t) = value.parse::<i64>() {
                    value = ctime(&Tm::from_clock(cx.vars.now().at(t)));
                }
            }
            Modifier::Basename => {
                let trimmed = value.trim_end_matches('/');
                value = match trimmed.rfind('/') {
                    Some(i) => trimmed[i + 1..].to_string(),
                    None if trimmed.is_empty() && !value.is_empty() => "/".to_string(),
                    None => trimmed.to_string(),
                };
            }
            Modifier::Dirname => {
                value = match value.trim_end_matches('/').rfind('/') {
                    Some(0) => "/".to_string(),
                    Some(i) => value[..i].to_string(),
                    None => ".".to_string(),
                };
            }
            Modifier::Quote => value = shell_quote(&value),
            Modifier::Subst(a, b) if !a.is_empty() => value = replace_capped(&value, a, b, cx.bytes),
            _ => {}
        }
    }
    for m in &mods {
        match m {
            Modifier::Truncate(n, marker) => {
                let count = value.chars().count() as u64;
                let keep = n.unsigned_abs();
                if count > keep {
                    let keep = keep as usize;
                    value = if *n > 0 {
                        value.chars().take(keep).collect::<String>() + marker
                    } else {
                        marker.clone() + &value.chars().skip(count as usize - keep).collect::<String>()
                    };
                }
            }
            Modifier::Pad(n) => {
                let count = value.chars().count() as u64;
                let want = n.unsigned_abs();
                if count < want {
                    // Never more padding than the output can still take.
                    let pad = " ".repeat((want - count).min(cx.bytes as u64) as usize);
                    value = if *n > 0 { value + &pad } else { pad + &value };
                }
            }
            Modifier::Length => value = value.chars().count().to_string(),
            Modifier::Width => value = crate::status::text_width(&value).to_string(),
            _ => {}
        }
    }
    cx.push(out, &value);
}

/// `s.replace(a, b)` producing at most `max` bytes (and so allocating at
/// most that): a short pattern with a long replacement multiplies the size.
fn replace_capped(s: &str, a: &str, b: &str, max: usize) -> String {
    let mut out = String::new();
    let mut last = 0;
    for (i, _) in s.match_indices(a) {
        for piece in [&s[last..i], b] {
            if out.len() + piece.len() > max {
                out.push_str(cut(piece, max - out.len()));
                return out;
            }
            out.push_str(piece);
        }
        last = i + a.len();
    }
    out.push_str(cut(&s[last..], max - out.len()));
    out
}

fn parse_modifiers<'a>(content: &'a str, cx: &mut Cx, depth: u32) -> Option<(Vec<Modifier>, &'a str)> {
    let b = content.as_bytes();
    let colon = skip_to(b, 0, b":")?;
    let spec = &content[..colon];
    let name = &content[colon + 1..];
    if spec.is_empty() {
        return None;
    }
    let mut mods = Vec::new();
    for m in spec.split(';') {
        let mut chars = m.chars();
        let c = chars.next()?;
        let rest = chars.as_str();
        let mut arg_num = |s: &str| -> Option<i64> { sub(s, cx, depth).trim().parse().ok() };
        let m = match c {
            '=' => {
                if let Some(r) = rest.strip_prefix('/') {
                    let mut it = r.splitn(2, '/');
                    let n = arg_num(it.next()?)?;
                    let marker = it.next().unwrap_or("").to_string();
                    Modifier::Truncate(n, marker)
                } else {
                    Modifier::Truncate(arg_num(rest)?, String::new())
                }
            }
            'p' => Modifier::Pad(arg_num(rest.trim_start_matches('/'))?),
            't' => Modifier::Time,
            'b' => Modifier::Basename,
            'd' => Modifier::Dirname,
            'l' => Modifier::Literal,
            'E' => Modifier::Expand,
            'T' => Modifier::ExpandTime,
            'q' => Modifier::Quote,
            'n' => Modifier::Length,
            'w' => Modifier::Width,
            's' => {
                let sep = rest.chars().next()?;
                let parts: Vec<&str> = rest[sep.len_utf8()..].split(sep).collect();
                if parts.len() < 2 {
                    return None;
                }
                Modifier::Subst(parts[0].to_string(), parts[1].to_string())
            }
            _ => return None,
        };
        mods.push(m);
    }
    Some((mods, name))
}

fn shell_quote(s: &str) -> String {
    let mut out = String::with_capacity(s.len());
    for c in s.chars() {
        if "|&;<>()$\u{60}\\\"' *?[#~=%".contains(c) {
            out.push('\\');
        }
        out.push(c);
    }
    out
}

/// fnmatch-style glob: `*`, `?`, `[...]`.
pub(crate) fn glob(p: &[u8], s: &[u8]) -> bool {
    let (mut pi, mut si) = (0, 0);
    let (mut star_p, mut star_s) = (usize::MAX, 0);
    let mut steps = 0u64;
    while si < s.len() {
        steps += 1;
        if steps > MAX_GLOB_STEPS {
            return false;
        }
        if pi < p.len() {
            match p[pi] {
                b'*' => {
                    star_p = pi;
                    star_s = si;
                    pi += 1;
                    continue;
                }
                b'?' => {
                    pi += 1;
                    si += 1;
                    continue;
                }
                b'[' => {
                    let close = p[pi + 1..].iter().position(|c| *c == b']');
                    // The scan for ']' is part of the cost of the step.
                    steps += close.unwrap_or(p.len() - pi) as u64;
                    if let Some(end) = close.map(|e| pi + 1 + e) {
                        let set = &p[pi + 1..end];
                        let (neg, set) = match set.first() {
                            Some(b'!') | Some(b'^') => (true, &set[1..]),
                            _ => (false, set),
                        };
                        let mut hit = false;
                        let mut k = 0;
                        while k < set.len() {
                            if k + 2 < set.len() && set[k + 1] == b'-' {
                                hit |= set[k] <= s[si] && s[si] <= set[k + 2];
                                k += 3;
                            } else {
                                hit |= set[k] == s[si];
                                k += 1;
                            }
                        }
                        if hit != neg {
                            pi = end + 1;
                            si += 1;
                            continue;
                        }
                    }
                }
                c if c == s[si] => {
                    pi += 1;
                    si += 1;
                    continue;
                }
                _ => {}
            }
        }
        if star_p == usize::MAX {
            return false;
        }
        pi = star_p + 1;
        star_s += 1;
        si = star_s;
    }
    p[pi..].iter().all(|c| *c == b'*')
}

#[cfg(test)]
mod tests {
    use super::*;
    use std::collections::HashMap;

    struct V(HashMap<&'static str, &'static str>);

    impl Vars for V {
        fn var(&self, name: &str) -> Option<String> {
            self.0.get(name).map(|s| s.to_string())
        }
        fn now(&self) -> WallClock {
            WallClock::new(1_794_682_220, 3 * 3600)
        }
    }

    fn v() -> V {
        V(HashMap::from([
            ("session_name", "work"),
            ("window_index", "1"),
            ("window_name", "vim"),
            ("window_flags", "*"),
            ("pane_index", "0"),
            ("pane_id", "%3"),
            ("pane_title", "a very long pane title that goes on"),
            ("pane_active", "1"),
            ("pane_dead", "0"),
            ("empty", ""),
            ("host", "box.local"),
            ("host_short", "box"),
            ("session_created", "1794682220"),
            ("path", "/usr/local/bin/"),
            ("status-left", "[#S]"),
            ("n", "7"),
        ]))
    }

    fn x(f: &str) -> String {
        expand(f, &v())
    }

    #[test]
    fn variables_and_aliases() {
        assert_eq!(x("#{session_name}:#{window_index}.#{pane_index}"), "work:1.0");
        assert_eq!(x("#S #I #W #P #F #D #H #h ##"), "work 1 vim 0 * %3 box.local box #");
        assert_eq!(x("#{nosuch}|#Z|#"), "|#Z|#");
        assert_eq!(x("#[fg=red]x#[default]"), "#[fg=red]x#[default]");
        assert_eq!(x("a#(echo hi)b"), "ab");
        assert_eq!(x("#{unterminated"), "#{unterminated");
    }

    #[test]
    fn conditionals_and_comparisons() {
        assert_eq!(x("#{?pane_active,yes,no}"), "yes");
        assert_eq!(x("#{?pane_dead,yes,no}"), "no");
        assert_eq!(x("#{?empty,yes,no}"), "no");
        assert_eq!(x("#{?window_flags,#{window_flags}, }"), "*");
        assert_eq!(x("#{?#{==:#{window_name},vim},V,O}"), "V");
        assert_eq!(x("#{?pane_dead,a,pane_active,b,c}"), "b");
        assert_eq!(x("#{?pane_active,a#,b,c}"), "a,b");
        assert_eq!(x("#{==:a,a}#{!=:a,a}#{<:a,b}#{>:a,b}#{||:0,1}#{&&:1,0}#{!:0}"), "1010101");
        assert_eq!(x("#{e|+|:#{n},3} #{e|*|:2,3} #{e|/|:7,0}"), "10 6 0");
        assert_eq!(x("#{m:*im,#{window_name}}#{m:x?,vim}"), "10");
    }

    #[test]
    fn modifiers() {
        assert_eq!(x("\"#{=21:pane_title}\""), "\"a very long pane titl\"");
        assert_eq!(x("#{=-5:pane_title}"), "es on");
        assert_eq!(x("#{=/4/...:pane_title}"), "a ve...");
        assert_eq!(x("#{p6:window_name}|#{p-6:window_name}"), "vim   |   vim");
        assert_eq!(x("#{t:session_created}"), "Sat Nov 14 21:50:20 2026");
        assert_eq!(x("#{b:path} #{d:path}"), "bin /usr/local");
        assert_eq!(x("#{l:#{session_name}}"), "#{session_name}");
        assert_eq!(x("#{E:status-left}"), "[work]");
        assert_eq!(x("#{n:window_name} #{w:window_name}"), "3 3");
        assert_eq!(x("#{s/vi/Vi/:window_name}"), "Vim");
        assert_eq!(x("#{q:pane_title}"), "a\\ very\\ long\\ pane\\ title\\ that\\ goes\\ on");
    }

    #[test]
    fn time_expansion() {
        assert_eq!(expand_time("%H:%M %d-%b-%y #S", &v()), "21:50 14-Nov-26 work");
    }

    #[test]
    fn globbing() {
        assert!(glob(b"*", b""));
        assert!(glob(b"a*c", b"abbbc"));
        assert!(!glob(b"a*c", b"abbb"));
        assert!(glob(b"[a-c]x", b"bx"));
        assert!(!glob(b"[!a-c]x", b"bx"));
        assert!(glob(b"w?rk", b"work"));
    }
}
