#![allow(dead_code)]

use std::os::fd::AsRawFd;
use std::process::ExitStatus;
use std::time::{Duration, Instant};

use lif_pty::{ChildReaper, Pty, ReadOutcome};

pub const TIMEOUT: Duration = Duration::from_secs(10);

/// Waits until `fd` is readable (or hung up) or the deadline passes.
pub fn wait_readable(fd: i32, deadline: Instant) -> bool {
    let now = Instant::now();
    if now >= deadline {
        return false;
    }
    let ms = (deadline - now).as_millis().min(i32::MAX as u128) as i32;
    let mut pfd = libc::pollfd {
        fd,
        events: libc::POLLIN,
        revents: 0,
    };
    // SAFETY: one valid pollfd.
    let r = unsafe { libc::poll(&mut pfd, 1, ms.max(1)) };
    r > 0
}

/// Reads until `needle` appears in the output. Panics on timeout or EOF.
pub fn read_until(pty: &mut Pty, needle: &[u8]) -> Vec<u8> {
    let deadline = Instant::now() + TIMEOUT;
    let mut out = Vec::new();
    let mut buf = [0u8; 4096];
    loop {
        if out.windows(needle.len()).any(|w| w == needle) {
            return out;
        }
        match pty.read(&mut buf).unwrap() {
            ReadOutcome::Data(n) => out.extend_from_slice(&buf[..n]),
            ReadOutcome::WouldBlock => {
                assert!(
                    wait_readable(pty.as_raw_fd(), deadline),
                    "timed out waiting for {:?}; got {:?}",
                    String::from_utf8_lossy(needle),
                    String::from_utf8_lossy(&out)
                );
            }
            ReadOutcome::Eof => panic!(
                "EOF before {:?}; got {:?}",
                String::from_utf8_lossy(needle),
                String::from_utf8_lossy(&out)
            ),
        }
    }
}

/// Reads everything until EOF. Panics on timeout.
pub fn read_to_eof(pty: &mut Pty) -> Vec<u8> {
    let deadline = Instant::now() + TIMEOUT;
    let mut out = Vec::new();
    let mut buf = [0u8; 4096];
    loop {
        match pty.read(&mut buf).unwrap() {
            ReadOutcome::Data(n) => out.extend_from_slice(&buf[..n]),
            ReadOutcome::WouldBlock => {
                assert!(
                    wait_readable(pty.as_raw_fd(), deadline),
                    "timed out waiting for EOF"
                );
            }
            ReadOutcome::Eof => return out,
        }
    }
}

/// Waits for `pid` to be reaped. Panics on timeout.
pub fn wait_exit(reaper: &mut ChildReaper, pid: u32) -> ExitStatus {
    let deadline = Instant::now() + TIMEOUT;
    loop {
        if let Some((_, status)) = reaper.reap().into_iter().find(|(p, _)| *p == pid) {
            return status;
        }
        assert!(
            wait_readable(reaper.as_raw_fd(), deadline),
            "timed out waiting for pid {pid}"
        );
    }
}

pub fn text(bytes: &[u8]) -> String {
    String::from_utf8_lossy(bytes).replace('\r', "")
}

pub fn unique_dir(name: &str) -> std::path::PathBuf {
    let dir = std::path::Path::new(env!("CARGO_TARGET_TMPDIR"))
        .join(format!("{name}-{}", std::process::id()));
    std::fs::create_dir_all(&dir).unwrap();
    dir.canonicalize().unwrap()
}
