//! Choosing the cheapest cursor movement.
//!
//! The cursor position may be unknown (`None`) on either axis, for example
//! after printing into the last column (with autowrap off terminals disagree
//! on where the cursor is) or after a grapheme cluster whose width the outer
//! terminal might compute differently.
//!
//! Linefeed is used only to reach column 0 (as `CR LF`), so a client tty
//! that still translates NL to CR NL cannot break it.

use crate::esc::{Sink, csi_n, csi_n_cost, digits, num};

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
enum V {
    None,
    Down(u16),
    Up(u16),
    Vpa,
}

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
enum H {
    None,
    Fwd(u16),
    Reprint,
    Bs(u16),
    Cub(u16),
    Cr,
    CrFwd,
    Cha,
}

fn v_cost(v: V, ty: u16) -> usize {
    match v {
        V::None => 0,
        V::Down(n) => csi_n_cost(n as u32),
        V::Up(1) => 2,
        V::Up(n) => csi_n_cost(n as u32),
        V::Vpa => csi_n_cost(ty as u32 + 1),
    }
}

fn h_cost(h: H, tx: u16, reprint: usize) -> usize {
    match h {
        H::None => 0,
        H::Fwd(n) => csi_n_cost(n as u32),
        H::Reprint => reprint,
        H::Bs(n) => n as usize,
        H::Cub(n) => csi_n_cost(n as u32),
        H::Cr => 1,
        H::CrFwd => 1 + csi_n_cost(tx as u32),
        H::Cha => csi_n_cost(tx as u32 + 1),
    }
}

fn cup_cost(tx: u16, ty: u16) -> usize {
    match (tx, ty) {
        (0, 0) => 3,
        (0, y) => 3 + digits(y as u32 + 1),
        (x, y) => 4 + digits(y as u32 + 1) + digits(x as u32 + 1),
    }
}

/// Append the cheapest bytes that move the cursor from (`fx`, `fy`) to
/// (`tx`, `ty`). `reprint`, when not empty, is what the screen already
/// shows between `fx` and `tx` on the same row in the current SGR, so
/// printing it again moves the cursor without changing anything.
pub(crate) fn move_cursor<S: Sink>(
    out: &mut S,
    fx: Option<u16>,
    fy: Option<u16>,
    tx: u16,
    ty: u16,
    reprint: &[u8],
) {
    if fx == Some(tx) && fy == Some(ty) {
        return;
    }
    // Vertical part (keeps the column).
    let vpa = (v_cost(V::Vpa, ty), V::Vpa);
    let (vc, v) = match fy {
        Some(y) if y == ty => (0, V::None),
        Some(y) if ty > y => {
            let d = (v_cost(V::Down(ty - y), ty), V::Down(ty - y));
            if d.0 <= vpa.0 { d } else { vpa }
        }
        Some(y) => {
            let u = (v_cost(V::Up(y - ty), ty), V::Up(y - ty));
            if u.0 <= vpa.0 { u } else { vpa }
        }
        None => vpa,
    };
    // Horizontal part.
    let mut hs: [(usize, H); 3] = [(usize::MAX, H::None); 3];
    hs[0] = if tx == 0 {
        (1, H::Cr)
    } else {
        (h_cost(H::CrFwd, tx, 0), H::CrFwd)
    };
    hs[1] = (h_cost(H::Cha, tx, 0), H::Cha);
    match fx {
        Some(x) if x == tx => hs[2] = (0, H::None),
        Some(x) if tx > x => {
            let n = tx - x;
            hs[2] = if v == V::None
                && reprint.len() == n as usize
                && reprint.len() < csi_n_cost(n as u32)
            {
                (reprint.len(), H::Reprint)
            } else {
                (h_cost(H::Fwd(n), tx, 0), H::Fwd(n))
            };
        }
        Some(x) => {
            let n = x - tx;
            let bs = (n as usize, H::Bs(n));
            let cub = (h_cost(H::Cub(n), tx, 0), H::Cub(n));
            hs[2] = if bs.0 <= cub.0 { bs } else { cub };
        }
        None => {}
    }
    let (hc, h) = *hs.iter().min_by_key(|c| c.0).unwrap();
    let mut best = vc + hc;
    // 0 = vertical + horizontal, 1 = CR LF..., 2 = CUP.
    let mut kind = 0;
    if let Some(y) = fy
        && tx == 0
        && ty > y
    {
        let c = usize::from(fx != Some(0)) + (ty - y) as usize;
        if c < best {
            best = c;
            kind = 1;
        }
    }
    if cup_cost(tx, ty) < best {
        kind = 2;
    }
    match kind {
        1 => {
            if fx != Some(0) {
                out.byte(b'\r');
            }
            for _ in 0..ty - fy.unwrap() {
                out.byte(b'\n');
            }
        }
        2 => {
            out.bytes(b"\x1b[");
            if tx != 0 || ty != 0 {
                num(out, ty as u32 + 1);
            }
            if tx != 0 {
                out.byte(b';');
                num(out, tx as u32 + 1);
            }
            out.byte(b'H');
        }
        _ => {
            match v {
                V::None => {}
                V::Down(n) => csi_n(out, n as u32, b'B'),
                V::Up(1) => out.bytes(b"\x1bM"),
                V::Up(n) => csi_n(out, n as u32, b'A'),
                V::Vpa => csi_n(out, ty as u32 + 1, b'd'),
            }
            match h {
                H::None => {}
                H::Fwd(n) => csi_n(out, n as u32, b'C'),
                H::Reprint => out.bytes(reprint),
                H::Bs(n) => {
                    for _ in 0..n {
                        out.byte(8);
                    }
                }
                H::Cub(n) => csi_n(out, n as u32, b'D'),
                H::Cr => out.byte(b'\r'),
                H::CrFwd => {
                    out.byte(b'\r');
                    csi_n(out, tx as u32, b'C');
                }
                H::Cha => csi_n(out, tx as u32 + 1, b'G'),
            }
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn mv(fx: Option<u16>, fy: Option<u16>, tx: u16, ty: u16, reprint: &[u8]) -> String {
        let mut out = Vec::new();
        move_cursor(&mut out, fx, fy, tx, ty, reprint);
        String::from_utf8(out).unwrap()
    }

    #[test]
    fn no_move() {
        assert_eq!(mv(Some(3), Some(4), 3, 4, b""), "");
    }

    #[test]
    fn same_row() {
        assert_eq!(mv(Some(3), Some(4), 4, 4, b""), "\x1b[C");
        assert_eq!(mv(Some(3), Some(4), 13, 4, b""), "\x1b[14G");
        assert_eq!(mv(Some(3), Some(4), 9, 4, b""), "\x1b[6C");
        assert_eq!(mv(Some(3), Some(4), 5, 4, b"ab"), "ab");
        assert_eq!(mv(Some(3), Some(4), 4, 4, b"a"), "a");
        assert_eq!(mv(Some(5), Some(4), 4, 4, b""), "\x08");
        assert_eq!(mv(Some(5), Some(4), 2, 4, b""), "\x08\x08\x08");
        assert_eq!(mv(Some(50), Some(4), 2, 4, b""), "\x1b[3G");
        assert_eq!(mv(Some(50), Some(4), 0, 4, b""), "\r");
        assert_eq!(mv(Some(90), Some(4), 40, 4, b""), "\x1b[41G");
        // Unknown column: CR-based or CHA.
        assert_eq!(mv(None, Some(4), 0, 4, b""), "\r");
        assert_eq!(mv(None, Some(4), 1, 4, b""), "\r\x1b[C");
        assert_eq!(mv(None, Some(4), 60, 4, b""), "\x1b[61G");
    }

    #[test]
    fn rows() {
        assert_eq!(mv(Some(7), Some(4), 0, 5, b""), "\r\n");
        assert_eq!(mv(Some(0), Some(4), 0, 5, b""), "\n");
        assert_eq!(mv(Some(7), Some(4), 7, 5, b""), "\x1b[B");
        assert_eq!(mv(Some(7), Some(4), 7, 3, b""), "\x1bM");
        assert_eq!(mv(Some(7), Some(4), 7, 1, b""), "\x1b[3A");
        assert_eq!(mv(Some(7), Some(4), 8, 5, b""), "\x1b[B\x1b[C");
        assert_eq!(mv(Some(7), Some(4), 0, 3, b""), "\x1bM\r");
        // Far away: CUP.
        assert_eq!(mv(Some(70), Some(40), 10, 3, b""), "\x1b[4;11H");
        assert_eq!(mv(None, None, 0, 0, b""), "\x1b[H");
        assert_eq!(mv(None, None, 0, 9, b""), "\x1b[10H");
        assert_eq!(mv(None, None, 5, 9, b""), "\x1b[10;6H");
        // Unknown row, known column: VPA keeps the column.
        assert_eq!(mv(Some(5), None, 5, 9, b""), "\x1b[10d");
        // Reprinting is only for the same row.
        assert_eq!(mv(Some(3), Some(3), 5, 4, b"ab"), "\x1b[5;6H");
    }
}
