//! SIGWINCH / SIGHUP / SIGTERM delivered through a self-pipe, so the
//! single-threaded loop sees them as an fd becoming readable in poll(2).

use std::io;
use std::os::fd::{AsRawFd, FromRawFd, OwnedFd, RawFd};
use std::sync::atomic::{AtomicI32, Ordering};
use std::sync::Mutex;

use crate::sys;

pub(crate) const SIGNALS: [libc::c_int; 3] = [libc::SIGWINCH, libc::SIGHUP, libc::SIGTERM];

/// Write ends of every live self-pipe. A process normally has one attach
/// loop; more slots let tests run loops side by side.
static PIPES: [AtomicI32; 16] = [const { AtomicI32::new(-1) }; 16];

struct Installed {
    users: usize,
    previous: [Option<libc::sigaction>; 3],
}

static INSTALLED: Mutex<Installed> = Mutex::new(Installed { users: 0, previous: [None; 3] });

extern "C" fn on_signal(sig: libc::c_int) {
    let errno = sys::errno_location();
    // SAFETY: errno_location is this thread's errno; read and restored so
    // the interrupted code does not see a changed value.
    let saved = unsafe { *errno };
    let byte = sig as u8;
    for p in &PIPES {
        let fd = p.load(Ordering::Relaxed);
        if fd >= 0 {
            // SAFETY: write(2) is async-signal-safe; the pipe is non-blocking
            // so a full pipe drops the byte instead of blocking (a full pipe
            // already guarantees a wakeup).
            unsafe { libc::write(fd, (&byte as *const u8).cast(), 1) };
        }
    }
    // SAFETY: as above.
    unsafe { *errno = saved };
}

/// A registered self-pipe. While any exists, the handlers are installed;
/// dropping the last one restores the previous dispositions.
pub(crate) struct SignalPipe {
    read: OwnedFd,
    _write: OwnedFd,
    slot: usize,
}

fn pipe() -> io::Result<(OwnedFd, OwnedFd)> {
    let mut fds = [0 as libc::c_int; 2];
    // SAFETY: pipe writes two fds into the array.
    sys::cvt(unsafe { libc::pipe(fds.as_mut_ptr()) })?;
    // SAFETY: both fds were just created and are owned by nobody else.
    let (r, w) = unsafe { (OwnedFd::from_raw_fd(fds[0]), OwnedFd::from_raw_fd(fds[1])) };
    for fd in [r.as_raw_fd(), w.as_raw_fd()] {
        sys::set_flags(fd, sys::get_flags(fd)? | libc::O_NONBLOCK)?;
        // SAFETY: F_SETFD takes an int argument.
        sys::cvt(unsafe { libc::fcntl(fd, libc::F_SETFD, libc::FD_CLOEXEC) })?;
    }
    Ok((r, w))
}

impl SignalPipe {
    pub(crate) fn install() -> io::Result<SignalPipe> {
        let (read, write) = pipe()?;
        let slot = PIPES
            .iter()
            .position(|p| p.compare_exchange(-1, write.as_raw_fd(), Ordering::AcqRel, Ordering::Relaxed).is_ok())
            .ok_or_else(|| io::Error::other("too many attach loops in one process"))?;
        let mut st = INSTALLED.lock().unwrap_or_else(|e| e.into_inner());
        if st.users == 0 {
            for (i, &sig) in SIGNALS.iter().enumerate() {
                // SAFETY: sigaction is plain data; zero is a valid start.
                let mut sa: libc::sigaction = unsafe { std::mem::zeroed() };
                sa.sa_sigaction = on_signal as extern "C" fn(libc::c_int) as libc::sighandler_t;
                // Restart other syscalls; poll(2) still wakes via the pipe.
                sa.sa_flags = libc::SA_RESTART;
                // SAFETY: sa_mask is a valid sigset_t.
                unsafe { libc::sigemptyset(&mut sa.sa_mask) };
                // SAFETY: zeroed sigaction is a valid output buffer.
                let mut old: libc::sigaction = unsafe { std::mem::zeroed() };
                // SAFETY: both pointers are valid sigaction structs.
                if let Err(e) = sys::cvt(unsafe { libc::sigaction(sig, &sa, &mut old) }) {
                    restore(&mut st);
                    PIPES[slot].store(-1, Ordering::Release);
                    return Err(e);
                }
                st.previous[i] = Some(old);
            }
        }
        st.users += 1;
        Ok(SignalPipe { read, _write: write, slot })
    }

    pub(crate) fn fd(&self) -> RawFd {
        self.read.as_raw_fd()
    }

    /// Calls `f` for every signal received since the last call.
    pub(crate) fn drain(&self, mut f: impl FnMut(libc::c_int)) {
        let mut buf = [0u8; 64];
        while let Ok(n @ 1..) = sys::read(self.fd(), &mut buf) {
            buf[..n].iter().for_each(|&b| f(b as libc::c_int));
        }
    }
}

fn restore(st: &mut Installed) {
    for (i, &sig) in SIGNALS.iter().enumerate() {
        if let Some(old) = st.previous[i].take() {
            // SAFETY: old came from a successful sigaction call.
            unsafe { libc::sigaction(sig, &old, std::ptr::null_mut()) };
        }
    }
}

impl Drop for SignalPipe {
    fn drop(&mut self) {
        // Unregister before the fd closes, so the handler never writes to
        // a closed (or reused) fd number.
        PIPES[self.slot].store(-1, Ordering::Release);
        let mut st = INSTALLED.lock().unwrap_or_else(|e| e.into_inner());
        st.users -= 1;
        if st.users == 0 {
            restore(&mut st);
        }
    }
}
