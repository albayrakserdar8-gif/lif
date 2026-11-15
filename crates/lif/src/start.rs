//! Starting the server: lock, double fork, daemonize, bind, signal ready.

use std::fs::OpenOptions;
use std::io::{self, Read};
use std::os::fd::{AsRawFd, FromRawFd, OwnedFd, RawFd};
use std::os::unix::fs::{FileTypeExt, MetadataExt, OpenOptionsExt, PermissionsExt};
use std::os::unix::net::{UnixListener, UnixStream};
use std::path::{Path, PathBuf};
use std::time::{Duration, Instant};

fn cvt(r: libc::c_int) -> io::Result<libc::c_int> {
    if r == -1 { Err(io::Error::last_os_error()) } else { Ok(r) }
}

fn lock_path(socket: &Path) -> PathBuf {
    let mut s = socket.as_os_str().to_owned();
    s.push(".lock");
    PathBuf::from(s)
}

/// How long a client waits for a server another process holds the lock
/// for (it is starting, or it is running) to accept a connection.
const LOCKED_WAIT: Duration = Duration::from_secs(3);

/// `<socket>.lock`, opened for flock: never through a symlink, never
/// someone else's file, never inherited by the panes.
fn open_lock(path: &Path, create: bool) -> io::Result<std::fs::File> {
    let f = OpenOptions::new()
        .read(true)
        .write(true)
        .create(create)
        .truncate(false)
        .mode(0o600)
        .custom_flags(libc::O_NOFOLLOW | libc::O_CLOEXEC)
        .open(path)?;
    let m = f.metadata()?;
    // SAFETY: getuid cannot fail.
    let uid = unsafe { libc::getuid() };
    if !m.file_type().is_file() || m.uid() != uid {
        return Err(io::Error::new(io::ErrorKind::PermissionDenied, format!("{}: not a regular file owned by you", path.display())));
    }
    if m.mode() & 0o777 != 0o600 {
        f.set_permissions(std::fs::Permissions::from_mode(0o600))?;
    }
    Ok(f)
}

/// Takes the exclusive lock without waiting. False if another process
/// holds it.
fn try_lock(f: &std::fs::File) -> io::Result<bool> {
    loop {
        // SAFETY: flock on an fd we own.
        match cvt(unsafe { libc::flock(f.as_raw_fd(), libc::LOCK_EX | libc::LOCK_NB) }) {
            Ok(_) => return Ok(true),
            Err(e) if e.kind() == io::ErrorKind::Interrupted => continue,
            Err(e) if e.kind() == io::ErrorKind::WouldBlock => return Ok(false),
            Err(e) => return Err(e),
        }
    }
}

/// Whether a server process lives on `socket`: it holds the lock for as
/// long as it runs. For telling "not responding" from "not running".
pub(crate) fn server_holds_lock(socket: &Path) -> bool {
    match open_lock(&lock_path(socket), false) {
        // Closing our fd releases the lock we may have taken.
        Ok(f) => matches!(try_lock(&f), Ok(false)),
        Err(_) => false,
    }
}

/// Removes what a dead server left at `socket`. Only a socket of ours: on
/// Linux connecting to a regular file also fails with ECONNREFUSED, and
/// the path may be anything the user named with -S.
fn remove_stale(socket: &Path) -> io::Result<()> {
    let m = match std::fs::symlink_metadata(socket) {
        Ok(m) => m,
        Err(e) if e.kind() == io::ErrorKind::NotFound => return Ok(()),
        Err(e) => return Err(e),
    };
    // SAFETY: getuid cannot fail.
    let uid = unsafe { libc::getuid() };
    if !m.file_type().is_socket() || m.uid() != uid {
        return Err(io::Error::new(
            io::ErrorKind::AlreadyExists,
            format!("{} exists and is not a socket of yours; not removing it", socket.display()),
        ));
    }
    match std::fs::remove_file(socket) {
        Err(e) if e.kind() != io::ErrorKind::NotFound => Err(e),
        _ => Ok(()),
    }
}

/// The error for a server that holds the lock but does not accept.
pub(crate) fn not_responding(socket: &Path) -> io::Error {
    io::Error::new(io::ErrorKind::TimedOut, format!("server on {} is not responding", socket.display()))
}

/// Starts a server on `socket` unless one is already running there, and
/// returns once it accepts connections.
///
/// A server holds an exclusive lock on `<socket>.lock` for its whole life
/// (the daemon inherits the lock of the client that starts it). A client
/// that gets the lock knows no server lives on the path, so whatever is
/// at the socket path is stale; one that does not get it waits for the
/// server (being started by another client, or running) to accept, and
/// reports a server that never does as not responding. It never removes
/// the socket of a live server: one that is stopped, or whose listen
/// backlog is full, refuses connections like a dead one.
/// `config` is `lif -f`: the configuration file the new server reads
/// (a server already running keeps its own, as in tmux).
pub(crate) fn start_server(socket: &Path, verbose: bool, config: Option<&Path>) -> io::Result<()> {
    let lock_path = lock_path(socket);
    let lock = open_lock(&lock_path, true)?;
    let deadline = Instant::now() + LOCKED_WAIT;
    while !try_lock(&lock)? {
        if UnixStream::connect(socket).is_ok() {
            return Ok(());
        }
        if Instant::now() >= deadline {
            return Err(not_responding(socket));
        }
        std::thread::sleep(Duration::from_millis(10));
    }
    remove_stale(socket)?;
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
        // A relative -f is relative to where the user ran lif, not to the
        // daemon's / working directory.
        let config = config.map(|c| std::env::current_dir().map(|d| d.join(c)).unwrap_or_else(|_| c.to_path_buf()));
        let code = daemon_main(socket, &lock_path, verbose, config, ready_w.as_raw_fd(), lock.as_raw_fd());
        std::process::exit(code);
    }
    drop(ready_w);
    let mut status = 0;
    // SAFETY: waits for the intermediate child we just forked.
    unsafe { libc::waitpid(pid, &mut status, 0) };
    let mut answer = Vec::new();
    let mut f = std::fs::File::from(ready_r);
    let _ = f.read_to_end(&mut answer);
    // Closing our descriptor does not release the lock: the daemon holds
    // the same open file description. (LOCK_UN would release it for both.)
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
fn daemon_main(socket: &Path, lock: &Path, verbose: bool, config: Option<PathBuf>, ready: RawFd, lock_fd: RawFd) -> i32 {
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
            // The lock stays open (and locked) until the server exits.
            if fd > 2 && fd != ready && fd != lock_fd {
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
    lif_server::run(
        listener,
        lif_server::ServerOptions { socket: socket.to_path_buf(), lock: Some(lock.to_path_buf()), verbose, config },
    )
}
