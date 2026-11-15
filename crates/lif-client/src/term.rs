//! Raw mode with guaranteed restore.

use std::io;
use std::os::fd::RawFd;
use std::sync::{Mutex, Once};
use std::time::{Duration, Instant};

use crate::sys;

/// Restores the outer terminal when the server cannot (it crashed, hung,
/// or the client is panicking): a self-contained superset of what
/// `lif_input::setup` (`write_enter`, `write_mouse`, the mode switches and
/// `write_leave`) can turn on, plus a few widely supported resets. Any
/// terminal ignores sequences it does not implement, so a superset is safe.
/// Kept here rather than built by `lif-input` so the client does not depend
/// on the input parser; a test checks it covers everything `setup.rs` emits.
///
/// Order matters: CAN first aborts an escape sequence the server may have
/// been cut off in the middle of; synchronized output ends first so the
/// rest takes effect at once; the kitty pop comes before leaving the
/// alternate screen because kitty keeps one flag stack per screen; leaving
/// the alternate screen comes last and restores the shell's cursor.
pub const FALLBACK_RESTORE: &[u8] = b"\x18\
\x1b[?2026l\
\x1b[?1003l\x1b[?1002l\x1b[?1000l\x1b[?1016l\x1b[?1006l\
\x1b[?2048l\x1b[?2031l\
\x1b[<u\x1b[>4m\
\x1b[?1004l\x1b[?2004l\
\x1b>\
\x1b[0 q\x1b[0m\x1b[?25h\x1b[?7h\
\x1b[?1049l";

/// How long the panic hook may spend writing [`FALLBACK_RESTORE`] to a
/// terminal that does not take it.
const PANIC_WRITE: Duration = Duration::from_millis(200);

/// What to put back on one terminal.
#[derive(Clone, Copy)]
struct Saved {
    id: u64,
    tty: RawFd,
    /// Where [`FALLBACK_RESTORE`] goes if we panic.
    output: RawFd,
    termios: libc::termios,
    /// (fd, original fcntl flags) for the input and output fds.
    flags: [(RawFd, libc::c_int); 2],
}

impl Saved {
    fn restore(&self) {
        for &(fd, fl) in &self.flags {
            let _ = sys::set_flags(fd, fl);
        }
        // SAFETY: termios was filled by tcgetattr on this fd.
        unsafe { libc::tcsetattr(self.tty, libc::TCSANOW, &self.termios) };
    }
}

/// Every terminal currently in raw mode. Read by the panic hook: with
/// `panic = "abort"` (the release profile) destructors never run, but the
/// hook does.
static ACTIVE: Mutex<Vec<Saved>> = Mutex::new(Vec::new());
static HOOK: Once = Once::new();
static NEXT_ID: Mutex<u64> = Mutex::new(0);

fn install_panic_hook() {
    HOOK.call_once(|| {
        let prev = std::panic::take_hook();
        std::panic::set_hook(Box::new(move |info| {
            // try_lock: a panic while the lock is held must not deadlock.
            if let Ok(active) = ACTIVE.try_lock() {
                let deadline = Instant::now() + PANIC_WRITE;
                for s in active.iter() {
                    // Before the modes: the fd is still non-blocking, so
                    // the write is bounded.
                    let _ = sys::write_all_until(s.output, FALLBACK_RESTORE, deadline);
                    s.restore();
                }
            }
            prev(info);
        }));
    });
}

/// Puts `tty` in raw mode and `input` / `output` in non-blocking mode;
/// dropping it restores all three.
pub(crate) struct RawGuard {
    saved: Saved,
    /// The terminal is in raw mode now (not while suspended).
    raw: bool,
}

impl RawGuard {
    pub(crate) fn enter(tty: RawFd, input: RawFd, output: RawFd) -> io::Result<RawGuard> {
        install_panic_hook();
        // SAFETY: termios is plain data; tcgetattr fills it.
        let mut termios: libc::termios = unsafe { std::mem::zeroed() };
        // SAFETY: valid termios pointer.
        sys::cvt(unsafe { libc::tcgetattr(tty, &mut termios) })?;
        let flags = [(input, sys::get_flags(input)?), (output, sys::get_flags(output)?)];
        let id = {
            let mut n = NEXT_ID.lock().unwrap_or_else(|e| e.into_inner());
            *n += 1;
            *n
        };
        let saved = Saved { id, tty, output, termios, flags };
        // Registered before anything changes, so a failure half way is still undone.
        ACTIVE.lock().unwrap_or_else(|e| e.into_inner()).push(saved);
        let mut guard = RawGuard { saved, raw: true };
        guard.enter_raw()?;
        Ok(guard)
    }

    /// Raw mode (again, after a suspension).
    pub(crate) fn enter_raw(&mut self) -> io::Result<()> {
        let mut raw = self.saved.termios;
        // SAFETY: valid termios pointer.
        unsafe { libc::cfmakeraw(&mut raw) };
        raw.c_cc[libc::VMIN] = 1;
        raw.c_cc[libc::VTIME] = 0;
        self.raw = true;
        // TCSANOW, not TCSAFLUSH: keys typed before attaching are not dropped.
        // SAFETY: valid termios pointer.
        sys::cvt(unsafe { libc::tcsetattr(self.saved.tty, libc::TCSANOW, &raw) })?;
        // The output is non-blocking so a slow terminal shows up as EAGAIN
        // and the loop can stop reading the socket instead of stalling.
        for (fd, fl) in self.saved.flags {
            sys::set_flags(fd, fl | libc::O_NONBLOCK)?;
        }
        Ok(())
    }

    /// The terminal as it was before attaching, for the shell, while the
    /// client is stopped.
    pub(crate) fn leave_raw(&mut self) {
        if std::mem::take(&mut self.raw) {
            self.saved.restore();
        }
    }
}

impl Drop for RawGuard {
    fn drop(&mut self) {
        // Not again after a suspension: the shell has the terminal (and
        // tcsetattr from the background would stop us with SIGTTOU).
        self.leave_raw();
        ACTIVE.lock().unwrap_or_else(|e| e.into_inner()).retain(|s| s.id != self.saved.id);
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn panic_hook_writes_the_fallback() {
        let (mut m, mut s) = (-1, -1);
        // SAFETY: valid out pointers; null name, termios and winsize mean defaults.
        let r = unsafe { libc::openpty(&mut m, &mut s, std::ptr::null_mut(), std::ptr::null_mut::<libc::termios>(), std::ptr::null_mut()) };
        assert_eq!(r, 0);
        // A real panic: the hook runs before unwinding (and is all that runs
        // with panic = "abort").
        let res = std::panic::catch_unwind(|| {
            let _g = RawGuard::enter(s, s, s).unwrap();
            panic!("test panic while attached");
        });
        assert!(res.is_err());
        let mut buf = [0u8; 1024];
        // SAFETY: buf is valid for buf.len() bytes; the bytes are already queued.
        let n = unsafe { libc::read(m, buf.as_mut_ptr().cast(), buf.len()) };
        assert_eq!(&buf[..n.max(0) as usize], FALLBACK_RESTORE);
        // SAFETY: both fds came from openpty and are closed once.
        unsafe {
            libc::close(m);
            libc::close(s);
        }
    }
}
