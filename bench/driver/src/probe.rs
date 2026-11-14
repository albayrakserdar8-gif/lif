//! `escprobe`: runs inside a pane. Puts its terminal in raw mode and, for
//! every ESC byte it receives, prints a marker character and erases it again
//! shortly after, so the driver can time ESC delivery through a multiplexer.

use std::time::Duration;

pub fn escprobe(ready_marker: &str, mark: u8) -> i32 {
    // SAFETY: termios calls on fd 0 with a zeroed, then filled, struct.
    let saved = unsafe {
        let mut t: libc::termios = std::mem::zeroed();
        if libc::tcgetattr(0, &mut t) != 0 {
            eprintln!("escprobe: stdin is not a terminal");
            return 1;
        }
        let saved = t;
        libc::cfmakeraw(&mut t);
        libc::tcsetattr(0, libc::TCSANOW, &t);
        saved
    };
    // Raw mode is on, so an ESC sent from now on arrives as a byte.
    if std::fs::write(ready_marker, b"").is_err() {
        return 1;
    }
    let out = |b: &[u8]| {
        // SAFETY: b is a valid readable slice.
        unsafe { libc::write(1, b.as_ptr().cast(), b.len()) };
    };
    let mut buf = [0u8; 256];
    loop {
        // SAFETY: buf is a valid writable buffer.
        let n = unsafe { libc::read(0, buf.as_mut_ptr().cast(), buf.len()) };
        if n <= 0 {
            break;
        }
        let got = &buf[..n as usize];
        // Ctrl-C or Ctrl-D ends the probe (signals are off in raw mode).
        if got.contains(&3) || got.contains(&4) {
            break;
        }
        for _ in got.iter().filter(|&&b| b == 0x1b) {
            out(&[mark]);
            std::thread::sleep(Duration::from_millis(2));
            out(b"\x08 \x08");
        }
    }
    // SAFETY: restoring the attributes read above.
    unsafe { libc::tcsetattr(0, libc::TCSANOW, &saved) };
    0
}
