//! `hold`: keeps a client on a PTY, discards its output as fast as it
//! arrives, and executes timing commands read line by line from stdin.
//! Every command answers with exactly one JSON line on stdout.

use crate::answer::Answerer;
use crate::pty::{Pty, Read, wait_readable};
use crate::scan::Scanner;
use std::io::Write;
use std::path::Path;
use std::time::{Duration, Instant};

pub struct Session {
    pty: Pty,
    buf: Vec<u8>,
    spawned: Instant,
    /// Bytes received from the client since spawn.
    bytes: u64,
    last_out: Instant,
    eof: bool,
    scan: Option<(Scanner, u8)>,
    seen: Option<Instant>,
    rng: u64,
    answer: Option<Answerer>,
    replies: Vec<u8>,
    /// Debug copy of the first bytes of client output.
    capture: Option<(std::fs::File, usize)>,
}

fn now() -> Instant {
    Instant::now()
}

fn us(d: Duration) -> f64 {
    d.as_secs_f64() * 1e6
}

impl Session {
    pub fn new(argv: &[String], cols: u16, rows: u16) -> std::io::Result<Session> {
        let pty = Pty::spawn(argv, cols, rows)?;
        let t = now();
        Ok(Session {
            pty,
            buf: vec![0; 64 * 1024],
            spawned: t,
            bytes: 0,
            last_out: t,
            eof: false,
            scan: None,
            seen: None,
            rng: 0x9e37_79b9_7f4a_7c15,
            capture: None,
            answer: Some(Answerer::new(cols, rows)),
            replies: Vec::new(),
        })
    }

    pub fn master(&self) -> i32 {
        self.pty.master
    }

    pub fn pid(&self) -> i32 {
        self.pty.pid
    }

    fn rand(&mut self) -> u64 {
        // xorshift64: gaps only need to be decorrelated from frame timers.
        self.rng ^= self.rng << 13;
        self.rng ^= self.rng >> 7;
        self.rng ^= self.rng << 17;
        self.rng
    }

    /// Reads everything available without blocking.
    pub fn drain(&mut self) {
        if self.eof {
            return;
        }
        loop {
            match self.pty.read(&mut self.buf) {
                Read::Data(n) => {
                    let t = now();
                    self.bytes += n as u64;
                    self.last_out = t;
                    if let Some((f, left)) = self.capture.as_mut() {
                        let k = n.min(*left);
                        if k > 0 {
                            let _ = f.write_all(&self.buf[..k]);
                            *left -= k;
                        }
                    }
                    if let Some(a) = self.answer.as_mut() {
                        a.feed(&self.buf[..n], &mut self.replies);
                    }
                    if let Some((sc, target)) = self.scan.as_mut()
                        && sc.feed(&self.buf[..n], *target)
                        && self.seen.is_none()
                    {
                        self.seen = Some(t);
                    }
                }
                Read::WouldBlock => break,
                Read::Eof => {
                    self.eof = true;
                    return;
                }
            }
        }
        if !self.replies.is_empty() {
            let r = std::mem::take(&mut self.replies);
            let _ = self.pty.write_all(&r);
            self.replies = r;
            self.replies.clear();
        }
    }

    /// Waits up to `max` for output, then drains it.
    fn pump(&mut self, max: Duration) {
        if self.eof {
            std::thread::sleep(max);
            return;
        }
        let ms = max.as_micros().div_ceil(1000).min(i32::MAX as u128) as i32;
        if wait_readable(self.pty.master, ms) {
            self.drain();
        }
    }

    fn pump_until(&mut self, deadline: Instant, mut done: impl FnMut(&Self) -> bool) -> bool {
        loop {
            if done(self) {
                return true;
            }
            let t = now();
            if t >= deadline {
                return false;
            }
            self.pump((deadline - t).min(Duration::from_millis(1)));
        }
    }

    /// Waits until the client has been silent for `quiet`.
    fn wait_quiet(&mut self, quiet: Duration, deadline: Instant) -> bool {
        self.pump_until(deadline, |s| s.eof || s.last_out.elapsed() >= quiet)
    }

    fn type_bytes(&mut self, b: &[u8]) -> Result<(), String> {
        self.pty.write_all(b).map_err(|e| format!("write: {e}"))
    }

    fn cmd_ready(&mut self, marker: &str, timeout_ms: u64, retry_ms: u64) -> String {
        // Keys typed before a pane exists can be dropped, so keep typing the
        // command until the shell in the pane runs it.
        let _ = std::fs::remove_file(marker);
        let deadline = now() + Duration::from_millis(timeout_ms);
        let line = format!("touch '{marker}'\r");
        let mut tries = 0;
        loop {
            if let Err(e) = self.type_bytes(line.as_bytes()) {
                return err(&e);
            }
            tries += 1;
            let next = (now() + Duration::from_millis(retry_ms)).min(deadline);
            if self.pump_until(next, |_| Path::new(marker).exists()) {
                let since = self.spawned.elapsed();
                return format!(
                    "{{\"ok\":true,\"since_spawn_ms\":{:.3},\"tries\":{tries}}}",
                    since.as_secs_f64() * 1e3
                );
            }
            if now() >= deadline || self.eof {
                return err(&format!("not ready after {tries} tries (eof={})", self.eof));
            }
        }
    }

    fn cmd_quiet(&mut self, quiet_ms: u64, timeout_ms: u64) -> String {
        let t0 = now();
        let ok = self.wait_quiet(Duration::from_millis(quiet_ms), t0 + Duration::from_millis(timeout_ms));
        format!("{{\"ok\":{ok},\"ms\":{:.3}}}", t0.elapsed().as_secs_f64() * 1e3)
    }

    /// Types `line` and measures until `marker` exists, then how long the
    /// client keeps receiving output afterwards.
    fn cmd_run(&mut self, marker: &str, timeout_ms: u64, quiet_ms: u64, line: &[u8]) -> String {
        let _ = std::fs::remove_file(marker);
        let (last, head) = match line.split_last() {
            Some(x) => x,
            None => return err("empty line"),
        };
        if let Err(e) = self.type_bytes(head) {
            return err(&e);
        }
        self.drain();
        let bytes0 = self.bytes;
        let t0 = now();
        if let Err(e) = self.type_bytes(&[*last]) {
            return err(&e);
        }
        let deadline = t0 + Duration::from_millis(timeout_ms);
        let mut next_stat = t0;
        let mut done_at = None;
        while now() < deadline {
            self.pump(Duration::from_millis(1));
            let t = now();
            if t >= next_stat {
                // A stat per millisecond is noise next to the work measured.
                next_stat = t + Duration::from_millis(1);
                if Path::new(marker).exists() {
                    done_at = Some(t);
                    break;
                }
            }
            if self.eof {
                break;
            }
        }
        let Some(done) = done_at else {
            return format!(
                "{{\"ok\":false,\"error\":\"timeout\",\"secs\":{:.6},\"bytes\":{},\"eof\":{}}}",
                t0.elapsed().as_secs_f64(),
                self.bytes - bytes0,
                self.eof
            );
        };
        let bytes_at_done = self.bytes - bytes0;
        let qd = Duration::from_millis(quiet_ms);
        let quiet = self.wait_quiet(qd, done + Duration::from_millis(timeout_ms));
        let drain = self.last_out.saturating_duration_since(done);
        format!(
            "{{\"ok\":true,\"secs\":{:.6},\"drain_secs\":{:.6},\"bytes_at_done\":{},\"bytes\":{},\"quiet\":{quiet}}}",
            (done - t0).as_secs_f64(),
            drain.as_secs_f64(),
            bytes_at_done,
            self.bytes - bytes0
        )
    }

    /// Keystroke-to-echo latency. In `echo` mode each sample types one of
    /// `chars` (the pane echoes it) and then DEL to erase it again; in
    /// `esc` mode it sends a bare ESC and waits for the probe's `chars[0]`.
    fn cmd_latency(&mut self, mode: &str, n: usize, chars: &[u8], gap_ms: u64, timeout_ms: u64) -> String {
        if chars.is_empty() {
            return err("no chars");
        }
        let esc = match mode {
            "echo" => false,
            "esc" => true,
            _ => return err("mode must be echo or esc"),
        };
        let mut samples: Vec<f64> = Vec::with_capacity(n);
        let mut timeouts = 0u64;
        let mut stray = 0u64;
        let gap = Duration::from_millis(gap_ms);
        let bytes0 = self.bytes;
        for i in 0..n {
            let c = if esc { chars[0] } else { chars[i % chars.len()] };
            // Watch for the char during the gap too: seeing it before it is
            // typed means the measurement would be contaminated.
            self.scan = Some((Scanner::new(), c));
            self.seen = None;
            let jitter = Duration::from_micros(self.rand() % (gap_ms * 1000 + 1));
            let until = now() + gap + jitter;
            self.pump_until(until, |_| false);
            if self.seen.is_some() {
                stray += 1;
            }
            self.drain();
            self.seen = None;
            let key = if esc { 0x1b } else { c };
            let t0 = now();
            if let Err(e) = self.type_bytes(&[key]) {
                return err(&e);
            }
            let got = self.pump_until(t0 + Duration::from_millis(timeout_ms), |s| s.seen.is_some());
            match (got, self.seen) {
                (true, Some(t)) => samples.push(us(t.saturating_duration_since(t0))),
                _ => timeouts += 1,
            }
            if !esc && let Err(e) = self.type_bytes(&[0x7f]) {
                return err(&e);
            }
            if self.eof {
                break;
            }
        }
        self.scan = None;
        let list: Vec<String> = samples.iter().map(|v| format!("{v:.1}")).collect();
        format!(
            "{{\"ok\":true,\"timeouts\":{timeouts},\"stray\":{stray},\"bytes\":{},\"samples_us\":[{}]}}",
            self.bytes - bytes0,
            list.join(",")
        )
    }

    fn cmd_stats(&mut self) -> String {
        self.drain();
        let alive = self.pty.try_wait().is_none();
        format!(
            "{{\"ok\":true,\"pid\":{},\"bytes\":{},\"eof\":{},\"alive\":{alive},\"answered\":{}}}",
            self.pty.pid,
            self.bytes,
            self.eof,
            self.answer.as_ref().map_or(0, |a| a.answered)
        )
    }

    pub fn execute(&mut self, line: &str) -> (String, bool) {
        let w: Vec<&str> = line.split_whitespace().collect();
        let num = |i: usize, d: u64| w.get(i).and_then(|s| s.parse().ok()).unwrap_or(d);
        let reply = match w.first().copied() {
            Some("ready") if w.len() >= 2 => self.cmd_ready(w[1], num(2, 20_000), num(3, 250)),
            Some("type") if w.len() == 2 => match unhex(w[1]) {
                Some(b) => match self.type_bytes(&b) {
                    Ok(()) => "{\"ok\":true}".to_string(),
                    Err(e) => err(&e),
                },
                None => err("bad hex"),
            },
            Some("quiet") => self.cmd_quiet(num(1, 300), num(2, 30_000)),
            Some("run") if w.len() == 5 => match unhex(w[4]) {
                Some(b) => self.cmd_run(w[1], num(2, 600_000), num(3, 250), &b),
                None => err("bad hex"),
            },
            Some("latency") if w.len() == 6 => match unhex(w[3]) {
                Some(c) => self.cmd_latency(w[1], num(2, 1000) as usize, &c, num(4, 10), num(5, 2000)),
                None => err("bad hex"),
            },
            Some("stats") => self.cmd_stats(),
            Some("quit") => {
                let st = self.pty.shutdown(Duration::from_millis(num(1, 2000)));
                return (format!("{{\"ok\":true,\"status\":{}}}", st.unwrap_or(-1)), true);
            }
            _ => err(&format!("unknown command: {line}")),
        };
        (reply, false)
    }
}

fn err(msg: &str) -> String {
    let m: String = msg.chars().map(|c| if c == '"' || c == '\\' { '\'' } else { c }).collect();
    format!("{{\"ok\":false,\"error\":\"{m}\"}}")
}

pub fn unhex(s: &str) -> Option<Vec<u8>> {
    if !s.len().is_multiple_of(2) {
        return None;
    }
    (0..s.len())
        .step_by(2)
        .map(|i| u8::from_str_radix(s.get(i..i + 2)?, 16).ok())
        .collect()
}

/// The `hold` main loop: client output and stdin commands, until `quit`
/// or stdin closes.
pub fn hold(argv: &[String], cols: u16, rows: u16, capture: Option<&str>, answer: bool) -> i32 {
    let mut s = match Session::new(argv, cols, rows) {
        Ok(s) => s,
        Err(e) => {
            eprintln!("spawn failed: {e}");
            return 1;
        }
    };
    if !answer {
        s.answer = None;
    }
    if let Some(path) = capture {
        match std::fs::File::create(path) {
            Ok(f) => s.capture = Some((f, 1 << 20)),
            Err(e) => eprintln!("capture: {e}"),
        }
    }
    let mut out = std::io::stdout().lock();
    let _ = writeln!(out, "{{\"ok\":true,\"pid\":{}}}", s.pid());
    let _ = out.flush();
    let mut inbuf: Vec<u8> = Vec::new();
    let mut chunk = [0u8; 4096];
    loop {
        let mut fds = [
            libc::pollfd { fd: 0, events: libc::POLLIN, revents: 0 },
            libc::pollfd { fd: if s.eof { -1 } else { s.master() }, events: libc::POLLIN, revents: 0 },
        ];
        // SAFETY: fds is a valid array of two pollfds; fd -1 is ignored.
        let r = unsafe { libc::poll(fds.as_mut_ptr(), 2, -1) };
        if r < 0 {
            continue;
        }
        if fds[1].revents != 0 {
            s.drain();
        }
        if fds[0].revents != 0 {
            // SAFETY: chunk is a valid writable buffer.
            let n = unsafe { libc::read(0, chunk.as_mut_ptr().cast(), chunk.len()) };
            if n <= 0 {
                s.pty.shutdown(Duration::from_millis(2000));
                return 0;
            }
            inbuf.extend_from_slice(&chunk[..n as usize]);
            while let Some(pos) = inbuf.iter().position(|&b| b == b'\n') {
                let line: Vec<u8> = inbuf.drain(..=pos).collect();
                let line = String::from_utf8_lossy(&line).trim().to_string();
                if line.is_empty() {
                    continue;
                }
                let (reply, quit) = s.execute(&line);
                let _ = writeln!(out, "{reply}");
                let _ = out.flush();
                if quit {
                    return 0;
                }
            }
        }
    }
}
