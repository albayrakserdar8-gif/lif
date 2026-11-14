//! Thread-free, race-free child reaping for a single-threaded event loop.
//!
//! Design: a process-wide SIGCHLD handler writes one byte to the self-pipe of
//! every live [`ChildReaper`]. The pipe's read end is what the event loop
//! registers (mio `SourceFd`, readable interest). On wakeup the loop calls
//! [`ChildReaper::reap`], which drains the pipe and then calls
//! `waitpid(pid, WNOHANG)` for each pid it watches.
//!
//! Why this is race-free: the pipe is drained *before* the waitpid sweep, so
//! a child exiting during the sweep either is seen by it or leaves a byte in
//! the pipe for the next wakeup. Exit statuses are never lost because
//! zombies stay until waited for; a SIGCHLD that arrived before a pid was
//! watched only means the next `reap` (from any later wakeup, or one call
//! right after `watch`) picks it up.
//!
//! Why not `waitpid(-1)`: the process may have other children (e.g. ones
//! started with `std::process::Command`) whose statuses belong to someone
//! else. Why not signalfd / kqueue EVFILT_PROC: neither is portable across
//! the two targets, and the self-pipe costs one fd and zero idle wakeups.

use std::io;
use std::os::fd::{AsFd, AsRawFd, BorrowedFd, OwnedFd, RawFd};
use std::os::unix::process::ExitStatusExt;
use std::process::ExitStatus;
use std::sync::Mutex;
use std::sync::atomic::{AtomicBool, AtomicI32, AtomicUsize, Ordering};

use libc::c_int;

use crate::sys::{self, cvt, errno};

/// Reapers that may exist at once in one process. The server needs one;
/// the rest is room for tests and tools.
const MAX_REAPERS: usize = 64;

static SLOTS: [AtomicI32; MAX_REAPERS] = [const { AtomicI32::new(-1) }; MAX_REAPERS];
/// Handlers currently between reading a slot and writing to it; a reaper
/// being dropped waits for this to reach zero before closing its pipe, so a
/// handler never writes to a reused fd number.
static IN_HANDLER: AtomicUsize = AtomicUsize::new(0);
/// The SIGCHLD disposition we replaced, called after ours.
static PREV_HANDLER: AtomicUsize = AtomicUsize::new(libc::SIG_DFL);
static PREV_SIGINFO: AtomicBool = AtomicBool::new(false);
static INSTALL: Mutex<bool> = Mutex::new(false);

extern "C" fn on_sigchld(sig: c_int, info: *mut libc::siginfo_t, ctx: *mut libc::c_void) {
    let errno_ptr = sys::errno_location();
    // SAFETY: errno_ptr is this thread's errno.
    let saved = unsafe { *errno_ptr };
    IN_HANDLER.fetch_add(1, Ordering::SeqCst);
    for slot in &SLOTS {
        let fd = slot.load(Ordering::SeqCst);
        if fd >= 0 {
            let byte = 1u8;
            // SAFETY: write is async-signal-safe; a full pipe (EAGAIN) means a
            // wakeup is already pending.
            unsafe { libc::write(fd, (&byte as *const u8).cast(), 1) };
        }
    }
    IN_HANDLER.fetch_sub(1, Ordering::SeqCst);

    let prev = PREV_HANDLER.load(Ordering::Relaxed);
    if prev != libc::SIG_DFL && prev != libc::SIG_IGN {
        // SAFETY: prev was installed as a SIGCHLD handler with the matching
        // calling convention, recorded in PREV_SIGINFO.
        unsafe {
            if PREV_SIGINFO.load(Ordering::Relaxed) {
                let f: extern "C" fn(c_int, *mut libc::siginfo_t, *mut libc::c_void) =
                    std::mem::transmute(prev);
                f(sig, info, ctx);
            } else {
                let f: extern "C" fn(c_int) = std::mem::transmute(prev);
                f(sig);
            }
        }
    }
    // SAFETY: as above.
    unsafe { *errno_ptr = saved };
}

fn install_handler() -> io::Result<()> {
    let mut installed = INSTALL.lock().unwrap_or_else(|e| e.into_inner());
    if *installed {
        return Ok(());
    }
    // SAFETY: the sigaction structs are plain values; on_sigchld has the
    // SA_SIGINFO signature.
    unsafe {
        let mut sa: libc::sigaction = std::mem::zeroed();
        sa.sa_sigaction = on_sigchld as *const () as usize;
        sa.sa_flags = libc::SA_SIGINFO | libc::SA_RESTART | libc::SA_NOCLDSTOP;
        libc::sigemptyset(&mut sa.sa_mask);
        let mut old: libc::sigaction = std::mem::zeroed();
        cvt(libc::sigaction(libc::SIGCHLD, &sa, &mut old))?;
        PREV_SIGINFO.store(old.sa_flags & libc::SA_SIGINFO != 0, Ordering::Relaxed);
        PREV_HANDLER.store(old.sa_sigaction, Ordering::Relaxed);
    }
    *installed = true;
    Ok(())
}

/// Collects exit statuses of children spawned by this crate.
///
/// Register [`ChildReaper`] (its raw fd) for readability in the event loop;
/// when it fires, call [`ChildReaper::reap`]. Pids are added with
/// [`ChildReaper::watch`]. Installing it sets a process-wide SIGCHLD
/// handler (chaining to any previous one) that stays installed.
#[derive(Debug)]
pub struct ChildReaper {
    read: OwnedFd,
    // Kept open for the handler, which writes to its raw number.
    _write: OwnedFd,
    slot: usize,
    pids: Vec<libc::pid_t>,
}

impl ChildReaper {
    pub fn new() -> io::Result<ChildReaper> {
        let (read, write) = sys::pipe_cloexec()?;
        sys::set_nonblocking(read.as_raw_fd())?;
        sys::set_nonblocking(write.as_raw_fd())?;
        install_handler()?;
        let slot = SLOTS
            .iter()
            .position(|s| {
                s.compare_exchange(-1, write.as_raw_fd(), Ordering::SeqCst, Ordering::SeqCst)
                    .is_ok()
            })
            .ok_or_else(|| io::Error::other("too many ChildReapers in this process"))?;
        Ok(ChildReaper {
            read,
            _write: write,
            slot,
            pids: Vec::new(),
        })
    }

    /// Starts tracking `pid` (from [`crate::Pty::pid`]). If the child may
    /// already have exited, call [`ChildReaper::reap`] afterwards; its
    /// SIGCHLD wakeup can come before the pid is watched.
    pub fn watch(&mut self, pid: u32) {
        let pid = pid as libc::pid_t;
        if !self.pids.contains(&pid) {
            self.pids.push(pid);
        }
    }

    /// Number of watched children not yet reaped.
    pub fn len(&self) -> usize {
        self.pids.len()
    }

    pub fn is_empty(&self) -> bool {
        self.pids.is_empty()
    }

    /// Drains the wakeup pipe and returns every watched child that has
    /// exited, with its status. Never blocks.
    pub fn reap(&mut self) -> Vec<(u32, ExitStatus)> {
        let mut buf = [0u8; 64];
        // SAFETY: buf is valid for its length; the fd is non-blocking.
        while unsafe { libc::read(self.read.as_raw_fd(), buf.as_mut_ptr().cast(), buf.len()) } > 0 {
        }

        let mut done = Vec::new();
        self.pids.retain(|&pid| {
            let mut status = 0;
            loop {
                // SAFETY: plain syscall with a valid out-pointer.
                let r = unsafe { libc::waitpid(pid, &mut status, libc::WNOHANG) };
                if r == pid {
                    done.push((pid as u32, ExitStatus::from_raw(status)));
                    return false;
                }
                if r == 0 {
                    return true;
                }
                // ECHILD: someone else reaped it; nothing left to report.
                if errno() != libc::EINTR {
                    return false;
                }
            }
        });
        done
    }
}

impl AsRawFd for ChildReaper {
    fn as_raw_fd(&self) -> RawFd {
        self.read.as_raw_fd()
    }
}

impl AsFd for ChildReaper {
    fn as_fd(&self) -> BorrowedFd<'_> {
        self.read.as_fd()
    }
}

impl Drop for ChildReaper {
    fn drop(&mut self) {
        SLOTS[self.slot].store(-1, Ordering::SeqCst);
        while IN_HANDLER.load(Ordering::SeqCst) != 0 {
            std::hint::spin_loop();
        }
    }
}
