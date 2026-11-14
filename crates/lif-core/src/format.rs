//! tmux format expansion: `#{variable}`, the short aliases (`#S` `#I` `#W`
//! `#P` `#T` `#F` `#H` `#h` `#D` `##`), conditionals `#{?cond,a,b}`,
//! comparisons (`#{==:a,b}` `!=` `<` `>` `<=` `>=` `||` `&&` `!`), and the
//! modifiers `=N` / `=-N` (truncate), `pN` (pad), `t:` (time), `b:` `d:`
//! (basename / dirname), `l:` (literal), `E:` / `T:` (expand an option),
//! `q:` (shell quote), `n:` / `w:` (length / width), `s/a/b/:` (literal
//! substitution), `m:` (glob match) and `e|op|:` (integer arithmetic).
//!
//! `#[style]` directives are copied through unchanged for the status line
//! renderer. `#(shell command)` expands to nothing: lif never runs commands
//! from formats.

use crate::time::{Tm, WallClock, ctime, strftime};

/// Where a format gets its values.
pub trait Vars {
    /// The value of a variable, or `None` if it is unknown (expands to "").
    fn var(&self, name: &str) -> Option<String>;
    /// The current wall-clock time, for `t:`.
    fn now(&self) -> WallClock;
}

const MAX_DEPTH: u32 = 32;

/// Expand `fmt`.
pub fn expand(fmt: &str, vars: &dyn Vars) -> String {
    let mut out = String::with_capacity(fmt.len());
    expand_into(fmt, vars, &mut out, 0);
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

fn expand_into(fmt: &str, vars: &dyn Vars, out: &mut String, depth: u32) {
    if depth > MAX_DEPTH {
        return;
    }
    let b = fmt.as_bytes();
    let mut i = 0;
    let mut lit = 0;
    while i < b.len() {
        if b[i] != b'#' || i + 1 >= b.len() {
            i += 1;
            continue;
        }
        out.push_str(&fmt[lit..i]);
        let c = b[i + 1];
        match c {
            b'#' | b',' | b'}' | b':' => {
                out.push(c as char);
                i += 2;
            }
            b'{' => match skip_to(b, i, b"}") {
                Some(end) => {
                    expand_var(&fmt[i + 2..end], vars, out, depth + 1);
                    i = end + 1;
                }
                None => {
                    out.push_str(&fmt[i..]);
                    i = b.len();
                }
            },
            b'[' => {
                let end = fmt[i..].find(']').map(|e| i + e + 1).unwrap_or(b.len());
                out.push_str(&fmt[i..end]);
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
                i = (j + 1).min(b.len());
            }
            _ => {
                let ch = fmt[i + 1..].chars().next().unwrap_or('#');
                match short_alias(ch) {
                    Some(name) => out.push_str(&vars.var(name).unwrap_or_default()),
                    None => {
                        out.push('#');
                        out.push(ch);
                    }
                }
                i += 1 + ch.len_utf8();
            }
        }
        lit = i;
    }
    out.push_str(&fmt[lit..]);
}

fn sub(s: &str, vars: &dyn Vars, depth: u32) -> String {
    let mut out = String::new();
    expand_into(s, vars, &mut out, depth);
    out
}

/// Value of a condition or bare name: a format if it contains `#`,
/// otherwise a variable.
fn value_of(s: &str, vars: &dyn Vars, depth: u32) -> String {
    if s.contains('#') { sub(s, vars, depth) } else { vars.var(s).unwrap_or_default() }
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

fn expand_var(content: &str, vars: &dyn Vars, out: &mut String, depth: u32) {
    // Conditional: #{?c1,a,c2,b,...,else}
    if let Some(rest) = content.strip_prefix('?') {
        let parts = split_args(rest);
        let mut k = 0;
        while k < parts.len() {
            if k + 1 >= parts.len() {
                expand_into(parts[k], vars, out, depth);
                return;
            }
            if is_true(&value_of(parts[k], vars, depth)) {
                expand_into(parts[k + 1], vars, out, depth);
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
            let (a, b) = (sub(parts[0], vars, depth), sub(parts[1], vars, depth));
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
            out.push(if r { '1' } else { '0' });
            return;
        }
    }
    if let Some(rest) = content.strip_prefix("!:") {
        out.push(if is_true(&sub(rest, vars, depth)) { '0' } else { '1' });
        return;
    }
    if let Some(rest) = content.strip_prefix("m:") {
        let parts = split_args(rest);
        if parts.len() == 2 {
            let (pat, s) = (sub(parts[0], vars, depth), sub(parts[1], vars, depth));
            out.push(if glob(pat.as_bytes(), s.as_bytes()) { '1' } else { '0' });
        }
        return;
    }
    if let Some(rest) = content.strip_prefix("e|") {
        let Some((op, args)) = rest.split_once("|:") else { return };
        let parts = split_args(args);
        if parts.len() != 2 {
            return;
        }
        let a: i64 = sub(parts[0], vars, depth).trim().parse().unwrap_or(0);
        let b: i64 = sub(parts[1], vars, depth).trim().parse().unwrap_or(0);
        let v = match op {
            "+" => a.wrapping_add(b),
            "-" => a.wrapping_sub(b),
            "*" => a.wrapping_mul(b),
            "/" if b != 0 => a / b,
            "m" | "%" if b != 0 => a % b,
            "==" => (a == b) as i64,
            "!=" => (a != b) as i64,
            "<" => (a < b) as i64,
            ">" => (a > b) as i64,
            "<=" => (a <= b) as i64,
            ">=" => (a >= b) as i64,
            _ => 0,
        };
        out.push_str(&v.to_string());
        return;
    }
    // Modifier chain "m1;m2:name".
    let (mods, name) = match parse_modifiers(content, vars, depth) {
        Some(x) => x,
        None => (Vec::new(), content),
    };
    let literal = mods.iter().any(|m| matches!(m, Modifier::Literal));
    let mut value = if literal { name.to_string() } else { String::new() };
    if !literal {
        let expand_opt = mods.iter().any(|m| matches!(m, Modifier::Expand | Modifier::ExpandTime));
        let raw = vars.var(name).unwrap_or_default();
        value = if expand_opt {
            if mods.iter().any(|m| matches!(m, Modifier::ExpandTime)) {
                expand_time(&raw, vars)
            } else {
                sub(&raw, vars, depth)
            }
        } else {
            raw
        };
    }
    for m in &mods {
        match m {
            Modifier::Time => {
                if let Ok(t) = value.parse::<i64>() {
                    value = ctime(&Tm::from_clock(vars.now().at(t)));
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
            Modifier::Subst(a, b) if !a.is_empty() => value = value.replace(a.as_str(), b),
            _ => {}
        }
    }
    for m in &mods {
        match m {
            Modifier::Truncate(n, marker) => {
                let count = value.chars().count() as i64;
                if *n > 0 && count > *n {
                    value = value.chars().take(*n as usize).collect::<String>() + marker;
                } else if *n < 0 && count > -n {
                    value = marker.clone() + &value.chars().skip((count + n) as usize).collect::<String>();
                }
            }
            Modifier::Pad(n) => {
                let count = value.chars().count() as i64;
                let want = n.abs();
                if count < want {
                    let pad = " ".repeat((want - count) as usize);
                    value = if *n > 0 { value + &pad } else { pad + &value };
                }
            }
            Modifier::Length => value = value.chars().count().to_string(),
            Modifier::Width => value = crate::status::text_width(&value).to_string(),
            _ => {}
        }
    }
    out.push_str(&value);
}

fn parse_modifiers<'a>(content: &'a str, vars: &dyn Vars, depth: u32) -> Option<(Vec<Modifier>, &'a str)> {
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
        let arg_num = |s: &str| -> Option<i64> { sub(s, vars, depth).trim().parse().ok() };
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
    while si < s.len() {
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
                    if let Some(end) = p[pi + 1..].iter().position(|c| *c == b']').map(|e| pi + 1 + e) {
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
