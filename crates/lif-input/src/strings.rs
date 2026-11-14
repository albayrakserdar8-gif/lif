//! OSC / DCS reply decoding.

use crate::{ColorSlot, Rgb, TermResponse};

#[derive(Clone, Copy, PartialEq, Eq, Debug)]
pub(crate) enum StrKind {
    Osc,
    Dcs,
    Apc,
    Pm,
    Sos,
}

impl StrKind {
    /// The 7-bit introducer's second byte.
    pub fn intro(self) -> u8 {
        match self {
            StrKind::Osc => b']',
            StrKind::Dcs => b'P',
            StrKind::Apc => b'_',
            StrKind::Pm => b'^',
            StrKind::Sos => b'X',
        }
    }
}

/// Decodes a complete string body. Pushes zero or more responses; returns
/// false if the body is not something we understand.
pub(crate) fn dispatch(kind: StrKind, body: &[u8], truncated: bool, out: &mut Vec<TermResponse>) -> bool {
    match kind {
        StrKind::Osc => osc(body, truncated, out),
        StrKind::Dcs if !truncated => dcs(body, out),
        _ => false,
    }
}

fn split1(b: &[u8], sep: u8) -> (&[u8], Option<&[u8]>) {
    match b.iter().position(|&c| c == sep) {
        Some(i) => (&b[..i], Some(&b[i + 1..])),
        None => (b, None),
    }
}

fn num(b: &[u8]) -> Option<u32> {
    if b.is_empty() || b.len() > 9 {
        return None;
    }
    let mut v = 0u32;
    for &c in b {
        if !c.is_ascii_digit() {
            return None;
        }
        v = v * 10 + (c - b'0') as u32;
    }
    Some(v)
}

fn osc(body: &[u8], truncated: bool, out: &mut Vec<TermResponse>) -> bool {
    let (n, rest) = split1(body, b';');
    let Some(n) = num(n) else { return false };
    let Some(rest) = rest else { return false };
    let before = out.len();
    match n {
        52 => {
            let (sel, data) = split1(rest, b';');
            let Some(data) = data else { return false };
            if data == b"?" {
                return false;
            }
            let Some(decoded) = base64(data) else { return false };
            out.push(TermResponse::Clipboard {
                selection: String::from_utf8_lossy(sel).into_owned(),
                data: decoded,
                truncated,
            });
        }
        _ if truncated => return false,
        4 => {
            let mut it = rest.split(|&c| c == b';');
            while let (Some(i), Some(spec)) = (it.next(), it.next()) {
                match (num(i), parse_color(spec)) {
                    (Some(i), Some(rgb)) if i <= 255 => {
                        out.push(TermResponse::Color { slot: ColorSlot::Palette(i as u8), rgb })
                    }
                    _ => {}
                }
            }
        }
        10..=12 => {
            // "OSC 10 ; fg ; bg" reports consecutive dynamic colors.
            for (k, spec) in rest.split(|&c| c == b';').enumerate() {
                let slot = match n as usize + k {
                    10 => ColorSlot::Foreground,
                    11 => ColorSlot::Background,
                    12 => ColorSlot::Cursor,
                    _ => break,
                };
                if let Some(rgb) = parse_color(spec) {
                    out.push(TermResponse::Color { slot, rgb });
                }
            }
        }
        _ => {}
    }
    out.len() > before
}

fn hexval(c: u8) -> Option<u32> {
    (c as char).to_digit(16)
}

/// `rgb:r/g/b` (1-4 hex digits each, scaled) or `#rgb` forms (1-4 digits
/// each, high bits).
pub(crate) fn parse_color(spec: &[u8]) -> Option<Rgb> {
    let comp_scaled = |s: &[u8]| -> Option<u8> {
        if s.is_empty() || s.len() > 4 {
            return None;
        }
        let mut v = 0u32;
        for &c in s {
            v = v * 16 + hexval(c)?;
        }
        let max = (1u32 << (4 * s.len())) - 1;
        Some(((v * 255 + max / 2) / max) as u8)
    };
    if let Some(rest) = spec.strip_prefix(b"rgb:").or_else(|| spec.strip_prefix(b"rgba:")) {
        let mut it = rest.split(|&c| c == b'/');
        let r = comp_scaled(it.next()?)?;
        let g = comp_scaled(it.next()?)?;
        let b = comp_scaled(it.next()?)?;
        return Some(Rgb::new(r, g, b));
    }
    let hex = spec.strip_prefix(b"#")?;
    if hex.is_empty() || !hex.len().is_multiple_of(3) || hex.len() > 12 {
        return None;
    }
    let w = hex.len() / 3;
    let comp = |s: &[u8]| -> Option<u8> {
        let mut v = 0u32;
        for &c in s {
            v = v * 16 + hexval(c)?;
        }
        let bits = 4 * w as u32;
        Some(if bits >= 8 { (v >> (bits - 8)) as u8 } else { (v << (8 - bits)) as u8 })
    };
    Some(Rgb::new(comp(&hex[..w])?, comp(&hex[w..2 * w])?, comp(&hex[2 * w..])?))
}

/// Standard base64 with optional padding; whitespace is ignored. Returns
/// None on any other byte.
pub(crate) fn base64(s: &[u8]) -> Option<Vec<u8>> {
    let mut out = Vec::with_capacity(s.len() / 4 * 3 + 3);
    let mut acc = 0u32;
    let mut n = 0;
    for &c in s {
        let v = match c {
            b'A'..=b'Z' => c - b'A',
            b'a'..=b'z' => c - b'a' + 26,
            b'0'..=b'9' => c - b'0' + 52,
            b'+' => 62,
            b'/' => 63,
            b'=' => break,
            b' ' | b'\n' | b'\r' | b'\t' => continue,
            _ => return None,
        };
        acc = (acc << 6) | v as u32;
        n += 1;
        if n == 4 {
            out.extend_from_slice(&[(acc >> 16) as u8, (acc >> 8) as u8, acc as u8]);
            acc = 0;
            n = 0;
        }
    }
    match n {
        2 => out.push((acc >> 4) as u8),
        3 => out.extend_from_slice(&[(acc >> 10) as u8, (acc >> 2) as u8]),
        _ => {}
    }
    Some(out)
}

fn unhex(s: &[u8]) -> Option<String> {
    if !s.len().is_multiple_of(2) {
        return None;
    }
    let bytes: Option<Vec<u8>> =
        s.chunks(2).map(|p| Some((hexval(p[0])? * 16 + hexval(p[1])?) as u8)).collect();
    Some(String::from_utf8_lossy(&bytes?).into_owned())
}

fn dcs(body: &[u8], out: &mut Vec<TermResponse>) -> bool {
    if let Some(v) = body.strip_prefix(b">|") {
        out.push(TermResponse::XtVersion(String::from_utf8_lossy(v).into_owned()));
        return true;
    }
    if let Some(v) = body.strip_prefix(b"!|") {
        out.push(TermResponse::TertiaryDa(String::from_utf8_lossy(v).into_owned()));
        return true;
    }
    if let Some(v) = body.strip_prefix(b"1+r") {
        let mut caps = Vec::new();
        for item in v.split(|&c| c == b';') {
            let (name, value) = split1(item, b'=');
            let Some(name) = unhex(name) else { return false };
            let value = match value {
                Some(v) => match unhex(v) {
                    Some(v) => Some(v),
                    None => return false,
                },
                None => None,
            };
            caps.push((name, value));
        }
        out.push(TermResponse::TermCap(caps));
        return true;
    }
    if let Some(v) = body.strip_prefix(b"0+r") {
        let mut names = Vec::new();
        if !v.is_empty() {
            for item in v.split(|&c| c == b';') {
                let Some(name) = unhex(item) else { return false };
                names.push(name);
            }
        }
        out.push(TermResponse::TermCapFailed(names));
        return true;
    }
    false
}
