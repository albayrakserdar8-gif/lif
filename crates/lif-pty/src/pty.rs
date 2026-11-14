use std::fmt;
use std::io;
use std::os::fd::{AsFd, AsRawFd, BorrowedFd, FromRawFd, OwnedFd, RawFd};
use std::path::PathBuf;

use libc::c_int;
use lif_types::{PixelSize, Size};

use crate::spawn::{self, Prepared, SpawnOptions};
use crate::sys::{self, cvt, errno};

/// Result of a non-blocking read from the PTY master.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum ReadOutcome {
    /// This many bytes were read into the buffer.
    Data(usize),
    /// Nothing to read right now; wait for readiness.
    WouldBlock,
    /// The slave side is closed: every process holding it has exited. Linux
    /// reports this as EIO, macOS as a zero-length read; both end up here.
    Eof,
}

/// A PTY master with a child process running on its slave side.
///
/// Dropping it closes the master; the kernel then hangs up the slave and
/// sends SIGHUP to the child's session. The child is not waited for here:
/// hand [`Pty::pid`] to a [`crate::ChildReaper`].
pub struct Pty {
    master: OwnedFd,
    pid: libc::pid_t,
}

impl fmt::Debug for Pty {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        f.debug_struct("Pty")
            .field("fd", &self.master.as_raw_fd())
            .field("pid", &self.pid)
            .finish()
    }
}

/// Where the child failed, sent through the error pipe.
#[derive(Clone, Copy)]
#[repr(i32)]
enum Stage {
    Setsid = 1,
    ControllingTerminal = 2,
    Stdio = 3,
    Chdir = 4,
    Exec = 5,
}

impl Stage {
    fn describe(code: i32) -> &'static str {
        match code {
            1 => "setsid",
            2 => "set controlling terminal",
            3 => "redirect stdio",
            4 => "chdir",
            _ => "exec",
        }
    }
}

impl Pty {
    /// Opens a PTY and starts `opts` on it. Errors in the child before exec
    /// (bad program, failed setsid, ...) are returned here, not left as a
    /// dead pane.
    pub fn spawn(opts: &SpawnOptions) -> io::Result<Pty> {
        let mut prep = spawn::prepare(opts)?;
        let (master, slave_name) = sys::open_master()?;

        // SAFETY: slave_name is NUL-terminated; the result is checked.
        let slave = cvt(unsafe {
            libc::open(
                slave_name.as_ptr(),
                libc::O_RDWR | libc::O_NOCTTY | libc::O_CLOEXEC,
            )
        })?;
        // SAFETY: open returned a fresh fd nothing else owns.
        let slave = unsafe { OwnedFd::from_raw_fd(slave) };
        // On macOS TIOCSWINSZ on the master fails with ENOTTY until the slave
        // has been opened once; setting it on the slave works everywhere.
        sys::set_winsize(slave.as_raw_fd(), &sys::winsize(opts.size, opts.pixels))?;
        enable_iutf8(slave.as_raw_fd());

        let (err_read, err_write) = sys::pipe_cloexec()?;
        let pid = fork_child(&mut prep, slave.as_raw_fd(), err_write.as_raw_fd())?;
        drop(slave);
        drop(err_write);

        if let Some((stage, code)) = read_child_error(err_read.as_raw_fd()) {
            wait_blocking(pid);
            let os = io::Error::from_raw_os_error(code);
            let what = Stage::describe(stage);
            return Err(io::Error::new(
                os.kind(),
                format!("spawning {}: {what} failed: {os}", prep.display_name),
            ));
        }

        sys::set_nonblocking(master.as_raw_fd())?;
        Ok(Pty { master, pid })
    }

    /// Reads what the child wrote. Never blocks.
    pub fn read(&mut self, buf: &mut [u8]) -> io::Result<ReadOutcome> {
        if buf.is_empty() {
            return Ok(ReadOutcome::Data(0));
        }
        loop {
            // SAFETY: buf is valid for buf.len() bytes.
            let n =
                unsafe { libc::read(self.master.as_raw_fd(), buf.as_mut_ptr().cast(), buf.len()) };
            if n > 0 {
                return Ok(ReadOutcome::Data(n as usize));
            }
            if n == 0 {
                return Ok(ReadOutcome::Eof);
            }
            match errno() {
                libc::EINTR => continue,
                libc::EAGAIN => return Ok(ReadOutcome::WouldBlock),
                libc::EIO => return Ok(ReadOutcome::Eof),
                e => return Err(io::Error::from_raw_os_error(e)),
            }
        }
    }

    /// Writes input for the child. Never blocks: returns how many bytes the
    /// kernel took (possibly fewer than `buf.len()`), or an error of kind
    /// [`io::ErrorKind::WouldBlock`] when it took none.
    pub fn write(&mut self, buf: &[u8]) -> io::Result<usize> {
        if buf.is_empty() {
            return Ok(0);
        }
        loop {
            // SAFETY: buf is valid for buf.len() bytes.
            let n = unsafe { libc::write(self.master.as_raw_fd(), buf.as_ptr().cast(), buf.len()) };
            if n >= 0 {
                return Ok(n as usize);
            }
            let err = io::Error::last_os_error();
            if err.raw_os_error() != Some(libc::EINTR) {
                return Err(err);
            }
        }
    }

    /// Sets the terminal size; the kernel sends SIGWINCH to the foreground
    /// process group.
    pub fn resize(&self, size: Size, pixels: PixelSize) -> io::Result<()> {
        sys::set_winsize(self.master.as_raw_fd(), &sys::winsize(size, pixels))
    }

    /// The child's pid (also its session and process group id).
    pub fn pid(&self) -> u32 {
        self.pid as u32
    }

    /// Sends `signal` to the child. Only meaningful until the child has been
    /// reaped: after that the pid may belong to an unrelated process.
    pub fn kill(&self, signal: c_int) -> io::Result<()> {
        // SAFETY: plain syscall.
        cvt(unsafe { libc::kill(self.pid, signal) })?;
        Ok(())
    }

    /// Process group currently in the foreground of the terminal.
    pub fn foreground_pgrp(&self) -> Option<u32> {
        // SAFETY: tcgetpgrp only queries the terminal.
        let pgrp = unsafe { libc::tcgetpgrp(self.master.as_raw_fd()) };
        (pgrp > 0).then_some(pgrp as u32)
    }

    /// Name of the foreground process (e.g. "vim"), for window names. Uses
    /// the process group leader, as tmux does.
    pub fn foreground_process_name(&self) -> Option<String> {
        sys::process_name(self.foreground_pgrp()? as libc::pid_t)
    }

    /// Working directory of the foreground process.
    pub fn foreground_cwd(&self) -> Option<PathBuf> {
        sys::process_cwd(self.foreground_pgrp()? as libc::pid_t)
    }
}

impl AsRawFd for Pty {
    fn as_raw_fd(&self) -> RawFd {
        self.master.as_raw_fd()
    }
}

impl AsFd for Pty {
    fn as_fd(&self) -> BorrowedFd<'_> {
        self.master.as_fd()
    }
}

/// Lets line editing in canonical mode erase whole UTF-8 characters.
fn enable_iutf8(fd: RawFd) {
    // SAFETY: an all-zero termios is a valid out-parameter; fd is a terminal.
    unsafe {
        let mut t: libc::termios = std::mem::zeroed();
        if libc::tcgetattr(fd, &mut t) == 0 {
            t.c_iflag |= libc::IUTF8;
            libc::tcsetattr(fd, libc::TCSANOW, &t);
        }
    }
}

fn fork_child(prep: &mut Prepared, slave: RawFd, err_fd: RawFd) -> io::Result<libc::pid_t> {
    // Block every signal across fork so no handler of ours runs in the child
    // before it has reset dispositions.
    // SAFETY: the sigsets are plain values initialised by sigfillset/sigemptyset.
    unsafe {
        let mut all: libc::sigset_t = std::mem::zeroed();
        let mut old: libc::sigset_t = std::mem::zeroed();
        let mut empty: libc::sigset_t = std::mem::zeroed();
        libc::sigfillset(&mut all);
        libc::sigemptyset(&mut empty);
        libc::pthread_sigmask(libc::SIG_SETMASK, &all, &mut old);
        let pid = libc::fork();
        if pid == 0 {
            child(prep, slave, err_fd, &empty);
        }
        let fork_err = io::Error::last_os_error();
        libc::pthread_sigmask(libc::SIG_SETMASK, &old, std::ptr::null_mut());
        if pid < 0 { Err(fork_err) } else { Ok(pid) }
    }
}

/// Reports a failure to the parent and exits. Async-signal-safe.
fn fail(err_fd: RawFd, stage: Stage) -> ! {
    let code = errno();
    let mut msg = [0u8; 8];
    msg[..4].copy_from_slice(&(stage as i32).to_ne_bytes());
    msg[4..].copy_from_slice(&code.to_ne_bytes());
    // SAFETY: write and _exit are async-signal-safe; msg is valid for 8 bytes.
    unsafe {
        libc::write(err_fd, msg.as_ptr().cast(), msg.len());
        libc::_exit(127);
    }
}

/// The forked child. Only async-signal-safe calls, no allocation, no panics:
/// another thread of the parent may have held the allocator lock at fork.
///
/// # Safety
/// Must only be called in a freshly forked child.
unsafe fn child(prep: &mut Prepared, slave: RawFd, err_fd: RawFd, empty: &libc::sigset_t) -> ! {
    // SAFETY: every call below is async-signal-safe and gets valid pointers
    // into memory prepared before fork.
    unsafe {
        let mut dfl: libc::sigaction = std::mem::zeroed();
        dfl.sa_sigaction = libc::SIG_DFL;
        // Covers the real-time signals on Linux; invalid numbers just fail.
        for sig in 1..65 {
            libc::sigaction(sig, &dfl, std::ptr::null_mut());
        }
        libc::sigprocmask(libc::SIG_SETMASK, empty, std::ptr::null_mut());

        // The dup2s below would clobber fds 0-2 if the parent had them closed.
        let mut err_fd = err_fd;
        if err_fd < 3 {
            err_fd = libc::fcntl(err_fd, libc::F_DUPFD_CLOEXEC, 3);
            if err_fd < 0 {
                libc::_exit(127);
            }
        }
        if libc::setsid() < 0 {
            fail(err_fd, Stage::Setsid);
        }
        let mut slave = slave;
        if slave < 3 {
            slave = libc::fcntl(slave, libc::F_DUPFD, 3);
            if slave < 0 {
                fail(err_fd, Stage::Stdio);
            }
        }
        if libc::ioctl(slave, libc::TIOCSCTTY as _, 0) < 0 {
            fail(err_fd, Stage::ControllingTerminal);
        }
        for target in 0..3 {
            if libc::dup2(slave, target) < 0 {
                fail(err_fd, Stage::Stdio);
            }
        }
        prep.closer.close_all_except(err_fd);

        if !prep.dirs.iter().any(|d| libc::chdir(d.as_ptr()) == 0) {
            fail(err_fd, Stage::Chdir);
        }

        // execvp's rules: skip candidates that do not exist, remember EACCES,
        // stop at any other error.
        let mut saw_eacces = false;
        for program in &prep.programs {
            libc::execve(program.as_ptr(), prep.argv.as_ptr(), prep.envp.as_ptr());
            match errno() {
                libc::ENOENT | libc::ENOTDIR => {}
                libc::EACCES => saw_eacces = true,
                _ => fail(err_fd, Stage::Exec),
            }
        }
        if saw_eacces {
            *sys::errno_location() = libc::EACCES;
        }
        fail(err_fd, Stage::Exec);
    }
}

/// Reads the child's error report: `None` when exec succeeded (the pipe
/// closed on exec without data).
fn read_child_error(fd: RawFd) -> Option<(i32, i32)> {
    let mut msg = [0u8; 8];
    let mut got = 0;
    while got < msg.len() {
        // SAFETY: the slice from got is valid for msg.len() - got bytes.
        let n = unsafe { libc::read(fd, msg[got..].as_mut_ptr().cast(), msg.len() - got) };
        if n > 0 {
            got += n as usize;
        } else if n == 0 || errno() != libc::EINTR {
            break;
        }
    }
    match got {
        0 => None,
        8 => Some((
            i32::from_ne_bytes([msg[0], msg[1], msg[2], msg[3]]),
            i32::from_ne_bytes([msg[4], msg[5], msg[6], msg[7]]),
        )),
        _ => Some((Stage::Exec as i32, libc::EIO)),
    }
}

/// Reaps a child that is known to be exiting (it reported an error).
fn wait_blocking(pid: libc::pid_t) {
    let mut status = 0;
    // SAFETY: plain syscall with a valid out-pointer.
    while unsafe { libc::waitpid(pid, &mut status, 0) } < 0 && errno() == libc::EINTR {}
}
