//! Raw mode with guaranteed restore.

use std::io;
use std::os::fd::RawFd;
use std::sync::{Mutex, Once};

use crate::sys;

/// What to put back on one terminal.
#[derive(Clone, Copy)]
struct Saved {
    id: u64,
    tty: RawFd,
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
                active.iter().for_each(Saved::restore);
            }
            prev(info);
        }));
    });
}

/// Puts `tty` in raw mode and `input` / `output` in non-blocking mode;
/// dropping it restores all three.
pub(crate) struct RawGuard {
    saved: Saved,
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
        let saved = Saved { id, tty, termios, flags };
        // Registered before anything changes, so a failure half way is still undone.
        ACTIVE.lock().unwrap_or_else(|e| e.into_inner()).push(saved);
        let guard = RawGuard { saved };

        let mut raw = termios;
        // SAFETY: valid termios pointer.
        unsafe { libc::cfmakeraw(&mut raw) };
        raw.c_cc[libc::VMIN] = 1;
        raw.c_cc[libc::VTIME] = 0;
        // TCSANOW, not TCSAFLUSH: keys typed before attaching are not dropped.
        // SAFETY: valid termios pointer.
        sys::cvt(unsafe { libc::tcsetattr(tty, libc::TCSANOW, &raw) })?;
        // The output is non-blocking so a slow terminal shows up as EAGAIN
        // and the loop can stop reading the socket instead of stalling.
        for (fd, fl) in flags {
            sys::set_flags(fd, fl | libc::O_NONBLOCK)?;
        }
        Ok(guard)
    }
}

impl Drop for RawGuard {
    fn drop(&mut self) {
        self.saved.restore();
        ACTIVE.lock().unwrap_or_else(|e| e.into_inner()).retain(|s| s.id != self.saved.id);
    }
}
