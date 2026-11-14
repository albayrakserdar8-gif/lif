//! Where an idle pane's memory goes. Measurements, not assertions: run with
//! `cargo test -p lif-vt --release --test memory -- --ignored --nocapture`.
#![cfg(target_os = "macos")]

use lif_types::{PaneFrame, PixelSize, Size};
use lif_vt::{GhosttyVt, VtEngine, VtOptions};

/// (phys_footprint, resident) of this process in bytes.
fn mem() -> (u64, u64) {
    // SAFETY: rusage_info_v2 is plain data that proc_pid_rusage fills.
    let mut ri: libc::rusage_info_v2 = unsafe { std::mem::zeroed() };
    // SAFETY: valid pointer to a v2 struct, as the flavor says.
    let r = unsafe { libc::proc_pid_rusage(libc::getpid(), libc::RUSAGE_INFO_V2, (&mut ri as *mut libc::rusage_info_v2).cast()) };
    assert_eq!(r, 0);
    (ri.ri_phys_footprint, ri.ri_resident_size)
}

fn step(label: &str, n: usize, before: &mut (u64, u64)) {
    let now = mem();
    eprintln!(
        "{label:<44} footprint {:>7.1} KiB/pane  rss {:>7.1} KiB/pane",
        (now.0 as f64 - before.0 as f64) / 1024.0 / n as f64,
        (now.1 as f64 - before.1 as f64) / 1024.0 / n as f64
    );
    *before = now;
}

fn run(size: Size) {
    const N: usize = 64;
    eprintln!("--- {N} engines at {}x{}", size.cols, size.rows);
    let mut m = mem();
    let mut vts: Vec<GhosttyVt> = Vec::new();
    for _ in 0..N {
        vts.push(GhosttyVt::new(&VtOptions { size, pixels: PixelSize::default(), ..VtOptions::default() }).unwrap());
    }
    step("new", N, &mut m);
    for v in &mut vts {
        v.feed(b"$ ");
    }
    step("feed a prompt", N, &mut m);
    let mut frames: Vec<PaneFrame> = vts.iter().map(|_| PaneFrame::new(size)).collect();
    step("PaneFrame::new", N, &mut m);
    for (v, f) in vts.iter_mut().zip(&mut frames) {
        v.refresh(f);
    }
    step("first refresh", N, &mut m);
    for v in &mut vts {
        v.feed(b"ls\r\nfile1 file2\r\n$ ");
    }
    for (v, f) in vts.iter_mut().zip(&mut frames) {
        v.refresh(f);
    }
    step("some output + refresh", N, &mut m);
    for v in &mut vts {
        v.compact();
    }
    step("compact", N, &mut m);
    drop(frames);
    step("drop frames", N, &mut m);
    let usage: usize = vts.iter().map(|v| v.memory().resident_bytes).sum();
    eprintln!("engine-reported resident bytes: {:.1} KiB/pane", usage as f64 / 1024.0 / N as f64);
    drop(vts);
    step("drop engines", N, &mut m);
}

/// What a tiled pane goes through: born large, shrunk step by step by the
/// splits after it, refreshed at every size.
fn shrink_history(release: bool) {
    const N: usize = 64;
    eprintln!("--- {N} engines born at 200x49, shrunk to 28x6 (refresh at each size; compact + release_caches: {release})");
    let mut m = mem();
    let sizes = [(200, 49), (100, 49), (100, 24), (66, 24), (50, 24), (50, 12), (40, 12), (33, 12), (33, 8), (28, 8), (28, 6)];
    let mut vts = Vec::new();
    let mut frames = Vec::new();
    for _ in 0..N {
        let mut v = GhosttyVt::new(&VtOptions { size: Size::new(200, 49), pixels: PixelSize::default(), ..VtOptions::default() }).unwrap();
        v.feed(b"$ ");
        let mut f = PaneFrame::new(Size::new(200, 49));
        for &(c, r) in &sizes {
            v.resize(Size::new(c, r), PixelSize::default());
            v.refresh(&mut f);
        }
        if release {
            // What the server does: compact once the pane is quiet after a
            // resize, and drop the caches of panes nobody sees.
            v.compact();
            v.release_caches();
            f = PaneFrame::new(Size::new(28, 6));
            v.refresh(&mut f);
        }
        vts.push(v);
        frames.push(f);
    }
    step("born large, now 28x6", N, &mut m);
    for v in &mut vts {
        v.compact();
    }
    step("compact", N, &mut m);
    drop(frames);
    drop(vts);
    step("drop all", N, &mut m);
}

/// Scrollback content of several kinds, at 200 columns, budget unlimited:
/// resident bytes as filled, and after compaction.
#[test]
#[ignore]
fn scrollback_compression() {
    type Line = fn(usize) -> String;
    let kinds: [(&str, Line); 5] = [
        ("shell (ls -l)", |i| format!("-rw-r--r--  1 user  staff  {:>8} Nov 15 00:{:02} file_{i:05}.txt\r\n", i * 37 % 100000, i % 60)),
        ("log lines", |i| format!("2026-11-15T00:{:02}:{:02}.{:03}Z INFO  server::conn{{id={i}}}: request handled path=/api/v1/items/{} status=200 took={}us\r\n", i / 60 % 60, i % 60, i % 1000, i * 7 % 9973, i * 13 % 5000)),
        ("colored build", |i| format!("\x1b[1m\x1b[32m   Compiling\x1b[0m crate-{i} v0.{}.{} (/path/to/workspace/crates/crate-{i})\r\n\x1b[33mwarning\x1b[0m\x1b[1m: unused variable: `x{i}`\x1b[0m\r\n", i % 10, i % 7)),
        ("random ASCII 80-120", |i| {
            let mut s = String::new();
            let mut x = (i as u64).wrapping_mul(6364136223846793005).wrapping_add(1442695040888963407);
            let n = 80 + (x % 41) as usize;
            for _ in 0..n {
                x = x.wrapping_mul(6364136223846793005).wrapping_add(1442695040888963407);
                s.push((b' ' + ((x >> 33) % 95) as u8) as char);
            }
            s + "\r\n"
        }),
        ("seq", |i| format!("{i}\r\n")),
    ];
    for (name, line) in kinds {
        const LINES: usize = 20_000;
        let mut m = mem();
        let mut v = GhosttyVt::new(&VtOptions { size: Size::new(200, 50), scrollback_bytes: usize::MAX, ..VtOptions::default() }).unwrap();
        let mut bytes = 0;
        for i in 0..LINES {
            let l = line(i);
            bytes += l.len();
            v.feed(l.as_bytes());
        }
        let filled = mem();
        let t = std::time::Instant::now();
        let mut calls = 0;
        let mut last = v.memory();
        loop {
            v.compact();
            calls += 1;
            let now = v.memory();
            if now == last || calls > 10_000 {
                break;
            }
            last = now;
        }
        let took = t.elapsed();
        let done = mem();
        let mm = v.memory();
        eprintln!(
            "{name:<20} {LINES} lines ({:.1} MB input, {} rows): resident {:.0} KiB = {:.0} B/line; compacted in {calls} calls / {took:?}: engine {:.0} KiB ({} pages compressed, {:.0} KiB of it), process footprint {:.0} -> {:.0} KiB, rss {:.0} -> {:.0} KiB",
            bytes as f64 / 1e6,
            v.scrollback_rows(),
            (filled.0 - m.0) as f64 / 1024.0,
            (filled.0 - m.0) as f64 / LINES as f64,
            mm.resident_bytes as f64 / 1024.0,
            mm.compressed_pages,
            mm.compressed_bytes as f64 / 1024.0,
            (filled.0 - m.0) as f64 / 1024.0,
            (done.0 - m.0) as f64 / 1024.0,
            (filled.1 - m.1) as f64 / 1024.0,
            (done.1 - m.1) as f64 / 1024.0,
        );
        drop(v);
        m = mem();
        let _ = m;
    }
}

unsafe extern "C" {
    fn malloc_zone_pressure_relief(zone: *mut libc::c_void, goal: libc::size_t) -> libc::size_t;
}

/// Whether macOS malloc gives freed small blocks back when asked.
#[test]
#[ignore]
fn malloc_keeps_freed_memory() {
    let mut m = mem();
    let mut big: Vec<Vec<u8>> = Vec::new();
    let mut small: Vec<Vec<u8>> = Vec::new();
    for i in 0..2000 {
        big.push(vec![i as u8; 14 * 1024]);
        small.push(vec![i as u8; 1024]);
    }
    step("2000 x (14 KiB + 1 KiB)", 1, &mut m);
    drop(big);
    step("free the 14 KiB blocks", 1, &mut m);
    // SAFETY: null zone = all zones.
    let r = unsafe { malloc_zone_pressure_relief(std::ptr::null_mut(), 0) };
    step(&format!("pressure relief ({r} bytes)"), 1, &mut m);
    drop(small);
    step("free the rest", 1, &mut m);
    // SAFETY: as above.
    let r = unsafe { malloc_zone_pressure_relief(std::ptr::null_mut(), 0) };
    step(&format!("pressure relief ({r} bytes)"), 1, &mut m);
}

#[test]
#[ignore]
fn idle_pane_memory() {
    shrink_history(false);
    shrink_history(true);
    run(Size::new(28, 6));
    run(Size::new(80, 24));
    run(Size::new(200, 50));
}
