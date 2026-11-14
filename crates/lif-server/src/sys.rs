//! The few OS calls the server makes itself (signals, clock, host name).

use std::io;
use std::os::fd::{AsRawFd, FromRawFd, OwnedFd, RawFd};
use std::sync::atomic::{AtomicI32, Ordering};

use lif_core::WallClock;

static SIGNAL_FD: AtomicI32 = AtomicI32::new(-1);

fn cvt(r: libc::c_int) -> io::Result<libc::c_int> {
    if r == -1 { Err(io::Error::last_os_error()) } else { Ok(r) }
}

#[cfg(any(target_os = "macos", target_os = "ios", target_os = "freebsd"))]
fn errno_ptr() -> *mut libc::c_int {
    // SAFETY: returns the calling thread's errno location.
    unsafe { libc::__error() }
}

#[cfg(not(any(target_os = "macos", target_os = "ios", target_os = "freebsd")))]
fn errno_ptr() -> *mut libc::c_int {
    // SAFETY: returns the calling thread's errno location.
    unsafe { libc::__errno_location() }
}

extern "C" fn on_signal(sig: libc::c_int) {
    let fd = SIGNAL_FD.load(Ordering::Relaxed);
    if fd < 0 {
        return;
    }
    let p = errno_ptr();
    // SAFETY: errno is thread-local and valid; write(2) is async-signal-safe
    // and the pipe is non-blocking, so a full pipe drops the byte (the loop
    // is awake anyway).
    unsafe {
        let saved = *p;
        let b = sig as u8;
        libc::write(fd, (&b as *const u8).cast(), 1);
        *p = saved;
    }
}

/// SIGTERM / SIGINT / SIGUSR1 delivered as bytes on a pipe the loop polls.
pub(crate) struct SignalPipe {
    read: OwnedFd,
    _write: OwnedFd,
}

impl SignalPipe {
    pub(crate) fn install() -> io::Result<SignalPipe> {
        let mut fds = [0; 2];
        // SAFETY: fds has room for the two descriptors pipe(2) returns.
        cvt(unsafe { libc::pipe(fds.as_mut_ptr()) })?;
        // SAFETY: pipe returned two fresh fds that nothing else owns.
        let (read, write) = unsafe { (OwnedFd::from_raw_fd(fds[0]), OwnedFd::from_raw_fd(fds[1])) };
        for fd in [read.as_raw_fd(), write.as_raw_fd()] {
            set_nonblocking_cloexec(fd)?;
        }
        SIGNAL_FD.store(write.as_raw_fd(), Ordering::Relaxed);
        for sig in [libc::SIGTERM, libc::SIGINT, libc::SIGUSR1] {
            set_handler(sig, on_signal as *const () as usize)?;
        }
        for sig in [libc::SIGHUP, libc::SIGPIPE] {
            set_handler(sig, libc::SIG_IGN)?;
        }
        Ok(SignalPipe { read, _write: write })
    }

    pub(crate) fn fd(&self) -> RawFd {
        self.read.as_raw_fd()
    }

    /// Signals received since the last call.
    pub(crate) fn drain(&self, out: &mut Vec<libc::c_int>) {
        let mut buf = [0u8; 64];
        loop {
            // SAFETY: buf is valid for its length.
            let n = unsafe { libc::read(self.read.as_raw_fd(), buf.as_mut_ptr().cast(), buf.len()) };
            if n <= 0 {
                return;
            }
            out.extend(buf[..n as usize].iter().map(|b| *b as libc::c_int));
        }
    }
}

impl Drop for SignalPipe {
    fn drop(&mut self) {
        SIGNAL_FD.store(-1, Ordering::Relaxed);
    }
}

fn set_handler(sig: libc::c_int, handler: libc::sighandler_t) -> io::Result<()> {
    // SAFETY: a zeroed sigaction is a valid starting point; the handler is
    // either SIG_IGN or an extern "C" fn(c_int) that is async-signal-safe.
    unsafe {
        let mut sa: libc::sigaction = std::mem::zeroed();
        sa.sa_sigaction = handler;
        sa.sa_flags = libc::SA_RESTART;
        libc::sigemptyset(&mut sa.sa_mask);
        cvt(libc::sigaction(sig, &sa, std::ptr::null_mut()))?;
    }
    Ok(())
}

fn set_nonblocking_cloexec(fd: RawFd) -> io::Result<()> {
    // SAFETY: fcntl on an fd we own.
    unsafe {
        let fl = cvt(libc::fcntl(fd, libc::F_GETFL))?;
        cvt(libc::fcntl(fd, libc::F_SETFL, fl | libc::O_NONBLOCK))?;
        let fd_fl = cvt(libc::fcntl(fd, libc::F_GETFD))?;
        cvt(libc::fcntl(fd, libc::F_SETFD, fd_fl | libc::FD_CLOEXEC))?;
    }
    Ok(())
}

/// Opens a client's terminal for the server's own use: by path, so the
/// open file description (and its O_NONBLOCK) is the server's, not shared
/// with the shell that gets the terminal back after detaching. It must be
/// a terminal and the device the client named.
pub(crate) fn open_tty(path: &str, dev: u64) -> io::Result<OwnedFd> {
    let c = std::ffi::CString::new(path).map_err(|_| io::Error::from(io::ErrorKind::InvalidInput))?;
    // SAFETY: c is a valid NUL-terminated path.
    let fd = cvt(unsafe { libc::open(c.as_ptr(), libc::O_RDWR | libc::O_NOCTTY | libc::O_NONBLOCK | libc::O_CLOEXEC) })?;
    // SAFETY: open returned a fresh fd nothing else owns.
    let fd = unsafe { OwnedFd::from_raw_fd(fd) };
    // SAFETY: stat is plain data; fstat fills it for a valid fd.
    let mut st: libc::stat = unsafe { std::mem::zeroed() };
    // SAFETY: valid fd and stat pointer.
    cvt(unsafe { libc::fstat(fd.as_raw_fd(), &mut st) })?;
    // SAFETY: isatty on a valid fd.
    let tty = unsafe { libc::isatty(fd.as_raw_fd()) } == 1;
    if !tty || st.st_mode & libc::S_IFMT != libc::S_IFCHR || st.st_rdev as u64 != dev {
        return Err(io::Error::new(io::ErrorKind::InvalidInput, "not the client's terminal"));
    }
    Ok(fd)
}

/// Now, with the local UTC offset.
pub(crate) fn wall_clock() -> WallClock {
    // SAFETY: time(NULL) and localtime_r with valid pointers.
    unsafe {
        let t = libc::time(std::ptr::null_mut());
        let mut tm: libc::tm = std::mem::zeroed();
        let off = if libc::localtime_r(&t, &mut tm).is_null() { 0 } else { tm.tm_gmtoff as i32 };
        WallClock::new(t as i64, off)
    }
}

pub(crate) fn hostname() -> String {
    let mut buf = [0u8; 256];
    // SAFETY: buf is valid for its length; gethostname NUL-terminates when it fits.
    let r = unsafe { libc::gethostname(buf.as_mut_ptr().cast(), buf.len()) };
    if r != 0 {
        return String::new();
    }
    let n = buf.iter().position(|b| *b == 0).unwrap_or(buf.len());
    String::from_utf8_lossy(&buf[..n]).into_owned()
}

/// Sends `sig` to the process group `pgid` (a pane's child is its leader).
pub(crate) fn kill_group(pgid: u32, sig: libc::c_int) {
    if pgid > 1 {
        // SAFETY: plain syscall; a group that is gone gives ESRCH.
        unsafe {
            libc::kill(-(pgid as libc::pid_t), sig);
        }
    }
}

/// Blocking-ish write of all of `buf` to a non-blocking fd, waiting at
/// most `timeout_ms` in total. Only for shutdown and the panic hook.
pub(crate) fn write_all_timeout(fd: RawFd, mut buf: &[u8], timeout_ms: i32) -> bool {
    let mut waited = 0;
    while !buf.is_empty() {
        // SAFETY: buf is valid for its length.
        let n = unsafe { libc::write(fd, buf.as_ptr().cast(), buf.len()) };
        if n > 0 {
            buf = &buf[n as usize..];
            continue;
        }
        let err = io::Error::last_os_error();
        if n < 0 && err.kind() == io::ErrorKind::Interrupted {
            continue;
        }
        if n < 0 && err.kind() != io::ErrorKind::WouldBlock {
            return false;
        }
        if waited >= timeout_ms {
            return false;
        }
        let mut p = libc::pollfd { fd, events: libc::POLLOUT, revents: 0 };
        // SAFETY: one valid pollfd.
        unsafe { libc::poll(&mut p, 1, 20) };
        waited += 20;
    }
    true
}

/// Resident set size of this process in bytes, for the stats log.
#[cfg(target_os = "macos")]
pub(crate) fn rss_bytes() -> u64 {
    // SAFETY: proc_pidinfo fills a proc_taskinfo of the given size.
    unsafe {
        let mut info: libc::proc_taskinfo = std::mem::zeroed();
        let size = std::mem::size_of::<libc::proc_taskinfo>() as libc::c_int;
        let n = libc::proc_pidinfo(libc::getpid(), libc::PROC_PIDTASKINFO, 0, (&mut info as *mut libc::proc_taskinfo).cast(), size);
        if n == size { info.pti_resident_size } else { 0 }
    }
}

#[cfg(not(target_os = "macos"))]
pub(crate) fn rss_bytes() -> u64 {
    std::fs::read_to_string("/proc/self/statm")
        .ok()
        .and_then(|s| s.split_whitespace().nth(1)?.parse::<u64>().ok())
        .map(|pages| pages * 4096)
        .unwrap_or(0)
}

/// The parent of process `pid` (`detach-client -P`).
#[cfg(target_os = "macos")]
pub(crate) fn parent_pid(pid: u32) -> Option<u32> {
    // SAFETY: proc_pidinfo fills a proc_bsdinfo of the given size; an
    // all-zero one is a valid out-parameter.
    unsafe {
        let mut info: libc::proc_bsdinfo = std::mem::zeroed();
        let size = std::mem::size_of::<libc::proc_bsdinfo>() as libc::c_int;
        let n = libc::proc_pidinfo(pid as libc::c_int, libc::PROC_PIDTBSDINFO, 0, (&mut info as *mut libc::proc_bsdinfo).cast(), size);
        (n == size).then_some(info.pbi_ppid)
    }
}

#[cfg(not(target_os = "macos"))]
pub(crate) fn parent_pid(pid: u32) -> Option<u32> {
    // Field 4 of /proc/<pid>/stat, after the parenthesised command name
    // (which may itself contain spaces and parentheses).
    let stat = std::fs::read_to_string(format!("/proc/{pid}/stat")).ok()?;
    stat.rsplit_once(')')?.1.split_whitespace().nth(1)?.parse().ok()
}
