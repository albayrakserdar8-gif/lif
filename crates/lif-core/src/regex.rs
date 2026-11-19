//! A small regular expression engine: copy-mode search (`lif-copy`),
//! `find-window -r` and the `#{m/r:...}` format. It lives here, not in
//! `lif-copy`, because `lif-copy` depends on this crate.
//!
//! Supports the POSIX extended syntax tmux's search-forward accepts in
//! practice: literals, `.`, bracket expressions (ranges, negation and the
//! common `[:class:]` names), `* + ? {n} {n,} {n,m}`, alternation, groups,
//! `^` and `$` (anchored at the logical line), plus the `\d \w \s` (and
//! upper-case negations) shorthands people type out of habit.
//!
//! Matching simulates the Thompson NFA in lock step (Pike VM without
//! captures), so the cost is O(text length x program size) whatever the
//! pattern or the text: hostile scrollback cannot trigger backtracking
//! blow-ups. Matches are leftmost-longest (POSIX), or rightmost-longest for
//! backward search.

const MAX_PROG: usize = 20_000;
const MAX_REPEAT: u32 = 1000;

#[derive(Clone, Debug)]
enum Inst {
    Char(char),
    Any,
    Class(Box<Class>),
    Bol,
    Eol,
    Split(usize, usize),
    Jmp(usize),
    Match,
}

#[derive(Clone, Debug, Default)]
struct Class {
    negated: bool,
    ranges: Vec<(char, char)>,
}

impl Class {
    fn has(&self, c: char) -> bool {
        self.ranges.iter().any(|&(a, b)| a <= c && c <= b)
    }

    fn matches(&self, c: char, icase: bool) -> bool {
        let mut hit = self.has(c);
        if !hit && icase {
            // Text is folded to lower case; the class may name upper case.
            for u in c.to_uppercase() {
                if self.has(u) {
                    hit = true;
                    break;
                }
            }
        }
        hit != self.negated
    }
}

#[derive(Debug)]
enum Node {
    Empty,
    Char(char),
    Any,
    Class(Class),
    Bol,
    Eol,
    Concat(Vec<Node>),
    Alt(Vec<Node>),
    Repeat { node: Box<Node>, min: u32, max: Option<u32> },
}

/// A compiled pattern.
#[derive(Clone, Debug)]
pub struct Regex {
    prog: Vec<Inst>,
    icase: bool,
}

/// Reusable matcher memory, so a search allocates nothing per line.
#[derive(Default, Debug)]
pub struct Scratch {
    clist: Threads,
    nlist: Threads,
    stack: Vec<usize>,
}

#[derive(Default, Debug)]
struct Threads {
    sparse: Vec<u32>,
    dense: Vec<(u32, u32)>,
}

impl Threads {
    fn reset(&mut self, n: usize) {
        if self.sparse.len() < n {
            self.sparse.resize(n, 0);
        }
        self.dense.clear();
    }

    fn contains(&self, pc: usize) -> bool {
        let i = self.sparse[pc] as usize;
        i < self.dense.len() && self.dense[i].0 as usize == pc
    }

    fn insert(&mut self, pc: usize, start: usize) {
        self.sparse[pc] = self.dense.len() as u32;
        self.dense.push((pc as u32, start as u32));
    }
}

/// Fold a character for case-insensitive comparison.
pub fn fold(c: char) -> char {
    if c.is_ascii() {
        c.to_ascii_lowercase()
    } else {
        let mut l = c.to_lowercase();
        match (l.next(), l.next()) {
            (Some(x), None) => x,
            _ => c,
        }
    }
}

/// tmux's smart case: a pattern without upper-case letters ignores case.
pub fn smart_case(pattern: &str) -> bool {
    !pattern.chars().any(char::is_uppercase)
}

impl Regex {
    /// Compile `pattern` as a regular expression.
    pub fn new(pattern: &str, icase: bool) -> Result<Regex, String> {
        let chars: Vec<char> = pattern.chars().collect();
        let mut p = Parser { s: &chars, i: 0, icase };
        let node = p.alt()?;
        if p.i != chars.len() {
            return Err("unmatched )".into());
        }
        let mut c = Compiler { prog: Vec::new() };
        c.node(&node)?;
        c.push(Inst::Match)?;
        Ok(Regex { prog: c.prog, icase })
    }

    /// A pattern matching `text` literally.
    pub fn literal(text: &str, icase: bool) -> Regex {
        let mut prog: Vec<Inst> =
            text.chars().map(|c| Inst::Char(if icase { fold(c) } else { c })).collect();
        prog.push(Inst::Match);
        Regex { prog, icase }
    }

    /// Whether the pattern matches anywhere in `text`, an empty match
    /// included (as `regexec` does: `a*` matches `b`). Folds the text
    /// itself when the pattern ignores case.
    pub fn is_match(&self, text: &str) -> bool {
        let chars: Vec<char> = if self.icase { text.chars().map(fold).collect() } else { text.chars().collect() };
        let n = chars.len();
        let mut sc = Scratch::default();
        if self.find(&chars, 0, n + 1, false, &mut sc).is_some() {
            return true;
        }
        // Empty matches: only `^` and `$` tell positions apart, so the
        // start, the end and any one position between stand for all.
        let mut at = vec![0, n];
        if n >= 2 {
            at.push(1);
        }
        at.into_iter().any(|i| {
            sc.clist.reset(self.prog.len());
            self.add(&mut sc.clist, &mut sc.stack, 0, i, i, n);
            sc.clist.dense.iter().any(|&(pc, _)| matches!(self.prog[pc as usize], Inst::Match))
        })
    }

    /// Whether text must be folded with [`fold`] before matching.
    pub fn icase(&self) -> bool {
        self.icase
    }

    /// Find a non-empty match in `text` whose start lies in `lo..hi`.
    /// Returns the char range `(start, end)`. With `last` the match with
    /// the greatest start wins (backward search), otherwise the smallest;
    /// among matches with the same start the longest wins.
    pub fn find(&self, text: &[char], lo: usize, hi: usize, last: bool, sc: &mut Scratch) -> Option<(usize, usize)> {
        let n = text.len();
        let hi = hi.min(n + 1);
        if lo >= hi || self.prog.len() <= 1 {
            return None;
        }
        let np = self.prog.len();
        sc.clist.reset(np);
        sc.nlist.reset(np);
        let mut best: Option<(usize, usize)> = None;
        let mut i = lo;
        // In forward mode new starts go last (lowest priority, the list stays
        // ordered by increasing start); in backward mode they go first.
        self.add(&mut sc.clist, &mut sc.stack, 0, i, i, n);
        loop {
            sc.nlist.dense.clear();
            let next = i + 1;
            if last && next < hi && next <= n {
                self.add(&mut sc.nlist, &mut sc.stack, 0, next, next, n);
            }
            for k in 0..sc.clist.dense.len() {
                let (pc, s) = sc.clist.dense[k];
                let (pc, s) = (pc as usize, s as usize);
                if let Some((bs, _)) = best
                    && (if last { s < bs } else { s > bs })
                {
                    continue;
                }
                match &self.prog[pc] {
                    Inst::Match => {
                        if i > s {
                            let better = match best {
                                None => true,
                                Some((bs, be)) => {
                                    if last {
                                        s > bs || (s == bs && i > be)
                                    } else {
                                        s < bs || (s == bs && i > be)
                                    }
                                }
                            };
                            if better {
                                best = Some((s, i));
                            }
                        }
                    }
                    Inst::Char(c) => {
                        if i < n && text[i] == *c {
                            self.add(&mut sc.nlist, &mut sc.stack, pc + 1, s, next, n);
                        }
                    }
                    Inst::Any => {
                        if i < n {
                            self.add(&mut sc.nlist, &mut sc.stack, pc + 1, s, next, n);
                        }
                    }
                    Inst::Class(cl) => {
                        if i < n && cl.matches(text[i], self.icase) {
                            self.add(&mut sc.nlist, &mut sc.stack, pc + 1, s, next, n);
                        }
                    }
                    Inst::Bol | Inst::Eol | Inst::Split(..) | Inst::Jmp(_) => {}
                }
            }
            if !last && best.is_none() && next < hi && next <= n {
                self.add(&mut sc.nlist, &mut sc.stack, 0, next, next, n);
            }
            std::mem::swap(&mut sc.clist, &mut sc.nlist);
            i = next;
            if i > n {
                break;
            }
            if sc.clist.dense.is_empty() {
                if best.is_some() && !last {
                    break;
                }
                if i >= hi {
                    break;
                }
            }
        }
        best
    }

    fn add(&self, list: &mut Threads, stack: &mut Vec<usize>, pc: usize, start: usize, at: usize, n: usize) {
        stack.clear();
        stack.push(pc);
        while let Some(pc) = stack.pop() {
            if list.contains(pc) {
                continue;
            }
            list.insert(pc, start);
            match &self.prog[pc] {
                Inst::Jmp(a) => stack.push(*a),
                Inst::Split(a, b) => {
                    stack.push(*b);
                    stack.push(*a);
                }
                Inst::Bol if at == 0 => stack.push(pc + 1),
                Inst::Eol if at == n => stack.push(pc + 1),
                _ => {}
            }
        }
    }
}

struct Parser<'a> {
    s: &'a [char],
    i: usize,
    icase: bool,
}

impl Parser<'_> {
    fn peek(&self) -> Option<char> {
        self.s.get(self.i).copied()
    }

    fn alt(&mut self) -> Result<Node, String> {
        let mut alts = vec![self.concat()?];
        while self.peek() == Some('|') {
            self.i += 1;
            alts.push(self.concat()?);
        }
        Ok(if alts.len() == 1 { alts.pop().unwrap_or(Node::Empty) } else { Node::Alt(alts) })
    }

    fn concat(&mut self) -> Result<Node, String> {
        let mut items = Vec::new();
        while let Some(c) = self.peek() {
            if c == '|' || c == ')' {
                break;
            }
            let atom = self.atom()?;
            let atom = self.repeat(atom)?;
            items.push(atom);
        }
        Ok(match items.len() {
            0 => Node::Empty,
            1 => items.pop().unwrap_or(Node::Empty),
            _ => Node::Concat(items),
        })
    }

    fn repeat(&mut self, mut atom: Node) -> Result<Node, String> {
        loop {
            let (min, max) = match self.peek() {
                Some('*') => {
                    self.i += 1;
                    (0, None)
                }
                Some('+') => {
                    self.i += 1;
                    (1, None)
                }
                Some('?') => {
                    self.i += 1;
                    (0, Some(1))
                }
                Some('{') => match self.bounds() {
                    Some(b) => b,
                    None => return Ok(atom),
                },
                _ => return Ok(atom),
            };
            if min > MAX_REPEAT || max.is_some_and(|m| m > MAX_REPEAT || m < min) {
                return Err("bad repetition".into());
            }
            atom = Node::Repeat { node: Box::new(atom), min, max };
        }
    }

    /// Parse `{n}`, `{n,}` or `{n,m}` at the cursor; `None` (nothing
    /// consumed) if it is not a valid bound, in which case `{` is literal.
    fn bounds(&mut self) -> Option<(u32, Option<u32>)> {
        let start = self.i;
        self.i += 1;
        let num = |p: &mut Self| -> Option<u32> {
            let b = p.i;
            while p.peek().is_some_and(|c| c.is_ascii_digit()) {
                p.i += 1;
            }
            if p.i == b {
                return None;
            }
            p.s[b..p.i].iter().collect::<String>().parse().ok()
        };
        let Some(min) = num(self) else {
            self.i = start;
            return None;
        };
        let max = if self.peek() == Some(',') {
            self.i += 1;
            if self.peek() == Some('}') { None } else { num(self).or(Some(u32::MAX)) }
        } else {
            Some(min)
        };
        if self.peek() != Some('}') || max == Some(u32::MAX) {
            self.i = start;
            return None;
        }
        self.i += 1;
        Some((min, max))
    }

    fn lit(&self, c: char) -> Node {
        Node::Char(if self.icase { fold(c) } else { c })
    }

    fn atom(&mut self) -> Result<Node, String> {
        let Some(c) = self.peek() else { return Ok(Node::Empty) };
        self.i += 1;
        Ok(match c {
            '(' => {
                let inner = self.alt()?;
                if self.peek() != Some(')') {
                    return Err("unmatched (".into());
                }
                self.i += 1;
                inner
            }
            '.' => Node::Any,
            '^' => Node::Bol,
            '$' => Node::Eol,
            '[' => Node::Class(self.class()?),
            '*' | '+' | '?' => return Err("nothing to repeat".into()),
            '\\' => {
                let Some(e) = self.peek() else { return Err("trailing backslash".into()) };
                self.i += 1;
                match e {
                    'd' | 'D' | 'w' | 'W' | 's' | 'S' => {
                        let mut cl = Class::default();
                        match e.to_ascii_lowercase() {
                            'd' => cl.ranges.push(('0', '9')),
                            'w' => cl.ranges.extend([('0', '9'), ('a', 'z'), ('A', 'Z'), ('_', '_')]),
                            _ => cl.ranges.extend([(' ', ' '), ('\t', '\r')]),
                        }
                        cl.negated = e.is_ascii_uppercase();
                        Node::Class(cl)
                    }
                    't' => self.lit('\t'),
                    'n' => self.lit('\n'),
                    other => self.lit(other),
                }
            }
            other => self.lit(other),
        })
    }

    fn class(&mut self) -> Result<Class, String> {
        let mut cl = Class::default();
        if self.peek() == Some('^') {
            cl.negated = true;
            self.i += 1;
        }
        let mut first = true;
        loop {
            let Some(c) = self.peek() else { return Err("unmatched [".into()) };
            self.i += 1;
            if c == ']' && !first {
                break;
            }
            first = false;
            if c == '[' && self.peek() == Some(':') {
                let rest: String = self.s[self.i..].iter().collect();
                if let Some(end) = rest.find(":]") {
                    let name = &rest[1..end];
                    let ranges: &[(char, char)] = match name {
                        "alpha" => &[('a', 'z'), ('A', 'Z')],
                        "digit" => &[('0', '9')],
                        "alnum" => &[('0', '9'), ('a', 'z'), ('A', 'Z')],
                        "upper" => &[('A', 'Z')],
                        "lower" => &[('a', 'z')],
                        "space" => &[(' ', ' '), ('\t', '\r')],
                        "blank" => &[(' ', ' '), ('\t', '\t')],
                        "punct" => &[('!', '/'), (':', '@'), ('[', '\x60'), ('{', '~')],
                        "xdigit" => &[('0', '9'), ('a', 'f'), ('A', 'F')],
                        "word" => &[('0', '9'), ('a', 'z'), ('A', 'Z'), ('_', '_')],
                        _ => return Err(format!("unknown class {name}")),
                    };
                    cl.ranges.extend_from_slice(ranges);
                    self.i += rest[..end + 2].chars().count();
                    continue;
                }
            }
            let lo = if c == '\\' && self.peek().is_some() {
                let e = self.s[self.i];
                self.i += 1;
                e
            } else {
                c
            };
            if self.peek() == Some('-') && self.s.get(self.i + 1).is_some_and(|&n| n != ']') {
                let hi = self.s[self.i + 1];
                self.i += 2;
                if hi < lo {
                    return Err("bad range".into());
                }
                cl.ranges.push((lo, hi));
            } else {
                cl.ranges.push((lo, lo));
            }
        }
        if self.icase {
            let extra: Vec<(char, char)> = cl
                .ranges
                .iter()
                .filter(|(a, b)| a.is_ascii_uppercase() && b.is_ascii_uppercase())
                .map(|(a, b)| (a.to_ascii_lowercase(), b.to_ascii_lowercase()))
                .collect();
            cl.ranges.extend(extra);
        }
        Ok(cl)
    }
}

struct Compiler {
    prog: Vec<Inst>,
}

impl Compiler {
    fn push(&mut self, i: Inst) -> Result<usize, String> {
        if self.prog.len() >= MAX_PROG {
            return Err("pattern too large".into());
        }
        self.prog.push(i);
        Ok(self.prog.len() - 1)
    }

    fn node(&mut self, n: &Node) -> Result<(), String> {
        match n {
            Node::Empty => {}
            Node::Char(c) => {
                self.push(Inst::Char(*c))?;
            }
            Node::Any => {
                self.push(Inst::Any)?;
            }
            Node::Class(c) => {
                self.push(Inst::Class(Box::new(c.clone())))?;
            }
            Node::Bol => {
                self.push(Inst::Bol)?;
            }
            Node::Eol => {
                self.push(Inst::Eol)?;
            }
            Node::Concat(v) => {
                for x in v {
                    self.node(x)?;
                }
            }
            Node::Alt(v) => {
                let mut jumps = Vec::new();
                for (k, x) in v.iter().enumerate() {
                    if k + 1 < v.len() {
                        let split = self.push(Inst::Split(0, 0))?;
                        self.node(x)?;
                        jumps.push(self.push(Inst::Jmp(0))?);
                        let next = self.prog.len();
                        self.prog[split] = Inst::Split(split + 1, next);
                    } else {
                        self.node(x)?;
                    }
                }
                let end = self.prog.len();
                for j in jumps {
                    self.prog[j] = Inst::Jmp(end);
                }
            }
            Node::Repeat { node, min, max } => {
                for _ in 0..*min {
                    self.node(node)?;
                }
                match max {
                    None => {
                        let split = self.push(Inst::Split(0, 0))?;
                        self.node(node)?;
                        self.push(Inst::Jmp(split))?;
                        let end = self.prog.len();
                        self.prog[split] = Inst::Split(split + 1, end);
                    }
                    Some(m) => {
                        let mut splits = Vec::new();
                        for _ in *min..*m {
                            splits.push(self.push(Inst::Split(0, 0))?);
                            self.node(node)?;
                        }
                        let end = self.prog.len();
                        for s in splits {
                            self.prog[s] = Inst::Split(s + 1, end);
                        }
                    }
                }
            }
        }
        Ok(())
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn find(p: &str, t: &str, last: bool) -> Option<(usize, usize)> {
        let icase = smart_case(p);
        let re = Regex::new(p, icase).unwrap();
        let text: Vec<char> = t.chars().map(|c| if icase { fold(c) } else { c }).collect();
        re.find(&text, 0, text.len() + 1, last, &mut Scratch::default())
    }

    #[test]
    fn basics() {
        assert_eq!(find("b.c", "a b]c d", false), Some((2, 5)));
        assert_eq!(find("a+", "xaaay", false), Some((1, 4)));
        assert_eq!(find("a|ab", "xab", false), Some((1, 3)), "longest");
        assert_eq!(find("^ab", "ab ab", true), Some((0, 2)));
        assert_eq!(find("ab$", "ab ab", false), Some((3, 5)));
        assert_eq!(find("ab", "ab ab", true), Some((3, 5)));
        assert_eq!(find("[0-9]{2,3}", "a1234", false), Some((1, 4)));
        assert_eq!(find("[[:digit:]]+", "ab12", false), Some((2, 4)));
        assert_eq!(find("[^a ]+", "aa bcd", false), Some((3, 6)));
        assert_eq!(find("x*", "aaa", false), None, "empty matches are ignored");
        assert_eq!(find("hello", "HeLLo", false), Some((0, 5)), "smart case");
        assert_eq!(find("Hello", "hello", false), None);
        assert_eq!(find("(ab)+c", "ababababc", false), Some((0, 9)));
        assert_eq!(find("a{,", "xa{,", false), Some((1, 4)), "a brace without bounds is literal");
        assert_eq!(find("\\d+\\.\\d", "v 12.5", false), Some((2, 6)));
        assert!(Regex::new("(a", false).is_err());
        assert!(Regex::new("*a", false).is_err());
        assert!(Regex::new("a{2000}", false).is_err());
    }

    #[test]
    fn ranges_limit_start() {
        let re = Regex::new("ab", false).unwrap();
        let t: Vec<char> = "ab ab ab".chars().collect();
        let sc = &mut Scratch::default();
        assert_eq!(re.find(&t, 1, 9, false, sc), Some((3, 5)));
        assert_eq!(re.find(&t, 0, 6, true, sc), Some((3, 5)));
        assert_eq!(re.find(&t, 0, 3, true, sc), Some((0, 2)));
        assert_eq!(re.find(&t, 7, 9, false, sc), None);
    }

    #[test]
    fn hostile_patterns_are_linear() {
        // (a*)*b against a long run of a's: exponential for a backtracker.
        let re = Regex::new("(a*)*b", false).unwrap();
        let t: Vec<char> = std::iter::repeat_n('a', 50_000).collect();
        let t0 = std::time::Instant::now();
        assert_eq!(re.find(&t, 0, t.len() + 1, false, &mut Scratch::default()), None);
        assert!(t0.elapsed().as_secs() < 5);
    }
}
