//! SIGWINCH and the terminating signals delivered through a self-pipe, so
//! the single-threaded loop sees them as an fd becoming readable in poll(2).
//!
//! SIGINT and SIGQUIT are caught too: raw mode turns off ISIG, so the
//! terminal never sends them, but `kill -INT` can, and dying of one with
//! the default disposition would leave the terminal raw.

use std::io;
use std::os::fd::{AsRawFd, FromRawFd, OwnedFd, RawFd};
use std::sync::atomic::{AtomicI32, Ordering};
use std::sync::Mutex;

use crate::sys;

pub(crate) const SIGNALS: [libc::c_int; 7] =
    [libc::SIGWINCH, libc::SIGHUP, libc::SIGTERM, libc::SIGINT, libc::SIGQUIT, libc::SIGTSTP, libc::SIGCONT];

/// SIGHUP, SIGTERM, SIGINT and SIGQUIT end the attachment; SIGWINCH,
/// SIGTSTP (suspend) and SIGCONT (resume) do not.
pub(crate) fn is_terminating(sig: libc::c_int) -> bool {
    !matches!(sig, libc::SIGWINCH | libc::SIGTSTP | libc::SIGCONT) && SIGNALS.contains(&sig)
}

/// Stops the process the way SIGTSTP's default action does, and returns
/// once it is continued (SIGCONT; its handler has written to the pipes
/// by then). The caller has already given the terminal back.
pub(crate) fn stop_self() {
    // SAFETY: sigaction structs are plain data initialized by zeroing and
    // sigemptyset; the pointers are valid for each call.
    unsafe {
        let mut dfl: libc::sigaction = std::mem::zeroed();
        dfl.sa_sigaction = libc::SIG_DFL;
        libc::sigemptyset(&mut dfl.sa_mask);
        let mut ours: libc::sigaction = std::mem::zeroed();
        libc::sigaction(libc::SIGTSTP, &dfl, &mut ours);
        let mut set: libc::sigset_t = std::mem::zeroed();
        libc::sigemptyset(&mut set);
        libc::sigaddset(&mut set, libc::SIGTSTP);
        libc::pthread_sigmask(libc::SIG_UNBLOCK, &set, std::ptr::null_mut());
        // With the default action the process stops here, in raise.
        libc::raise(libc::SIGTSTP);
        libc::sigaction(libc::SIGTSTP, &ours, std::ptr::null_mut());
    }
}

/// Write ends of every live self-pipe. A process normally has one attach
/// loop; more slots let tests run loops side by side.
static PIPES: [AtomicI32; 16] = [const { AtomicI32::new(-1) }; 16];

struct Installed {
    users: usize,
    previous: [Option<libc::sigaction>; SIGNALS.len()],
}

static INSTALLED: Mutex<Installed> = Mutex::new(Installed { users: 0, previous: [None; SIGNALS.len()] });

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
    /// Still counted in `Installed::users` (`finish` already uncounted it).
    counted: bool,
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
        Ok(SignalPipe { read, _write: write, slot, counted: true })
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

impl SignalPipe {
    /// Puts the previous dispositions back (if this is the last pipe), then
    /// returns the first terminating signal still in the pipe. In this
    /// order nothing is lost: a signal that arrives before the handlers go
    /// is in the pipe, one that arrives after meets the old disposition.
    pub(crate) fn finish(mut self) -> Option<libc::c_int> {
        self.uncount();
        let mut first = None;
        self.drain(|sig| {
            if is_terminating(sig) {
                first.get_or_insert(sig);
            }
        });
        first
    }

    fn uncount(&mut self) {
        if !std::mem::take(&mut self.counted) {
            return;
        }
        let mut st = INSTALLED.lock().unwrap_or_else(|e| e.into_inner());
        st.users -= 1;
        if st.users == 0 {
            restore(&mut st);
        }
    }
}

/// Delivers `sig` again, after [`SignalPipe::finish`] put the previous
/// disposition back. For a process that had the default one (the lif
/// binary) this ends it by the signal, so its parent sees "killed by
/// SIGTERM" instead of a clean exit; a disposition the process inherited
/// as ignored (`nohup`, background jobs and SIGINT) stays ignored and this
/// returns.
pub(crate) fn reraise(sig: libc::c_int) {
    // SAFETY: sigset_t is plain data, initialized by sigemptyset; the
    // pointers are valid for the calls.
    unsafe {
        let mut set: libc::sigset_t = std::mem::zeroed();
        libc::sigemptyset(&mut set);
        libc::sigaddset(&mut set, sig);
        // A signal blocked by the caller would stay pending and be lost
        // at exit.
        libc::pthread_sigmask(libc::SIG_UNBLOCK, &set, std::ptr::null_mut());
        libc::raise(sig);
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
        self.uncount();
    }
}
