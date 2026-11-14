//! Answers the terminal queries multiplexer clients send at startup (device
//! attributes, colors, size in pixels, mode reports) the way a real terminal
//! emulator would. Without answers some clients wait on timeouts; tmux 3.8,
//! for one, holds a bare ESC for 500 ms while its DA request is outstanding,
//! which no user with a real terminal would see.

const MAX: usize = 128;

#[derive(Clone, Copy, PartialEq, Eq)]
enum St {
    Ground,
    Esc,
    Csi,
    Osc,
    OscEsc,
    Dcs,
    DcsEsc,
}

pub struct Answerer {
    st: St,
    seq: [u8; MAX],
    len: usize,
    overflow: bool,
    cols: u16,
    rows: u16,
    pub answered: u64,
}

impl Answerer {
    pub fn new(cols: u16, rows: u16) -> Self {
        Answerer { st: St::Ground, seq: [0; MAX], len: 0, overflow: false, cols, rows, answered: 0 }
    }

    fn push(&mut self, b: u8) {
        if self.len < MAX {
            self.seq[self.len] = b;
            self.len += 1;
        } else {
            self.overflow = true;
        }
    }

    /// Feeds client output; appends any replies to `out`.
    pub fn feed(&mut self, data: &[u8], out: &mut Vec<u8>) {
        let mut i = 0;
        while i < data.len() {
            if self.st == St::Ground {
                // Fast path: plain text up to the next ESC.
                match data[i..].iter().position(|&b| b == 0x1b) {
                    Some(k) => {
                        i += k + 1;
                        self.st = St::Esc;
                        self.len = 0;
                        self.overflow = false;
                        continue;
                    }
                    None => return,
                }
            }
            let b = data[i];
            i += 1;
            match self.st {
                St::Ground => unreachable!(),
                St::Esc => {
                    self.st = match b {
                        b'[' => St::Csi,
                        b']' => St::Osc,
                        b'P' => St::Dcs,
                        0x1b => St::Esc,
                        _ => St::Ground,
                    };
                }
                St::Csi => {
                    if (0x40..=0x7e).contains(&b) {
                        self.push(b);
                        self.st = St::Ground;
                        if !self.overflow {
                            self.csi(out);
                        }
                    } else if b == 0x1b {
                        self.st = St::Esc;
                        self.len = 0;
                    } else {
                        self.push(b);
                    }
                }
                St::Osc => match b {
                    0x07 => {
                        self.st = St::Ground;
                        if !self.overflow {
                            self.osc(out, b"\x07");
                        }
                    }
                    0x1b => self.st = St::OscEsc,
                    _ => self.push(b),
                },
                St::OscEsc => {
                    self.st = St::Ground;
                    if b == b'\\' && !self.overflow {
                        self.osc(out, b"\x1b\\");
                    } else if b != b'\\' {
                        i -= 1;
                        self.st = St::Esc;
                    }
                }
                St::Dcs => {
                    if b == 0x1b {
                        self.st = St::DcsEsc;
                    }
                }
                St::DcsEsc => {
                    self.st = if b == b'\\' { St::Ground } else { St::Dcs };
                }
            }
        }
    }

    fn csi(&mut self, out: &mut Vec<u8>) {
        let s = &self.seq[..self.len];
        let reply: Vec<u8> = match s {
            b"c" | b"0c" => b"\x1b[?62;22c".to_vec(),
            b">c" | b">0c" => b"\x1b[>1;10;0c".to_vec(),
            b">q" | b">0q" => b"\x1bP>|lifbench-driver\x1b\\".to_vec(),
            b"5n" => b"\x1b[0n".to_vec(),
            b"6n" => b"\x1b[1;1R".to_vec(),
            b"?996n" => b"\x1b[?997;1n".to_vec(),
            b"14t" => format!("\x1b[4;{};{}t", self.rows as u32 * 20, self.cols as u32 * 8).into_bytes(),
            b"16t" => b"\x1b[6;20;8t".to_vec(),
            b"18t" => format!("\x1b[8;{};{}t", self.rows, self.cols).into_bytes(),
            _ => {
                // DECRQM: CSI ? Ps $ p. Report synchronized output (2026) as
                // supported, like current terminals; everything else unknown.
                if s.len() > 3 && s[0] == b'?' && s.ends_with(b"$p") {
                    let ps = &s[1..s.len() - 2];
                    let state = if ps == b"2026" { b'2' } else { b'0' };
                    let mut r = b"\x1b[?".to_vec();
                    r.extend_from_slice(ps);
                    r.push(b';');
                    r.push(state);
                    r.extend_from_slice(b"$y");
                    r
                } else {
                    return;
                }
            }
        };
        out.extend_from_slice(&reply);
        self.answered += 1;
    }

    fn osc(&mut self, out: &mut Vec<u8>, term: &[u8]) {
        let reply: &[u8] = match &self.seq[..self.len] {
            b"10;?" => b"\x1b]10;rgb:ffff/ffff/ffff",
            b"11;?" => b"\x1b]11;rgb:0000/0000/0000",
            _ => return,
        };
        out.extend_from_slice(reply);
        out.extend_from_slice(term);
        self.answered += 1;
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn run(chunks: &[&[u8]]) -> Vec<u8> {
        let mut a = Answerer::new(200, 50);
        let mut out = Vec::new();
        for c in chunks {
            a.feed(c, &mut out);
        }
        out
    }

    #[test]
    fn device_attributes() {
        assert_eq!(run(&[b"hello\x1b[c"]), b"\x1b[?62;22c");
        assert_eq!(run(&[b"\x1b[>c"]), b"\x1b[>1;10;0c");
        assert_eq!(run(&[b"\x1b[18t"]), b"\x1b[8;50;200t");
    }

    #[test]
    fn split_across_reads_and_osc() {
        assert_eq!(run(&[b"x\x1b", b"]11;", b"?\x1b", b"\\y"]), b"\x1b]11;rgb:0000/0000/0000\x1b\\");
        assert_eq!(run(&[b"\x1b]10;?\x07"]), b"\x1b]10;rgb:ffff/ffff/ffff\x07");
    }

    #[test]
    fn decrqm() {
        assert_eq!(run(&[b"\x1b[?2026$p\x1b[?2027$p"]), b"\x1b[?2026;2$y\x1b[?2027;0$y");
    }

    #[test]
    fn ignores_ordinary_output() {
        assert!(run(&[b"\x1b[38;5;1mred\x1b[0m\x1b[2J\x1b[1;1H\x1b]0;title\x07\x1bP1$r\x1b\\"]).is_empty());
        let long = [b"\x1b]8;;".as_slice(), &[b'a'; 300], b"\x1b\\\x1b[c"].concat();
        assert_eq!(run(&[&long]), b"\x1b[?62;22c");
    }
}
