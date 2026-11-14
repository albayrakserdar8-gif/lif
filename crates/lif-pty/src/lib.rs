//! PTYs and child processes for the lif server.
//!
//! - [`Pty::spawn`] opens a PTY pair and starts a child on the slave side.
//!   The master is non-blocking and close-on-exec, ready to be registered
//!   with mio through [`std::os::fd::AsRawFd`].
//! - [`ChildReaper`] collects exit statuses without threads: a SIGCHLD
//!   handler pokes a self-pipe the event loop watches, and
//!   [`ChildReaper::reap`] calls `waitpid(pid, WNOHANG)` for the pids it
//!   was told about, never `waitpid(-1)`.
//!
//! Spawning works with raw `posix_openpt` / `fork` / `setsid` /
//! `TIOCSCTTY` / `execve` so every step between fork and exec is under our
//! control and async-signal-safe: all strings, the environment and the
//! candidate program paths are built before `fork`.

mod pty;
mod reaper;
mod spawn;
mod sys;

pub use pty::{Pty, ReadOutcome};
pub use reaper::ChildReaper;
pub use spawn::SpawnOptions;
pub use std::process::ExitStatus;
