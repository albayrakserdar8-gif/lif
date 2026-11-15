//! The server's bounds and its protocol hygiene, end to end: what a hostile
//! or broken client, a program that never reads, or a starting client can
//! do to a server every session depends on.
mod common;

use std::io::{Read, Write};
use std::os::fd::AsRawFd;
use std::os::unix::fs::{FileTypeExt, MetadataExt, PermissionsExt};
use std::os::unix::net::{UnixListener, UnixStream};
use std::path::Path;
use std::time::{Duration, Instant};

use common::*;
use lif_proto::{Caps, ClientMsg, Decoder, Hello, PROTOCOL_VERSION, ServerMsg};
use lif_types::Size;

/// A client speaking the protocol directly, to do what lif's own client
/// never does.
struct Raw {
    s: UnixStream,
    dec: Decoder<ServerMsg>,
}

impl Raw {
    fn connect(env: &Env) -> Raw {
        let s = UnixStream::connect(&env.sock).expect("connect");
        Raw { s, dec: Decoder::new() }
    }

    fn send(&mut self, m: &ClientMsg) {
        let mut b = Vec::new();
        m.encode(&mut b).unwrap();
        let _ = self.s.write_all(&b);
    }

    fn hello_with(&mut self, caps: Caps, env: Vec<(String, String)>) {
        let h = Hello {
            protocol: PROTOCOL_VERSION,
            caps,
            pid: std::process::id(),
            size: Size::new(80, 24),
            term: "xterm-256color".into(),
            cwd: "/".into(),
            env,
            is_tty: true,
            ..Default::default()
        };
        self.send(&ClientMsg::Hello(h));
        assert!(matches!(self.recv(Duration::from_secs(5)), Some(ServerMsg::Welcome { .. })));
    }

    fn hello(&mut self) {
        self.hello_with(Caps::NONE, Vec::new());
    }

    /// The next message (Output skipped), or None on timeout or EOF.
    fn recv(&mut self, timeout: Duration) -> Option<ServerMsg> {
        let end = Instant::now() + timeout;
        loop {
            match self.dec.next() {
                Ok(Some(ServerMsg::Output(_))) => continue,
                Ok(Some(m)) => return Some(m),
                Ok(None) => {}
                Err(e) => panic!("decode: {e}"),
            }
            let left = end.checked_duration_since(Instant::now())?;
            self.s.set_read_timeout(Some(left.max(Duration::from_millis(1)))).ok()?;
            match self.dec.read_from(&mut self.s) {
                Ok(0) => return None,
                Ok(_) => {}
                Err(e) if matches!(e.kind(), std::io::ErrorKind::WouldBlock | std::io::ErrorKind::TimedOut) => return None,
                Err(_) => return None,
            }
        }
    }

    /// Waits for the server to close the connection (messages before are
    /// skipped). False on timeout.
    fn closed(&mut self, timeout: Duration) -> bool {
        let end = Instant::now() + timeout;
        let mut buf = [0u8; 65536];
        loop {
            let Some(left) = end.checked_duration_since(Instant::now()) else { return false };
            // macOS refuses socket options once the peer has closed.
            if self.s.set_read_timeout(Some(left.max(Duration::from_millis(1)))).is_err() {
                return true;
            }
            match self.s.read(&mut buf) {
                Ok(0) => return true,
                Ok(_) => {}
                Err(e) if matches!(e.kind(), std::io::ErrorKind::WouldBlock | std::io::ErrorKind::TimedOut) => return false,
                Err(_) => return true,
            }
        }
    }

    fn command(&mut self, argv: &[&str]) -> ServerMsg {
        self.send(&ClientMsg::Command { argv: argv.iter().map(|s| s.to_string()).collect() });
        self.recv(Duration::from_secs(30)).expect("an answer")
    }
}

fn kill(pid: u32, sig: i32) {
    // SAFETY: signalling a process this test started (the private server).
    unsafe { libc::kill(pid as i32, sig) };
}

fn read_file(p: &Path) -> String {
    String::from_utf8_lossy(&std::fs::read(p).unwrap_or_default()).into_owned()
}

fn wait_file(env: &Env, p: &Path, needle: &str) -> String {
    let mut last = String::new();
    let ok = env.wait(Duration::from_secs(10), || {
        last = read_file(p);
        last.contains(needle)
    });
    assert!(ok, "{} never contained {needle:?}: {last:?}", p.display());
    last
}

// ----- A: server lifetime and the socket ------------------------------------

/// A live server whose listen backlog is full refuses connections exactly
/// like a dead one. It must not be taken for dead: its socket stays, no
/// second server starts on the path, and the user is told it does not
/// respond.
#[test]
fn a_stopped_server_with_a_full_backlog_is_not_replaced() {
    let env = Env::new();
    env.ok(&["new-session", "-d", "-s", "one"]);
    let pid = env.server_pid();
    let ino = std::fs::symlink_metadata(&env.sock).unwrap().ino();
    kill(pid, libc::SIGSTOP);
    // Linux queues as many connections as net.core.somaxconn says (4096):
    // room for that many descriptors here, and connections that never
    // wait for room in the queue.
    // SAFETY: getrlimit and setrlimit with a valid struct.
    unsafe {
        let mut lim: libc::rlimit = std::mem::zeroed();
        if libc::getrlimit(libc::RLIMIT_NOFILE, &mut lim) == 0 && lim.rlim_cur < 16384 {
            lim.rlim_cur = lim.rlim_max.min(16384);
            libc::setrlimit(libc::RLIMIT_NOFILE, &lim);
        }
    }
    let mut held = Vec::new();
    let mut refused = false;
    for _ in 0..12000 {
        match lif_client::connect_stream(&env.sock) {
            Ok(s) => held.push(s),
            Err(e) => {
                refused = e.kind() == std::io::ErrorKind::ConnectionRefused;
                assert!(refused, "connecting failed otherwise after {} connections: {e}", held.len());
                break;
            }
        }
    }
    assert!(refused, "the backlog never filled ({} connections)", held.len());
    let o = env.run(&["new-session", "-d", "-s", "two"]);
    let o2 = env.run(&["list-sessions"]);
    let pids = env.server_pids();
    let ino_after = std::fs::symlink_metadata(&env.sock).map(|m| m.ino()).ok();
    kill(pid, libc::SIGCONT);
    drop(held);
    assert_eq!(o.status.code(), Some(1), "{}", String::from_utf8_lossy(&o.stderr));
    assert!(String::from_utf8_lossy(&o.stderr).contains("is not responding"), "{}", String::from_utf8_lossy(&o.stderr));
    assert_eq!(o2.status.code(), Some(1));
    assert!(String::from_utf8_lossy(&o2.stderr).contains("is not responding"), "{}", String::from_utf8_lossy(&o2.stderr));
    assert_eq!(pids, vec![pid], "a second server was started next to the stopped one");
    assert_eq!(ino_after, Some(ino), "the live server's socket was replaced");
    // The server is fine once it runs again.
    let ok = env.wait(Duration::from_secs(10), || env.run(&["has-session", "-t", "one"]).status.success());
    assert!(ok, "server did not recover");
}

/// Something at the socket path that is not a socket of ours is never
/// removed (on Linux connecting to a regular file is refused like a stale
/// socket).
#[test]
fn a_path_that_is_not_a_socket_is_not_removed() {
    let env = Env::new();
    std::fs::write(&env.sock, "precious").unwrap();
    let o = env.run(&["new-session", "-d"]);
    assert_eq!(o.status.code(), Some(1));
    // macOS: connect says ENOTSOCK; Linux: ECONNREFUSED, then the start
    // refuses to remove it.
    let err = String::from_utf8_lossy(&o.stderr);
    assert!(err.contains("not a socket of yours") || err.contains("Socket operation on non-socket"), "{err}");
    assert_eq!(std::fs::read_to_string(&env.sock).unwrap(), "precious");
    std::fs::remove_file(&env.sock).unwrap();
}

/// A stale socket of ours (no server holds the lock) is replaced.
#[test]
fn a_stale_socket_is_replaced() {
    let env = Env::new();
    let l = UnixListener::bind(&env.sock).unwrap();
    drop(l);
    env.ok(&["new-session", "-d", "-s", "x"]);
    assert_eq!(env.ok(&["list-sessions", "-F", "#{session_name}"]), "x\n");
}

/// An exiting server removes the socket path only if it is still the
/// socket it bound.
#[test]
fn an_exiting_server_removes_only_its_own_socket() {
    let env = Env::new();
    env.ok(&["new-session", "-d", "-s", "x"]);
    let pid = env.server_pid();
    let moved = env.dir.join("moved");
    std::fs::rename(&env.sock, &moved).unwrap();
    let other = UnixListener::bind(&env.sock).unwrap();
    let o = env.command().args(["-S"]).arg(&moved).args(["kill-server"]).output().unwrap();
    // (-S given twice: the last one wins.)
    assert!(o.status.success(), "{}", String::from_utf8_lossy(&o.stderr));
    assert!(env.wait(Duration::from_secs(10), || unsafe { libc::kill(pid as i32, 0) } != 0), "server did not exit");
    assert!(std::fs::symlink_metadata(&env.sock).unwrap().file_type().is_socket(), "the other socket was removed");
    drop(other);
    let _ = std::fs::remove_file(&env.sock);
}

/// The lock file and the -v log are the user's alone.
#[test]
fn lock_file_and_log_are_private() {
    let env = Env::new();
    let o = env.command().args(["-v", "new-session", "-d"]).output().unwrap();
    assert!(o.status.success());
    let lock = std::fs::metadata(env.sock.with_extension("lock")).unwrap();
    assert_eq!(lock.mode() & 0o777, 0o600);
    let pid = env.server_pid();
    let log = env.dir.join(format!("lif-server-{pid}.log"));
    assert_eq!(std::fs::metadata(&log).unwrap().mode() & 0o777, 0o600);
    // A lock file someone else could write is refused, not used.
    let env2 = Env::new();
    std::os::unix::fs::symlink("/etc/hosts", env2.sock.with_extension("lock")).unwrap();
    let o = env2.run(&["new-session", "-d"]);
    assert_eq!(o.status.code(), Some(1), "a symlinked lock file was followed");
    let _ = std::fs::Permissions::from_mode(0o600);
}

/// A shell's low descriptor limit (Terminal.app: 256) does not cap the
/// server's panes; panes still get the limit they would have had.
#[test]
fn the_descriptor_limit_is_raised_for_the_server_not_for_panes() {
    let env = Env::new();
    let out = env.dir.join("ulimit");
    let script = format!(
        "ulimit -Sn 256 && exec {} -S {} new-session -d -s big \"sh -c 'ulimit -n > {}; exec sleep 1000'\"",
        BIN,
        env.sock.display(),
        out.display()
    );
    let o = std::process::Command::new("/bin/sh").arg("-c").arg(&script).envs(env.vars()).env_remove("LIF").output().unwrap();
    assert!(o.status.success(), "{}", String::from_utf8_lossy(&o.stderr));
    assert_eq!(wait_file(&env, &out, "\n").trim(), "256");
    for i in 0..300 {
        let o = env.run(&["new-window", "-d", "-t", "big", "exec sleep 1000"]);
        assert!(o.status.success(), "window {i}: {}", String::from_utf8_lossy(&o.stderr));
    }
    assert_eq!(env.ok(&["display-message", "-p", "-t", "big", "#{session_windows}"]), "301\n");
}

/// Out of descriptors, the server still answers every connection (with an
/// error that says so) instead of leaving it queued forever, refuses new
/// panes cleanly, and recovers when descriptors are freed.
#[test]
fn running_out_of_descriptors_is_survivable() {
    let env = Env::new();
    let script = format!("ulimit -n 64 && exec {} -S {} new-session -d -s small", BIN, env.sock.display());
    let o = std::process::Command::new("/bin/sh").arg("-c").arg(&script).envs(env.vars()).env_remove("LIF").output().unwrap();
    assert!(o.status.success(), "{}", String::from_utf8_lossy(&o.stderr));
    let mut refused = None;
    for _ in 0..80 {
        let o = env.run(&["new-window", "-d", "-t", "small", "exec sleep 1000"]);
        if !o.status.success() {
            refused = Some(String::from_utf8_lossy(&o.stderr).into_owned());
            break;
        }
    }
    let refused = refused.expect("pane creation never stopped");
    assert!(refused.contains("file descriptors"), "{refused}");
    // Use up the rest with idle connections.
    let mut held = Vec::new();
    for _ in 0..80 {
        if let Ok(s) = UnixStream::connect(&env.sock) {
            held.push(s);
        }
    }
    // The server accepts the held connections at its own pace (slower when
    // the machine is busy), so poll until it has run out rather than sleep.
    let mut last = String::new();
    let refused = env.wait(Duration::from_secs(10), || {
        let t = Instant::now();
        let o = env.run(&["list-sessions"]);
        assert!(t.elapsed() < Duration::from_secs(5), "a connection waited {:?}", t.elapsed());
        last = String::from_utf8_lossy(&o.stderr).into_owned();
        o.status.code() == Some(1) && last.contains("out of file descriptors")
    });
    assert!(refused, "the server never reported running out: {last}");
    drop(held);
    let ok = env.wait(Duration::from_secs(5), || env.run(&["list-sessions"]).status.success());
    assert!(ok, "the server did not recover");
    // Room for a pane again once some are gone.
    env.ok(&["kill-window", "-t", "small:1"]);
    env.ok(&["kill-window", "-t", "small:2"]);
    env.ok(&["new-window", "-d", "-t", "small", "exec sleep 1000"]);
}

// ----- C: the event loop's bounds ---------------------------------------------

fn attach(env: &Env, name: &str) -> Term {
    let mut t = Term::spawn(env, &["new-session", "-s", name], Size::new(80, 24));
    t.wait_text("$");
    t
}

/// A program that does not read its input does not freeze its client:
/// prefix keys still work, other panes still get keys, a paste over the
/// bound is dropped with a message, and a respawned program gets the
/// input typed afterwards.
#[test]
fn a_pane_that_does_not_read_never_blocks_its_client() {
    let env = Env::new();
    let mut t = attach(&env, "stuck");
    t.type_str("stty raw -echo; sleep 100000\r");
    t.wait_for("sleep running", secs(10), |t| t.screen().contains("sleep 100000"));
    t.settle(Duration::from_millis(200));
    // 200 KB: more than the PTY takes, less than the queue bound.
    let mut paste = b"\x1b[200~".to_vec();
    paste.extend(std::iter::repeat_n(b'p', 200 * 1024));
    paste.extend_from_slice(b"\x1b[201~");
    t.write(&paste);
    // Prefix keys still work: a new window, typed into.
    t.write(b"\x02c");
    t.wait_for("second window", secs(10), |t| t.lines()[23].contains("1:") && t.lines()[0].starts_with('$'));
    t.type_str("echo WIN''DOW2\r");
    t.wait_text("WINDOW2");
    t.write(b"\x02p");
    // A paste over the bound: dropped, and said so. (The queue is filled
    // most of the way with send-keys: a PTY takes typing 1 KiB at a time.)
    // (In pieces: Linux takes at most 128 KiB in one argument.)
    let piece = "s".repeat(100 * 1024);
    let mut fill = vec!["send-keys", "-t", "stuck:0", "-l"];
    fill.extend(std::iter::repeat_n(piece.as_str(), 8));
    env.ok(&fill);
    let mut big = b"\x1b[200~".to_vec();
    big.extend(std::iter::repeat_n(b'q', 100 * 1024));
    big.extend_from_slice(b"\x1b[201~");
    t.write(&big);
    t.wait_text("not reading its input");
    // The respawned program gets what is typed now.
    let f = env.dir.join("got");
    let cmd = format!("stty raw -echo; head -c 3 > {}", f.display());
    env.ok(&["respawn-pane", "-k", "-t", "stuck:0", &cmd]);
    std::thread::sleep(Duration::from_millis(300));
    t.type_str("abc");
    assert_eq!(wait_file(&env, &f, "abc"), "abc");
    t.write(b"\x02d");
    assert_eq!(t.wait_exit(), 0);
}

/// A program asking for the cursor position in a loop without reading the
/// answers: the answers stop being queued at the bound (the server's
/// memory stays put), and send-keys to it fails instead of queueing more.
#[test]
fn terminal_replies_to_a_program_that_does_not_read_are_bounded() {
    let env = Env::new();
    env.ok(&["new-session", "-d", "-s", "dsr", "stty raw -echo; yes \"$(printf '\\033[6n')\""]);
    let pid = env.server_pid();
    std::thread::sleep(Duration::from_secs(1));
    let rss0 = rss_kib(pid);
    std::thread::sleep(Duration::from_secs(3));
    let rss1 = rss_kib(pid);
    eprintln!("DSR flood: server rss {rss0} -> {rss1} KiB");
    assert!(rss1 < rss0 + 8 * 1024, "replies queued without bound: {rss0} -> {rss1} KiB");
    // (Replies fill the queue to within a reply's length of the bound; how
    // soon depends on how much the PTY takes first.)
    let mut o = env.run(&["send-keys", "-t", "dsr", "-l", &"hello".repeat(1000)]);
    let end = Instant::now() + Duration::from_secs(20);
    while o.status.code() == Some(0) && Instant::now() < end {
        std::thread::sleep(Duration::from_millis(100));
        o = env.run(&["send-keys", "-t", "dsr", "-l", &"hello".repeat(1000)]);
    }
    assert_eq!(o.status.code(), Some(1));
    assert!(String::from_utf8_lossy(&o.stderr).contains("input queue full"), "{}", String::from_utf8_lossy(&o.stderr));
    // (From a file: Linux takes at most 128 KiB in one argument.)
    let big = env.dir.join("big");
    std::fs::write(&big, "x".repeat(300 * 1024)).unwrap();
    let o = env.run(&["load-buffer", big.to_str().unwrap()]);
    assert!(o.status.success());
    let o = env.run(&["paste-buffer", "-t", "dsr"]);
    assert_eq!(o.status.code(), Some(1), "paste-buffer into a full queue succeeded");
    assert_eq!(env.ok(&["display-message", "-p", "ok"]), "ok\n");
}

fn run_with_deadline(env: &Env, args: &[&str], limit: Duration) -> std::process::Output {
    let mut child = env.command().args(args).stdout(std::process::Stdio::piped()).stderr(std::process::Stdio::piped()).spawn().unwrap();
    let mut out = Vec::new();
    let mut stdout = child.stdout.take().unwrap();
    let reader = std::thread::spawn(move || {
        let _ = stdout.read_to_end(&mut out);
        out
    });
    let end = Instant::now() + limit;
    loop {
        if let Some(status) = child.try_wait().unwrap() {
            let mut err = Vec::new();
            let _ = child.stderr.take().unwrap().read_to_end(&mut err);
            return std::process::Output { status, stdout: reader.join().unwrap(), stderr: err };
        }
        if Instant::now() > end {
            let _ = child.kill();
            let _ = child.wait();
            panic!("lif {args:?} did not finish in {limit:?}");
        }
        std::thread::sleep(Duration::from_millis(20));
    }
}

/// Command output larger than one frame (16 MiB) reaches the client in
/// parts; a client that cannot take parts gets an error, never silence;
/// the -v log records sizes, not payloads.
#[test]
fn command_output_larger_than_a_frame_is_delivered() {
    let env = Env::new();
    let o = env.command().args(["-v", "new-session", "-d", "-s", "big", "-x", "200", "-y", "20", "sleep 100000"]).output().unwrap();
    assert!(o.status.success());
    env.ok(&["set-option", "-g", "history-limit", "300000"]);
    env.ok(&["set-option", "-g", "scrollback-bytes", "400000000"]);
    let line = "y".repeat(100);
    let cmd = format!("yes {line} | head -n 200000; exec sleep 100000");
    env.ok(&["respawn-pane", "-k", "-t", "big", &cmd]);
    let ok = env.wait(Duration::from_secs(60), || {
        env.ok(&["display-message", "-p", "-t", "big", "#{history_size}"]).trim().parse::<u64>().unwrap_or(0) >= 199_980
    });
    assert!(ok, "history never filled");
    let o = run_with_deadline(&env, &["capture-pane", "-p", "-S", "-", "-E", "-", "-t", "big"], Duration::from_secs(60));
    assert!(o.status.success(), "{}", String::from_utf8_lossy(&o.stderr));
    assert!(o.stdout.len() > 20_000_000, "only {} bytes", o.stdout.len());
    assert!(o.stdout.windows(line.len()).filter(|w| *w == line.as_bytes()).count() >= 199_980);
    // A client that does not take output in parts.
    let mut raw = Raw::connect(&env);
    raw.hello();
    match raw.command(&["capture-pane", "-p", "-S", "-", "-E", "-", "-t", "big"]) {
        ServerMsg::CommandDone { status, stderr, .. } => {
            assert_eq!(status, 1);
            assert!(String::from_utf8_lossy(&stderr).contains("too large"), "{}", String::from_utf8_lossy(&stderr));
        }
        m => panic!("unexpected {m:?}"),
    }
    let pid = env.server_pid();
    let log = std::fs::read(env.dir.join(format!("lif-server-{pid}.log"))).unwrap();
    let longest = log.split(|&b| b == b'\n').map(|l| l.len()).max().unwrap_or(0);
    assert!(longest < 4096, "a log line of {longest} bytes: a payload was logged");
}

/// A client that leaves in the middle of a paste: the program gets the end
/// of the paste, so what is typed later is not taken as pasted.
#[test]
fn a_client_leaving_mid_paste_closes_the_paste() {
    let env = Env::new();
    let f = env.dir.join("pasted");
    let cmd = format!("printf '\\033[?2004h'; stty raw -echo; exec cat > {}", f.display());
    env.ok(&["new-session", "-d", "-s", "p", &cmd]);
    std::thread::sleep(Duration::from_millis(300));
    let mut raw = Raw::connect(&env);
    raw.hello();
    assert_eq!(raw.command(&["attach-session", "-t", "p"]), ServerMsg::Attached);
    raw.send(&ClientMsg::Input(b"\x1b[200~hello".to_vec()));
    std::thread::sleep(Duration::from_millis(200));
    drop(raw);
    let got = wait_file(&env, &f, "\x1b[201~");
    assert_eq!(got, "\x1b[200~hello\x1b[201~");
}

/// Messages out of place close the connection: a second Hello (it used to
/// leave a zombie attachment), anything before Hello, terminal messages
/// before attaching, a command while attached.
#[test]
fn messages_out_of_place_close_the_connection() {
    let env = Env::new();
    env.ok(&["new-session", "-d", "-s", "m"]);
    type Case = (&'static str, Box<dyn Fn(&mut Raw)>);
    let cases: Vec<Case> = vec![
        (
            "second hello",
            Box::new(|r: &mut Raw| {
                r.hello();
                assert_eq!(r.command(&["attach-session", "-t", "m"]), ServerMsg::Attached);
                r.send(&ClientMsg::Hello(Hello { protocol: PROTOCOL_VERSION, is_tty: true, ..Default::default() }));
            }),
        ),
        ("command before hello", Box::new(|r: &mut Raw| r.send(&ClientMsg::Command { argv: vec!["list-sessions".into()] }))),
        (
            "input before attaching",
            Box::new(|r: &mut Raw| {
                r.hello();
                r.send(&ClientMsg::Input(b"x".to_vec()));
            }),
        ),
        (
            "use-tty before attaching",
            Box::new(|r: &mut Raw| {
                r.hello();
                r.send(&ClientMsg::UseTty { path: "/dev/tty".into(), dev: 0 });
            }),
        ),
        (
            "command while attached",
            Box::new(|r: &mut Raw| {
                r.hello();
                assert_eq!(r.command(&["attach-session", "-t", "m"]), ServerMsg::Attached);
                r.send(&ClientMsg::Command { argv: vec!["list-sessions".into()] });
            }),
        ),
        (
            "suspend without the capability",
            Box::new(|r: &mut Raw| {
                r.hello();
                assert_eq!(r.command(&["attach-session", "-t", "m"]), ServerMsg::Attached);
                r.send(&ClientMsg::Suspend);
            }),
        ),
    ];
    for (what, f) in cases {
        let mut r = Raw::connect(&env);
        f(&mut r);
        assert!(r.closed(Duration::from_secs(5)), "{what}: connection left open");
    }
    // No zombie client is left attached, and the server is fine.
    assert!(env.wait(Duration::from_secs(5), || env.ok(&["list-clients"]).is_empty()), "{}", env.ok(&["list-clients"]));
    env.ok(&["kill-session", "-t", "m"]);
}

/// The server keeps only the allowed variables of a client's environment:
/// a raw client cannot plant LD_PRELOAD, PATH, HOME, SHELL or BASH_ENV
/// into the panes of a session.
#[test]
fn client_environment_is_filtered_by_the_server() {
    let env = Env::new();
    env.ok(&["new-session", "-d", "-s", "keep"]);
    let f = env.dir.join("env");
    let mut raw = Raw::connect(&env);
    let planted = ["LD_PRELOAD", "DYLD_INSERT_LIBRARIES", "PATH", "HOME", "SHELL", "BASH_ENV", "FOO"]
        .iter()
        .map(|k| (k.to_string(), "/planted".to_string()))
        .chain([("LANG".to_string(), "fr_FR.UTF-8".to_string())])
        .collect();
    raw.hello_with(Caps::NONE, planted);
    let cmd = format!("env > {}; exec sleep 1000", f.display());
    assert!(matches!(raw.command(&["new-session", "-d", "-s", "e", &cmd]), ServerMsg::CommandDone { status: 0, .. }));
    let got = wait_file(&env, &f, "LIF_PANE=");
    assert!(!got.contains("/planted"), "planted variables reached the pane:\n{got}");
    assert!(got.contains("LANG=fr_FR.UTF-8"), "{got}");
}

/// A program that keeps the PTY of an exited pane busy (a background job
/// ignoring SIGHUP and writing forever) does not keep the loop draining
/// it. (On macOS the kernel revokes the terminal when the session leader
/// exits, so only Linux could hang here.)
#[test]
fn an_exited_pane_with_a_writer_left_behind_is_closed() {
    let env = Env::new();
    env.ok(&["new-session", "-d", "-s", "a"]);
    // The writer stops once its writes fail (the PTY closed), so it does
    // not outlive the test.
    env.ok(&["new-window", "-d", "-t", "a", "trap '' HUP; (trap '' HUP; while echo xxxxxxxxxxxxxxxxxxxxxxxx; do :; done) & sleep 0.3; exit 0"]);
    std::thread::sleep(Duration::from_secs(2));
    let t = Instant::now();
    assert_eq!(env.ok(&["display-message", "-p", "ok"]), "ok\n");
    assert!(t.elapsed() < Duration::from_secs(2));
    assert!(env.wait(Duration::from_secs(10), || env.ok(&["list-windows", "-t", "a", "-F", "#{window_index}"]) == "0\n"));
}

/// Sizes from a client are clamped: a terminal claiming 65535x65535 does
/// not make the server allocate billions of cells.
#[test]
fn huge_client_sizes_are_clamped() {
    let env = Env::new();
    env.ok(&["new-session", "-d", "-s", "h"]);
    let mut raw = Raw::connect(&env);
    raw.hello();
    assert_eq!(raw.command(&["attach-session", "-t", "h"]), ServerMsg::Attached);
    raw.send(&ClientMsg::Resize { size: Size::new(65535, 65535), pixels: Default::default() });
    std::thread::sleep(Duration::from_millis(300));
    let got = env.ok(&["display-message", "-p", "-t", "h", "#{window_width} #{window_height}"]);
    let (w, h) = got.trim().split_once(' ').unwrap();
    let (w, h): (u32, u32) = (w.parse().unwrap(), h.parse().unwrap());
    assert_eq!(w, 10000, "{got}");
    assert!(w * (h + 1) <= 4 * 1024 * 1024, "{got}");
    for (x, y) in [("20000", "20"), ("10000", "10000"), ("80", "65535")] {
        let o = env.run(&["new-session", "-d", "-x", x, "-y", y]);
        assert_eq!(o.status.code(), Some(1), "{x}x{y} was allowed");
    }
    let _ = raw.s.as_raw_fd();
}
