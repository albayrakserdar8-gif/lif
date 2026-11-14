//! Tokenizer for command strings (tmux's `cmd-parse.y` lexer, minus
//! environment expansion and `%if`).

pub(crate) enum Word {
    Str(String),
    Block(Vec<Vec<Word>>),
}

struct Lexer {
    c: Vec<char>,
    i: usize,
}

fn is_blank(c: Option<char>) -> bool {
    matches!(c, None | Some(' ' | '\t' | '\n' | '\r'))
}

/// Split `s` into commands, each a list of words.
pub(crate) fn lex(s: &str) -> Result<Vec<Vec<Word>>, String> {
    let mut l = Lexer { c: s.chars().collect(), i: 0 };
    l.commands(false)
}

impl Lexer {
    fn peek(&self) -> Option<char> {
        self.c.get(self.i).copied()
    }

    fn peek2(&self) -> Option<char> {
        self.c.get(self.i + 1).copied()
    }

    fn commands(&mut self, in_block: bool) -> Result<Vec<Vec<Word>>, String> {
        let mut cmds = Vec::new();
        let mut cur: Vec<Word> = Vec::new();
        loop {
            while matches!(self.peek(), Some(' ' | '\t' | '\r')) {
                self.i += 1;
            }
            match self.peek() {
                None => {
                    if in_block {
                        return Err("syntax error: missing }".into());
                    }
                    if !cur.is_empty() {
                        cmds.push(cur);
                    }
                    return Ok(cmds);
                }
                Some('\n') | Some(';') => {
                    self.i += 1;
                    if !cur.is_empty() {
                        cmds.push(std::mem::take(&mut cur));
                    }
                }
                Some('#') if self.peek2() != Some('{') => {
                    while !matches!(self.peek(), None | Some('\n')) {
                        self.i += 1;
                    }
                }
                Some('}') if in_block && (is_blank(self.peek2()) || self.peek2() == Some(';')) => {
                    self.i += 1;
                    if !cur.is_empty() {
                        cmds.push(cur);
                    }
                    return Ok(cmds);
                }
                Some('{') if is_blank(self.peek2()) => {
                    self.i += 1;
                    let inner = self.commands(true)?;
                    cur.push(Word::Block(inner));
                }
                Some('\\') if self.peek2() == Some('\n') => self.i += 2,
                Some(_) => {
                    let (w, ends) = self.word()?;
                    cur.push(Word::Str(w));
                    if ends {
                        cmds.push(std::mem::take(&mut cur));
                    }
                }
            }
        }
    }

    /// One word; the bool is set when a trailing `;` ended the command.
    fn word(&mut self) -> Result<(String, bool), String> {
        let mut s = String::new();
        loop {
            let Some(ch) = self.peek() else { return Ok((s, false)) };
            match ch {
                ' ' | '\t' | '\n' | '\r' => return Ok((s, false)),
                ';' if is_blank(self.peek2()) => {
                    self.i += 1;
                    return Ok((s, true));
                }
                '\'' => {
                    self.i += 1;
                    loop {
                        match self.peek() {
                            None => return Err("syntax error: unterminated '".into()),
                            Some('\'') => {
                                self.i += 1;
                                break;
                            }
                            Some(c) => {
                                s.push(c);
                                self.i += 1;
                            }
                        }
                    }
                }
                '"' => {
                    self.i += 1;
                    loop {
                        match self.peek() {
                            None => return Err("syntax error: unterminated \"".into()),
                            Some('"') => {
                                self.i += 1;
                                break;
                            }
                            Some('\\') => {
                                self.i += 1;
                                self.escape(&mut s)?;
                            }
                            Some(c) => {
                                s.push(c);
                                self.i += 1;
                            }
                        }
                    }
                }
                '\\' => {
                    self.i += 1;
                    if self.peek() == Some('\n') {
                        self.i += 1;
                    } else {
                        self.escape(&mut s)?;
                    }
                }
                '#' if self.peek2() == Some('{') => {
                    // A format: copy up to the matching brace verbatim.
                    let mut depth = 0;
                    loop {
                        match self.peek() {
                            None => return Err("syntax error: unterminated #{".into()),
                            Some('#') if self.peek2() == Some('{') => {
                                depth += 1;
                                s.push_str("#{");
                                self.i += 2;
                            }
                            Some('}') => {
                                s.push('}');
                                self.i += 1;
                                depth -= 1;
                                if depth == 0 {
                                    break;
                                }
                            }
                            Some(c) => {
                                s.push(c);
                                self.i += 1;
                            }
                        }
                    }
                }
                c => {
                    s.push(c);
                    self.i += 1;
                }
            }
        }
    }

    fn escape(&mut self, s: &mut String) -> Result<(), String> {
        let Some(ch) = self.peek() else {
            s.push('\\');
            return Ok(());
        };
        self.i += 1;
        let out = match ch {
            'a' => '\x07',
            'b' => '\x08',
            'e' => '\x1b',
            'f' => '\x0c',
            's' => ' ',
            'v' => '\x0b',
            'r' => '\r',
            'n' => '\n',
            't' => '\t',
            'u' | 'U' => {
                let n = if ch == 'u' { 4 } else { 8 };
                let hex: String = self.c[self.i..(self.i + n).min(self.c.len())].iter().collect();
                self.i += hex.len();
                let v = u32::from_str_radix(&hex, 16).map_err(|_| "invalid \\u argument".to_string())?;
                char::from_u32(v).ok_or_else(|| "invalid \\u argument".to_string())?
            }
            '0'..='7' => {
                let mut v = ch as u32 - '0' as u32;
                for _ in 0..2 {
                    match self.peek() {
                        Some(d @ '0'..='7') => {
                            v = v * 8 + (d as u32 - '0' as u32);
                            self.i += 1;
                        }
                        _ => break,
                    }
                }
                char::from_u32(v).unwrap_or('?')
            }
            c => c,
        };
        s.push(out);
        Ok(())
    }
}
