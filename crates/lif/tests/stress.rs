//! Flooding panes, slow clients, big output, leaks and idle wakeups.
mod common;

use std::io::Write;
use std::time::{Duration, Instant};

use common::*;
use lif_types::Size;

fn big_file(path: &std::path::Path, bytes: usize) -> String {
    let mut f = std::io::BufWriter::new(std::fs::File::create(path).unwrap());
    let mut n = 0usize;
    let mut i = 0u64;
    let mut last = String::new();
    while n < bytes {
        last = format!("line {i:010} the quick brown fox jumps over the lazy dog 0123456789 abcdefghijklmnopqrstuvwxyz\n");
        f.write_all(last.as_bytes()).unwrap();
        n += last.len();
        i += 1;
    }
    last.trim_end().to_string()
}

fn median(v: &mut [Duration]) -> Duration {
    v.sort();
    v[v.len() / 2]
}

#[test]
fn slow_client_does_not_grow_server() {
    let env = Env::new();
    let big = env.home.join("big.txt");
    big_file(&big, 20 << 20);
    env.ok(&["new-session", "-d", "-s", "flood", "-x", "80", "-y", "24"]);
    env.wait_capture("flood", "$");
    env.ok(&["send-keys", "-t", "flood", "yes", "Enter"]);
    let loop_cmd = format!("while :; do cat {}; done", big.display());
    env.ok(&["split-window", "-t", "flood", &loop_cmd]);
    let mut slow = Term::spawn(&env, &["attach", "-t", "flood"], Size::new(80, 25));
    slow.wait_text("[flood]");
    let mut fast = Term::spawn(&env, &["attach", "-t", "flood"], Size::new(80, 25));
    fast.wait_text("[flood]");
    let pid = env.server_pid();
    // From here on `slow` is never read: its terminal fills, the client
    // blocks, its socket fills.
    let warm = Instant::now() + Duration::from_secs(3);
    while Instant::now() < warm {
        fast.pump(Duration::from_millis(10));
    }
    let mut rss = Vec::new();
    let mut lat = Vec::new();
    let mut fast_bytes = Vec::new();
    let end = Instant::now() + Duration::from_secs(5);
    while Instant::now() < end {
        let mut got = 0;
        let tick = Instant::now() + Duration::from_millis(250);
        while Instant::now() < tick {
            got += fast.pump(Duration::from_millis(10));
        }
        fast_bytes.push(got);
        rss.push(rss_kib(pid));
        let t0 = Instant::now();
        assert_eq!(env.ok(&["display-message", "-p", "x"]), "x\n");
        lat.push(t0.elapsed());
    }
    let (lo, hi) = (*rss.iter().min().unwrap(), *rss.iter().max().unwrap());
    let med = median(&mut lat.clone());
    let max = *lat.iter().max().unwrap();
    eprintln!("flood: server rss {lo}..{hi} KiB, CLI latency median {med:?} max {max:?}, fast client bytes per 250 ms {fast_bytes:?}");
    assert!(hi - lo < 4096, "server RSS grew from {lo} to {hi} KiB");
    assert!(fast_bytes.iter().all(|b| *b > 0), "the other client kept getting frames: {fast_bytes:?}");
    assert!(med < Duration::from_millis(50), "CLI latency median {med:?}");
    assert!(max < Duration::from_millis(300), "CLI latency max {max:?}");
    // Stop the flood while the slow client is still stalled.
    env.ok(&["kill-pane", "-t", "flood:0.1"]);
    env.ok(&["send-keys", "-t", "flood:0.0", "C-c"]);
    env.ok(&["send-keys", "-t", "flood:0.0", "clear; echo FINAL''MARK", "Enter"]);
    let cap = env.wait_capture("flood:0.0", "FINALMARK");
    // Resume: the slow client gets the current state, not the backlog.
    let mut got = 0;
    let deadline = Instant::now() + Duration::from_secs(20);
    while !slow.screen().contains("FINALMARK") && Instant::now() < deadline {
        got += slow.pump(Duration::from_millis(20));
    }
    slow.settle(Duration::from_millis(200));
    eprintln!("slow client caught up with {got} bytes");
    assert!(slow.screen().contains("FINALMARK"), "{}", slow.screen());
    assert!(got < 2 << 20, "slow client was sent {got} bytes: a backlog, not a frame");
    let want: Vec<&str> = cap.lines().map(|l| l.trim_end()).filter(|l| !l.is_empty()).collect();
    let have: Vec<String> = slow.lines().iter().map(|l| l.trim_end().to_string()).filter(|l| !l.is_empty()).collect();
    assert_eq!(have[..want.len()], want[..], "slow client screen matches the pane");
}

#[test]
fn cat_100mb_completes() {
    let env = Env::new();
    let big = env.home.join("big.txt");
    let last = big_file(&big, 100 << 20);
    let mut t = Term::spawn(&env, &["new-session", "-s", "cat"], Size::new(120, 40));
    t.wait_text("$");
    t.settle(Duration::from_millis(100));
    let start = Instant::now();
    t.type_str(&format!("cat {}; echo CAT''DONE\r", big.display()));
    t.wait_for("cat to finish", Duration::from_secs(180), |t| t.screen().contains("CATDONE"));
    let took = start.elapsed();
    t.settle(Duration::from_millis(100));
    eprintln!("100 MB cat with an attached client: {took:?}");
    let lines = t.lines();
    let done = lines.iter().position(|l| l.contains("CATDONE")).unwrap();
    assert_eq!(lines[done - 1].trim_end(), last, "{}", t.screen());
}

#[test]
fn attach_detach_cycles_leak_nothing() {
    let env = Env::new();
    env.ok(&["new-session", "-d", "-s", "cyc"]);
    let pid = env.server_pid();
    let cycle = || {
        let mut t = Term::spawn(&env, &["attach", "-t", "cyc"], Size::new(80, 24));
        t.wait_text("[cyc]");
        t.write(b"\x02d");
        assert_eq!(t.wait_exit(), 0);
    };
    for _ in 0..10 {
        cycle();
    }
    std::thread::sleep(Duration::from_millis(200));
    let (fd0, rss0) = (open_fds(pid), rss_kib(pid));
    for _ in 0..200 {
        cycle();
    }
    std::thread::sleep(Duration::from_millis(200));
    let (fd1, rss1) = (open_fds(pid), rss_kib(pid));
    eprintln!("200 attach/detach cycles: fds {fd0} -> {fd1}, rss {rss0} -> {rss1} KiB");
    assert_eq!(fd0, fd1, "server leaked file descriptors");
    assert!(rss1 < rss0 + 1024, "server RSS grew from {rss0} to {rss1} KiB");
}

fn iterations(log: &std::path::Path) -> u64 {
    let text = std::fs::read_to_string(log).unwrap_or_default();
    text.lines()
        .rev()
        .find_map(|l| l.split("iterations=").nth(1)?.split_whitespace().next()?.parse().ok())
        .unwrap_or(0)
}

fn stats(pid: u32, log: &std::path::Path) -> u64 {
    let before = std::fs::read_to_string(log).unwrap_or_default().matches("stats:").count();
    // SAFETY: signalling our own test server.
    unsafe { libc::kill(pid as i32, libc::SIGUSR1) };
    let end = Instant::now() + Duration::from_secs(5);
    while std::fs::read_to_string(log).unwrap_or_default().matches("stats:").count() == before {
        assert!(Instant::now() < end, "no stats in the log");
        std::thread::sleep(Duration::from_millis(10));
    }
    iterations(log)
}

#[test]
fn idle_server_makes_no_wakeups() {
    let env = Env::new();
    let o = env.command().args(["-v", "new-session", "-d", "-s", "idle"]).output().unwrap();
    assert!(o.status.success());
    let pid = env.server_pid();
    let log = env.dir.join(format!("lif-server-{pid}.log"));
    let mut t = Term::spawn(&env, &["attach", "-t", "idle"], Size::new(80, 24));
    t.wait_text("$");
    t.settle(Duration::from_millis(300));
    // Past the one-shot deadlines that follow activity (compaction after
    // 2 s, the window-name recheck after 1 s).
    std::thread::sleep(Duration::from_millis(2500));
    let a = stats(pid, &log);
    std::thread::sleep(Duration::from_secs(5));
    let b = stats(pid, &log);
    eprintln!("idle: {} loop iterations in 5 s (one is the SIGUSR1 itself)", b - a);
    // One for the signal, at most one more for the status clock's minute tick.
    if b - a > 2 {
        let text = std::fs::read_to_string(&log).unwrap_or_default();
        let tail: Vec<&str> = text.lines().rev().take(30).collect();
        panic!("idle server iterated {} times; log tail (newest first):\n{}", b - a, tail.join("\n"));
    }
    t.pump(Duration::from_millis(10));
    assert!(t.exit.is_none());
}
