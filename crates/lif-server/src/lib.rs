//! The lif server: one thread, one `mio::Poll`, every pane and client.
//!
//! The binary (`crates/lif`) daemonizes, binds the socket and calls [`run`].
//! Everything else happens in the event loop of [`server`]:
//!
//! - PTY output is read with a per-pane budget, fed to the pane's
//!   `VtEngine` and never queued for clients: clients get the newest state
//!   rendered as a diff when they can take it ([`frame`]).
//! - Input from clients is parsed (`lif-input`), routed through the key
//!   tables of `lif-core` and encoded for the program in the pane.
//! - `lif-core` owns the model; [`host::Rt`] implements its `Host` trait.
//!
//! The loop only wakes for I/O or for one of a few explicit one-shot
//! deadlines (see `Server::next_deadline`): an idle server makes no wakeups.
#![deny(unsafe_op_in_unsafe_fn)]

#[macro_use]
mod log;
mod client;
mod frame;
mod host;
mod input;
pub mod mode;
mod pane;
mod panic;
mod prompt;
mod server;
mod sys;

use std::os::unix::net::UnixListener;
use std::path::PathBuf;
use std::time::Duration;

pub use mode::{ModeAction, ModeCtx, PaneMode};

/// Shortest time between two frames sent to one client. A keypress echo is
/// rendered in the same loop iteration as long as the previous frame is
/// older than this; output arriving faster is coalesced. 8 ms (125 Hz) is
/// above the refresh rate of nearly every display, so pacing never shows.
pub const MIN_FRAME_INTERVAL: Duration = Duration::from_millis(8);

/// Bytes read from one PTY per loop iteration before other panes and the
/// clients get their turn.
pub const PANE_READ_BUDGET: usize = 256 * 1024;

/// Bytes queued for a pane's program (keys, pastes, replies) above which
/// lif stops reading from the client typing into it. The queue can exceed it
/// by what one client read encodes to (64 KiB of input from the client), so
/// the hard bound per pane is this plus one read.
pub const PANE_WRITE_HIGH: usize = 64 * 1024;

/// A pane whose program opened a synchronized update (mode 2026) is not
/// drawn in its intermediate state for at most this long.
pub const SYNC_TIMEOUT: Duration = Duration::from_millis(150);

/// After this long without output a pane's engine is asked to compact its
/// scrollback (once, until it outputs again).
pub const COMPACT_AFTER: Duration = Duration::from_secs(2);

/// How the server was started.
#[derive(Clone, Debug)]
pub struct ServerOptions {
    /// The socket the listener is bound to (for `$LIF` and clean-up).
    pub socket: PathBuf,
    /// The start-up lock file next to it, removed on exit.
    pub lock: Option<PathBuf>,
    /// Log to `lif-server-<pid>.log` in the socket's directory.
    pub verbose: bool,
}

/// Commands run once at start-up, before the first client command (the
/// configuration file). The seam the configuration loader fills in.
pub fn load_startup_commands() -> Vec<String> {
    Vec::new()
}

/// Runs the server on an already bound and listening socket until it exits.
/// Returns the process exit status.
pub fn run(listener: UnixListener, opts: ServerOptions) -> i32 {
    if opts.verbose {
        let dir = opts.socket.parent().map(PathBuf::from).unwrap_or_else(|| PathBuf::from("."));
        log::init(&dir.join(format!("lif-server-{}.log", std::process::id())));
    }
    panic::install_hook();
    match server::Server::new(listener, opts) {
        Ok(mut s) => {
            let code = s.run();
            log!("server exiting with status {code}");
            code
        }
        Err(e) => {
            log!("server failed to start: {e}");
            1
        }
    }
}
