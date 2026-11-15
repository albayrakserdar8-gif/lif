//! tmux's passthrough escape, `ESC P tmux; <sequence with every ESC
//! doubled> ESC \`, which programs that believe they run inside tmux use to
//! reach the outer terminal (Claude Code wraps its notifications and
//! clipboard writes in it). lif unwraps the sequence and feeds it to the
//! pane's terminal as if the program had written it directly, so the
//! sequences lif already passes on (OSC 52 clipboard, OSC 9/777/99
//! notifications) take their usual path and nothing unknown is written to
//! the outer terminal.
//!
//! Only panes with `allow-passthrough` (or `tmux-compat`) on run their
//! output through this; the others never pay for it.

/// The wrapped sequence is held until its end; past this the sequence is
/// dropped (tmux has no limit, but nothing sensible is that long).
pub(crate) const MAX_WRAPPED: usize = 1024 * 1024;

const INTRO: &[u8] = b"Ptmux;";

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
enum St {
    Ground,
    /// Saw ESC; held back until the next byte shows whether it starts the
    /// wrapper.
    Esc,
    /// Matched `ESC` and the first `n` bytes of `Ptmux;`.
    Intro(usize),
    Body,
    BodyEsc,
    /// The body outgrew `MAX_WRAPPED`: skip to its end.
    Skip,
    SkipEsc,
}

/// Incremental unwrapper; sequences may be split across reads anywhere.
#[derive(Debug)]
pub(crate) struct Unwrap {
    st: St,
    body: Vec<u8>,
}

impl Default for Unwrap {
    fn default() -> Self {
        Unwrap { st: St::Ground, body: Vec::new() }
    }
}

impl Unwrap {
    /// Pass `input` on to `out` in order, with wrapped sequences replaced
    /// by what they wrap.
    pub(crate) fn feed(&mut self, input: &[u8], out: &mut impl FnMut(&[u8])) {
        let mut i = 0;
        // Start of bytes in `input` not yet passed on (ground state only).
        let mut from = 0;
        while i < input.len() {
            let b = input[i];
            match self.st {
                St::Ground => {
                    match input[i..].iter().position(|&c| c == 0x1b) {
                        Some(k) => {
                            i += k;
                            if i > from {
                                out(&input[from..i]);
                            }
                            self.st = St::Esc;
                            i += 1;
                            from = i;
                        }
                        None => {
                            i = input.len();
                        }
                    }
                    continue;
                }
                St::Esc => {
                    if b == INTRO[0] {
                        self.st = St::Intro(1);
                        i += 1;
                    } else {
                        // Not ours: the held ESC goes out, this byte is
                        // looked at again in the ground state.
                        out(b"\x1b");
                        self.st = St::Ground;
                    }
                    from = i;
                    continue;
                }
                St::Intro(n) => {
                    if b == INTRO[n] {
                        self.st = if n + 1 == INTRO.len() { St::Body } else { St::Intro(n + 1) };
                        self.body.clear();
                        i += 1;
                    } else {
                        out(b"\x1b");
                        out(&INTRO[..n]);
                        self.st = St::Ground;
                    }
                    from = i;
                    continue;
                }
                St::Body => match b {
                    0x1b => self.st = St::BodyEsc,
                    _ if self.body.len() >= MAX_WRAPPED => self.st = St::Skip,
                    _ => self.body.push(b),
                },
                St::BodyEsc => match b {
                    b'\\' => {
                        out(&self.body);
                        self.body.clear();
                        self.st = St::Ground;
                    }
                    // A doubled ESC is one ESC of the wrapped sequence;
                    // anything else after a single ESC is kept as is.
                    0x1b => {
                        self.body.push(0x1b);
                        self.st = St::Body;
                    }
                    _ => {
                        self.body.push(0x1b);
                        self.body.push(b);
                        self.st = St::Body;
                    }
                },
                St::Skip => {
                    if b == 0x1b {
                        self.st = St::SkipEsc;
                    }
                }
                St::SkipEsc => {
                    self.st = match b {
                        b'\\' => {
                            self.body = Vec::new();
                            St::Ground
                        }
                        _ => St::Skip,
                    };
                }
            }
            i += 1;
            from = i;
        }
        if self.st == St::Ground && from < input.len() {
            out(&input[from..]);
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn run(chunks: &[&[u8]]) -> Vec<u8> {
        let mut u = Unwrap::default();
        let mut got = Vec::new();
        for c in chunks {
            u.feed(c, &mut |b| got.extend_from_slice(b));
        }
        got
    }

    #[test]
    fn plain_output_passes_unchanged() {
        let s: &[u8] = b"ab\x1b[31mred\x1b[m\x1bPq#0;2;0;0;0\x1b\\ \x1bPtmu";
        // What may start a wrapper is held until the next bytes decide.
        assert_eq!(run(&[s]), s[..s.len() - 5].to_vec());
        assert_eq!(run(&[s, b"y"]), [s, b"y"].concat(), "a non-tmux DCS and a broken intro are not touched");
        assert_eq!(run(&[b"x\x1b", b"[1mY"]), b"x\x1b[1mY");
    }

    #[test]
    fn wrapped_sequences_are_unwrapped() {
        let wrapped = b"a\x1bPtmux;\x1b\x1b]9;hi there\x07\x1b\\b";
        assert_eq!(run(&[wrapped]), b"a\x1b]9;hi there\x07b");
        let st = b"\x1bPtmux;\x1b\x1b]52;c;aGk=\x1b\x1b\\\x1b\\";
        assert_eq!(run(&[st]), b"\x1b]52;c;aGk=\x1b\\");
    }

    #[test]
    fn split_anywhere() {
        let whole: &[u8] = b"pre\x1bPtmux;\x1b\x1b]777;notify;T;B\x1b\x1b\\\x1b\\post\x1b[0m";
        let want = run(&[whole]);
        assert_eq!(want, b"pre\x1b]777;notify;T;B\x1b\\post\x1b[0m");
        for cut in 0..whole.len() {
            assert_eq!(run(&[&whole[..cut], &whole[cut..]]), want, "cut at {cut}");
        }
        let bytes: Vec<&[u8]> = whole.chunks(1).collect();
        assert_eq!(run(&bytes), want);
    }

    /// `cargo test --release -p lif-server -- --ignored --nocapture unwrap_speed`
    #[test]
    #[ignore]
    fn unwrap_speed() {
        let line = "\x1b[38;5;196mred \x1b[1;38;2;10;20;30mtrue\x1b[0m çé 中文 plain text to fill the line up\r\n";
        let data: Vec<u8> = line.as_bytes().iter().copied().cycle().take(64 << 20).collect();
        let mut u = Unwrap::default();
        let mut n = 0usize;
        let t = std::time::Instant::now();
        for chunk in data.chunks(64 * 1024) {
            u.feed(chunk, &mut |b| n += b.len());
        }
        let secs = t.elapsed().as_secs_f64();
        assert_eq!(n, data.len());
        eprintln!("unwrap: {:.0} MB/s on SGR-heavy text", data.len() as f64 / secs / 1e6);
    }

    #[test]
    fn oversized_bodies_are_dropped() {
        let mut s = b"\x1bPtmux;".to_vec();
        s.extend(std::iter::repeat_n(b'x', MAX_WRAPPED + 10));
        s.extend_from_slice(b"\x1b\\after");
        assert_eq!(run(&[&s]), b"after");
    }
}
