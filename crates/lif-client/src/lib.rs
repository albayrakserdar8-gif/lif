//! The thin lif client.
//!
//! It connects to the server, sends one command, and if that command attached
//! it, relays bytes between the terminal and the server until told to stop.
//! It does no terminal emulation and no rendering: the server sends the exact
//! bytes to write.

mod attach;
mod conn;
mod path;
mod signals;
mod sys;
mod term;

pub use attach::{attach_loop, attach_loop_with, ExitCause, ExitInfo, TermFds};
pub use conn::{connect, connect_with, hello_from_env, run_command, CommandOutcome, Connection, ServerInfo, ENV_ALLOW_LIST};
pub use path::{default_socket_path, resolve_socket_path, secure_dir, SocketPath};
