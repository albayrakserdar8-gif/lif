//! The thin lif client.
//!
//! It connects to the server, sends one command, and if that command attached
//! it, puts the terminal in raw mode and either hands the terminal to the
//! server (`Caps::TTY`: the server opens it and reads keys and writes frames
//! itself, so the client is out of every keystroke's path) or relays bytes
//! between the terminal and the server, until told to stop. It does no
//! terminal emulation and no rendering: the server sends the exact bytes to
//! write.

mod attach;
mod conn;
mod path;
mod signals;
mod sys;
mod term;

pub use attach::{attach_loop, attach_loop_with, ExitCause, ExitInfo, TermFds};
pub use term::FALLBACK_RESTORE;
pub use conn::{connect, connect_with, hello_from_env, run_command, CommandOutcome, Connection, ServerInfo, ENV_ALLOW_LIST};
pub use path::{default_socket_path, resolve_socket_path, secure_dir, SocketPath};
