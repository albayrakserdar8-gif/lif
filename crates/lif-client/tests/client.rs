//! Client tests against a real Unix socket (a fake server thread) and a real
//! PTY standing in for the user's terminal.

use std::io::Write;
use std::os::fd::{AsRawFd, FromRawFd, OwnedFd, RawFd};
use std::os::unix::net::{UnixListener, UnixStream};
use std::path::PathBuf;
use std::sync::atomic::{AtomicUsize, Ordering};
use std::sync::{mpsc, RwLock};
use std::thread::{self, JoinHandle};
use std::time::{Duration, Instant};

use lif_client::*;
use lif_proto::*;
use lif_types::{PixelSize, Size};

const TIMEOUT: Duration = Duration::from_secs(30);

/// Tests that raise process-wide signals take this exclusively; every other
/// attach test holds it shared, so a SIGTERM never hits someone else's loop.
static SIGNALS: RwLock<()> = RwLock::new(());

fn shared() -> std::sync::RwLockReadGuard<'static, ()> {
    SIGNALS.read().unwrap_or_else(|e| e.into_inner())
}
fn exclusive() -> std::sync::RwLockWriteGuard<'static, ()> {
    SIGNALS.write().unwrap_or_else(|e| e.into_inner())
}

// --- PTY --------------------------------------------------------------------

struct Pty {
    master: OwnedFd,
    slave: OwnedFd,
}

impl Pty {
    fn new(cols: u16, rows: u16) -> Pty {
        let (mut m, mut s) = (-1, -1);
        let mut ws = libc::winsize { ws_row: rows, ws_col: cols, ws_xpixel: cols * 8, ws_ypixel: rows * 16 };
        // SAFETY: valid out pointers; null name and termios mean defaults.
        let r = unsafe { libc::openpty(&mut m, &mut s, std::ptr::null_mut(), std::ptr::null_mut::<libc::termios>(), &mut ws) };
        assert_eq!(r, 0, "openpty: {}", std::io::Error::last_os_error());
        // SAFETY: fresh fds owned by nobody else.
        let pty = unsafe { Pty { master: OwnedFd::from_raw_fd(m), slave: OwnedFd::from_raw_fd(s) } };
        set_nonblocking(pty.master.as_raw_fd());
        pty
    }

    fn fds(&self) -> TermFds {
        let s = self.slave.as_raw_fd();
        TermFds { input: s, output: s, tty: s }
    }

    fn set_size(&self, cols: u16, rows: u16, xpix: u16, ypix: u16) {
        let ws = libc::winsize { ws_row: rows, ws_col: cols, ws_xpixel: xpix, ws_ypixel: ypix };
        // SAFETY: TIOCSWINSZ reads one winsize.
        assert_eq!(unsafe { libc::ioctl(self.master.as_raw_fd(), libc::TIOCSWINSZ, &ws) }, 0);
    }
}

fn set_nonblocking(fd: RawFd) {
    // SAFETY: plain fcntl calls on a valid fd.
    unsafe { libc::fcntl(fd, libc::F_SETFL, libc::fcntl(fd, libc::F_GETFL) | libc::O_NONBLOCK) };
}

fn wait_fd(fd: RawFd, events: libc::c_short, deadline: Instant) {
    let left = deadline.saturating_duration_since(Instant::now());
    assert!(!left.is_zero(), "timed out waiting on fd {fd}");
    let mut p = libc::pollfd { fd, events, revents: 0 };
    // SAFETY: one valid pollfd.
    unsafe { libc::poll(&mut p, 1, left.as_millis().min(1000) as libc::c_int) };
}

/// Reads from the master until `done` says the bytes so far are enough.
fn read_master(fd: RawFd, done: impl Fn(&[u8]) -> bool) -> Vec<u8> {
    let deadline = Instant::now() + TIMEOUT;
    let mut got = Vec::new();
    let mut buf = vec![0u8; 64 * 1024];
    while !done(&got) {
        // SAFETY: buf is valid for buf.len() bytes.
        let n = unsafe { libc::read(fd, buf.as_mut_ptr().cast(), buf.len()) };
        if n > 0 {
            got.extend_from_slice(&buf[..n as usize]);
        } else {
            wait_fd(fd, libc::POLLIN, deadline);
        }
    }
    got
}

fn write_master(fd: RawFd, mut data: &[u8]) {
    let deadline = Instant::now() + TIMEOUT;
    while !data.is_empty() {
        // SAFETY: data is valid for data.len() bytes.
        let n = unsafe { libc::write(fd, data.as_ptr().cast(), data.len()) };
        if n > 0 {
            data = &data[n as usize..];
        } else {
            wait_fd(fd, libc::POLLOUT, deadline);
        }
    }
}

#[derive(Debug, PartialEq, Eq)]
struct Modes {
    iflag: u64,
    oflag: u64,
    cflag: u64,
    lflag: u64,
    cc: Vec<u8>,
    nonblock: bool,
}

// tcflag_t is u64 on macOS and u32 on Linux.
#[allow(clippy::unnecessary_cast)]
fn modes(fd: RawFd) -> Modes {
    // SAFETY: tcgetattr fills a zeroed termios; fcntl F_GETFL takes no pointer.
    unsafe {
        let mut t: libc::termios = std::mem::zeroed();
        assert_eq!(libc::tcgetattr(fd, &mut t), 0);
        Modes {
            iflag: t.c_iflag as u64,
            oflag: t.c_oflag as u64,
            cflag: t.c_cflag as u64,
            // PENDIN is kernel bookkeeping (macOS sets it when leaving
            // non-canonical mode with input queued), not a mode we set.
            lflag: t.c_lflag as u64 & !(libc::PENDIN as u64),
            cc: t.c_cc.to_vec(),
            // Only the flag we change; macOS also reports kernel-internal
            // bits (e.g. "was written") through F_GETFL.
            nonblock: libc::fcntl(fd, libc::F_GETFL) & libc::O_NONBLOCK != 0,
        }
    }
}

#[allow(clippy::unnecessary_cast)]
fn is_raw(fd: RawFd) -> bool {
    let m = modes(fd);
    m.lflag & (libc::ICANON | libc::ECHO) as u64 == 0
}

// --- Fake server ----------------------------------------------------------

static N: AtomicUsize = AtomicUsize::new(0);

fn listen() -> (PathBuf, UnixListener) {
    let path = std::env::temp_dir().join(format!("lif-ct-{}-{}.sock", std::process::id(), N.fetch_add(1, Ordering::Relaxed)));
    let _ = std::fs::remove_file(&path);
    let l = UnixListener::bind(&path).unwrap();
    l.set_nonblocking(true).unwrap();
    (path, l)
}

struct Server {
    stream: UnixStream,
    dec: Decoder<ClientMsg>,
}

impl Server {
    fn accept(l: &UnixListener) -> Server {
        let deadline = Instant::now() + TIMEOUT;
        loop {
            match l.accept() {
                Ok((s, _)) => {
                    s.set_nonblocking(false).unwrap();
                    s.set_read_timeout(Some(TIMEOUT)).unwrap();
                    s.set_write_timeout(Some(TIMEOUT)).unwrap();
                    return Server { stream: s, dec: Decoder::new() };
                }
                Err(_) => wait_fd(l.as_raw_fd(), libc::POLLIN, deadline),
            }
        }
    }

    fn recv(&mut self) -> ClientMsg {
        loop {
            if let Some(m) = self.dec.next().unwrap() {
                return m;
            }
            let n = self.dec.read_from(&mut self.stream).expect("server read (timed out?)");
            assert!(n > 0, "client closed the connection");
        }
    }

    fn send(&mut self, m: &ServerMsg) {
        let mut out = Vec::new();
        m.encode(&mut out).unwrap();
        self.stream.write_all(&out).unwrap();
    }

    fn welcome(&mut self) -> Hello {
        let ClientMsg::Hello(h) = self.recv() else { panic!("expected Hello") };
        self.send(&ServerMsg::Welcome { protocol: PROTOCOL_VERSION, caps: Caps(0b110), server_version: "fake 9".into(), server_pid: 777 });
        h
    }

    /// Handshake, then answer the first command with `Attached`.
    fn attach(&mut self) {
        self.welcome();
        let ClientMsg::Command { .. } = self.recv() else { panic!("expected Command") };
        self.send(&ServerMsg::Attached);
    }
}

fn serve<T: Send + 'static>(l: UnixListener, f: impl FnOnce(Server) -> T + Send + 'static) -> JoinHandle<T> {
    thread::spawn(move || f(Server::accept(&l)))
}

fn join<T>(h: JoinHandle<T>, what: &str) -> T {
    let deadline = Instant::now() + TIMEOUT;
    while !h.is_finished() {
        assert!(Instant::now() < deadline, "{what} timed out");
        thread::sleep(Duration::from_millis(2));
    }
    match h.join() {
        Ok(v) => v,
        Err(p) => std::panic::resume_unwind(p),
    }
}

fn hello(size: Size) -> Hello {
    Hello { protocol: PROTOCOL_VERSION, size, pixels: PixelSize { width: size.cols * 8, height: size.rows * 16 }, is_tty: true, ..hello_from_env() }
}

/// Connects, runs one command that must attach, and runs the loop on `fds`.
fn client(path: PathBuf, fds: TermFds, size: Size) -> JoinHandle<std::io::Result<ExitInfo>> {
    thread::spawn(move || {
        let mut c = connect_with(&path, hello(size))?;
        assert_eq!(run_command(&mut c, ["attach"])?, CommandOutcome::Attached);
        attach_loop_with(c, fds)
    })
}

struct XorShift(u64);

impl XorShift {
    fn next(&mut self) -> u64 {
        self.0 ^= self.0 << 13;
        self.0 ^= self.0 >> 7;
        self.0 ^= self.0 << 17;
        self.0
    }
    fn bytes(&mut self, n: usize) -> Vec<u8> {
        (0..n).map(|_| self.next() as u8).collect()
    }
}

// --- Tests ----------------------------------------------------------------

#[test]
fn handshake_sends_environment_and_reads_welcome() {
    let (path, l) = listen();
    let srv = serve(l, |mut s| {
        // A frame from the future before Welcome must be skipped.
        let h = match s.recv() {
            ClientMsg::Hello(h) => h,
            m => panic!("{m:?}"),
        };
        let mut out = Vec::new();
        out.extend_from_slice(&[3, 0, 0, 0, 0xee, 0x01, 1, 2, 3]);
        ServerMsg::Welcome { protocol: PROTOCOL_VERSION, caps: Caps(0b110), server_version: "fake 9".into(), server_pid: 777 }.encode(&mut out).unwrap();
        s.stream.write_all(&out).unwrap();
        h
    });
    let conn = connect_with(&path, Hello { caps: Caps(0b011), ..hello_from_env() }).unwrap();
    let h = join(srv, "server");
    assert_eq!(h.protocol, PROTOCOL_VERSION);
    assert_eq!(h.pid, std::process::id());
    assert_eq!(h.term, std::env::var("TERM").unwrap_or_default());
    assert_eq!(h.cwd, std::env::current_dir().unwrap().to_string_lossy());
    assert!(h.env.iter().all(|(k, _)| ENV_ALLOW_LIST.contains(&k.as_str())));
    for k in ENV_ALLOW_LIST {
        assert_eq!(h.env.iter().find(|(n, _)| n == k).map(|(_, v)| v.clone()), std::env::var(k).ok(), "{k}");
    }
    assert_eq!(conn.server(), &ServerInfo { protocol: PROTOCOL_VERSION, caps: Caps(0b110), version: "fake 9".into(), pid: 777 });
    assert_eq!(conn.caps(), Caps(0b010));
    let _ = std::fs::remove_file(path);
}

#[test]
fn handshake_rejects_other_framing_version() {
    let (path, l) = listen();
    let srv = serve(l, |mut s| {
        s.recv();
        s.send(&ServerMsg::Welcome { protocol: 2, caps: Caps::NONE, server_version: String::new(), server_pid: 1 });
    });
    let err = connect_with(&path, hello_from_env()).unwrap_err();
    assert_eq!(err.kind(), std::io::ErrorKind::Unsupported);
    join(srv, "server");
    let _ = std::fs::remove_file(path);
}

#[test]
fn command_done_then_attached() {
    let (path, l) = listen();
    let srv = serve(l, |mut s| {
        s.welcome();
        let ClientMsg::Command { argv } = s.recv() else { panic!() };
        assert_eq!(argv, ["list-sessions", "-F", "#{session_name}"]);
        s.send(&ServerMsg::CommandDone { status: 3, stdout: b"work\n".to_vec(), stderr: b"warn\xff\n".to_vec() });
        let ClientMsg::Command { argv } = s.recv() else { panic!() };
        assert_eq!(argv, ["attach"]);
        s.send(&ServerMsg::Attached);
    });
    let mut c = connect_with(&path, hello_from_env()).unwrap();
    let out = run_command(&mut c, ["list-sessions", "-F", "#{session_name}"]).unwrap();
    assert_eq!(out, CommandOutcome::Done { status: 3, stdout: b"work\n".to_vec(), stderr: b"warn\xff\n".to_vec() });
    assert_eq!(run_command(&mut c, vec!["attach".to_string()]).unwrap(), CommandOutcome::Attached);
    join(srv, "server");
    let _ = std::fs::remove_file(path);
}

#[test]
fn relay_is_byte_exact_both_ways() {
    let _g = shared();
    const LEN: usize = 4 * 1024 * 1024;
    let mut rng = XorShift(0x2545_f491_4f6c_dd1d);
    let screen = rng.bytes(LEN);
    let keys = rng.bytes(LEN);

    let pty = Pty::new(100, 30);
    let before = modes(pty.slave.as_raw_fd());
    let (path, l) = listen();
    let screen2 = screen.clone();
    let srv = serve(l, move |mut s| {
        s.attach();
        let mut tx = s.stream.try_clone().unwrap();
        let sender = thread::spawn(move || {
            let mut rng = XorShift(7);
            let mut rest = &screen2[..];
            let mut out = Vec::new();
            let mut i = 0;
            while !rest.is_empty() {
                let n = (1 + rng.next() as usize % 100_000).min(rest.len());
                out.clear();
                encode_output(&rest[..n], &mut out);
                if i % 17 == 0 {
                    // Frames this client does not know are skipped mid-stream.
                    out.extend_from_slice(&[2, 0, 0, 0, 0xff, 0x01, 0xaa, 0xbb]);
                }
                tx.write_all(&out).unwrap();
                rest = &rest[n..];
                i += 1;
            }
        });
        let mut input = Vec::with_capacity(LEN);
        let mut frames = 0;
        while input.len() < LEN {
            match s.recv() {
                ClientMsg::Input(b) => {
                    input.extend_from_slice(&b);
                    frames += 1;
                }
                ClientMsg::Resize { .. } => {}
                m => panic!("unexpected {m:?}"),
            }
        }
        sender.join().unwrap();
        s.send(&ServerMsg::Exit { reason: ExitReason::Detached, message: String::new() });
        (input, frames)
    });

    let t0 = Instant::now();
    let cl = client(path.clone(), pty.fds(), Size::new(100, 30));
    let m = pty.master.as_raw_fd();
    let reader = thread::spawn(move || read_master(m, |g| g.len() >= LEN));
    // Writing before raw mode would let the line discipline echo and edit.
    let deadline = Instant::now() + TIMEOUT;
    while !is_raw(pty.slave.as_raw_fd()) {
        assert!(Instant::now() < deadline);
        thread::sleep(Duration::from_millis(1));
    }
    let keys2 = keys.clone();
    let writer = thread::spawn(move || write_master(m, &keys2));

    let got_screen = join(reader, "screen reader");
    join(writer, "keyboard writer");
    let (got_keys, frames) = join(srv, "server");
    let exit = join(cl, "client").unwrap();
    let dt = t0.elapsed();
    assert_eq!(got_screen.len(), LEN);
    assert!(got_screen == screen, "screen bytes differ");
    assert_eq!(got_keys.len(), LEN);
    assert!(got_keys == keys, "keyboard bytes differ");
    assert_eq!(exit, ExitInfo { cause: ExitCause::Server, reason: ExitReason::Detached, message: String::new() });
    assert_eq!(modes(pty.slave.as_raw_fd()), before);
    println!(
        "relay: {} MiB each way through a PTY in {:.0} ms ({:.0} MiB/s combined), {} Input frames",
        LEN >> 20,
        dt.as_secs_f64() * 1000.0,
        (2 * LEN) as f64 / (1 << 20) as f64 / dt.as_secs_f64(),
        frames
    );
    let _ = std::fs::remove_file(path);
}

#[test]
fn slow_terminal_pushes_back_on_the_server() {
    let _g = shared();
    const LEN: usize = 16 * 1024 * 1024;
    let screen = XorShift(99).bytes(LEN);
    let pty = Pty::new(80, 24);
    let (path, l) = listen();
    let (stalled_tx, stalled_rx) = mpsc::channel();
    let screen2 = screen.clone();
    let srv = serve(l, move |mut s| {
        s.attach();
        let mut wire = Vec::new();
        encode_output(&screen2[..LEN / 2], &mut wire);
        encode_output(&screen2[LEN / 2..], &mut wire);
        // Nobody reads the PTY yet: the client must stop reading us.
        s.stream.set_nonblocking(true).unwrap();
        let mut sent = 0;
        loop {
            match s.stream.write(&wire[sent..]) {
                Ok(n) => sent += n,
                Err(e) if e.kind() == std::io::ErrorKind::WouldBlock => {
                    // Still blocked after a pause means the client has stopped
                    // reading, not that it is momentarily busy.
                    let mut p = libc::pollfd { fd: s.stream.as_raw_fd(), events: libc::POLLOUT, revents: 0 };
                    // SAFETY: one valid pollfd.
                    if unsafe { libc::poll(&mut p, 1, 300) } == 0 {
                        break;
                    }
                }
                Err(e) => panic!("{e}"),
            }
        }
        stalled_tx.send(sent).unwrap();
        s.stream.set_nonblocking(false).unwrap();
        s.stream.write_all(&wire[sent..]).unwrap();
        s.send(&ServerMsg::Exit { reason: ExitReason::Detached, message: String::new() });
    });
    let cl = client(path.clone(), pty.fds(), Size::new(80, 24));
    let sent = stalled_rx.recv_timeout(TIMEOUT).expect("server never stalled");
    println!("server stalled after {sent} bytes with the terminal not reading");
    assert!(sent < 1024 * 1024, "client buffered {sent} bytes for a stuck terminal");
    let got = read_master(pty.master.as_raw_fd(), |g| g.len() >= LEN);
    join(srv, "server");
    join(cl, "client").unwrap();
    assert!(got == screen, "screen bytes differ");
    let _ = std::fs::remove_file(path);
}

#[test]
fn exit_restores_modes_then_prints_message() {
    let _g = shared();
    let pty = Pty::new(80, 24);
    let s = pty.slave.as_raw_fd();
    let before = modes(s);
    let (path, l) = listen();
    let srv = serve(l, |mut s| {
        s.attach();
        s.send(&ServerMsg::Output(b"\x1b[?1049lbye".to_vec()));
        s.send(&ServerMsg::Exit { reason: ExitReason::SessionEnded, message: "[exited]".into() });
    });
    let exit = join(client(path.clone(), pty.fds(), Size::new(80, 24)), "client").unwrap();
    join(srv, "server");
    assert_eq!(exit, ExitInfo { cause: ExitCause::Server, reason: ExitReason::SessionEnded, message: "[exited]".into() });
    assert_eq!(modes(s), before);
    // Output processing is back on, so the message's newline became CRLF.
    let got = read_master(pty.master.as_raw_fd(), |g| g.ends_with(b"\r\n"));
    assert_eq!(got, b"\x1b[?1049lbye[exited]\r\n");
    let _ = std::fs::remove_file(path);
}

#[test]
fn abrupt_server_close_is_an_error_and_restores_modes() {
    let _g = shared();
    let pty = Pty::new(80, 24);
    let s = pty.slave.as_raw_fd();
    let before = modes(s);
    let (path, l) = listen();
    let srv = serve(l, |mut s| {
        s.attach();
        // Half a frame, then gone.
        s.send(&ServerMsg::Output(b"partial".to_vec()));
        s.stream.write_all(&[100, 0, 0, 0, 0x04, 0x01, 1]).unwrap();
    });
    let res = join(client(path.clone(), pty.fds(), Size::new(80, 24)), "client");
    join(srv, "server");
    let err = res.unwrap_err();
    assert_eq!(err.kind(), std::io::ErrorKind::UnexpectedEof);
    assert_eq!(modes(s), before);
    assert_eq!(read_master(pty.master.as_raw_fd(), |g| g.len() >= 7), b"partial");
    let _ = std::fs::remove_file(path);
}

#[test]
fn sigwinch_sends_resize() {
    let _g = exclusive();
    let pty = Pty::new(100, 30);
    let (path, l) = listen();
    let (ready_tx, ready_rx) = mpsc::channel::<()>();
    let srv = serve(l, move |mut s| {
        s.attach();
        // Hello said 80x24 but the terminal is 100x30 by the time we attach.
        assert_eq!(s.recv(), ClientMsg::Resize { size: Size::new(100, 30), pixels: PixelSize { width: 800, height: 480 } });
        s.send(&ServerMsg::Output(b"ready".to_vec()));
        ready_rx.recv_timeout(TIMEOUT).unwrap();
        let m = s.recv();
        s.send(&ServerMsg::Exit { reason: ExitReason::Detached, message: String::new() });
        m
    });
    let cl = client(path.clone(), pty.fds(), Size::new(80, 24));
    read_master(pty.master.as_raw_fd(), |g| g == b"ready");
    pty.set_size(132, 43, 1056, 688);
    // SAFETY: the loop's handler is installed (it has relayed output).
    unsafe { libc::kill(libc::getpid(), libc::SIGWINCH) };
    ready_tx.send(()).unwrap();
    let m = join(srv, "server");
    assert_eq!(m, ClientMsg::Resize { size: Size::new(132, 43), pixels: PixelSize { width: 1056, height: 688 } });
    assert_eq!(join(cl, "client").unwrap().cause, ExitCause::Server);
    let _ = std::fs::remove_file(path);
}

#[test]
fn sigterm_detaches_and_restores_modes() {
    let _g = exclusive();
    for sig in [libc::SIGTERM, libc::SIGHUP] {
        let pty = Pty::new(80, 24);
        let s = pty.slave.as_raw_fd();
        let before = modes(s);
        let (path, l) = listen();
        let srv = serve(l, |mut s| {
            s.attach();
            s.send(&ServerMsg::Output(b"ready".to_vec()));
            s.recv()
        });
        let cl = client(path.clone(), pty.fds(), Size::new(80, 24));
        read_master(pty.master.as_raw_fd(), |g| g == b"ready");
        // SAFETY: the loop's handler is installed (it has relayed output), and
        // the exclusive lock keeps every other loop out.
        unsafe { libc::kill(libc::getpid(), sig) };
        assert_eq!(join(srv, "server"), ClientMsg::Detach);
        let exit = join(cl, "client").unwrap();
        assert_eq!(exit, ExitInfo { cause: ExitCause::Signal(sig), reason: ExitReason::Detached, message: String::new() });
        assert_eq!(modes(s), before);
        let _ = std::fs::remove_file(path);
    }
}

#[test]
fn terminal_hangup_detaches() {
    let _g = shared();
    let pty = Pty::new(80, 24);
    let (path, l) = listen();
    let srv = serve(l, |mut s| {
        s.attach();
        s.send(&ServerMsg::Output(b"ready".to_vec()));
        loop {
            match s.recv() {
                ClientMsg::Resize { .. } => {}
                m => return m,
            }
        }
    });
    let cl = client(path.clone(), pty.fds(), Size::new(80, 24));
    read_master(pty.master.as_raw_fd(), |g| g == b"ready");
    let Pty { master, slave } = pty;
    drop(master);
    assert_eq!(join(srv, "server"), ClientMsg::Detach);
    assert_eq!(join(cl, "client").unwrap().cause, ExitCause::InputClosed);
    drop(slave);
    let _ = std::fs::remove_file(path);
}

#[test]
fn socket_path_rules() {
    let os = |s: &str| Some(std::ffi::OsString::from(s));
    assert_eq!(
        resolve_socket_path(os("/x/my.sock"), os("/run/user/5"), os("/t"), 5),
        SocketPath { socket: "/x/my.sock".into(), private_dir: None }
    );
    assert_eq!(
        resolve_socket_path(None, os("/run/user/5"), os("/t"), 5),
        SocketPath { socket: "/run/user/5/lif/default".into(), private_dir: Some("/run/user/5/lif".into()) }
    );
    assert_eq!(
        resolve_socket_path(os(""), os(""), os("/var/folders/T/"), 501),
        SocketPath { socket: "/var/folders/T/lif-501/default".into(), private_dir: Some("/var/folders/T/lif-501".into()) }
    );
    assert_eq!(
        resolve_socket_path(None, None, None, 0),
        SocketPath { socket: "/tmp/lif-0/default".into(), private_dir: Some("/tmp/lif-0".into()) }
    );
}

#[test]
fn secure_dir_creates_and_refuses() {
    use std::os::unix::fs::PermissionsExt;
    let base = std::env::temp_dir().join(format!("lif-ct-dir-{}", std::process::id()));
    let _ = std::fs::remove_dir_all(&base);
    std::fs::create_dir(&base).unwrap();

    let d = base.join("new");
    secure_dir(&d).unwrap();
    assert_eq!(std::fs::metadata(&d).unwrap().permissions().mode() & 0o777, 0o700);
    secure_dir(&d).unwrap();

    std::fs::set_permissions(&d, std::fs::Permissions::from_mode(0o750)).unwrap();
    assert_eq!(secure_dir(&d).unwrap_err().kind(), std::io::ErrorKind::PermissionDenied);

    let f = base.join("file");
    std::fs::write(&f, b"").unwrap();
    assert!(secure_dir(&f).is_err());

    let target = base.join("target");
    secure_dir(&target).unwrap();
    let link = base.join("link");
    std::os::unix::fs::symlink(&target, &link).unwrap();
    assert_eq!(secure_dir(&link).unwrap_err().kind(), std::io::ErrorKind::PermissionDenied);

    std::fs::remove_dir_all(&base).unwrap();
}

