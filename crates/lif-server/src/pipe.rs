//! `copy-pipe` and `pipe-pane`: text written to a command's standard
//! input (the copied text; a pane's output as it arrives), and for
//! `pipe-pane -I` a command's output written to a pane as its input.
//!
//! The command runs as `/bin/sh -c command` (as tmux runs it), as the
//! server's user. Its stdin is a non-blocking pipe the event loop writes
//! when it is writable, so a command that reads slowly (or never) cannot
//! stall the server; the reaper collects it. Its output is discarded,
//! except with `-I`: then the event loop reads it only while the pane's
//! input queue has room, so a program that does not read its input holds
//! up the command (in the kernel's pipe buffer), not the server.

use std::io::{self, Write};
use std::path::Path;
use std::process::{Command, Stdio};

use mio::Registry;
use mio::unix::pipe::{Receiver, Sender};

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

/// Output of a pane waiting for its `pipe-pane` command to read it, at
/// most; what arrives beyond that is not piped (the pane itself is never
/// held up by a command that reads slowly).
pub(crate) const PANE_PIPE_MAX: usize = 1024 * 1024;

/// A `pipe-pane` command and what it has not read yet.
pub(crate) struct PanePipe {
    pub(crate) pid: u32,
    /// Its standard input, which gets the pane's output (`-O`, the default).
    pub(crate) tx: Option<Sender>,
    /// Its standard output, which goes to the pane as input (`-I`).
    pub(crate) rx: Option<Receiver>,
    /// `rx` had more to read when the pane's input queue was full: read
    /// it when the queue has room (the readiness edge is not repeated).
    pub(crate) rx_more: bool,
    queue: Vec<u8>,
    off: usize,
    /// Bytes left out because the queue was full.
    pub(crate) dropped: u64,
    /// The command closed its input (or writing failed), or it exited:
    /// the pane's output goes no further.
    pub(crate) dead: bool,
}

impl PanePipe {
    /// Both directions are over (what the command printed has all been
    /// read): the pipe can go.
    pub(crate) fn finished(&self) -> bool {
        (self.dead || self.tx.is_none()) && self.rx.is_none()
    }

    /// Stops polling the pipes; dropping `self` then closes them.
    pub(crate) fn deregister(&mut self, registry: &Registry) {
        if let Some(tx) = self.tx.as_mut() {
            let _ = registry.deregister(tx);
        }
        if let Some(rx) = self.rx.as_mut() {
            let _ = registry.deregister(rx);
        }
    }

    /// More output of the pane: queue it and write what the pipe takes.
    pub(crate) fn push(&mut self, bytes: &[u8]) {
        if self.dead || self.tx.is_none() {
            return;
        }
        // Written data leaves the front; compact when it is all gone.
        if self.off == self.queue.len() {
            self.queue.clear();
            self.off = 0;
        }
        let room = PANE_PIPE_MAX.saturating_sub(self.queue.len() - self.off);
        let take = bytes.len().min(room);
        self.dropped += (bytes.len() - take) as u64;
        self.queue.extend_from_slice(&bytes[..take]);
        self.flush();
    }

    /// Write what the pipe takes now.
    pub(crate) fn flush(&mut self) {
        let Some(tx) = self.tx.as_mut() else { return };
        while self.off < self.queue.len() {
            match tx.write(&self.queue[self.off..]) {
                Ok(0) => {
                    self.dead = true;
                    return;
                }
                Ok(n) => self.off += n,
                Err(e) if e.kind() == io::ErrorKind::WouldBlock => {
                    // Keep the queue from growing by what was written.
                    if self.off > PANE_PIPE_MAX / 2 {
                        self.queue.drain(..self.off);
                        self.off = 0;
                    }
                    return;
                }
                Err(e) if e.kind() == io::ErrorKind::Interrupted => {}
                Err(_) => {
                    self.dead = true;
                    return;
                }
            }
        }
    }
}

/// Start `command` for `pipe-pane`: with `output` its stdin is a pipe
/// the pane's output is written to, with `input` its stdout is a pipe
/// read into the pane.
pub(crate) fn spawn_pane_pipe(command: &str, cwd: Option<&Path>, input: bool, output: bool) -> io::Result<PanePipe> {
    let piped = |on: bool| if on { Stdio::piped() } else { Stdio::null() };
    let mut c = Command::new("/bin/sh");
    c.arg("-c").arg(command).stdin(piped(output)).stdout(piped(input)).stderr(Stdio::null());
    if let Some(dir) = cwd.filter(|d| d.is_dir()) {
        c.current_dir(dir);
    }
    let mut child = c.spawn()?;
    let pid = child.id();
    let tx = child.stdin.take().map(Sender::from);
    let rx = child.stdout.take().map(Receiver::from);
    if let Some(tx) = tx.as_ref() {
        tx.set_nonblocking(true)?;
    }
    if let Some(rx) = rx.as_ref() {
        rx.set_nonblocking(true)?;
    }
    // As in spawn: the reaper collects the exit status.
    drop(child);
    Ok(PanePipe { pid, tx, rx, rx_more: false, queue: Vec::new(), off: 0, dropped: 0, dead: false })
}
