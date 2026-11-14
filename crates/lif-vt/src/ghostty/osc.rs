//! OSC handling libghostty leaves to the embedder: OSC 52 clipboard reads
//! (which libghostty can only answer synchronously) and OSC 99 notifications.

use libghostty_vt::Terminal;
use libghostty_vt::terminal::{Boundary, StringEnd};

/// OSC content kept when it is neither OSC 52 nor OSC 99: enough to read
/// the command number.
const WATCH_MAX: usize = 32;
/// Bound on one OSC 99 sequence and on an assembled notification.
pub(super) const OSC99_MAX: usize = 8 * 1024;

/// What `OscWatch` found in the stream.
pub(super) enum OscEvent {
    /// OSC 52 read request for this selection.
    ClipboardRead(u8),
    Notification { title: String, body: String },
}

/// Feeds the terminal while spotting the OSC strings libghostty does not
/// surface the way lif needs them:
///
/// - OSC 52 reads: libghostty answers a read inside its callback (an empty
///   answer if the callback does not reply), but lif answers later, after
///   asking the client. So no read callback is installed (libghostty then
///   ignores the request) and the request is recognized here.
/// - OSC 99 (kitty notifications): libghostty parses it but has no callback.
///
/// Recognition uses the OSC boundaries `vt_write_until` reports, from the
/// same parse that updates the terminal.
#[derive(Default)]
pub(super) struct OscWatch {
    active: bool,
    buf: Vec<u8>,
    overflow: bool,
    notify: Osc99,
}

impl OscWatch {
    pub(super) fn write(
        &mut self,
        term: &mut Terminal<'static, 'static>,
        bytes: &[u8],
        mut on_event: impl FnMut(OscEvent),
    ) {
        let want = Boundary::OSC_START | Boundary::OSC_END;
        let mut off = 0;
        while off < bytes.len() {
            let rest = &bytes[off..];
            let stop = match term.vt_write_until(rest, want) {
                Ok(Some(stop)) if stop.consumed > 0 => stop,
                Ok(None) => {
                    if self.active {
                        self.capture(rest);
                    }
                    return;
                }
                // Cannot happen with these flags; still never drop input.
                Ok(Some(_)) | Err(_) => {
                    term.vt_write(rest);
                    self.active = false;
                    return;
                }
            };
            let consumed = &rest[..stop.consumed];
            off += stop.consumed;
            if stop.reached.contains(Boundary::OSC_END) {
                if self.active {
                    // The last consumed byte is the terminator.
                    self.capture(&consumed[..consumed.len() - 1]);
                    let completed =
                        !matches!(stop.end, Some(StringEnd::Cancelled | StringEnd::Aborted));
                    if completed && let Some(ev) = self.finish() {
                        on_event(ev);
                    }
                }
                self.active = false;
            } else if self.active {
                self.capture(consumed);
            }
            if stop.reached.contains(Boundary::OSC_START) {
                self.active = true;
                self.buf.clear();
                self.overflow = false;
            }
        }
    }

    fn capture(&mut self, data: &[u8]) {
        let max = if self.buf.starts_with(b"99;") { OSC99_MAX } else { WATCH_MAX };
        let room = max.saturating_sub(self.buf.len());
        let n = data.len().min(room);
        self.buf.extend_from_slice(&data[..n]);
        self.overflow |= n < data.len();
        if n < data.len() && self.buf.starts_with(b"99;") && max == WATCH_MAX {
            // The prefix only became known with this chunk: retry with the
            // larger bound.
            self.overflow = false;
            let more = (OSC99_MAX - self.buf.len()).min(data.len() - n);
            self.buf.extend_from_slice(&data[n..n + more]);
            self.overflow = n + more < data.len();
        }
    }

    fn finish(&mut self) -> Option<OscEvent> {
        if self.overflow {
            return None;
        }
        if let Some(rest) = self.buf.strip_prefix(b"52;") {
            // `52;<selection>;?` -> the first selection byte (`c` if none).
            let semi = rest.iter().position(|&b| b == b';')?;
            if &rest[semi + 1..] != b"?" {
                return None;
            }
            return Some(OscEvent::ClipboardRead(rest[..semi].first().copied().unwrap_or(b'c')));
        }
        let rest = self.buf.strip_prefix(b"99;")?;
        let (title, body) = self.notify.feed(rest)?;
        Some(OscEvent::Notification { title, body })
    }
}

/// Assembles kitty desktop notifications (OSC 99), which may arrive in
/// chunks (`d=0`) and base64-encoded (`e=1`).
#[derive(Default)]
pub(super) struct Osc99 {
    title: String,
    body: String,
}

impl Osc99 {
    /// `seq` is the content after `99;`. Returns a finished (title, body).
    pub(super) fn feed(&mut self, seq: &[u8]) -> Option<(String, String)> {
        let (meta, payload) = match seq.iter().position(|&b| b == b';') {
            Some(i) => (&seq[..i], &seq[i + 1..]),
            None => (seq, &[][..]),
        };
        let mut done = true;
        let mut part: &[u8] = b"title";
        let mut b64 = false;
        for kv in meta.split(|&b| b == b':') {
            match kv {
                [b'd', b'=', v @ ..] => done = v != b"0",
                [b'p', b'=', v @ ..] => part = v,
                [b'e', b'=', v @ ..] => b64 = v == b"1",
                _ => {}
            }
        }
        let decoded;
        let text = if b64 {
            decoded = base64_decode(payload);
            String::from_utf8_lossy(&decoded)
        } else {
            String::from_utf8_lossy(payload)
        };
        let dst = match part {
            b"title" => Some(&mut self.title),
            b"body" => Some(&mut self.body),
            // Icons, buttons, queries (`p=?`) and close requests carry no text.
            _ => None,
        };
        if let Some(dst) = dst
            && dst.len() + text.len() <= OSC99_MAX
        {
            dst.push_str(&text);
        }
        if !done || (self.title.is_empty() && self.body.is_empty()) {
            return None;
        }
        Some((std::mem::take(&mut self.title), std::mem::take(&mut self.body)))
    }
}

const B64: &[u8; 64] = b"ABCDEFGHIJKLMNOPQRSTUVWXYZabcdefghijklmnopqrstuvwxyz0123456789+/";

pub(super) fn base64_encode(data: &[u8], out: &mut Vec<u8>) {
    out.reserve(data.len().div_ceil(3) * 4);
    for chunk in data.chunks(3) {
        let b = [chunk[0], *chunk.get(1).unwrap_or(&0), *chunk.get(2).unwrap_or(&0)];
        let n = (u32::from(b[0]) << 16) | (u32::from(b[1]) << 8) | u32::from(b[2]);
        out.push(B64[(n >> 18) as usize & 63]);
        out.push(B64[(n >> 12) as usize & 63]);
        out.push(if chunk.len() > 1 { B64[(n >> 6) as usize & 63] } else { b'=' });
        out.push(if chunk.len() > 2 { B64[n as usize & 63] } else { b'=' });
    }
}

fn base64_decode(data: &[u8]) -> Vec<u8> {
    let mut out = Vec::with_capacity(data.len() / 4 * 3);
    let (mut acc, mut bits) = (0u32, 0u32);
    for &c in data {
        let v = match c {
            b'A'..=b'Z' => c - b'A',
            b'a'..=b'z' => c - b'a' + 26,
            b'0'..=b'9' => c - b'0' + 52,
            b'+' | b'-' => 62,
            b'/' | b'_' => 63,
            _ => continue,
        };
        acc = (acc << 6) | u32::from(v);
        bits += 6;
        if bits >= 8 {
            bits -= 8;
            out.push((acc >> bits) as u8);
        }
    }
    out
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn base64_round_trip() {
        for s in [&b""[..], b"f", b"fo", b"foo", b"foob", b"hello world\xff"] {
            let mut enc = Vec::new();
            base64_encode(s, &mut enc);
            assert_eq!(base64_decode(&enc), s);
        }
        let mut enc = Vec::new();
        base64_encode(b"hi", &mut enc);
        assert_eq!(enc, b"aGk=");
    }

    #[test]
    fn osc99_chunks() {
        let mut n = Osc99::default();
        assert_eq!(n.feed(b"i=1:d=0;Hello"), None);
        assert_eq!(n.feed(b"i=1:p=body;World"), Some(("Hello".into(), "World".into())));
        assert_eq!(n.feed(b"e=1;aGk="), Some(("hi".into(), String::new())));
    }
}
