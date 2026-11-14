//! Starting the server: lock, double fork, daemonize, bind, signal ready.

use std::fs::OpenOptions;
use std::io::{self, Read};
use std::os::fd::{AsRawFd, FromRawFd, OwnedFd, RawFd};
use std::os::unix::fs::{OpenOptionsExt, PermissionsExt};
use std::os::unix::net::{UnixListener, UnixStream};
use std::path::{Path, PathBuf};

fn cvt(r: libc::c_int) -> io::Result<libc::c_int> {
    if r == -1 { Err(io::Error::last_os_error()) } else { Ok(r) }
}

fn lock_path(socket: &Path) -> PathBuf {
    let mut s = socket.as_os_str().to_owned();
    s.push(".lock");
    PathBuf::from(s)
}

/// Starts a server on `socket` unless one is already running there, and
/// returns once it accepts connections. An exclusive lock on
/// `<socket>.lock` serializes clients racing to start one.
pub(crate) fn start_server(socket: &Path, verbose: bool) -> io::Result<()> {
    let lock_path = lock_path(socket);
    let lock = OpenOptions::new().read(true).write(true).create(true).truncate(false).mode(0o600).open(&lock_path)?;
    loop {
        // SAFETY: flock on an fd we own.
        match cvt(unsafe { libc::flock(lock.as_raw_fd(), libc::LOCK_EX) }) {
            Ok(_) => break,
            Err(e) if e.kind() == io::ErrorKind::Interrupted => continue,
            Err(e) => return Err(e),
        }
    }
    // Whoever held the lock before us may have started it.
    if UnixStream::connect(socket).is_ok() {
        return Ok(());
    }
    match std::fs::remove_file(socket) {
        Ok(()) => {}
        Err(e) if e.kind() == io::ErrorKind::NotFound => {}
        Err(e) => return Err(e),
    }
    let mut fds = [0; 2];
    // SAFETY: fds has room for two descriptors.
    cvt(unsafe { libc::pipe(fds.as_mut_ptr()) })?;
    // SAFETY: fresh fds from pipe(2).
    let (ready_r, ready_w) = unsafe { (OwnedFd::from_raw_fd(fds[0]), OwnedFd::from_raw_fd(fds[1])) };
    // SAFETY: the client is single-threaded here, so the child may keep
    // using the allocator and std after fork.
    let pid = cvt(unsafe { libc::fork() })?;
    if pid == 0 {
        drop(ready_r);
        // SAFETY: plain syscalls in the child; the intermediate child only
        // makes the grandchild an orphan in its own session and exits.
        unsafe {
            libc::setsid();
            match libc::fork() {
                0 => {}
                -1 => libc::_exit(1),
                _ => libc::_exit(0),
            }
        }
        let code = daemon_main(socket, &lock_path, verbose, ready_w.as_raw_fd());
        std::process::exit(code);
    }
    drop(ready_w);
    let mut status = 0;
    // SAFETY: waits for the intermediate child we just forked.
    unsafe { libc::waitpid(pid, &mut status, 0) };
    let mut answer = Vec::new();
    let mut f = std::fs::File::from(ready_r);
    let _ = f.read_to_end(&mut answer);
    drop(lock);
    match answer.first() {
        Some(b'R') => Ok(()),
        Some(b'E') => Err(io::Error::other(String::from_utf8_lossy(&answer[1..]).into_owned())),
        _ => Err(io::Error::other("server exited before it was ready")),
    }
}

/// Open descriptors of this process. Listing them is much cheaper than
/// closing every number up to the limit, which can be tens of thousands.
fn open_fds() -> Vec<RawFd> {
    let dir = if cfg!(target_os = "linux") { "/proc/self/fd" } else { "/dev/fd" };
    match std::fs::read_dir(dir) {
        Ok(rd) => rd.filter_map(|e| e.ok()?.file_name().to_str()?.parse().ok()).collect(),
        // SAFETY: getdtablesize cannot fail.
        Err(_) => (0..unsafe { libc::getdtablesize() }.clamp(64, 65536)).collect(),
    }
}

/// The daemon: detached from the terminal and the client's process group,
/// cwd `/`, stdio on /dev/null, no inherited fds, default signals.
fn daemon_main(socket: &Path, lock: &Path, verbose: bool, ready: RawFd) -> i32 {
    // SAFETY: plain syscalls on fds this process owns.
    unsafe {
        libc::chdir(c"/".as_ptr());
        let null = libc::open(c"/dev/null".as_ptr(), libc::O_RDWR);
        if null >= 0 {
            for fd in 0..3 {
                libc::dup2(null, fd);
            }
            if null > 2 {
                libc::close(null);
            }
        }
        for fd in open_fds() {
            if fd > 2 && fd != ready {
                libc::close(fd);
            }
        }
        for sig in 1..32 {
            if sig != libc::SIGKILL && sig != libc::SIGSTOP {
                libc::signal(sig, libc::SIG_DFL);
            }
        }
        let mut set: libc::sigset_t = std::mem::zeroed();
        libc::sigemptyset(&mut set);
        libc::sigprocmask(libc::SIG_SETMASK, &set, std::ptr::null_mut());
        libc::fcntl(ready, libc::F_SETFD, libc::FD_CLOEXEC);
    }
    let tell = |msg: &[u8]| {
        // SAFETY: writes msg to the ready pipe.
        unsafe { libc::write(ready, msg.as_ptr().cast(), msg.len()) };
    };
    // Panes inherit the user's umask; only the socket gets 0600.
    // SAFETY: umask cannot fail.
    let old = unsafe { libc::umask(0o177) };
    let bound = UnixListener::bind(socket);
    // SAFETY: as above.
    unsafe { libc::umask(old) };
    let listener = match bound {
        Ok(l) => l,
        Err(e) => {
            tell(format!("E{e}").as_bytes());
            return 1;
        }
    };
    let _ = std::fs::set_permissions(socket, std::fs::Permissions::from_mode(0o600));
    tell(b"R");
    // SAFETY: closes our end of the ready pipe; the client sees EOF.
    unsafe { libc::close(ready) };
    lif_server::run(listener, lif_server::ServerOptions { socket: socket.to_path_buf(), lock: Some(lock.to_path_buf()), verbose })
}
