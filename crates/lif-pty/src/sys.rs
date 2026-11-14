//! Thin wrappers over the libc calls this crate needs, per platform.

use std::ffi::{CStr, CString, OsString};
use std::io;
use std::os::fd::{AsRawFd, FromRawFd, OwnedFd, RawFd};
use std::os::unix::ffi::OsStringExt;
use std::path::PathBuf;

use libc::c_int;

/// Turns a libc return value of -1 into the current errno.
pub(crate) fn cvt(ret: c_int) -> io::Result<c_int> {
    if ret == -1 {
        Err(io::Error::last_os_error())
    } else {
        Ok(ret)
    }
}

pub(crate) fn errno() -> c_int {
    io::Error::last_os_error().raw_os_error().unwrap_or(0)
}

/// Pointer to the calling thread's errno, for the signal handler, which must
/// leave errno as it found it.
pub(crate) fn errno_location() -> *mut c_int {
    // SAFETY: both functions only return the address of the thread's errno.
    #[cfg(target_vendor = "apple")]
    unsafe {
        libc::__error()
    }
    #[cfg(target_os = "linux")]
    unsafe {
        libc::__errno_location()
    }
}

pub(crate) fn set_cloexec(fd: RawFd) -> io::Result<()> {
    // SAFETY: fcntl on an fd we own; F_GETFD/F_SETFD take an int argument.
    unsafe {
        let flags = cvt(libc::fcntl(fd, libc::F_GETFD))?;
        if flags & libc::FD_CLOEXEC == 0 {
            cvt(libc::fcntl(fd, libc::F_SETFD, flags | libc::FD_CLOEXEC))?;
        }
    }
    Ok(())
}

pub(crate) fn set_nonblocking(fd: RawFd) -> io::Result<()> {
    // SAFETY: fcntl on an fd we own; F_GETFL/F_SETFL take an int argument.
    unsafe {
        let flags = cvt(libc::fcntl(fd, libc::F_GETFL))?;
        if flags & libc::O_NONBLOCK == 0 {
            cvt(libc::fcntl(fd, libc::F_SETFL, flags | libc::O_NONBLOCK))?;
        }
    }
    Ok(())
}

/// A close-on-exec pipe, returned as (read, write).
pub(crate) fn pipe_cloexec() -> io::Result<(OwnedFd, OwnedFd)> {
    let mut fds = [-1 as c_int; 2];
    #[cfg(target_os = "linux")]
    {
        // SAFETY: fds is a valid array of two ints.
        cvt(unsafe { libc::pipe2(fds.as_mut_ptr(), libc::O_CLOEXEC) })?;
    }
    #[cfg(not(target_os = "linux"))]
    {
        // macOS has no pipe2. Another thread forking in the gap could inherit
        // these fds; children spawned by this crate close every fd anyway,
        // and the lif server is single-threaded.
        // SAFETY: fds is a valid array of two ints.
        cvt(unsafe { libc::pipe(fds.as_mut_ptr()) })?;
    }
    // SAFETY: pipe succeeded, so both fds are open and owned by nobody else.
    let (r, w) = unsafe { (OwnedFd::from_raw_fd(fds[0]), OwnedFd::from_raw_fd(fds[1])) };
    #[cfg(not(target_os = "linux"))]
    {
        set_cloexec(r.as_raw_fd())?;
        set_cloexec(w.as_raw_fd())?;
    }
    Ok((r, w))
}

/// Opens a PTY master (close-on-exec, still blocking) and returns it with the
/// path of its slave.
pub(crate) fn open_master() -> io::Result<(OwnedFd, CString)> {
    // SAFETY: plain libc call; the result is checked.
    let fd = cvt(unsafe { libc::posix_openpt(libc::O_RDWR | libc::O_NOCTTY | libc::O_CLOEXEC) })?;
    // SAFETY: posix_openpt returned a fresh fd that nothing else owns.
    let master = unsafe { OwnedFd::from_raw_fd(fd) };
    // Not every libc passes O_CLOEXEC through to the open of /dev/ptmx.
    set_cloexec(fd)?;
    // SAFETY: fd is an open PTY master.
    unsafe {
        cvt(libc::grantpt(fd))?;
        cvt(libc::unlockpt(fd))?;
    }
    let name = slave_name(fd)?;
    Ok((master, name))
}

#[cfg(target_vendor = "apple")]
fn slave_name(fd: RawFd) -> io::Result<CString> {
    // TIOCPTYGNAME fills a 128-byte buffer and, unlike ptsname, uses no
    // static storage, so concurrent spawns cannot clobber each other.
    let mut buf = [0 as libc::c_char; 128];
    // SAFETY: TIOCPTYGNAME writes at most 128 bytes, the size of buf.
    cvt(unsafe { libc::ioctl(fd, libc::TIOCPTYGNAME as libc::c_ulong, buf.as_mut_ptr()) })?;
    // SAFETY: the kernel NUL-terminates the name within the buffer.
    Ok(unsafe { CStr::from_ptr(buf.as_ptr()) }.to_owned())
}

#[cfg(target_os = "linux")]
fn slave_name(fd: RawFd) -> io::Result<CString> {
    let mut buf = [0 as libc::c_char; 128];
    // SAFETY: buf is valid for buf.len() bytes.
    let rc = unsafe { libc::ptsname_r(fd, buf.as_mut_ptr(), buf.len()) };
    if rc != 0 {
        return Err(io::Error::from_raw_os_error(rc));
    }
    // SAFETY: ptsname_r succeeded, so buf holds a NUL-terminated string.
    Ok(unsafe { CStr::from_ptr(buf.as_ptr()) }.to_owned())
}

pub(crate) fn winsize(size: lif_types::Size, pixels: lif_types::PixelSize) -> libc::winsize {
    libc::winsize {
        ws_row: size.rows,
        ws_col: size.cols,
        ws_xpixel: pixels.width,
        ws_ypixel: pixels.height,
    }
}

pub(crate) fn set_winsize(fd: RawFd, ws: &libc::winsize) -> io::Result<()> {
    // SAFETY: TIOCSWINSZ reads one winsize from the pointer.
    cvt(unsafe { libc::ioctl(fd, libc::TIOCSWINSZ, ws as *const libc::winsize) })?;
    Ok(())
}

/// Login shell and home directory from the password database.
pub(crate) fn passwd_entry() -> Option<(CString, CString)> {
    let mut buf = vec![0 as libc::c_char; 4096];
    loop {
        // SAFETY: an all-zero passwd is a valid out-parameter.
        let mut pw: libc::passwd = unsafe { std::mem::zeroed() };
        let mut result: *mut libc::passwd = std::ptr::null_mut();
        // SAFETY: every pointer is valid; buf outlives the strings we copy.
        let rc = unsafe {
            libc::getpwuid_r(
                libc::getuid(),
                &mut pw,
                buf.as_mut_ptr(),
                buf.len(),
                &mut result,
            )
        };
        if rc == libc::ERANGE && buf.len() < 1 << 20 {
            buf.resize(buf.len() * 2, 0);
            continue;
        }
        if rc != 0 || result.is_null() {
            return None;
        }
        let get = |p: *const libc::c_char| {
            if p.is_null() {
                CString::default()
            } else {
                // SAFETY: getpwuid_r points the fields into buf, NUL-terminated.
                unsafe { CStr::from_ptr(p) }.to_owned()
            }
        };
        return Some((get(pw.pw_shell), get(pw.pw_dir)));
    }
}

pub(crate) fn is_executable(path: &CStr) -> bool {
    // SAFETY: path is NUL-terminated.
    unsafe { libc::access(path.as_ptr(), libc::X_OK) == 0 }
}

/// Upper bound for the brute-force close loop when no faster way to find
/// open fds works.
pub(crate) fn fd_limit() -> c_int {
    // SAFETY: an all-zero rlimit is a valid out-parameter.
    let mut rl: libc::rlimit = unsafe { std::mem::zeroed() };
    // SAFETY: rl is a valid out-parameter.
    if unsafe { libc::getrlimit(libc::RLIMIT_NOFILE, &mut rl) } != 0 {
        return 1024;
    }
    if rl.rlim_cur == libc::RLIM_INFINITY || rl.rlim_cur > 1 << 20 {
        1 << 20
    } else {
        rl.rlim_cur as c_int
    }
}

/// Everything the child needs to close inherited fds, prepared before fork.
pub(crate) struct FdCloser {
    limit: c_int,
    #[cfg(target_vendor = "apple")]
    list: Vec<libc::proc_fdinfo>,
}

impl FdCloser {
    pub(crate) fn new() -> Self {
        #[cfg(target_vendor = "apple")]
        {
            // SAFETY: a null buffer asks only for the size estimate.
            let bytes = unsafe {
                libc::proc_pidinfo(
                    libc::getpid(),
                    libc::PROC_PIDLISTFDS,
                    0,
                    std::ptr::null_mut(),
                    0,
                )
            };
            let entries = (bytes.max(0) as usize) / size_of::<libc::proc_fdinfo>() + 256;
            let list = vec![
                libc::proc_fdinfo {
                    proc_fd: -1,
                    proc_fdtype: 0
                };
                entries
            ];
            Self {
                limit: fd_limit(),
                list,
            }
        }
        #[cfg(not(target_vendor = "apple"))]
        Self { limit: fd_limit() }
    }

    /// Closes every fd >= 3 except `keep`. Runs in the forked child: only
    /// async-signal-safe calls, no allocation.
    pub(crate) fn close_all_except(&mut self, keep: RawFd) {
        #[cfg(target_vendor = "apple")]
        {
            let cap = self.list.len() * size_of::<libc::proc_fdinfo>();
            // SAFETY: the buffer is valid for cap bytes; proc_pidinfo is a
            // bare syscall wrapper.
            let got = unsafe {
                libc::proc_pidinfo(
                    libc::getpid(),
                    libc::PROC_PIDLISTFDS,
                    0,
                    self.list.as_mut_ptr().cast(),
                    cap as c_int,
                )
            };
            // A full buffer may be truncated; fall back to the slow loop.
            if got > 0 && (got as usize) < cap {
                let n = got as usize / size_of::<libc::proc_fdinfo>();
                for info in self.list.iter().take(n) {
                    if info.proc_fd > 2 && info.proc_fd != keep {
                        // SAFETY: closing an fd in the child; nothing else uses it.
                        unsafe { libc::close(info.proc_fd) };
                    }
                }
                return;
            }
        }
        #[cfg(target_os = "linux")]
        {
            // SAFETY: close_range only closes fds; failure (ENOSYS on old
            // kernels) falls through to the loop.
            let ok = unsafe {
                (keep <= 3
                    || libc::syscall(libc::SYS_close_range, 3u32, (keep - 1) as u32, 0u32) == 0)
                    && libc::syscall(
                        libc::SYS_close_range,
                        (keep + 1).max(3) as u32,
                        u32::MAX,
                        0u32,
                    ) == 0
            };
            if ok {
                return;
            }
        }
        for fd in 3..self.limit {
            if fd != keep {
                // SAFETY: closing an fd in the child; EBADF is harmless.
                unsafe { libc::close(fd) };
            }
        }
    }
}

/// Name of process `pid` (the executable's base name).
pub(crate) fn process_name(pid: libc::pid_t) -> Option<String> {
    #[cfg(target_vendor = "apple")]
    {
        let mut buf = [0u8; 256];
        // SAFETY: buf is valid for buf.len() bytes.
        let n = unsafe { libc::proc_name(pid, buf.as_mut_ptr().cast(), buf.len() as u32) };
        if n > 0 {
            return Some(String::from_utf8_lossy(&buf[..n as usize]).into_owned());
        }
        let mut path = vec![0u8; libc::PROC_PIDPATHINFO_MAXSIZE as usize];
        // SAFETY: path is valid for its length.
        let n = unsafe { libc::proc_pidpath(pid, path.as_mut_ptr().cast(), path.len() as u32) };
        if n <= 0 {
            return None;
        }
        path.truncate(n as usize);
        let base = path.rsplit(|&b| b == b'/').next().unwrap_or(&path);
        Some(String::from_utf8_lossy(base).into_owned())
    }
    #[cfg(target_os = "linux")]
    {
        let comm = std::fs::read(format!("/proc/{pid}/comm")).ok()?;
        let comm = comm.strip_suffix(b"\n").unwrap_or(&comm);
        Some(String::from_utf8_lossy(comm).into_owned())
    }
}

/// Current directory of process `pid`.
pub(crate) fn process_cwd(pid: libc::pid_t) -> Option<PathBuf> {
    #[cfg(target_vendor = "apple")]
    {
        // SAFETY: an all-zero proc_vnodepathinfo is a valid out-parameter.
        let mut info: libc::proc_vnodepathinfo = unsafe { std::mem::zeroed() };
        let size = size_of::<libc::proc_vnodepathinfo>() as c_int;
        // SAFETY: info is valid for size bytes.
        let n = unsafe {
            libc::proc_pidinfo(
                pid,
                libc::PROC_PIDVNODEPATHINFO,
                0,
                (&mut info as *mut libc::proc_vnodepathinfo).cast(),
                size,
            )
        };
        if n != size {
            return None;
        }
        let bytes: Vec<u8> = info
            .pvi_cdir
            .vip_path
            .iter()
            .flatten()
            .map(|&c| c as u8)
            .take_while(|&b| b != 0)
            .collect();
        if bytes.is_empty() {
            return None;
        }
        Some(PathBuf::from(OsString::from_vec(bytes)))
    }
    #[cfg(target_os = "linux")]
    {
        std::fs::read_link(format!("/proc/{pid}/cwd")).ok()
    }
}
