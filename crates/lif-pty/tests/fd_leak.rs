//! Own test binary: counting open fds needs no other test running beside it.
mod common;

use std::os::fd::AsRawFd;
use std::time::Instant;

use common::*;
use lif_pty::{ChildReaper, Pty, SpawnOptions};

fn open_fds() -> usize {
    let dir = if cfg!(target_os = "linux") {
        "/proc/self/fd"
    } else {
        "/dev/fd"
    };
    std::fs::read_dir(dir).unwrap().count()
}

/// Reaps until nothing is watched. Polls with a short timeout because the
/// SIGCHLD byte may already have been drained by an earlier reap().
fn reap_all(reaper: &mut ChildReaper) -> usize {
    let deadline = Instant::now() + TIMEOUT;
    let mut n = 0;
    loop {
        n += reaper.reap().len();
        if reaper.is_empty() {
            return n;
        }
        assert!(
            Instant::now() < deadline,
            "{} children never exited",
            reaper.len()
        );
        let mut pfd = libc::pollfd {
            fd: reaper.as_raw_fd(),
            events: libc::POLLIN,
            revents: 0,
        };
        // SAFETY: one valid pollfd.
        unsafe { libc::poll(&mut pfd, 1, 50) };
    }
}

#[test]
fn hundred_ptys_open_and_close_without_leaks() {
    let mut reaper = ChildReaper::new().unwrap();
    // Warm up anything opened lazily on first use.
    let warm = Pty::spawn(&SpawnOptions::command(["true"])).unwrap();
    reaper.watch(warm.pid());
    drop(warm);
    reap_all(&mut reaper);

    let before = open_fds();
    let start = Instant::now();
    let mut ptys = Vec::new();
    for _ in 0..100 {
        let pty = Pty::spawn(&SpawnOptions::command(["cat"])).unwrap();
        reaper.watch(pty.pid());
        ptys.push(pty);
    }
    let spawned = start.elapsed();
    assert_eq!(open_fds(), before + 100);
    drop(ptys);
    assert_eq!(reap_all(&mut reaper), 100);
    assert_eq!(open_fds(), before);
    eprintln!(
        "spawned 100 PTYs in {:?} ({:?} each)",
        spawned,
        spawned / 100
    );
}
