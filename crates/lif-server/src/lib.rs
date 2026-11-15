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
mod pipe;
mod prompt;
mod server;
mod sys;

use std::os::unix::net::UnixListener;
use std::path::PathBuf;
use std::time::Duration;

pub use mode::{ModeAction, ModeCtx, PaneMode};

/// Shortest time between two frames sent to one client for output nobody
/// typed for: output arriving faster is coalesced (8 ms, 125 Hz, is above
/// the refresh rate of nearly every display). Pacing never applies to the
/// echo of input: see [`ECHO_WINDOW`].
pub const MIN_FRAME_INTERVAL: Duration = Duration::from_millis(8);

/// Interactive echo. The first output a pane produces after a client's
/// input reached it is drawn in the same loop iteration, whatever the
/// pacing, and only that pane is refreshed for it (a pane flooding output
/// next to it keeps its paced frames and cannot delay the echo). Output in
/// the following `ECHO_WINDOW` (a shell redrawing its line after the echo,
/// a program answering the key in several writes) is drawn at most
/// [`ECHO_INTERVAL`] apart, until [`ECHO_WINDOW_BYTES`] of output show
/// that the key started a flood, which is then paced like any other.
pub const ECHO_WINDOW: Duration = Duration::from_millis(20);

/// Shortest time between two frames of a pane inside its [`ECHO_WINDOW`].
pub const ECHO_INTERVAL: Duration = Duration::from_millis(1);

/// Output after a key beyond which the pane counts as flooding again.
pub const ECHO_WINDOW_BYTES: usize = 64 * 1024;

/// Bytes read from one PTY per loop iteration before other panes and the
/// clients get their turn.
pub const PANE_READ_BUDGET: usize = 256 * 1024;

/// Hard bound of the bytes queued for one pane's program: keys, pastes,
/// `send-keys`, terminal replies, reports, all of it (`Rt::write_pane`).
/// A program that does not read its input stops being sent more once this
/// much waits: replies and reports are dropped (as a real terminal drops
/// them), commands fail, typed and pasted input is dropped with a message.
/// The client typing is never stopped: its other keys (prefix, detach,
/// window switching) and resizes keep working. 1 MiB holds any paste a
/// person makes into a program that reads slowly.
pub const PANE_INPUT_MAX: usize = 1024 * 1024;

/// Part of [`PANE_INPUT_MAX`] only the end of a bracketed paste may use,
/// so a paste that was started is always closed.
pub const PANE_INPUT_RESERVE: usize = 64;

/// What is read from a pane's PTY after its process exited, at most,
/// before it is closed.
pub const EXIT_DRAIN: usize = 4 * PANE_READ_BUDGET;

/// How long a pane's process group has to exit after its hangup (SIGHUP and
/// the PTY closing) before it gets SIGKILL. A hangup alone is not enough:
/// a program may ignore it, and a shell whose SIGHUP handler is not
/// async-signal-safe (macOS's /bin/sh, bash 3.2, when the signal lands
/// while it writes) hangs in the handler and would outlive the server for
/// good. Long enough for a HUP trap that cleans up to run.
pub const PANE_KILL_GRACE: Duration = Duration::from_millis(500);

/// Descriptors a new pane leaves free for clients to connect with.
pub const FD_RESERVE: usize = 32;

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
    /// The configuration file given with `lif -f`; `None` means the
    /// default location (lif-config's `default_path`).
    pub config: Option<PathBuf>,
}

/// Commands run once at start-up, before the first client command: reading
/// the configuration file. A file named with `-f` must exist; the default
/// one may be missing. Problems never stop the server (see `server`).
pub fn load_startup_commands(opts: &ServerOptions) -> Vec<String> {
    let (path, quiet) = match &opts.config {
        Some(p) => (p.clone(), ""),
        None => match lif_config::default_path() {
            Some(p) => (p, "-q "),
            None => return Vec::new(),
        },
    };
    vec![format!("source-file {quiet}{}", lif_core::escape_arg(&path.to_string_lossy()))]
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
