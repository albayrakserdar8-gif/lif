use lif_types::{PixelSize, Size};

/// Message type ids. Client-to-server ids are below 0x100.
pub mod type_id {
    pub const HELLO: u16 = 0x01;
    pub const COMMAND: u16 = 0x02;
    pub const INPUT: u16 = 0x03;
    pub const RESIZE: u16 = 0x04;
    pub const DETACH: u16 = 0x05;
    pub const USE_TTY: u16 = 0x06;

    pub const WELCOME: u16 = 0x101;
    pub const COMMAND_DONE: u16 = 0x102;
    pub const ATTACHED: u16 = 0x103;
    pub const OUTPUT: u16 = 0x104;
    pub const EXIT: u16 = 0x105;
    pub const TTY: u16 = 0x106;
}

/// Optional capabilities, exchanged in `Hello` / `Welcome`. A side may only
/// use a capability both sides announced.
#[derive(Clone, Copy, Debug, Default, PartialEq, Eq, Hash)]
pub struct Caps(pub u64);

impl Caps {
    pub const NONE: Caps = Caps(0);
    /// The client's terminal is a tty the server may open itself and use
    /// directly while the client is attached (`UseTty` / `Tty`), instead
    /// of relaying every key and frame through the client: each keystroke
    /// echo then costs two fewer process wake-ups.
    pub const TTY: Caps = Caps(1);

    pub const fn contains(self, other: Caps) -> bool {
        self.0 & other.0 == other.0
    }

    pub const fn intersect(self, other: Caps) -> Caps {
        Caps(self.0 & other.0)
    }
}

/// First message on every connection.
#[derive(Clone, Debug, Default, PartialEq, Eq)]
pub struct Hello {
    pub protocol: u16,
    pub caps: Caps,
    pub pid: u32,
    /// The client's terminal size; zero when stdout is not a terminal.
    pub size: Size,
    pub pixels: PixelSize,
    /// `$TERM` of the client.
    pub term: String,
    /// The client's working directory (default start directory for new panes).
    pub cwd: String,
    /// Environment the client wants new sessions to inherit and existing
    /// sessions to refresh (e.g. `COLORTERM`, `SSH_AUTH_SOCK`, `DISPLAY`).
    pub env: Vec<(String, String)>,
    /// Whether the client's stdin and stdout are both a terminal. The server
    /// refuses to attach a client that is not.
    pub is_tty: bool,
}

/// Why the server ended an attached client. Encoded as u8.
#[derive(Clone, Copy, Debug, Default, PartialEq, Eq, Hash)]
pub enum ExitReason {
    /// The client detached (0).
    #[default]
    Detached,
    /// The session it was attached to ended (1).
    SessionEnded,
    /// The server is shutting down (2).
    ServerExit,
    /// The client was replaced or kicked by another client (3).
    Kicked,
    /// A protocol or internal error; see the message (4).
    Error,
}

/// Client to server.
#[derive(Clone, Debug, PartialEq, Eq)]
pub enum ClientMsg {
    Hello(Hello),
    /// Run one command, tmux style: `["new-session", "-s", "work"]`.
    ///
    /// The server answers with exactly one of:
    /// - `CommandDone`: the command finished; the client prints the output
    ///   and, unless it is attached, exits with `status`.
    /// - `Attached`: the command attached this client to a session; from
    ///   here on the server sends `Output` and the client sends `Input`.
    Command { argv: Vec<String> },
    /// Raw bytes read from the client's terminal. Only while attached.
    Input(Vec<u8>),
    /// The client's terminal changed size. Only while attached.
    Resize { size: Size, pixels: PixelSize },
    /// The client is going away (e.g. it got SIGHUP). Only while attached.
    Detach,
    /// Sent once, right after `Attached` and after the terminal was put in
    /// raw mode, when both sides announced [`Caps::TTY`]: the terminal's
    /// device path and `st_rdev` (an empty path declines). Until it
    /// arrives the server writes nothing for the terminal, and the client
    /// reads nothing from it until the server's [`ServerMsg::Tty`].
    UseTty { path: String, dev: u64 },
    /// A frame type this build does not know. Never sent.
    Unknown { type_id: u16 },
}

/// Server to client.
#[derive(Clone, Debug, PartialEq, Eq)]
pub enum ServerMsg {
    /// Answer to `Hello`.
    Welcome { protocol: u16, caps: Caps, server_version: String, server_pid: u32 },
    CommandDone { status: i32, stdout: Vec<u8>, stderr: Vec<u8> },
    Attached,
    /// Bytes to write to the client's terminal, verbatim.
    Output(Vec<u8>),
    /// The attachment is over. The server has already sent the bytes that
    /// restore the terminal; the client leaves raw mode, prints `message` if
    /// it is not empty, and exits.
    Exit { reason: ExitReason, message: String },
    /// Answer to `UseTty`. `direct`: the server opened the terminal and
    /// reads and writes it itself from now on; the client stops reading it
    /// and only forwards resizes and signals. Otherwise the client relays.
    Tty { direct: bool },
    /// A frame type this build does not know. Never sent.
    Unknown { type_id: u16 },
}
