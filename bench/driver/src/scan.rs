//! Finds a printable byte in a terminal output stream while skipping escape
//! sequences, so that a marker character inside `ESC [ ... m` or an OSC title
//! is not mistaken for an echoed keystroke.

#[derive(Clone, Copy, PartialEq, Eq, Debug)]
enum St {
    Ground,
    Esc,
    Csi,
    Str,
    StrEsc,
    OneMore,
}

pub struct Scanner {
    st: St,
}

impl Default for Scanner {
    fn default() -> Self {
        Self::new()
    }
}

impl Scanner {
    pub fn new() -> Self {
        Scanner { st: St::Ground }
    }

    /// Feeds bytes; returns true if `target` occurred as printable text.
    pub fn feed(&mut self, data: &[u8], target: u8) -> bool {
        let mut hit = false;
        for &b in data {
            if self.step(b) && b == target {
                hit = true;
            }
        }
        hit
    }

    /// Advances one byte; returns true if the byte is printable text.
    fn step(&mut self, b: u8) -> bool {
        // CAN and SUB abort any sequence.
        if b == 0x18 || b == 0x1a {
            self.st = St::Ground;
            return false;
        }
        match self.st {
            St::Ground => {
                if b == 0x1b {
                    self.st = St::Esc;
                    false
                } else {
                    b >= 0x20 && b != 0x7f
                }
            }
            St::Esc => {
                self.st = match b {
                    b'[' => St::Csi,
                    b']' | b'P' | b'X' | b'^' | b'_' => St::Str,
                    // Charset designation and other ESC-intermediate forms
                    // take exactly one more byte.
                    0x20..=0x2f => St::OneMore,
                    0x1b => St::Esc,
                    _ => St::Ground,
                };
                false
            }
            St::Csi => {
                if b == 0x1b {
                    self.st = St::Esc;
                } else if (0x40..=0x7e).contains(&b) {
                    self.st = St::Ground;
                }
                false
            }
            St::Str => {
                if b == 0x07 {
                    self.st = St::Ground;
                } else if b == 0x1b {
                    self.st = St::StrEsc;
                }
                false
            }
            St::StrEsc => {
                if b == b'\\' {
                    self.st = St::Ground;
                    false
                } else {
                    // An ESC inside a string that is not ST ends the string
                    // and starts a new sequence.
                    self.st = St::Esc;
                    self.step(b)
                }
            }
            St::OneMore => {
                self.st = St::Ground;
                false
            }
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn plain_text_hits() {
        let mut s = Scanner::new();
        assert!(s.feed(b"abc~def", b'~'));
        assert!(!s.feed(b"abcdef", b'~'));
    }

    #[test]
    fn sequences_are_skipped() {
        let mut s = Scanner::new();
        assert!(!s.feed(b"\x1b[38;5;1m\x1b[2~\x1b]0;ti~tle\x07\x1b(0q\x1b(B", b'~'));
        assert!(!s.feed(b"\x1b]2;x~\x1b\\", b'~'));
        assert!(!s.feed(b"\x1bP~~\x1b\\", b'~'));
        assert!(s.feed(b"\x1b[0m~", b'~'));
    }

    #[test]
    fn sequence_split_across_reads() {
        let mut s = Scanner::new();
        assert!(!s.feed(b"\x1b[", b'q'));
        assert!(!s.feed(b"2 q", b'q'));
        assert!(!s.feed(b"\x1b", b'~'));
        assert!(!s.feed(b"[3~", b'~'));
        assert!(s.feed(b"x~", b'~'));
    }

    #[test]
    fn charset_designation_takes_one_byte() {
        let mut s = Scanner::new();
        // ESC ( 0 selects line drawing; the q after it is text.
        assert!(!s.feed(b"\x1b(", b'0'));
        assert!(!s.feed(b"0", b'0'));
        assert!(s.feed(b"0", b'0'));
    }
}
