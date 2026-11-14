//! Small allocation-free helpers for writing escape sequences.

/// Something escape sequences can be written into: the output `Vec`, or a
/// test buffer.
pub(crate) trait Sink {
    fn byte(&mut self, b: u8);
    fn bytes(&mut self, b: &[u8]);
}

impl Sink for Vec<u8> {
    #[inline]
    fn byte(&mut self, b: u8) {
        self.push(b);
    }
    #[inline]
    fn bytes(&mut self, b: &[u8]) {
        self.extend_from_slice(b);
    }
}

/// Decimal digits of `n`.
#[inline]
pub(crate) fn digits(n: u32) -> usize {
    match n {
        0..=9 => 1,
        10..=99 => 2,
        100..=999 => 3,
        1000..=9999 => 4,
        10000..=99999 => 5,
        _ => 10,
    }
}

#[inline]
pub(crate) fn num<S: Sink>(out: &mut S, n: u32) {
    // Nearly every number in an escape sequence is a color component or a
    // coordinate below 1000.
    if n < 10 {
        out.byte(b'0' + n as u8);
        return;
    }
    if n < 100 {
        out.bytes(&[b'0' + (n / 10) as u8, b'0' + (n % 10) as u8]);
        return;
    }
    if n < 1000 {
        out.bytes(&[
            b'0' + (n / 100) as u8,
            b'0' + (n / 10 % 10) as u8,
            b'0' + (n % 10) as u8,
        ]);
        return;
    }
    let mut buf = [0u8; 10];
    let mut i = buf.len();
    let mut n = n;
    loop {
        i -= 1;
        buf[i] = b'0' + (n % 10) as u8;
        n /= 10;
        if n == 0 {
            break;
        }
    }
    out.bytes(&buf[i..]);
}

/// `CSI n final`, leaving out the parameter when it is the default 1.
#[inline]
pub(crate) fn csi_n<S: Sink>(out: &mut S, n: u32, fin: u8) {
    out.bytes(b"\x1b[");
    if n != 1 {
        num(out, n);
    }
    out.byte(fin);
}

/// Cost in bytes of [`csi_n`].
#[inline]
pub(crate) fn csi_n_cost(n: u32) -> usize {
    if n == 1 { 3 } else { 3 + digits(n) }
}
