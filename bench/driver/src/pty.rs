//! A child process on a PTY, read without blocking.

use std::ffi::CString;
use std::io;
use std::time::Duration;

pub struct Pty {
    pub master: i32,
    pub pid: i32,
    reaped: Option<i32>,
}

pub enum Read {
    Data(usize),
    WouldBlock,
    Eof,
}

impl Pty {
    /// Starts `argv` (searched on PATH) on a new PTY of the given size.
    pub fn spawn(argv: &[String], cols: u16, rows: u16) -> io::Result<Pty> {
        if argv.is_empty() {
            return Err(io::Error::new(io::ErrorKind::InvalidInput, "empty command"));
        }
        // Everything the child needs is allocated before fork.
        let cargs: Vec<CString> = argv
            .iter()
            .map(|a| CString::new(a.as_bytes()))
            .collect::<Result<_, _>>()
            .map_err(|e| io::Error::new(io::ErrorKind::InvalidInput, e))?;
        let mut ptrs: Vec<*const libc::c_char> = cargs.iter().map(|c| c.as_ptr()).collect();
        ptrs.push(std::ptr::null());
        let mut ws = libc::winsize { ws_row: rows, ws_col: cols, ws_xpixel: 0, ws_ypixel: 0 };
        let mut master = -1;
        // SAFETY: forkpty is given valid pointers. The driver is
        // single-threaded, so the child may call execvp (which can allocate)
        // without inheriting a lock held by another thread.
        let pid = unsafe { libc::forkpty(&mut master, std::ptr::null_mut(), std::ptr::null_mut(), &mut ws) };
        if pid < 0 {
            return Err(io::Error::last_os_error());
        }
        if pid == 0 {
            // SAFETY: ptrs is a NULL-terminated array of valid C strings.
            unsafe {
                libc::execvp(ptrs[0], ptrs.as_ptr());
                libc::_exit(127);
            }
        }
        // SAFETY: master is a valid fd returned by forkpty.
        unsafe {
            let fl = libc::fcntl(master, libc::F_GETFL);
            libc::fcntl(master, libc::F_SETFL, fl | libc::O_NONBLOCK);
            libc::fcntl(master, libc::F_SETFD, libc::FD_CLOEXEC);
        }
        Ok(Pty { master, pid, reaped: None })
    }

    pub fn read(&self, buf: &mut [u8]) -> Read {
        loop {
            // SAFETY: buf is a valid writable buffer of buf.len() bytes.
            let n = unsafe { libc::read(self.master, buf.as_mut_ptr().cast(), buf.len()) };
            if n > 0 {
                return Read::Data(n as usize);
            }
            if n == 0 {
                return Read::Eof;
            }
            match io::Error::last_os_error().raw_os_error() {
                Some(libc::EAGAIN) => return Read::WouldBlock,
                Some(libc::EINTR) => continue,
                // EIO: the slave side is closed (the client exited).
                _ => return Read::Eof,
            }
        }
    }

    /// Writes all of `data`, waiting for the PTY to accept it.
    pub fn write_all(&self, mut data: &[u8]) -> io::Result<()> {
        while !data.is_empty() {
            // SAFETY: data is a valid readable slice.
            let n = unsafe { libc::write(self.master, data.as_ptr().cast(), data.len()) };
            if n > 0 {
                data = &data[n as usize..];
                continue;
            }
            let e = io::Error::last_os_error();
            match e.raw_os_error() {
                Some(libc::EINTR) => {}
                Some(libc::EAGAIN) => {
                    let mut p = libc::pollfd { fd: self.master, events: libc::POLLOUT, revents: 0 };
                    // SAFETY: p is a valid pollfd.
                    unsafe { libc::poll(&mut p, 1, 100) };
                }
                _ => return Err(e),
            }
        }
        Ok(())
    }

    /// Non-blocking reap; returns the raw wait status once the child exited.
    pub fn try_wait(&mut self) -> Option<i32> {
        if self.reaped.is_none() {
            let mut st = 0;
            // SAFETY: plain waitpid on our own child.
            let r = unsafe { libc::waitpid(self.pid, &mut st, libc::WNOHANG) };
            if r == self.pid {
                self.reaped = Some(st);
            }
        }
        self.reaped
    }

    /// Hangs up the terminal (what closing a terminal window does), then
    /// kills the child if it is still there after `grace`.
    pub fn shutdown(&mut self, grace: Duration) -> Option<i32> {
        if self.master >= 0 {
            // SAFETY: closing our own fd once.
            unsafe { libc::close(self.master) };
            self.master = -1;
        }
        let t = std::time::Instant::now();
        while t.elapsed() < grace {
            if let Some(st) = self.try_wait() {
                return Some(st);
            }
            std::thread::sleep(Duration::from_millis(5));
        }
        if self.try_wait().is_none() {
            // SAFETY: signalling our own, not yet reaped, child.
            unsafe { libc::kill(self.pid, libc::SIGKILL) };
            let mut st = 0;
            // SAFETY: blocking reap of our own child.
            unsafe { libc::waitpid(self.pid, &mut st, 0) };
            self.reaped = Some(st);
        }
        self.reaped
    }
}

impl Drop for Pty {
    fn drop(&mut self) {
        self.shutdown(Duration::from_millis(500));
    }
}

/// poll(2) on one fd for readability, with a timeout in milliseconds
/// (negative waits forever). Returns true when readable or hung up.
pub fn wait_readable(fd: i32, timeout_ms: i32) -> bool {
    let mut p = libc::pollfd { fd, events: libc::POLLIN, revents: 0 };
    // SAFETY: p is a valid pollfd.
    let r = unsafe { libc::poll(&mut p, 1, timeout_ms) };
    r > 0
}
