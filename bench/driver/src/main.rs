//! lifbench-driver: the timing-sensitive half of the lif benchmark harness.
//!
//! Subcommands:
//!   hold [--cols N] [--rows N] [--capture FILE] [--no-answer] -- CMD...
//!                                          client on a PTY, commands on stdin
//!   attach-detach [--cols N] [--rows N] [--keys HEX] [--quiet-ms N] [--timeout-ms N] -- CMD...
//!   rusage PID...                          JSON line per pid
//!   waitfile PATH [TIMEOUT_MS]             exit 0 once PATH exists
//!   escprobe READY_MARKER [MARK_CHAR]      run inside a pane

mod answer;
mod probe;
mod pty;
mod rusage;
mod scan;
mod session;

use std::time::{Duration, Instant};

fn usage() -> i32 {
    eprintln!(
        "usage: lifbench-driver hold [--cols N] [--rows N] [--capture FILE] [--no-answer] -- CMD...\n\
         \x20      lifbench-driver attach-detach [--cols N] [--rows N] [--keys HEX] [--quiet-ms N] [--timeout-ms N] -- CMD...\n\
         \x20      lifbench-driver rusage PID...\n\
         \x20      lifbench-driver waitfile PATH [TIMEOUT_MS]\n\
         \x20      lifbench-driver escprobe READY_MARKER [MARK_CHAR]"
    );
    2
}

struct Opts {
    cols: u16,
    rows: u16,
    keys: Vec<u8>,
    timeout_ms: u64,
    capture: Option<String>,
    answer: bool,
    quiet_ms: u64,
    argv: Vec<String>,
}

fn parse_opts(args: &[String]) -> Option<Opts> {
    let mut o = Opts { cols: 200, rows: 50, keys: Vec::new(), timeout_ms: 10_000, capture: None, answer: true, quiet_ms: 0, argv: Vec::new() };
    let mut i = 0;
    while i < args.len() {
        let v = args.get(i + 1);
        match args[i].as_str() {
            "--" => {
                o.argv = args[i + 1..].to_vec();
                break;
            }
            "--cols" => o.cols = v?.parse().ok()?,
            "--rows" => o.rows = v?.parse().ok()?,
            "--keys" => o.keys = session::unhex(v?)?,
            "--timeout-ms" => o.timeout_ms = v?.parse().ok()?,
            "--quiet-ms" => o.quiet_ms = v?.parse().ok()?,
            "--capture" => o.capture = Some(v?.clone()),
            "--no-answer" => {
                o.answer = false;
                i += 1;
                continue;
            }
            _ => return None,
        }
        i += 2;
    }
    if o.argv.is_empty() { None } else { Some(o) }
}

/// Starts a client, waits for its first output and then until it has been
/// silent for `quiet_ms` (it has drawn the session), sends the detach keys
/// and waits for it to exit. Exits 0 only if the client detached by itself.
fn attach_detach(o: &Opts) -> i32 {
    let mut p = match pty::Pty::spawn(&o.argv, o.cols, o.rows) {
        Ok(p) => p,
        Err(e) => {
            eprintln!("spawn: {e}");
            return 1;
        }
    };
    let t0 = Instant::now();
    let deadline = t0 + Duration::from_millis(o.timeout_ms);
    let mut buf = vec![0u8; 64 * 1024];
    let mut sent = false;
    let verbose = std::env::var_os("LIFBENCH_VERBOSE").is_some();
    let mut answerer = answer::Answerer::new(o.cols, o.rows);
    let mut replies = Vec::new();
    let mut total = 0usize;
    let mut first: Option<Instant> = None;
    let mut last = t0;
    let quiet = Duration::from_millis(o.quiet_ms);
    loop {
        let left = deadline.saturating_duration_since(Instant::now());
        if left.is_zero() {
            eprintln!("attach-detach: timeout (keys sent: {sent})");
            p.shutdown(Duration::from_millis(200));
            return 1;
        }
        if !sent && first.is_some() && last.elapsed() >= quiet {
            if verbose {
                eprintln!("{:8.3} ms keys sent after {total} bytes", t0.elapsed().as_secs_f64() * 1e3);
            }
            if p.write_all(&o.keys).is_err() {
                return 1;
            }
            sent = true;
        }
        let step = if sent || first.is_none() { 100 } else { 1 };
        if pty::wait_readable(p.master, left.as_millis().min(step) as i32) {
            let mut eof = false;
            loop {
                match p.read(&mut buf) {
                    pty::Read::Data(n) => {
                        total += n;
                        answerer.feed(&buf[..n], &mut replies);
                        if !replies.is_empty() {
                            let _ = p.write_all(&replies);
                            replies.clear();
                        }
                        if first.is_none() && verbose {
                            eprintln!("{:8.3} ms first output", t0.elapsed().as_secs_f64() * 1e3);
                        }
                        first.get_or_insert_with(Instant::now);
                        last = Instant::now();
                    }
                    pty::Read::WouldBlock => break,
                    pty::Read::Eof => {
                        eof = true;
                        break;
                    }
                }
            }
            if eof {
                if verbose {
                    eprintln!("{:8.3} ms hangup, {total} bytes", t0.elapsed().as_secs_f64() * 1e3);
                }
                break;
            }
        }
        if p.try_wait().is_some() {
            break;
        }
    }
    if !sent {
        eprintln!("attach-detach: client exited before drawing anything");
        return 1;
    }
    // Wait for the exit status; the PTY hung up, so this is short.
    let t = Instant::now();
    while p.try_wait().is_none() && t.elapsed() < Duration::from_secs(5) {
        std::thread::sleep(Duration::from_micros(200));
    }
    if verbose {
        eprintln!("{:8.3} ms exited", t0.elapsed().as_secs_f64() * 1e3);
    }
    match p.try_wait() {
        Some(st) if libc::WIFEXITED(st) && libc::WEXITSTATUS(st) == 0 => 0,
        Some(st) => {
            eprintln!("attach-detach: client exit status {st}");
            1
        }
        None => {
            p.shutdown(Duration::ZERO);
            1
        }
    }
}

fn waitfile(path: &str, timeout_ms: u64) -> i32 {
    let deadline = Instant::now() + Duration::from_millis(timeout_ms);
    let p = std::path::Path::new(path);
    while Instant::now() < deadline {
        if p.exists() {
            return 0;
        }
        std::thread::sleep(Duration::from_micros(200));
    }
    1
}

fn main() {
    let args: Vec<String> = std::env::args().skip(1).collect();
    let code = match args.first().map(String::as_str) {
        Some("hold") => match parse_opts(&args[1..]) {
            Some(o) => session::hold(&o.argv, o.cols, o.rows, o.capture.as_deref(), o.answer),
            None => usage(),
        },
        Some("attach-detach") => match parse_opts(&args[1..]) {
            Some(o) => attach_detach(&o),
            None => usage(),
        },
        Some("rusage") if args.len() > 1 => {
            for a in &args[1..] {
                let pid: i32 = a.parse().unwrap_or(-1);
                println!("{}", rusage::to_json(pid, rusage::usage(pid)));
            }
            0
        }
        Some("waitfile") if args.len() >= 2 => {
            waitfile(&args[1], args.get(2).and_then(|s| s.parse().ok()).unwrap_or(10_000))
        }
        Some("escprobe") if args.len() >= 2 => {
            let mark = args.get(2).and_then(|s| s.bytes().next()).unwrap_or(b'~');
            probe::escprobe(&args[1], mark)
        }
        _ => usage(),
    };
    std::process::exit(code);
}
