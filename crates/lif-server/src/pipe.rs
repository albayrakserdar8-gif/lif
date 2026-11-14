//! `copy-pipe`: copied text written to a command's standard input.
//!
//! The command runs as `/bin/sh -c command` (as tmux runs it), as the
//! server's user, with output discarded. Its stdin is a non-blocking pipe
//! the event loop writes when it is writable, so a command that reads
//! slowly (or never) cannot stall the server; the reaper collects it.

use std::io::{self, Write};
use std::path::Path;
use std::process::{Command, Stdio};

use mio::unix::pipe::Sender;

/// Commands still being written to at once; more are refused. Each holds
/// its copy until the command took it, so this bounds what commands that
/// never read can keep in memory to this many copies.
pub(crate) const MAX_PIPES: usize = 8;

pub(crate) struct PipeJob {
    pub(crate) tx: Sender,
    data: Vec<u8>,
    off: usize,
}

impl PipeJob {
    /// Write what the pipe takes now. `Ok(true)`: all of it (drop the job
    /// to close the pipe, which the command reads as end of input).
    pub(crate) fn write(&mut self) -> io::Result<bool> {
        while self.off < self.data.len() {
            match self.tx.write(&self.data[self.off..]) {
                Ok(0) => return Err(io::ErrorKind::WriteZero.into()),
                Ok(n) => self.off += n,
                Err(e) if e.kind() == io::ErrorKind::WouldBlock => return Ok(false),
                Err(e) if e.kind() == io::ErrorKind::Interrupted => {}
                Err(e) => return Err(e),
            }
        }
        Ok(true)
    }
}

/// Start `command` with a pipe on its stdin; returns its pid and the job
/// that feeds it `data`.
pub(crate) fn spawn(command: &str, cwd: Option<&Path>, data: Vec<u8>) -> io::Result<(u32, PipeJob)> {
    let mut c = Command::new("/bin/sh");
    c.arg("-c").arg(command).stdin(Stdio::piped()).stdout(Stdio::null()).stderr(Stdio::null());
    // Not where the server was started (usually /): where the pane is.
    if let Some(dir) = cwd.filter(|d| d.is_dir()) {
        c.current_dir(dir);
    }
    let mut child = c.spawn()?;
    let pid = child.id();
    let stdin = child.stdin.take().ok_or_else(|| io::Error::other("no stdin pipe"))?;
    let tx = Sender::from(stdin);
    tx.set_nonblocking(true)?;
    // Dropping `child` neither waits nor kills: the server's ChildReaper
    // collects the exit status.
    drop(child);
    Ok((pid, PipeJob { tx, data, off: 0 }))
}
