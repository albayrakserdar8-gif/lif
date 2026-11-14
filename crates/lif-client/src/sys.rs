//! Small checked wrappers over libc.

use std::io;
use std::os::fd::RawFd;
use std::time::Instant;

/// Turns a -1 return into the OS error.
pub(crate) fn cvt(r: libc::c_int) -> io::Result<libc::c_int> {
    if r == -1 { Err(io::Error::last_os_error()) } else { Ok(r) }
}

pub(crate) fn read(fd: RawFd, buf: &mut [u8]) -> io::Result<usize> {
    // SAFETY: buf is valid for writes of buf.len() bytes.
    let n = unsafe { libc::read(fd, buf.as_mut_ptr().cast(), buf.len()) };
    if n < 0 { Err(io::Error::last_os_error()) } else { Ok(n as usize) }
}

pub(crate) fn write(fd: RawFd, buf: &[u8]) -> io::Result<usize> {
    // SAFETY: buf is valid for reads of buf.len() bytes.
    let n = unsafe { libc::write(fd, buf.as_ptr().cast(), buf.len()) };
    if n < 0 { Err(io::Error::last_os_error()) } else { Ok(n as usize) }
}

/// Writes all of buf, waiting for writability if the fd is non-blocking.
/// Only used off the hot path (the exit message).
pub(crate) fn write_all(fd: RawFd, mut buf: &[u8]) -> io::Result<()> {
    while !buf.is_empty() {
        match write(fd, buf) {
            Ok(0) => return Err(io::ErrorKind::WriteZero.into()),
            Ok(n) => buf = &buf[n..],
            Err(e) if e.kind() == io::ErrorKind::Interrupted => {}
            Err(e) if e.kind() == io::ErrorKind::WouldBlock => {
                let mut p = libc::pollfd { fd, events: libc::POLLOUT, revents: 0 };
                // SAFETY: one valid pollfd.
                unsafe { libc::poll(&mut p, 1, -1) };
            }
            Err(e) => return Err(e),
        }
    }
    Ok(())
}

/// [`write_all`] that gives up at `deadline` (`TimedOut`), for the
/// shutdown paths that must not hang on a terminal nobody reads.
pub(crate) fn write_all_until(fd: RawFd, mut buf: &[u8], deadline: Instant) -> io::Result<()> {
    while !buf.is_empty() {
        match write(fd, buf) {
            Ok(0) => return Err(io::ErrorKind::WriteZero.into()),
            Ok(n) => buf = &buf[n..],
            Err(e) if e.kind() == io::ErrorKind::Interrupted => {}
            Err(e) if e.kind() == io::ErrorKind::WouldBlock => {
                let ms = poll_ms(deadline).ok_or(io::ErrorKind::TimedOut)?;
                let mut p = libc::pollfd { fd, events: libc::POLLOUT, revents: 0 };
                // SAFETY: one valid pollfd.
                unsafe { libc::poll(&mut p, 1, ms) };
            }
            Err(e) => return Err(e),
        }
    }
    Ok(())
}

/// The poll(2) timeout that ends at `deadline`, rounded up so a poll never
/// returns just before it; `None` once it has passed.
pub(crate) fn poll_ms(deadline: Instant) -> Option<libc::c_int> {
    let left = deadline.checked_duration_since(Instant::now()).filter(|d| !d.is_zero())?;
    Some(left.as_micros().div_ceil(1000).min(libc::c_int::MAX as u128) as libc::c_int)
}

pub(crate) fn get_flags(fd: RawFd) -> io::Result<libc::c_int> {
    // SAFETY: F_GETFL takes no pointer argument.
    cvt(unsafe { libc::fcntl(fd, libc::F_GETFL) })
}

pub(crate) fn set_flags(fd: RawFd, flags: libc::c_int) -> io::Result<()> {
    // SAFETY: F_SETFL takes an int argument.
    cvt(unsafe { libc::fcntl(fd, libc::F_SETFL, flags) }).map(drop)
}

pub(crate) fn winsize(fd: RawFd) -> io::Result<libc::winsize> {
    // SAFETY: winsize is plain data; zero is a valid value.
    let mut ws: libc::winsize = unsafe { std::mem::zeroed() };
    // SAFETY: TIOCGWINSZ writes one winsize through the pointer.
    cvt(unsafe { libc::ioctl(fd, libc::TIOCGWINSZ, &mut ws) })?;
    Ok(ws)
}

/// The device path and `st_rdev` of the terminal on `tty`, provided
/// `input` and `output` are that same terminal.
pub(crate) fn tty_identity(tty: RawFd, input: RawFd, output: RawFd) -> Option<(String, u64)> {
    let rdev = |fd: RawFd| -> Option<u64> {
        // SAFETY: stat is plain data; fstat fills it for a valid fd.
        let mut st: libc::stat = unsafe { std::mem::zeroed() };
        // SAFETY: valid stat pointer; a bad fd is an error, not UB.
        if unsafe { libc::fstat(fd, &mut st) } != 0 || st.st_mode & libc::S_IFMT != libc::S_IFCHR {
            return None;
        }
        Some(st.st_rdev as u64)
    };
    let dev = rdev(tty)?;
    if !isatty(tty) || rdev(input)? != dev || rdev(output)? != dev {
        return None;
    }
    let mut buf = [0u8; 256];
    // SAFETY: buf is valid for its length; ttyname_r NUL-terminates on success.
    if unsafe { libc::ttyname_r(tty, buf.as_mut_ptr().cast(), buf.len()) } != 0 {
        return None;
    }
    let n = buf.iter().position(|&b| b == 0)?;
    Some((String::from_utf8(buf[..n].to_vec()).ok()?, dev))
}

pub(crate) fn isatty(fd: RawFd) -> bool {
    // SAFETY: isatty only inspects the fd.
    unsafe { libc::isatty(fd) == 1 }
}

#[cfg(any(target_os = "macos", target_os = "ios", target_os = "freebsd", target_os = "dragonfly"))]
pub(crate) fn errno_location() -> *mut libc::c_int {
    // SAFETY: returns the calling thread's errno slot; always valid.
    unsafe { libc::__error() }
}

#[cfg(any(target_os = "linux", target_os = "android"))]
pub(crate) fn errno_location() -> *mut libc::c_int {
    // SAFETY: returns the calling thread's errno slot; always valid.
    unsafe { libc::__errno_location() }
}

#[cfg(any(target_os = "openbsd", target_os = "netbsd"))]
pub(crate) fn errno_location() -> *mut libc::c_int {
    // SAFETY: returns the calling thread's errno slot; always valid.
    unsafe { libc::__errno() }
}
