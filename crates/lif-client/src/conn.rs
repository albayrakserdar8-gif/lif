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
/// cwd, and the variables in [`ENV_ALLOW_LIST`].
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
        caps: Caps(if is_tty { Caps::TTY.0 | Caps::SUSPEND.0 } else { 0 } | Caps::OUTPUT_STREAM.0),
        pid: std::process::id(),
        size,
        pixels,
        term: var("TERM").unwrap_or_default(),
        cwd: std::env::current_dir().map(|p| p.to_string_lossy().into_owned()).unwrap_or_default(),
        env: ENV_ALLOW_LIST.iter().filter_map(|k| Some((k.to_string(), var(k)?))).collect(),
        is_tty,
    }
}

/// Connects and performs the handshake with [`hello_from_env`].
pub fn connect(socket_path: impl AsRef<Path>) -> io::Result<Connection> {
    connect_with(socket_path, hello_from_env())
}

/// Connects and performs the handshake with the given `Hello`.
pub fn connect_with(socket_path: impl AsRef<Path>, hello: Hello) -> io::Result<Connection> {
    let stream = UnixStream::connect(socket_path)?;
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
