//! Connecting, the Hello/Welcome handshake and one-shot commands.

use std::io::{self, Write};
use std::os::unix::net::UnixStream;
use std::path::Path;
use std::time::Duration;

use lif_proto::{Caps, ClientMsg, Decoder, Hello, ServerMsg, PROTOCOL_VERSION};
use lif_types::{PixelSize, Size};

use crate::sys;

/// Environment variables passed to the server in `Hello`. Fixed, so a
/// client never leaks unrelated secrets from its environment (the server
/// enforces the same list).
pub use lif_proto::ENV_ALLOW_LIST;

/// Most environment variables a client sends in `Hello`.
pub const HELLO_ENV_MAX: usize = 512;

/// A server that accepts but never answers must not hang the client forever.
const HANDSHAKE_TIMEOUT: Duration = Duration::from_secs(10);

/// What the server said about itself in `Welcome`.
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct ServerInfo {
    pub protocol: u16,
    pub caps: Caps,
    pub version: String,
    pub pid: u32,
}

/// The most standard input a command can take (one protocol frame).
pub const STDIN_LIMIT: usize = lif_proto::MAX_PAYLOAD - 64;

/// A connection that has completed the handshake.
#[derive(Debug)]
pub struct Connection {
    pub(crate) stream: UnixStream,
    /// Owned by the connection so bytes the server sent right behind a reply
    /// (e.g. the first `Output` after `Attached`) are not lost between
    /// `run_command` and `attach_loop`.
    pub(crate) dec: Decoder<ServerMsg>,
    server: ServerInfo,
    caps: Caps,
    pub(crate) hello_size: Size,
    pub(crate) hello_pixels: PixelSize,
}

impl Connection {
    pub fn server(&self) -> &ServerInfo {
        &self.server
    }

    /// Capabilities both sides announced.
    pub fn caps(&self) -> Caps {
        self.caps
    }

    /// Standard input for the next command (`load-buffer -`), at most
    /// one frame: [`STDIN_LIMIT`] bytes.
    pub fn send_stdin(&mut self, data: Vec<u8>) -> io::Result<()> {
        if data.len() > STDIN_LIMIT {
            return Err(io::Error::new(io::ErrorKind::InvalidInput, "standard input too large"));
        }
        self.send(&ClientMsg::Stdin(data))
    }

    fn send(&mut self, msg: &ClientMsg) -> io::Result<()> {
        let mut out = Vec::new();
        msg.encode(&mut out)?;
        self.stream.write_all(&out)
    }

    /// The next known message, blocking. Unknown types are skipped (rule 2).
    fn recv(&mut self) -> io::Result<ServerMsg> {
        loop {
            match self.dec.next()? {
                Some(ServerMsg::Unknown { .. }) => continue,
                Some(m) => return Ok(m),
                None => {}
            }
            if self.dec.read_from(&mut self.stream)? == 0 {
                return Err(io::Error::new(io::ErrorKind::UnexpectedEof, "server closed the connection"));
            }
        }
    }
}

/// `Hello` describing this process: terminal size from stdout, `$TERM`,
/// cwd, and its environment (at most [`HELLO_ENV_MAX`] variables, none
/// longer than 4096 bytes). The server keeps only the variables of
/// [`ENV_ALLOW_LIST`] and those its `update-environment` option names.
pub fn hello_from_env() -> Hello {
    let (size, pixels) = match sys::winsize(1) {
        Ok(ws) => (Size::new(ws.ws_col, ws.ws_row), PixelSize { width: ws.ws_xpixel, height: ws.ws_ypixel }),
        Err(_) => Default::default(),
    };
    let var = |k: &str| std::env::var_os(k).map(|v| v.to_string_lossy().into_owned());
    let is_tty = sys::isatty(0) && sys::isatty(1);
    Hello {
        protocol: PROTOCOL_VERSION,
        // On a terminal the server may take it over while attached.
        caps: Caps(if is_tty { Caps::TTY.0 | Caps::SUSPEND.0 | Caps::LOCK.0 } else { 0 } | Caps::OUTPUT_STREAM.0),
        pid: std::process::id(),
        size,
        pixels,
        term: var("TERM").unwrap_or_default(),
        cwd: std::env::current_dir().map(|p| p.to_string_lossy().into_owned()).unwrap_or_default(),
        env: std::env::vars_os()
            .filter_map(|(k, v)| Some((k.into_string().ok()?, v.into_string().ok()?)))
            .filter(|(k, v)| !k.is_empty() && k.len() + v.len() <= 4096)
            .take(HELLO_ENV_MAX)
            .collect(),
        is_tty,
    }
}

/// Connects to the server's socket without ever waiting for room in its
/// listen queue. A server that is stopped with a full queue refuses the
/// connection on macOS; on Linux a blocking `connect` would wait for it
/// for ever, so the connection is made non-blocking (where a full queue
/// is `EAGAIN`, reported as a refusal here) and switched back.
pub fn connect_stream(socket_path: &Path) -> io::Result<UnixStream> {
    use std::os::fd::{FromRawFd, OwnedFd};
    use std::os::unix::ffi::OsStrExt;
    let bytes = socket_path.as_os_str().as_bytes();
    // SAFETY: a zeroed sockaddr_un is a valid value to fill in.
    let mut addr: libc::sockaddr_un = unsafe { std::mem::zeroed() };
    if bytes.len() >= addr.sun_path.len() || bytes.contains(&0) {
        return Err(io::Error::new(io::ErrorKind::InvalidInput, "socket path too long"));
    }
    addr.sun_family = libc::AF_UNIX as libc::sa_family_t;
    for (dst, src) in addr.sun_path.iter_mut().zip(bytes) {
        *dst = *src as libc::c_char;
    }
    // SAFETY: socket(2) with constant arguments; the descriptor is owned
    // from here on and closed on every path.
    let fd = unsafe {
        let fd = sys::cvt(libc::socket(libc::AF_UNIX, libc::SOCK_STREAM, 0))?;
        OwnedFd::from_raw_fd(fd)
    };
    let raw = std::os::fd::AsRawFd::as_raw_fd(&fd);
    // SAFETY: fcntl and connect on the descriptor just made, with an
    // address of the size given.
    unsafe {
        sys::cvt(libc::fcntl(raw, libc::F_SETFD, libc::FD_CLOEXEC))?;
        let flags = sys::cvt(libc::fcntl(raw, libc::F_GETFL))?;
        sys::cvt(libc::fcntl(raw, libc::F_SETFL, flags | libc::O_NONBLOCK))?;
        let len = std::mem::size_of::<libc::sockaddr_un>() as libc::socklen_t;
        if libc::connect(raw, (&raw const addr).cast(), len) != 0 {
            let e = io::Error::last_os_error();
            return Err(match e.kind() {
                io::ErrorKind::WouldBlock => io::Error::new(io::ErrorKind::ConnectionRefused, "the server's listen queue is full"),
                _ => e,
            });
        }
        sys::cvt(libc::fcntl(raw, libc::F_SETFL, flags))?;
    }
    Ok(UnixStream::from(fd))
}

/// Connects and performs the handshake with [`hello_from_env`].
pub fn connect(socket_path: impl AsRef<Path>) -> io::Result<Connection> {
    connect_with(socket_path, hello_from_env())
}

/// Connects and performs the handshake with the given `Hello`.
pub fn connect_with(socket_path: impl AsRef<Path>, hello: Hello) -> io::Result<Connection> {
    let stream = connect_stream(socket_path.as_ref())?;
    // macOS fails setsockopt with EINVAL once the peer has closed; a dead
    // server shows up on the first read or write instead.
    let _ = no_sigpipe(&stream);
    stream.set_read_timeout(Some(HANDSHAKE_TIMEOUT))?;
    stream.set_write_timeout(Some(HANDSHAKE_TIMEOUT))?;
    let mut conn = Connection {
        stream,
        dec: Decoder::new(),
        server: ServerInfo { protocol: 0, caps: Caps::NONE, version: String::new(), pid: 0 },
        caps: Caps::NONE,
        hello_size: hello.size,
        hello_pixels: hello.pixels,
    };
    let caps = hello.caps;
    conn.send(&ClientMsg::Hello(hello))?;
    match conn.recv()? {
        ServerMsg::Welcome { protocol, caps: server_caps, server_version, server_pid } => {
            if protocol != PROTOCOL_VERSION {
                return Err(io::Error::new(
                    io::ErrorKind::Unsupported,
                    format!("server speaks protocol {protocol}, this client speaks {PROTOCOL_VERSION}"),
                ));
            }
            conn.caps = caps.intersect(server_caps);
            conn.server = ServerInfo { protocol, caps: server_caps, version: server_version, pid: server_pid };
        }
        m => return Err(unexpected(&m, "Welcome")),
    }
    // Commands may legitimately take long. Errors as for no_sigpipe above.
    let _ = conn.stream.set_read_timeout(None);
    let _ = conn.stream.set_write_timeout(None);
    Ok(conn)
}

fn unexpected(m: &ServerMsg, wanted: &str) -> io::Error {
    io::Error::new(io::ErrorKind::InvalidData, format!("expected {wanted} from the server, got {m:?}"))
}

#[cfg(any(target_os = "macos", target_os = "ios", target_os = "freebsd"))]
fn no_sigpipe(s: &UnixStream) -> io::Result<()> {
    use std::os::fd::AsRawFd;
    let one: libc::c_int = 1;
    // SAFETY: SO_NOSIGPIPE reads one c_int through the pointer.
    sys::cvt(unsafe {
        libc::setsockopt(
            s.as_raw_fd(),
            libc::SOL_SOCKET,
            libc::SO_NOSIGPIPE,
            (&one as *const libc::c_int).cast(),
            std::mem::size_of::<libc::c_int>() as libc::socklen_t,
        )
    })
    .map(drop)
}

/// Elsewhere the Rust runtime already ignores SIGPIPE for the process.
#[cfg(not(any(target_os = "macos", target_os = "ios", target_os = "freebsd")))]
fn no_sigpipe(_: &UnixStream) -> io::Result<()> {
    Ok(())
}

/// How a command ended.
#[derive(Clone, Debug, PartialEq, Eq)]
pub enum CommandOutcome {
    /// Print `stdout` / `stderr` and exit with `status`.
    Done { status: i32, stdout: Vec<u8>, stderr: Vec<u8> },
    /// The command attached this client; call [`crate::attach_loop`].
    Attached,
}

/// Sends one command and waits for its outcome. Output a server sends in
/// parts (`CommandOutput`, for output larger than one frame) is joined.
pub fn run_command<S: Into<String>>(conn: &mut Connection, argv: impl IntoIterator<Item = S>) -> io::Result<CommandOutcome> {
    let argv = argv.into_iter().map(Into::into).collect();
    if let Err(e) = conn.send(&ClientMsg::Command { argv }) {
        // A server that refuses the connection (out of descriptors) has
        // answered and closed it before reading: its answer says why.
        return match conn.recv() {
            Ok(ServerMsg::CommandDone { status, stdout, stderr }) => Ok(CommandOutcome::Done { status, stdout, stderr }),
            _ => Err(e),
        };
    }
    let (mut out, mut err) = (Vec::new(), Vec::new());
    loop {
        match conn.recv()? {
            ServerMsg::CommandOutput { stdout, stderr } => {
                out.extend_from_slice(&stdout);
                err.extend_from_slice(&stderr);
            }
            ServerMsg::CommandDone { status, stdout, stderr } => {
                out.extend_from_slice(&stdout);
                err.extend_from_slice(&stderr);
                return Ok(CommandOutcome::Done { status, stdout: out, stderr: err });
            }
            ServerMsg::Attached => return Ok(CommandOutcome::Attached),
            m => return Err(unexpected(&m, "CommandDone or Attached")),
        }
    }
}
