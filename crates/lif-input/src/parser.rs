//! The streaming input state machine.

use core::time::Duration;

use lif_types::{Key, Mods, MouseAction, MouseButton, MouseEvent, PixelSize, Pos, Size};

use crate::csi::{self, Csi, Scan};
use crate::keys::{
    K, Text, action_param, byte_key, caps_lock, code_key, implied_text, letter_key, mods_param, ss3_keypad,
    tilde_key,
};
use crate::strings::{self, StrKind};
use crate::{ColorScheme, InputEvent, ModeState, TermResponse};

/// Longest incomplete sequence held between [`InputParser::feed`] calls.
/// Anything longer is flushed as best-effort events.
pub const MAX_PENDING: usize = 256;

/// Longest OSC / DCS / APC body kept (an OSC 52 clipboard reply is the only
/// big one). Bytes past it are dropped and the reply is marked truncated.
pub const MAX_STRING: usize = 1 << 20;

/// Suggested one-shot timer for [`Pending::Ambiguous`]: how long a lone ESC
/// (or ESC plus a sequence introducer) waits to see whether more bytes
/// follow before it is taken as the Escape (or Alt+key) it looks like. The
/// server decides; 5 ms is far below human perception and well above the
/// time a terminal needs to write one sequence.
pub const DEFAULT_ESC_TIMEOUT: Duration = Duration::from_millis(5);

/// Suggested timer for [`Pending::Incomplete`]: a sequence that is certainly
/// not finished (a split CSI, a partial UTF-8 character, an unterminated
/// OSC). Long enough to ride out a slow link, short enough that garbage
/// never wedges input.
pub const DEFAULT_INCOMPLETE_TIMEOUT: Duration = Duration::from_millis(100);

/// What the parser is holding back, if anything.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum Pending {
    None,
    /// Legacy keyboard mode and the buffer ends with a bare ESC (or ESC plus
    /// `[`, `O`, `P`, `]`, `_`, or ESC ESC): it may be a key or the start
    /// of a sequence. Arm a [`DEFAULT_ESC_TIMEOUT`] timer and call
    /// [`InputParser::flush_pending`] when it fires.
    Ambiguous,
    /// A sequence that is certainly incomplete. Arm a
    /// [`DEFAULT_INCOMPLETE_TIMEOUT`] timer.
    Incomplete,
}

#[derive(Clone, Copy, PartialEq, Eq, Debug)]
enum Mode {
    Ground,
    Paste,
    Str,
}

enum Tok {
    Key(K),
    Ev(InputEvent),
    /// The consumed bytes, verbatim, as [`InputEvent::Unknown`].
    Unknown,
    PasteStart,
    Str(StrKind),
}

const ESC: u8 = 0x1b;
const PASTE_END: &[u8; 6] = b"\x1b[201~";

/// Streaming parser for bytes from the client's terminal.
///
/// Feed it every read with [`InputParser::feed`]; sequences may be split at
/// any byte. After each feed, check [`InputParser::pending`]: if it is not
/// [`Pending::None`], arm a one-shot timer for
/// [`InputParser::pending_timeout`] and call [`InputParser::flush_pending`]
/// when it fires (and the parser still has something pending). Nothing is
/// held otherwise, so an idle parser needs no timer.
///
/// In legacy keyboard mode a bare ESC is ambiguous (Escape key, Alt prefix,
/// or sequence start). Bytes that follow in the same read decide it at once;
/// only a read that ends with it costs the short timer. With the kitty
/// keyboard protocol active (see [`InputParser::set_kitty_keyboard`]) the
/// Escape key arrives as `CSI 27 u`, so a trailing ESC is certainly a split
/// sequence and gets the longer [`Pending::Incomplete`] treatment instead.
#[derive(Debug)]
pub struct InputParser {
    pending: Vec<u8>,
    mode: Mode,
    /// Bytes of [`PASTE_END`] matched at the end of the previous feed.
    paste_match: u8,
    str_kind: StrKind,
    strbuf: Vec<u8>,
    str_overflow: bool,
    /// The string body ended with ESC at the end of a read: waiting to see `\`.
    str_esc: bool,
    kitty: bool,
    sgr_pixels: bool,
    cell: PixelSize,
    cpr_expected: u8,
    scratch: Vec<TermResponse>,
}

impl Default for InputParser {
    fn default() -> Self {
        Self::new()
    }
}

impl InputParser {
    pub fn new() -> Self {
        Self {
            pending: Vec::with_capacity(MAX_PENDING),
            mode: Mode::Ground,
            paste_match: 0,
            str_kind: StrKind::Osc,
            strbuf: Vec::new(),
            str_overflow: false,
            str_esc: false,
            kitty: false,
            sgr_pixels: false,
            cell: PixelSize::default(),
            cpr_expected: 0,
            scratch: Vec::new(),
        }
    }

    /// Whether the outer terminal has the kitty keyboard protocol pushed.
    /// Only affects [`InputParser::pending`]; both encodings always decode.
    pub fn set_kitty_keyboard(&mut self, active: bool) {
        self.kitty = active;
    }

    /// Whether SGR-pixel mouse reporting (mode 1016) is enabled on the outer
    /// terminal. Its reports look exactly like SGR (1006) ones, so the parser
    /// must be told. In pixel mode [`MouseEvent::pixel`] holds the reported
    /// pixel position as sent (0-based) and [`MouseEvent::cell`] is derived
    /// from the cell size (see [`InputParser::set_cell_size`]), or (0, 0)
    /// while the cell size is unknown.
    pub fn set_sgr_pixels(&mut self, on: bool) {
        self.sgr_pixels = on;
    }

    /// Cell size in pixels, for SGR-pixel mouse. Updated automatically from
    /// [`TermResponse::CellSize`] and [`TermResponse::InBandResize`].
    pub fn set_cell_size(&mut self, cell: PixelSize) {
        self.cell = cell;
    }

    pub fn cell_size(&self) -> PixelSize {
        self.cell
    }

    /// Call once per cursor position request (`CSI 6 n`) sent to the outer
    /// terminal. `CSI 1 ; m R` is both "cursor at row 1, column m" and xterm's
    /// modified F3; while a report is expected it decodes as a report,
    /// otherwise as F3 (the libtermkey / notcurses approach). Prefer DECXCPR
    /// (`CSI ? 6 n`), whose reply is unambiguous.
    pub fn expect_cursor_position(&mut self) {
        self.cpr_expected = self.cpr_expected.saturating_add(1);
    }

    /// Back to the initial state, dropping anything held (e.g. on reattach).
    pub fn reset(&mut self) {
        self.pending.clear();
        self.mode = Mode::Ground;
        self.paste_match = 0;
        self.strbuf = Vec::new();
        self.str_overflow = false;
        self.str_esc = false;
        self.cpr_expected = 0;
    }

    /// Bytes currently held: an incomplete sequence (at most
    /// [`MAX_PENDING`]) plus a reply string being collected (at most
    /// [`MAX_STRING`]).
    pub fn buffered_len(&self) -> usize {
        self.pending.len() + self.strbuf.len()
    }

    /// Inside a bracketed paste.
    pub fn in_paste(&self) -> bool {
        self.mode == Mode::Paste
    }

    pub fn pending(&self) -> Pending {
        match self.mode {
            Mode::Str => Pending::Incomplete,
            // Paste bytes are emitted as they arrive; a paste ends only with
            // its end marker (or reset).
            Mode::Paste => Pending::None,
            Mode::Ground => {
                let p = &self.pending[..];
                if p.is_empty() {
                    Pending::None
                } else if !self.kitty && is_ambiguous(p) {
                    Pending::Ambiguous
                } else {
                    Pending::Incomplete
                }
            }
        }
    }

    pub fn has_pending(&self) -> bool {
        self.pending() != Pending::None
    }

    /// The suggested timer for the current [`Pending`] state.
    pub fn pending_timeout(&self) -> Option<Duration> {
        match self.pending() {
            Pending::None => None,
            Pending::Ambiguous => Some(DEFAULT_ESC_TIMEOUT),
            Pending::Incomplete => Some(DEFAULT_INCOMPLETE_TIMEOUT),
        }
    }

    /// Decide everything held back as best-effort events: a lone ESC becomes
    /// Escape, ESC + introducer becomes Alt+key, a partial UTF-8 character
    /// becomes U+FFFD, an unfinished sequence becomes [`InputEvent::Unknown`].
    pub fn flush_pending(&mut self, out: &mut Vec<InputEvent>) {
        if self.mode == Mode::Str {
            if self.str_esc {
                self.finish_string(out);
            } else {
                self.abort_string(out);
            }
        }
        while !self.pending.is_empty() {
            let mut buf = core::mem::take(&mut self.pending);
            let mut pos = self.step_final(&buf, out);
            pos += self.run(&buf[pos..], out);
            buf.drain(..pos);
            self.pending = buf;
        }
    }

    /// Parse `bytes`, appending events to `out`.
    pub fn feed(&mut self, mut input: &[u8], out: &mut Vec<InputEvent>) {
        loop {
            if input.is_empty() {
                return;
            }
            if self.mode == Mode::Str && self.str_esc {
                self.str_esc = false;
                if input[0] == b'\\' {
                    self.finish_string(out);
                    input = &input[1..];
                } else {
                    self.abort_string(out);
                    // Ground mode always starts with nothing pending.
                    self.pending.push(ESC);
                }
                continue;
            }
            if self.pending.is_empty() {
                let pos = self.run(input, out);
                let rest = &input[pos..];
                if rest.len() >= MAX_PENDING {
                    let n = self.step_final(rest, out);
                    input = &rest[n..];
                    continue;
                }
                self.pending.extend_from_slice(rest);
                return;
            }
            let take = input.len().min(MAX_PENDING - self.pending.len());
            let mut buf = core::mem::take(&mut self.pending);
            buf.extend_from_slice(&input[..take]);
            input = &input[take..];
            let mut pos = self.run(&buf, out);
            if buf.len() - pos >= MAX_PENDING {
                pos += self.step_final(&buf[pos..], out);
                pos += self.run(&buf[pos..], out);
            }
            buf.drain(..pos);
            self.pending = buf;
        }
    }

    /// Consumes as much of `b` as decodes completely; returns the count.
    fn run(&mut self, b: &[u8], out: &mut Vec<InputEvent>) -> usize {
        let mut pos = 0;
        while pos < b.len() {
            match self.mode {
                Mode::Paste => pos += self.paste(&b[pos..], out),
                Mode::Str => pos += self.string(&b[pos..], out),
                Mode::Ground => match self.step(&b[pos..], false, out) {
                    Some(n) => pos += n,
                    None => break,
                },
            }
        }
        pos
    }

    fn step_final(&mut self, b: &[u8], out: &mut Vec<InputEvent>) -> usize {
        match self.step(b, true, out) {
            Some(n) => n,
            None => {
                out.push(InputEvent::Unknown(b.to_vec()));
                b.len()
            }
        }
    }

    fn step(&mut self, b: &[u8], fin: bool, out: &mut Vec<InputEvent>) -> Option<usize> {
        let (tok, n) = self.decode(b, fin)?;
        match tok {
            Tok::Key(k) => out.push(k.into_event()),
            Tok::Ev(e) => {
                if let InputEvent::Response(r) = &e {
                    self.observe(r);
                }
                out.push(e);
            }
            Tok::Unknown => out.push(InputEvent::Unknown(b[..n].to_vec())),
            Tok::PasteStart => {
                out.push(InputEvent::PasteStart);
                self.mode = Mode::Paste;
                self.paste_match = 0;
            }
            Tok::Str(kind) => {
                self.mode = Mode::Str;
                self.str_kind = kind;
                self.strbuf.clear();
                self.str_overflow = false;
                self.str_esc = false;
            }
        }
        Some(n)
    }

    fn observe(&mut self, r: &TermResponse) {
        match *r {
            TermResponse::CursorPosition { .. } => self.cpr_expected = self.cpr_expected.saturating_sub(1),
            TermResponse::CellSize(px) if px.width > 0 && px.height > 0 => self.cell = px,
            TermResponse::InBandResize { cells, pixels }
                if cells.cols > 0 && cells.rows > 0 && pixels.width > 0 && pixels.height > 0 =>
            {
                self.cell = PixelSize { width: pixels.width / cells.cols, height: pixels.height / cells.rows };
            }
            _ => {}
        }
    }

    // ---- bracketed paste -------------------------------------------------

    fn paste(&mut self, b: &[u8], out: &mut Vec<InputEvent>) -> usize {
        let mut i = 0;
        let mut start = 0;
        if self.paste_match > 0 {
            let mut m = self.paste_match as usize;
            while m < PASTE_END.len() && i < b.len() && b[i] == PASTE_END[m] {
                m += 1;
                i += 1;
            }
            if m == PASTE_END.len() {
                self.paste_match = 0;
                self.mode = Mode::Ground;
                out.push(InputEvent::PasteEnd);
                return i;
            }
            if i == b.len() {
                self.paste_match = m as u8;
                return i;
            }
            // The held bytes were data after all.
            out.push(InputEvent::PasteData(PASTE_END[..m].to_vec()));
            self.paste_match = 0;
            start = i;
        }
        let mut j = i;
        while let Some(off) = b[j..].iter().position(|&c| c == ESC) {
            let p = j + off;
            let avail = (b.len() - p).min(PASTE_END.len());
            if b[p..p + avail] == PASTE_END[..avail] {
                if p > start {
                    out.push(InputEvent::PasteData(b[start..p].to_vec()));
                }
                if avail == PASTE_END.len() {
                    self.mode = Mode::Ground;
                    out.push(InputEvent::PasteEnd);
                    return p + avail;
                }
                self.paste_match = avail as u8;
                return b.len();
            }
            j = p + 1;
        }
        if b.len() > start {
            out.push(InputEvent::PasteData(b[start..].to_vec()));
        }
        b.len()
    }

    // ---- OSC / DCS / APC / PM / SOS --------------------------------------

    fn string(&mut self, b: &[u8], out: &mut Vec<InputEvent>) -> usize {
        let end = b.iter().position(|&c| c < 0x20).unwrap_or(b.len());
        let room = MAX_STRING - self.strbuf.len();
        if end > room {
            self.str_overflow = true;
        }
        self.strbuf.extend_from_slice(&b[..end.min(room)]);
        if end == b.len() {
            return end;
        }
        match b[end] {
            0x07 => {
                self.finish_string(out);
                end + 1
            }
            ESC => match b.get(end + 1) {
                Some(b'\\') => {
                    self.finish_string(out);
                    end + 2
                }
                Some(_) => {
                    // ESC starts something else: the string was cut short.
                    self.abort_string(out);
                    end
                }
                None => {
                    self.str_esc = true;
                    end + 1
                }
            },
            _ => {
                // Any other control byte cannot be part of a reply; it is
                // most likely a key typed after garbage. Give it back.
                self.abort_string(out);
                end
            }
        }
    }

    fn finish_string(&mut self, out: &mut Vec<InputEvent>) {
        self.mode = Mode::Ground;
        self.str_esc = false;
        let mut scratch = core::mem::take(&mut self.scratch);
        if strings::dispatch(self.str_kind, &self.strbuf, self.str_overflow, &mut scratch) {
            for r in scratch.drain(..) {
                self.observe(&r);
                out.push(InputEvent::Response(r));
            }
        } else {
            scratch.clear();
            out.push(InputEvent::Unknown(self.string_bytes(true)));
        }
        self.scratch = scratch;
        self.release_strbuf();
    }

    fn abort_string(&mut self, out: &mut Vec<InputEvent>) {
        self.mode = Mode::Ground;
        self.str_esc = false;
        out.push(InputEvent::Unknown(self.string_bytes(false)));
        self.release_strbuf();
    }

    fn string_bytes(&self, terminated: bool) -> Vec<u8> {
        let mut v = Vec::with_capacity(self.strbuf.len() + 4);
        v.push(ESC);
        v.push(self.str_kind.intro());
        v.extend_from_slice(&self.strbuf);
        if terminated {
            v.extend_from_slice(b"\x1b\\");
        }
        v
    }

    fn release_strbuf(&mut self) {
        if self.strbuf.capacity() > 4096 {
            self.strbuf = Vec::new();
        } else {
            self.strbuf.clear();
        }
        self.str_overflow = false;
    }

    // ---- ground-state decoding --------------------------------------------

    /// Decodes the item at the start of `b` (non-empty). None means more
    /// bytes are needed; with `fin` it never returns None.
    fn decode(&self, b: &[u8], fin: bool) -> Option<(Tok, usize)> {
        let c = b[0];
        match c {
            ESC => self.decode_esc(b, fin),
            0x00..=0x7f => Some((Tok::Key(byte_key(c)), 1)),
            0x9b => self.decode_csi(b, 1, fin, false),
            0x8f => decode_ss3(b, 1, fin, false),
            0x90 => Some((Tok::Str(StrKind::Dcs), 1)),
            0x9d => Some((Tok::Str(StrKind::Osc), 1)),
            0x9f => Some((Tok::Str(StrKind::Apc), 1)),
            0x9e => Some((Tok::Str(StrKind::Pm), 1)),
            0x98 => Some((Tok::Str(StrKind::Sos), 1)),
            _ => utf8(b, fin).map(|(ch, n)| (Tok::Key(K::typed(ch)), n)),
        }
    }

    fn decode_esc(&self, b: &[u8], fin: bool) -> Option<(Tok, usize)> {
        let Some(&c) = b.get(1) else {
            return if fin { Some((Tok::Key(K::new(Key::Escape, Mods::NONE)), 1)) } else { None };
        };
        let alt = |c: u8| Some((Tok::Key(byte_key(c).with_alt()), 2));
        match c {
            b'[' => self.decode_csi(b, 2, fin, true),
            b'O' => decode_ss3(b, 2, fin, true),
            b'P' | b']' | b'_' => {
                let Some(&s) = b.get(2) else {
                    return if fin { alt(c) } else { None };
                };
                let kind = match c {
                    b'P' if matches!(s, b'0'..=b'9' | b';' | b'<'..=b'?' | b'!' | b'$' | b'+') => StrKind::Dcs,
                    b']' if s.is_ascii_digit() => StrKind::Osc,
                    b'_' if s == b'G' => StrKind::Apc,
                    _ => return alt(c),
                };
                Some((Tok::Str(kind), 2))
            }
            ESC => {
                // ESC ESC <key> is Alt+<key> (rxvt, Terminal.app with
                // Option as Meta); ESC ESC <anything else> starts with a
                // plain Escape.
                let Some(&n) = b.get(2) else {
                    return if fin {
                        Some((Tok::Key(K::new(Key::Escape, Mods::NONE).with_alt()), 2))
                    } else {
                        None
                    };
                };
                if n == ESC {
                    return Some((Tok::Key(K::new(Key::Escape, Mods::NONE)), 1));
                }
                match self.decode_esc(&b[1..], fin)? {
                    (Tok::Key(k), len) => Some((Tok::Key(k.with_alt()), 1 + len)),
                    _ => Some((Tok::Key(K::new(Key::Escape, Mods::NONE)), 1)),
                }
            }
            0x00..=0x7f => alt(c),
            0xc2..=0xf4 => {
                let (ch, n) = utf8(&b[1..], fin)?;
                Some((Tok::Key(K::typed(ch).with_alt()), 1 + n))
            }
            _ => Some((Tok::Key(K::new(Key::Escape, Mods::NONE)), 1)),
        }
    }

    fn decode_csi(&self, b: &[u8], start: usize, fin: bool, esc_form: bool) -> Option<(Tok, usize)> {
        let body = &b[start..];
        match csi::scan(body) {
            Scan::Incomplete => {
                if !fin {
                    None
                } else if body.is_empty() && esc_form {
                    Some((Tok::Key(byte_key(b'[').with_alt()), 2))
                } else {
                    Some((Tok::Unknown, b.len()))
                }
            }
            Scan::Invalid(0) if esc_form => Some((Tok::Key(byte_key(b'[').with_alt()), 2)),
            Scan::Invalid(at) => Some((Tok::Unknown, start + at)),
            Scan::Done(c, n) => self.dispatch_csi(&c, b, start + n, fin),
        }
    }

    fn dispatch_csi(&self, c: &Csi, b: &[u8], end: usize, fin: bool) -> Option<(Tok, usize)> {
        if c.odd {
            return Some((Tok::Unknown, end));
        }
        // Forms followed by raw bytes.
        if c.prefix == 0 && c.inter == 0 && c.bare {
            if c.fin == b'M' {
                if b.len() < end + 3 {
                    return if fin { Some((Tok::Unknown, b.len())) } else { None };
                }
                let ev = mouse(
                    (b[end] as u32).saturating_sub(32),
                    (b[end + 1] as u32).saturating_sub(32),
                    (b[end + 2] as u32).saturating_sub(32),
                    false,
                    true,
                    None,
                );
                return Some((ev.map_or(Tok::Unknown, |e| Tok::Ev(InputEvent::Mouse(e))), end + 3));
            }
            if c.fin == b'[' {
                // Linux console F1-F5: CSI [ A .. CSI [ E.
                let Some(&x) = b.get(end) else {
                    return if fin { Some((Tok::Unknown, end)) } else { None };
                };
                return Some(match x {
                    b'A'..=b'E' => (Tok::Key(K::new(Key::F(x - b'A' + 1), Mods::NONE)), end + 1),
                    _ => (Tok::Unknown, end),
                });
            }
        }
        Some((self.csi_tok(c), end))
    }

    fn csi_tok(&self, c: &Csi) -> Tok {
        let resp = |r| Tok::Ev(InputEvent::Response(r));
        let np = c.nparams();
        match (c.prefix, c.inter, c.fin) {
            (b'<', 0, b'M' | b'm') if np == 3 => {
                let px = if self.sgr_pixels { Some(self.cell) } else { None };
                match mouse(c.param(0, 0), c.param(1, 1), c.param(2, 1), c.fin == b'm', false, px) {
                    Some(e) => Tok::Ev(InputEvent::Mouse(e)),
                    None => Tok::Unknown,
                }
            }
            (b'?', 0, b'c') => resp(TermResponse::PrimaryDa((0..np).map(|g| c.param(g, 0)).collect())),
            (b'>', 0, b'c') => {
                resp(TermResponse::SecondaryDa { model: c.param(0, 0), version: c.param(1, 0), rom: c.param(2, 0) })
            }
            (b'?', 0, b'u') => resp(TermResponse::KittyKeyboard(c.param(0, 0))),
            (b'?' | 0, b'$', b'y') if np >= 2 => resp(TermResponse::ModeReport {
                mode: c.param(0, 0),
                private: c.prefix == b'?',
                state: ModeState::from_param(c.param(1, 0)),
            }),
            (b'?', 0, b'R') if np >= 2 => {
                resp(TermResponse::CursorPosition { row: c.param(0, 1), col: c.param(1, 1) })
            }
            (b'?', 0, b'n') if np == 2 && c.param(0, 0) == 997 => match c.param(1, 0) {
                1 => resp(TermResponse::ColorScheme(ColorScheme::Dark)),
                2 => resp(TermResponse::ColorScheme(ColorScheme::Light)),
                _ => Tok::Unknown,
            },
            (0, 0, b't') => window_report(c).map_or(Tok::Unknown, resp),
            (0, 0, b'R') => {
                if np == 2 && !c.has_subparams() {
                    let (row, col) = (c.param(0, 1), c.param(1, 1));
                    if self.cpr_expected > 0 || row != 1 || !(2..=256).contains(&col) {
                        return resp(TermResponse::CursorPosition { row, col });
                    }
                }
                if np <= 2 { letter(c, Key::F(3)) } else { Tok::Unknown }
            }
            (0, 0, b'I') if np == 0 => Tok::Ev(InputEvent::Focus(true)),
            (0, 0, b'O') if np == 0 => Tok::Ev(InputEvent::Focus(false)),
            (0, 0, b'M') if np == 3 => {
                // urxvt 1015: decimal X10.
                match mouse(c.param(0, 0).saturating_sub(32), c.param(1, 1), c.param(2, 1), false, true, None) {
                    Some(e) => Tok::Ev(InputEvent::Mouse(e)),
                    None => Tok::Unknown,
                }
            }
            (0, 0, b'~') => tilde(c),
            (0, 0, b'u') if np >= 1 => kitty(c),
            (0, 0, b'Z') if np <= 2 => {
                let mut k = K::new(Key::Tab, mods_param(c.sub(1, 0, 1)).union(Mods::SHIFT));
                k.action = action_param(c.sub(1, 1, 1));
                Tok::Key(k)
            }
            (0, 0, f) if np <= 2 => match letter_key(f) {
                Some(key) => letter(c, key),
                None => Tok::Unknown,
            },
            _ => Tok::Unknown,
        }
    }
}

fn is_ambiguous(p: &[u8]) -> bool {
    let rest = match p {
        [ESC, ESC, rest @ ..] => rest,
        [ESC, rest @ ..] => rest,
        _ => return false,
    };
    matches!(rest, [] | [b'[' | b'O' | b'P' | b']' | b'_'])
}

/// One UTF-8 character at the start of `b` (lead byte ≥ 0x80). Invalid
/// bytes become U+FFFD, consuming the maximal invalid prefix.
fn utf8(b: &[u8], fin: bool) -> Option<(char, usize)> {
    let w = match b[0] {
        0xc2..=0xdf => 2,
        0xe0..=0xef => 3,
        0xf0..=0xf4 => 4,
        _ => return Some((char::REPLACEMENT_CHARACTER, 1)),
    };
    let avail = b.len().min(w);
    match core::str::from_utf8(&b[..avail]) {
        Ok(s) => Some((s.chars().next().unwrap_or(char::REPLACEMENT_CHARACTER), avail)),
        Err(e) => match e.error_len() {
            Some(n) => Some((char::REPLACEMENT_CHARACTER, n)),
            None if fin => Some((char::REPLACEMENT_CHARACTER, avail)),
            None => None,
        },
    }
}

fn decode_ss3(b: &[u8], start: usize, fin: bool, esc_form: bool) -> Option<(Tok, usize)> {
    // SS3 [digits[;digits]] final. The digits are an old modifier form
    // (`ESC O 5 A`, `ESC O 1 ; 5 A`).
    let mut i = start;
    let mut mods = 0u32;
    while let Some(&x) = b.get(i) {
        match x {
            b'0'..=b'9' => mods = (mods * 10 + (x - b'0') as u32).min(1000),
            b';' => mods = 0,
            _ => break,
        }
        i += 1;
        if i - start > 8 {
            return Some((Tok::Unknown, i));
        }
    }
    let Some(&f) = b.get(i) else {
        if !fin {
            return None;
        }
        return Some(if i == start && esc_form {
            (Tok::Key(byte_key(b'O').with_alt()), 2)
        } else {
            (Tok::Unknown, b.len())
        });
    };
    if !(0x40..=0x7e).contains(&f) {
        if i == start && esc_form {
            return Some((Tok::Key(byte_key(b'O').with_alt()), 2));
        }
        return Some((Tok::Unknown, i));
    }
    let m = mods_param(mods.max(1));
    let key = if let Some(k) = letter_key(f) {
        K::new(k, m)
    } else if let Some(p) = ss3_keypad(f) {
        let mut k = K::new(Key::Keypad(p), m);
        k.text = implied_text(&k, None, false);
        k
    } else {
        return Some((Tok::Unknown, i + 1));
    };
    Some((Tok::Key(key), i + 1))
}

fn letter(c: &Csi, key: Key) -> Tok {
    let mut k = K::new(key, mods_param(c.sub(1, 0, 1)));
    k.action = action_param(c.sub(1, 1, 1));
    Tok::Key(k)
}

fn tilde(c: &Csi) -> Tok {
    let n = c.param(0, 0);
    let np = c.nparams();
    match n {
        200 if np == 1 => Tok::PasteStart,
        27 if np == 3 => {
            // xterm modifyOtherKeys: CSI 27 ; mods ; code ~
            let mp = c.param(1, 1);
            match code_key(c.param(2, 0)) {
                Some(key) => {
                    let mut k = K::new(key, mods_param(mp));
                    k.text = implied_text(&k, None, caps_lock(mp));
                    Tok::Key(k)
                }
                None => Tok::Unknown,
            }
        }
        _ if np <= 2 => match tilde_key(n) {
            Some(key) => letter(c, key),
            None => Tok::Unknown,
        },
        _ => Tok::Unknown,
    }
}

/// `CSI code[:shifted[:base]] [; mods[:event] [; text-codepoints]] u` (kitty),
/// which also covers fixterms `CSI code ; mods u`.
fn kitty(c: &Csi) -> Tok {
    if c.nparams() > 3 {
        return Tok::Unknown;
    }
    let code = c.sub(0, 0, 0);
    let alternate = |i| match c.sub(0, i, 0) {
        0 => None,
        s => char::from_u32(s).filter(|ch| !ch.is_control()),
    };
    let (shifted, base_layout) = (alternate(1), alternate(2));
    let mp = c.sub(1, 0, 1);
    let mut text = Text::EMPTY;
    for &cp in c.group(2) {
        if let Some(ch) = char::from_u32(cp)
            && !ch.is_control()
        {
            text.push(ch);
        }
    }
    let key = if code == 0 {
        // A pure text event (e.g. from an input method).
        match text.as_str().chars().next() {
            Some(ch) => Key::Char(ch),
            None => return Tok::Unknown,
        }
    } else {
        match code_key(code) {
            Some(k) => k,
            None => return Tok::Unknown,
        }
    };
    let mut k = K::new(key, mods_param(mp));
    k.action = action_param(c.sub(1, 1, 1));
    k.text = text;
    if let Key::Char(ch) = key {
        k.shifted = shifted;
        k.base_layout = base_layout.filter(|&b| b != ch);
    }
    if k.text.is_empty() {
        k.text = implied_text(&k, shifted, caps_lock(mp));
    }
    Tok::Key(k)
}

fn window_report(c: &Csi) -> Option<TermResponse> {
    let p = |g| c.param(g, 0).min(u16::MAX as u32) as u16;
    Some(match (c.param(0, 0), c.nparams()) {
        (48, 5) => TermResponse::InBandResize {
            cells: Size::new(p(2), p(1)),
            pixels: PixelSize { width: p(4), height: p(3) },
        },
        (4, 3) => TermResponse::TextAreaPixels(PixelSize { width: p(2), height: p(1) }),
        (8, 3) => TermResponse::TextAreaCells(Size::new(p(2), p(1))),
        (6, 3) => TermResponse::CellSize(PixelSize { width: p(2), height: p(1) }),
        _ => return None,
    })
}

/// Shared by SGR, X10 and urxvt mouse. `cb` is the button byte without the
/// X10 offset; `x`, `y` are 1-based cells, or pixels when `pixels` is Some
/// (holding the cell size).
fn mouse(cb: u32, x: u32, y: u32, release: bool, legacy: bool, pixels: Option<PixelSize>) -> Option<MouseEvent> {
    let mut mods = Mods::NONE;
    if cb & 4 != 0 {
        mods = mods.union(Mods::SHIFT);
    }
    if cb & 8 != 0 {
        mods = mods.union(Mods::ALT);
    }
    if cb & 16 != 0 {
        mods = mods.union(Mods::CTRL);
    }
    let motion = cb & 32 != 0;
    let base = cb & !(4 | 8 | 16 | 32);
    let button = match (base & !3, base & 3) {
        (0, 0) => Some(MouseButton::Left),
        (0, 1) => Some(MouseButton::Middle),
        (0, 2) => Some(MouseButton::Right),
        (0, 3) => None,
        (64, 0) => Some(MouseButton::WheelUp),
        (64, 1) => Some(MouseButton::WheelDown),
        (64, 2) => Some(MouseButton::WheelLeft),
        (64, 3) => Some(MouseButton::WheelRight),
        (128, 0) => Some(MouseButton::Back),
        (128, 1) => Some(MouseButton::Forward),
        _ => return None,
    };
    let action = if motion {
        MouseAction::Motion
    } else if release || (legacy && base == 3) {
        MouseAction::Release
    } else {
        MouseAction::Press
    };
    let c16 = |v: u32| v.min(u16::MAX as u32) as u16;
    let (cell, pixel) = match pixels {
        Some(cs) => {
            let cell = if cs.width > 0 && cs.height > 0 {
                Pos { x: c16(x / cs.width as u32), y: c16(y / cs.height as u32) }
            } else {
                Pos::default()
            };
            (cell, Some((c16(x), c16(y))))
        }
        None => (Pos { x: c16(x.saturating_sub(1)), y: c16(y.saturating_sub(1)) }, None),
    };
    Some(MouseEvent { action, button, mods, cell, pixel })
}
