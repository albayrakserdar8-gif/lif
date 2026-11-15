//! Shell commands run for the model: `run-shell`, `if-shell` and `#()`
//! in formats.
//!
//! A job is `/bin/sh -c command` in its own process group, as the
//! server's user, with standard input from `/dev/null` and standard
//! output on a non-blocking pipe the event loop reads. It is over when it
//! has been reaped and its output has been read to the end; the model then
//! gets its status and output (`Model::job_done`). Nothing here waits: a
//! command that never ends only keeps its slot.

use std::io::{self, Read};
use std::os::fd::OwnedFd;
use std::os::unix::process::{CommandExt, ExitStatusExt};
use std::path::Path;
use std::process::{Command, ExitStatus, Stdio};

use lif_core::{JobSpec, JobStatus};
use mio::unix::pipe::Receiver;

/// Jobs running at once; more are refused. Each holds a process, a pipe
/// and up to [`MAX_JOB_OUTPUT`] of output.
pub(crate) const MAX_JOBS: usize = 64;

/// Output kept of one job, in bytes; the rest is read and dropped, so a
/// command that prints without end neither blocks nor grows the server.
pub(crate) const MAX_JOB_OUTPUT: usize = 1024 * 1024;

/// Bytes read from one job per loop iteration before panes and clients
/// get their turn.
pub(crate) const JOB_READ_BUDGET: usize = 256 * 1024;

/// `run-shell -d` timers pending at once (the model parks at most as many
/// command lists).
pub(crate) const MAX_TIMERS: usize = 1024;

pub(crate) struct Job {
    /// The model's id for it.
    pub(crate) id: u64,
    pub(crate) pid: u32,
    /// `None` once the output was read to the end.
    pub(crate) rx: Option<Receiver>,
    pub(crate) out: Vec<u8>,
    /// Set when the process was reaped.
    pub(crate) status: Option<JobStatus>,
    /// The read budget ran out with output still waiting.
    pub(crate) read_more: bool,
}

impl Job {
    /// Read what the pipe has, up to the budget. True at the end of the
    /// output (every writer closed it), or on an error that ends it.
    pub(crate) fn read(&mut self, buf: &mut [u8]) -> bool {
        self.read_more = false;
        let Some(rx) = self.rx.as_mut() else { return true };
        let mut total = 0;
        loop {
            if total >= JOB_READ_BUDGET {
                self.read_more = true;
                return false;
            }
            match rx.read(buf) {
                Ok(0) => return true,
                Ok(n) => {
                    total += n;
                    let keep = n.min(MAX_JOB_OUTPUT - self.out.len());
                    self.out.extend_from_slice(&buf[..keep]);
                }
                Err(e) if e.kind() == io::ErrorKind::WouldBlock => return false,
                Err(e) if e.kind() == io::ErrorKind::Interrupted => {}
                Err(_) => return true,
            }
        }
    }

    pub(crate) fn done(&self) -> bool {
        self.rx.is_none() && self.status.is_some()
    }
}

pub(crate) fn status_of(s: ExitStatus) -> JobStatus {
    match (s.code(), s.signal()) {
        (Some(c), _) => JobStatus::Exited(c),
        (None, Some(sig)) => JobStatus::Signaled(sig),
        (None, None) => JobStatus::Exited(1),
    }
}

/// Start `spec` with `env` set (or removed) on top of the server's
/// environment.
pub(crate) fn spawn(spec: &JobSpec, env: &[(String, Option<String>)]) -> io::Result<Job> {
    let (rx, tx) = io::pipe()?;
    let mut c = Command::new("/bin/sh");
    c.arg("-c").arg(&spec.command).stdin(Stdio::null());
    if spec.stderr {
        c.stderr(tx.try_clone()?);
    } else {
        c.stderr(Stdio::null());
    }
    c.stdout(tx);
    // Its own group, so the whole command can be signalled when the
    // server exits.
    c.process_group(0);
    for (k, v) in env {
        match v {
            Some(v) => c.env(k, v),
            None => c.env_remove(k),
        };
    }
    let home = std::env::var_os("HOME").filter(|h| !h.is_empty());
    let cwd = Path::new(&spec.cwd);
    // PWD too, so the shell names the directory as it was given (not
    // with its symlinks resolved), as tmux's jobs see it.
    if !spec.cwd.is_empty() && cwd.is_dir() {
        c.current_dir(cwd).env("PWD", cwd);
    } else if let Some(h) = home.as_deref().map(Path::new).filter(|h| h.is_dir()) {
        c.current_dir(h).env("PWD", h);
    }
    // Spawning also drops the command's copies of the pipe's write end, so
    // the read end sees the end of the output when the job's processes
    // are gone. Dropping the child neither waits nor kills: the server's
    // ChildReaper collects the exit status.
    let pid = c.spawn()?.id();
    drop(c);
    let rx = Receiver::from(OwnedFd::from(rx));
    rx.set_nonblocking(true)?;
    Ok(Job { id: spec.id, pid, rx: Some(rx), out: Vec::new(), status: None, read_more: false })
}
