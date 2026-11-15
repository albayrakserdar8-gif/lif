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
/// a terminal, the device the client named, and the user's own: the path
/// is checked before it is opened (opening some devices has side effects)
/// and the descriptor after.
pub(crate) fn open_tty(path: &str, dev: u64) -> io::Result<OwnedFd> {
    let c = std::ffi::CString::new(path).map_err(|_| io::Error::from(io::ErrorKind::InvalidInput))?;
    let not_it = || io::Error::new(io::ErrorKind::InvalidInput, "not the client's terminal");
    // SAFETY: stat is plain data; stat(2) fills it for a valid path.
    let mut st: libc::stat = unsafe { std::mem::zeroed() };
    // SAFETY: c is a valid NUL-terminated path and st a valid out-pointer.
    cvt(unsafe { libc::stat(c.as_ptr(), &mut st) })?;
    // SAFETY: getuid cannot fail.
    let uid = unsafe { libc::getuid() };
    if st.st_mode & libc::S_IFMT != libc::S_IFCHR || st.st_rdev as u64 != dev || (st.st_uid != uid && uid != 0) {
        return Err(not_it());
    }
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

/// Blocks until `fd` is readable, a signal arrives or `timeout_ms` passed.
/// Only for shutdown.
pub(crate) fn wait_readable(fd: RawFd, timeout_ms: i32) {
    let mut p = libc::pollfd { fd, events: libc::POLLIN, revents: 0 };
    // SAFETY: one valid pollfd.
    unsafe { libc::poll(&mut p, 1, timeout_ms) };
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

/// Who is at the other end of a client socket.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub(crate) struct PeerCred {
    pub(crate) uid: u32,
    /// Not every system tells.
    pub(crate) pid: Option<u32>,
}

/// Only the server's own user may use it, and root (who can read the
/// user's files and terminals anyway, as tmux allows). The private 0700
/// directory and the 0600 socket already keep others out; this holds even
/// if those permissions were loosened.
pub(crate) fn peer_allowed(peer_uid: u32, own_uid: u32) -> bool {
    peer_uid == own_uid || peer_uid == 0
}

#[cfg(target_vendor = "apple")]
pub(crate) fn peer_cred(fd: RawFd) -> io::Result<PeerCred> {
    let (mut uid, mut gid) = (0, 0);
    // SAFETY: getpeereid writes two ids through valid pointers.
    cvt(unsafe { libc::getpeereid(fd, &mut uid, &mut gid) })?;
    let mut pid: libc::pid_t = 0;
    let mut len = std::mem::size_of::<libc::pid_t>() as libc::socklen_t;
    // SAFETY: LOCAL_PEERPID writes one pid_t; len says how much room there is.
    let r = unsafe { libc::getsockopt(fd, libc::SOL_LOCAL, libc::LOCAL_PEERPID, (&mut pid as *mut libc::pid_t).cast(), &mut len) };
    Ok(PeerCred { uid, pid: (r == 0 && pid > 0).then_some(pid as u32) })
}

#[cfg(target_os = "linux")]
pub(crate) fn peer_cred(fd: RawFd) -> io::Result<PeerCred> {
    // SAFETY: ucred is plain data; zero is a valid value.
    let mut cred: libc::ucred = unsafe { std::mem::zeroed() };
    let mut len = std::mem::size_of::<libc::ucred>() as libc::socklen_t;
    // SAFETY: SO_PEERCRED writes one ucred; len says how much room there is.
    cvt(unsafe { libc::getsockopt(fd, libc::SOL_SOCKET, libc::SO_PEERCRED, (&mut cred as *mut libc::ucred).cast(), &mut len) })?;
    Ok(PeerCred { uid: cred.uid, pid: (cred.pid > 0).then_some(cred.pid as u32) })
}

#[cfg(not(any(target_vendor = "apple", target_os = "linux")))]
pub(crate) fn peer_cred(fd: RawFd) -> io::Result<PeerCred> {
    let (mut uid, mut gid) = (0, 0);
    // SAFETY: getpeereid writes two ids through valid pointers.
    cvt(unsafe { libc::getpeereid(fd, &mut uid, &mut gid) })?;
    Ok(PeerCred { uid, pid: None })
}

/// Most descriptors the server asks for: far more than anyone runs panes,
/// and small enough that nothing sized by the limit gets large.
const MAX_NOFILE: u64 = 65536;

/// Raises the soft RLIMIT_NOFILE to the hard limit (at most [`MAX_NOFILE`]):
/// a shell's default soft limit (256 in Terminal.app) would cap the server
/// at a couple of hundred panes. Returns the limit the server had, for its
/// children, and the one it has now.
pub(crate) fn raise_fd_limit() -> (Option<u64>, u64) {
    // SAFETY: rlimit is plain data; getrlimit fills it.
    let mut rl: libc::rlimit = unsafe { std::mem::zeroed() };
    // SAFETY: valid out-pointer.
    if unsafe { libc::getrlimit(libc::RLIMIT_NOFILE, &mut rl) } != 0 {
        return (None, 256);
    }
    let old = rl.rlim_cur as u64;
    let hard = rl.rlim_max as u64;
    let want = hard.min(MAX_NOFILE);
    if want <= old {
        return (None, old);
    }
    // macOS refuses more than kern.maxfilesperproc (and OPEN_MAX for some
    // binaries): fall back to OPEN_MAX before giving up.
    for target in [want, want.min(10240)] {
        if target <= old {
            break;
        }
        let new = libc::rlimit { rlim_cur: target as libc::rlim_t, rlim_max: rl.rlim_max };
        // SAFETY: valid pointer to an rlimit.
        if unsafe { libc::setrlimit(libc::RLIMIT_NOFILE, &new) } == 0 {
            return (Some(old), target);
        }
    }
    (None, old)
}

/// Descriptors this process has open. Only for spawning a pane (rare), to
/// keep a reserve for clients.
#[cfg(target_vendor = "apple")]
pub(crate) fn open_fd_count() -> usize {
    let mut list: Vec<libc::proc_fdinfo> = Vec::new();
    let mut want = 1024usize;
    loop {
        list.resize(want, libc::proc_fdinfo { proc_fd: -1, proc_fdtype: 0 });
        let cap = (want * std::mem::size_of::<libc::proc_fdinfo>()) as libc::c_int;
        // SAFETY: the buffer is valid for cap bytes.
        let got = unsafe { libc::proc_pidinfo(libc::getpid(), libc::PROC_PIDLISTFDS, 0, list.as_mut_ptr().cast(), cap) };
        if got <= 0 {
            return 0;
        }
        let n = got as usize / std::mem::size_of::<libc::proc_fdinfo>();
        if n < want || want >= 1 << 20 {
            return n;
        }
        want *= 4;
    }
}

#[cfg(not(target_vendor = "apple"))]
pub(crate) fn open_fd_count() -> usize {
    std::fs::read_dir("/proc/self/fd").map(|d| d.count().saturating_sub(1)).unwrap_or(0)
}

/// Whether a client's terminal belongs to the client right now: it is the
/// client's controlling terminal and the client's process group is its
/// foreground group. When a shell stops the client (job control, or a
/// SIGSTOP the shell notices) it takes the terminal back by making its own
/// group the foreground one, and from then on keys typed there are the
/// shell's: the server must neither read them nor draw over the shell.
///
/// The server cannot ask the terminal (tcgetpgrp only answers for the
/// caller's own controlling terminal); it asks about the client process,
/// whose kernel record includes its terminal's foreground group: one
/// proc_pidinfo call on macOS, one pread of /proc/<pid>/stat on Linux.
pub(crate) struct TtyOwner {
    pid: u32,
    rdev: u64,
    #[cfg(target_os = "linux")]
    stat: OwnedFd,
}

impl TtyOwner {
    /// For client process `pid` and its terminal `rdev`. Fails when the
    /// terminal is not the client's controlling terminal (its foreground
    /// group cannot be known) or the client is not in the foreground now.
    pub(crate) fn new(pid: u32, rdev: u64) -> io::Result<TtyOwner> {
        #[cfg(target_os = "linux")]
        let owner = {
            let path = std::ffi::CString::new(format!("/proc/{pid}/stat")).map_err(|_| io::Error::from(io::ErrorKind::InvalidInput))?;
            // SAFETY: valid NUL-terminated path.
            let fd = cvt(unsafe { libc::open(path.as_ptr(), libc::O_RDONLY | libc::O_CLOEXEC) })?;
            // SAFETY: a fresh fd nothing else owns.
            TtyOwner { pid, rdev, stat: unsafe { OwnedFd::from_raw_fd(fd) } }
        };
        #[cfg(not(target_os = "linux"))]
        let owner = TtyOwner { pid, rdev };
        match owner.state() {
            Some((true, true)) => Ok(owner),
            Some((false, _)) => Err(io::Error::other("the terminal is not the client's controlling terminal")),
            Some((true, false)) => Err(io::Error::other("the client is not in the foreground")),
            None => Err(io::Error::other("cannot see the client process")),
        }
    }

    pub(crate) fn rdev(&self) -> u64 {
        self.rdev
    }

    /// The client is in the foreground of its terminal.
    pub(crate) fn foreground(&self) -> bool {
        self.state() == Some((true, true))
    }

    /// (the terminal is the client's controlling terminal, the client's
    /// group is in the foreground of it); `None` when the process is gone.
    #[cfg(target_vendor = "apple")]
    fn state(&self) -> Option<(bool, bool)> {
        // SAFETY: proc_pidinfo fills a proc_bsdinfo of the given size; an
        // all-zero one is a valid out-parameter.
        let info = unsafe {
            let mut info: libc::proc_bsdinfo = std::mem::zeroed();
            let size = std::mem::size_of::<libc::proc_bsdinfo>() as libc::c_int;
            let n = libc::proc_pidinfo(self.pid as libc::c_int, libc::PROC_PIDTBSDINFO, 0, (&mut info as *mut libc::proc_bsdinfo).cast(), size);
            if n != size {
                return None;
            }
            info
        };
        // e_tdev is the kernel's dev_t (32 bits); st_rdev was widened.
        let ctty = info.e_tdev != u32::MAX && info.e_tdev == self.rdev as u32;
        Some((ctty, ctty && info.e_tpgid != 0 && info.e_tpgid == info.pbi_pgid))
    }

    #[cfg(target_os = "linux")]
    fn state(&self) -> Option<(bool, bool)> {
        let mut buf = [0u8; 512];
        // SAFETY: buf is valid for its length; the fd is ours. /proc stat
        // files are generated afresh by every read from offset 0.
        let n = unsafe { libc::pread(self.stat.as_raw_fd(), buf.as_mut_ptr().cast(), buf.len(), 0) };
        if n <= 0 {
            return None;
        }
        let s = &buf[..n as usize];
        let close = s.iter().rposition(|&b| b == b')')?;
        // After the command name: state ppid pgrp session tty_nr tpgid ...
        let mut fields = s[close + 1..].split(|&b| b == b' ').filter(|f| !f.is_empty());
        let num = |f: Option<&[u8]>| -> Option<i64> { std::str::from_utf8(f?).ok()?.parse().ok() };
        let pgrp = num(fields.nth(2))?;
        let tty_nr = num(fields.nth(1))?;
        let tpgid = num(fields.next())?;
        let ctty = tty_nr != 0 && tty_nr as u64 == self.rdev;
        Some((ctty, ctty && tpgid > 0 && tpgid == pgrp))
    }

    #[cfg(not(any(target_vendor = "apple", target_os = "linux")))]
    fn state(&self) -> Option<(bool, bool)> {
        let _ = (self.pid, self.rdev);
        None
    }
}

/// A descriptor to give up when out of them (see `Server::refuse_one`).
pub(crate) fn spare_fd() -> Option<OwnedFd> {
    // SAFETY: valid NUL-terminated path.
    let fd = unsafe { libc::open(c"/dev/null".as_ptr(), libc::O_RDONLY | libc::O_CLOEXEC) };
    // SAFETY: a fresh fd nothing else owns.
    (fd >= 0).then(|| unsafe { OwnedFd::from_raw_fd(fd) })
}

#[cfg(test)]
mod tests {
    #[test]
    fn only_the_owner_and_root_may_connect() {
        assert!(super::peer_allowed(501, 501));
        assert!(super::peer_allowed(0, 501));
        assert!(!super::peer_allowed(502, 501));
        assert!(!super::peer_allowed(65534, 0));
    }

    #[test]
    fn peer_credentials_of_a_socket_pair_are_ours() {
        use std::os::fd::AsRawFd;
        let (a, _b) = std::os::unix::net::UnixStream::pair().unwrap();
        let c = super::peer_cred(a.as_raw_fd()).unwrap();
        // SAFETY: getuid cannot fail.
        assert_eq!(c.uid, unsafe { libc::getuid() });
        #[cfg(any(target_vendor = "apple", target_os = "linux"))]
        assert_eq!(c.pid, Some(std::process::id()));
    }
}
