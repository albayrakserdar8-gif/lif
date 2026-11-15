//! End-to-end harness: a private socket per test, the real `lif` binary,
//! clients in real PTYs, and a libghostty terminal standing in for the
//! user's terminal so tests can assert on what is visible.
#![allow(dead_code)]

use std::ffi::OsString;
use std::os::fd::AsRawFd;
use std::path::{Path, PathBuf};
use std::process::{Command, Output};
use std::sync::atomic::{AtomicU32, Ordering};
use std::time::{Duration, Instant};

use lif_pty::{Pty, ReadOutcome, SpawnOptions};
use lif_types::{PaneFrame, PixelSize, Size};
use lif_vt::{GhosttyVt, PaneModes, TextRange, VtEngine, VtEvent, VtOptions};

pub const BIN: &str = env!("CARGO_BIN_EXE_lif");

static COUNTER: AtomicU32 = AtomicU32::new(0);

/// A private server environment: socket directory, `HOME` with a
/// `.profile` that sets a fixed prompt, a fixed shell.
pub struct Env {
    pub dir: PathBuf,
    pub sock: PathBuf,
    pub home: PathBuf,
    pub extra: Vec<(String, String)>,
    /// Pane processes of a server the test ends by force, which cannot
    /// clean up after itself: `Drop` kills them ([`Env::track_panes`]).
    tracked: std::sync::Mutex<Vec<u32>>,
}

/// Whether process `pid` exists (a zombie counts until it is reaped).
pub fn pid_alive(pid: u32) -> bool {
    // SAFETY: probing a pid with signal 0.
    unsafe { libc::kill(pid as i32, 0) == 0 }
}

/// Whether any process is left in process group `pgid`.
pub fn group_alive(pgid: u32) -> bool {
    // SAFETY: probing a process group with signal 0.
    unsafe { libc::kill(-(pgid as i32), 0) == 0 }
}

impl Env {
    pub fn new() -> Env {
        let n = COUNTER.fetch_add(1, Ordering::Relaxed);
        let base = PathBuf::from(env!("CARGO_TARGET_TMPDIR"));
        let dir = base.join(format!("e{}-{n}", std::process::id() % 100000));
        let _ = std::fs::remove_dir_all(&dir);
        std::fs::create_dir_all(&dir).unwrap();
        use std::os::unix::fs::PermissionsExt;
        std::fs::set_permissions(&dir, std::fs::Permissions::from_mode(0o700)).unwrap();
        let home = dir.join("home");
        std::fs::create_dir_all(&home).unwrap();
        std::fs::write(home.join(".profile"), "PS1='$ '; export PS1\nunset PROMPT_COMMAND\n").unwrap();
        Env { sock: dir.join("s"), dir, home, extra: Vec::new(), tracked: Default::default() }
    }

    /// Environment for every process the test starts.
    pub fn vars(&self) -> Vec<(OsString, OsString)> {
        let mut v: Vec<(OsString, OsString)> = vec![
            ("HOME".into(), self.home.clone().into()),
            ("SHELL".into(), "/bin/sh".into()),
            ("PS1".into(), "$ ".into()),
            ("LANG".into(), "en_US.UTF-8".into()),
            ("LC_ALL".into(), "en_US.UTF-8".into()),
            ("BASH_SILENCE_DEPRECATION_WARNING".into(), "1".into()),
            ("PATH".into(), "/usr/bin:/bin:/usr/sbin:/sbin".into()),
        ];
        for (k, val) in &self.extra {
            v.push((k.into(), val.into()));
        }
        v
    }

    pub const REMOVE: &'static [&'static str] =
        &[
            "LIF", "LIF_PANE", "TMUX", "TMUX_PANE", "ENV", "PROMPT_COMMAND", "TERM_PROGRAM", "COLORTERM", "LIF_SOCKET",
            // The server reads these at start-up (config file, mode-keys).
            "LIF_CONFIG", "XDG_CONFIG_HOME", "VISUAL", "EDITOR",
        ];

    pub fn command(&self) -> Command {
        let mut c = Command::new(BIN);
        for k in Self::REMOVE {
            c.env_remove(k);
        }
        c.envs(self.vars());
        c.env("TERM", "xterm-256color");
        c.arg("-S").arg(&self.sock);
        c
    }

    /// Runs `lif -S sock args...` to completion.
    pub fn run(&self, args: &[&str]) -> Output {
        self.command().args(args).output().expect("run lif")
    }

    /// Runs a command that must succeed; returns its stdout.
    pub fn ok(&self, args: &[&str]) -> String {
        let o = self.run(args);
        assert!(o.status.success(), "lif {args:?} failed: {}{}", String::from_utf8_lossy(&o.stdout), String::from_utf8_lossy(&o.stderr));
        String::from_utf8(o.stdout).unwrap()
    }

    /// Pids of daemonized lif processes on this socket (the server is
    /// the client that started it, forked, so it has the same argv).
    pub fn server_pids(&self) -> Vec<u32> {
        let pat = format!("{} -S {} ", BIN, self.sock.display());
        let o = Command::new("pgrep").args(["-f", &pat]).output().unwrap();
        String::from_utf8_lossy(&o.stdout)
            .lines()
            .filter_map(|l| l.trim().parse::<u32>().ok())
            .filter(|pid| {
                let o = Command::new("ps").args(["-o", "ppid=", "-p", &pid.to_string()]).output().unwrap();
                String::from_utf8_lossy(&o.stdout).trim() == "1"
            })
            .collect()
    }

    /// The process of every pane the server has (none if it does not answer).
    pub fn pane_pids(&self) -> Vec<u32> {
        let o = self.run(&["list-panes", "-a", "-F", "#{pane_pid}"]);
        String::from_utf8_lossy(&o.stdout).lines().filter_map(|l| l.trim().parse().ok()).filter(|p| *p > 1).collect()
    }

    /// Call before killing the server with SIGKILL: its panes then only get
    /// the kernel's hangup, and what that does not end, `Drop` kills.
    pub fn track_panes(&self) {
        let pids = self.pane_pids();
        self.tracked.lock().unwrap().extend(pids);
    }

    pub fn server_pid(&self) -> u32 {
        let p = self.server_pids();
        assert_eq!(p.len(), 1, "expected one server, found {p:?}");
        p[0]
    }

    /// Polls `f` until it returns true or `timeout` passes.
    pub fn wait(&self, timeout: Duration, mut f: impl FnMut() -> bool) -> bool {
        let end = Instant::now() + timeout;
        loop {
            if f() {
                return true;
            }
            if Instant::now() >= end {
                return false;
            }
            std::thread::sleep(Duration::from_millis(10));
        }
    }

    /// Waits until the pane's visible text contains `needle`.
    pub fn wait_capture(&self, target: &str, needle: &str) -> String {
        let mut last = String::new();
        let ok = self.wait(Duration::from_secs(10), || {
            last = String::from_utf8_lossy(&self.run(&["capture-pane", "-p", "-t", target]).stdout).into_owned();
            last.contains(needle)
        });
        assert!(ok, "pane {target} never showed {needle:?}; it shows:\n{last}");
        last
    }
}

impl Drop for Env {
    fn drop(&mut self) {
        let tracked = std::mem::take(&mut *self.tracked.lock().unwrap_or_else(|e| e.into_inner()));
        let mut panes = Vec::new();
        if self.sock.exists() {
            panes = self.pane_pids();
            let _ = self.run(&["kill-server"]);
        }
        // No pane process may outlive a server that exited in good order
        // (issues/0001: shells left behind by the suite ran for hours).
        let gone = self.wait(Duration::from_secs(3), || !panes.iter().chain(&tracked).any(|p| pid_alive(*p)));
        let _ = std::fs::remove_dir_all(&self.dir);
        if gone {
            return;
        }
        let left: Vec<u32> = panes.iter().copied().filter(|p| pid_alive(*p)).collect();
        for pid in panes.iter().chain(&tracked) {
            // SAFETY: SIGKILL for process groups this test's server started.
            unsafe { libc::kill(-(*pid as i32), libc::SIGKILL) };
        }
        if !left.is_empty() && !std::thread::panicking() {
            panic!("pane processes outlived their server: {left:?}");
        }
    }
}

/// A `lif` client in a PTY, with a virtual outer terminal on the other side.
pub struct Term {
    pub pty: Pty,
    pub vt: GhosttyVt,
    pub frame: PaneFrame,
    pub events: Vec<VtEvent>,
    /// Everything the client wrote, for byte-level assertions.
    pub raw: Vec<u8>,
    pub exit: Option<i32>,
    pub eof: bool,
    /// Answer the client's queries as a terminal would.
    pub answer: bool,
}

impl Term {
    pub fn spawn(env: &Env, args: &[&str], size: Size) -> Term {
        Self::spawn_with(env, args, size, &[])
    }

    pub fn spawn_with(env: &Env, args: &[&str], size: Size, extra: &[(&str, &str)]) -> Term {
        let mut argv: Vec<OsString> = vec![BIN.into(), "-S".into(), env.sock.clone().into()];
        argv.extend(args.iter().map(OsString::from));
        Self::spawn_argv(env, argv, size, extra)
    }

    /// A terminal running `argv` (for a client under a shell of its own).
    pub fn spawn_argv(env: &Env, argv: Vec<OsString>, size: Size, extra: &[(&str, &str)]) -> Term {
        let mut vars = env.vars();
        for (k, v) in extra {
            vars.push(((*k).into(), (*v).into()));
        }
        let opts = SpawnOptions {
            argv,
            cwd: Some(env.home.clone()),
            env: vars,
            env_remove: Env::REMOVE.iter().map(OsString::from).collect(),
            term: "xterm-256color".into(),
            size,
            pixels: PixelSize::default(),
        };
        let pty = Pty::spawn(&opts).expect("spawn client");
        let vt = GhosttyVt::new(&VtOptions {
            size,
            default_fg: Some((0xdd, 0xdd, 0xdd)),
            default_bg: Some((0x11, 0x22, 0x33)),
            ..VtOptions::default()
        })
        .unwrap();
        Term { pty, vt, frame: PaneFrame::new(size), events: Vec::new(), raw: Vec::new(), exit: None, eof: false, answer: true }
    }

    /// Reads what the client wrote, waiting up to `timeout` for the first byte.
    pub fn pump(&mut self, timeout: Duration) -> usize {
        let mut total = 0;
        let mut buf = vec![0u8; 65536];
        let mut wait = timeout;
        loop {
            let mut p = libc::pollfd { fd: self.pty.as_raw_fd(), events: libc::POLLIN, revents: 0 };
            // SAFETY: one valid pollfd.
            let r = unsafe { libc::poll(&mut p, 1, wait.as_millis() as i32) };
            if r <= 0 {
                break;
            }
            match self.pty.read(&mut buf) {
                Ok(ReadOutcome::Data(n)) => {
                    total += n;
                    self.vt.feed(&buf[..n]);
                    if self.raw.len() < 64 << 20 {
                        self.raw.extend_from_slice(&buf[..n]);
                    }
                    let mut replies = Vec::new();
                    self.vt.drain_replies(&mut replies);
                    if self.answer && !replies.is_empty() {
                        self.write(&replies);
                    }
                    self.vt.drain_events(&mut self.events);
                }
                Ok(ReadOutcome::WouldBlock) => {}
                Ok(ReadOutcome::Eof) | Err(_) => {
                    self.eof = true;
                    break;
                }
            }
            wait = Duration::ZERO;
        }
        self.reap();
        total
    }

    fn reap(&mut self) {
        if self.exit.is_some() {
            return;
        }
        let mut status = 0;
        // SAFETY: waits for our own child without blocking.
        let r = unsafe { libc::waitpid(self.pty.pid() as libc::pid_t, &mut status, libc::WNOHANG) };
        if r > 0 {
            self.exit = Some(if libc::WIFEXITED(status) { libc::WEXITSTATUS(status) } else { 128 + libc::WTERMSIG(status) });
        }
    }

    pub fn write(&mut self, bytes: &[u8]) {
        let mut off = 0;
        let end = Instant::now() + Duration::from_secs(10);
        while off < bytes.len() {
            match self.pty.write(&bytes[off..]) {
                Ok(n) => off += n,
                Err(e) if e.kind() == std::io::ErrorKind::WouldBlock => {
                    assert!(Instant::now() < end, "client does not read its terminal");
                    self.pump(Duration::from_millis(5));
                }
                Err(e) => panic!("write to client: {e}"),
            }
        }
    }

    pub fn type_str(&mut self, s: &str) {
        self.write(s.as_bytes());
    }

    pub fn screen(&self) -> String {
        self.vt.text(TextRange::Screen, false)
    }

    pub fn lines(&self) -> Vec<String> {
        let rows = self.vt.size().rows as usize;
        let mut v: Vec<String> = self.screen().split('\n').map(|s| s.to_string()).collect();
        v.resize(rows, String::new());
        v
    }

    pub fn modes(&self) -> PaneModes {
        self.vt.modes()
    }

    pub fn cursor(&mut self) -> Option<(u16, u16)> {
        self.vt.refresh(&mut self.frame);
        self.frame.cursor.pos.map(|p| (p.x, p.y))
    }

    /// Pumps until `f` holds; panics with the screen after `timeout`.
    pub fn wait_for(&mut self, what: &str, timeout: Duration, mut f: impl FnMut(&mut Term) -> bool) {
        let end = Instant::now() + timeout;
        loop {
            if f(self) {
                return;
            }
            let now = Instant::now();
            if now >= end {
                panic!("timed out waiting for {what}; screen:\n{}\n(exit {:?})", self.screen(), self.exit);
            }
            self.pump((end - now).min(Duration::from_millis(20)));
        }
    }

    pub fn wait_text(&mut self, needle: &str) {
        let n = needle.to_string();
        self.wait_for(&format!("{needle:?} on screen"), Duration::from_secs(10), move |t| t.screen().contains(&n));
    }

    pub fn wait_exit(&mut self) -> i32 {
        self.wait_for("client exit", Duration::from_secs(10), |t| t.exit.is_some());
        self.pump(Duration::from_millis(20));
        self.exit.unwrap()
    }

    pub fn resize(&mut self, size: Size) {
        self.pty.resize(size, PixelSize::default()).unwrap();
        self.vt.resize(size, PixelSize::default());
    }

    /// Lets everything in flight arrive: pumps until nothing came for `quiet`.
    pub fn settle(&mut self, quiet: Duration) {
        let end = Instant::now() + Duration::from_secs(10);
        while Instant::now() < end && self.pump(quiet) > 0 {}
    }
}

impl Drop for Term {
    fn drop(&mut self) {
        if self.exit.is_none() {
            let _ = self.pty.kill(libc::SIGKILL);
            let mut status = 0;
            // SAFETY: reaps our own child.
            unsafe { libc::waitpid(self.pty.pid() as libc::pid_t, &mut status, 0) };
        }
    }
}

pub fn secs(n: u64) -> Duration {
    Duration::from_secs(n)
}

/// Resident set size of a process in KiB (ps).
pub fn rss_kib(pid: u32) -> u64 {
    let o = Command::new("ps").args(["-o", "rss=", "-p", &pid.to_string()]).output().unwrap();
    String::from_utf8_lossy(&o.stdout).trim().parse().unwrap_or(0)
}

/// Open file descriptors of a process (lsof).
pub fn open_fds(pid: u32) -> usize {
    let o = Command::new("lsof").args(["-n", "-P", "-p", &pid.to_string()]).output().unwrap();
    String::from_utf8_lossy(&o.stdout).lines().filter(|l| {
        let fd = l.split_whitespace().nth(3).unwrap_or("");
        fd.chars().next().is_some_and(|c| c.is_ascii_digit())
    }).count()
}

pub fn path_exists(p: &Path) -> bool {
    std::fs::symlink_metadata(p).is_ok()
}
