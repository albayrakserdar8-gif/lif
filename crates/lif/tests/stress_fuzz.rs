//! A randomized stress run against a private server, kept for running on
//! demand (it takes minutes):
//!
//! LIF_STRESS_SECS=180 cargo test -p lif --test stress_fuzz -- --ignored --nocapture
//!
//! Random CLI commands (hostile arguments included), attached clients
//! typing, pasting, clicking, resized to degenerate sizes, stopped and
//! continued, killed and re-attached, raw protocol clients sending junk,
//! and panes spewing hostile output. At the end the server must be the
//! same process, answering, with bounded memory. LIF_STRESS_SEED replays
//! a run.
mod common;

use std::io::Write;
use std::os::unix::net::UnixStream;
use std::time::{Duration, Instant};

use common::*;
use lif_types::Size;

/// Panes kept at most (the run kills windows above it).
const MAX_PANES: usize = 30;

struct Rng(u64);

impl Rng {
    fn next(&mut self) -> u64 {
        // xorshift64*
        self.0 ^= self.0 >> 12;
        self.0 ^= self.0 << 25;
        self.0 ^= self.0 >> 27;
        self.0.wrapping_mul(0x2545_f491_4f6c_dd1d)
    }
    fn below(&mut self, n: u64) -> u64 {
        self.next() % n.max(1)
    }
    fn pick<'a, T>(&mut self, v: &'a [T]) -> &'a T {
        &v[self.below(v.len() as u64) as usize]
    }
    fn chance(&mut self, percent: u64) -> bool {
        self.below(100) < percent
    }
}

const PROGRAMS: &[&str] = &[
    "exec sh",
    "stty raw -echo; exec sleep 100000",
    "yes \"$(printf '\\033[6n')\"",
    "while :; do head -c 4096 /dev/urandom; sleep 0.05; done",
    "while :; do printf '\\033]52;c;%s\\007\\033]0;%s\\007\\a' $(head -c 3000 /dev/urandom | base64 | tr -d '\\n') t$$; sleep 0.2; done",
    "yes",
    "exec cat",
    "sleep 0.5; exit 3",
];

fn random_target(r: &mut Rng) -> String {
    let s = *r.pick(&["s0", "s1", "s2", "nosuch", ""]);
    let w = *r.pick(&["", ":0", ":1", ":2", ":+", ":-", ":4294967295", ":2147483647", ":-1"]);
    let p = *r.pick(&["", ".0", ".1", ".+", ".-", ".9999", ".%0"]);
    format!("{s}{w}{p}")
}

fn random_format(r: &mut Rng) -> String {
    let parts = [
        "#{session_name}",
        "#{p99999999999:session_name}",
        "#{=-9223372036854775808:session_name}",
        "#{e|/|:-9223372036854775808,-1}",
        "#{e|%|:-9223372036854775808,-1}",
        "#{e|*|:9223372036854775807,9223372036854775807}",
        "#{E:@x}",
        "#{T:@x}",
        "#{s/a*/bb/:@x}",
        "#{?pane_active,#{window_index},#{pane_index}}",
        "#{t:@x}",
        "#{l:#{E:@x}}",
        "%H:%M #{host}",
    ];
    let n = 1 + r.below(4);
    (0..n).map(|_| *r.pick(&parts)).collect::<Vec<_>>().join("x")
}

fn random_command(r: &mut Rng) -> Vec<String> {
    let t = random_target(r);
    let s = |v: &[&str]| v.iter().map(|x| x.to_string()).collect::<Vec<_>>();
    let prog = r.pick(PROGRAMS).to_string();
    match r.below(30) {
        0 => s(&["new-window", "-d", "-t", &t, &prog]),
        1 => s(&["split-window", "-d", "-t", &t, if r.chance(50) { "-h" } else { "-v" }, &prog]),
        2 => s(&["kill-pane", "-t", &t]),
        3 => s(&["kill-window", "-t", &t]),
        4 => s(&["respawn-pane", "-k", "-t", &t, &prog]),
        5 => s(&["break-pane", "-d", "-t", &t]),
        6 => s(&["move-window", "-s", &t, "-t", &random_target(r)]),
        7 => s(&["swap-window", "-s", &t, "-t", &random_target(r)]),
        8 => s(&["swap-pane", "-s", &t, "-t", &random_target(r)]),
        9 => s(&["select-layout", "-t", &t, r.pick(&["tiled", "even-horizontal", "main-vertical", "a3f2,80x24,0,0{40x24,0,0,1,39x24,41,0[39x12,41,0,2,39x11,41,13,3]}", "garbage", "ffff,99999x99999,0,0,1"])]),
        10 => s(&["resize-pane", "-t", &t, r.pick(&["-L", "-R", "-U", "-D", "-Z"]), r.pick(&["1", "100", "4294967295", "99999999999"])]),
        11 => s(&["resize-pane", "-t", &t, "-x", r.pick(&["1", "50%", "4294967295", "0"]), "-y", r.pick(&["1", "100%", "65535"])]),
        12 => s(&["display-message", "-p", "-t", &t, &random_format(r)]),
        13 => s(&["set-option", "-g", "@x", &random_format(r)]),
        14 => s(&["set-option", "-g", r.pick(&["status-left", "status-right", "window-status-format"]), &random_format(r)]),
        15 => s(&["send-keys", "-t", &t, "-N", r.pick(&["1", "100", "5000000", "4294967295"]), r.pick(&["echo hi", "Enter", "C-c", "x"])]),
        16 => s(&["send-keys", "-t", &t, "-l", &"z".repeat(r.below(300_000) as usize)]),
        17 => s(&["send-keys", "-X", "-t", &t, "-N", r.pick(&["3", "4294967295"]), r.pick(&["cursor-up", "search-again", "page-down", "next-word"])]),
        18 => s(&["copy-mode", "-t", &t]),
        19 => s(&["capture-pane", "-p", "-t", &t, "-S", r.pick(&["-", "-100", "-9223372036854775808"]), "-E", r.pick(&["-", "10"])]),
        20 => s(&["new-session", "-d", "-s", r.pick(&["s0", "s1", "s2"]), "-x", r.pick(&["80", "1", "10000", "20000"]), "-y", r.pick(&["24", "1", "65535"]), &prog]),
        21 => s(&["kill-session", "-t", r.pick(&["s0", "s1", "s2"])]),
        22 => s(&["new-window", "-t", r.pick(&["s0:4294967295", "s1:2147483647", "s0:0"]), "-a", &prog]),
        23 => s(&["paste-buffer", "-t", &t]),
        24 => s(&["set-buffer", &"b".repeat(r.below(200_000) as usize)]),
        25 => s(&["source-file", r.pick(&["/dev/null", "/dev/zero", "/nonexistent", "/etc/hosts"])]),
        26 => s(&["rename-window", "-t", &t, &"n".repeat(r.below(5000) as usize)]),
        27 => s(&["list-windows", "-a", "-F", &random_format(r)]),
        28 => s(&["select-pane", "-t", &t]),
        _ => s(&["switch-client", "-t", &t]),
    }
}

/// Random terminal input: keys, prefix bindings, pastes, mouse, junk.
fn random_input(r: &mut Rng) -> Vec<u8> {
    match r.below(9) {
        0 => b"echo hello\r".to_vec(),
        1 => vec![0x02, *r.pick(b"cnp%\"o[]z{}!xq0123 ")],
        2 => {
            let mut v = b"\x1b[200~".to_vec();
            v.extend(std::iter::repeat_n(b'p', r.below(64 * 1024) as usize));
            if r.chance(80) {
                v.extend_from_slice(b"\x1b[201~");
            }
            v
        }
        3 => format!("\x1b[<{};{};{}{}", r.pick(&[0, 1, 2, 32, 64, 65, 35]), r.below(300), r.below(100), r.pick(&["M", "m"])).into_bytes(),
        4 => (0..r.below(200)).map(|_| r.next() as u8).collect(),
        5 => b"\x1b[A\x1b[B\x1b[5~\x1b[6~q".to_vec(),
        6 => b"\x1b[I\x1b[O\x1b".to_vec(),
        7 => b"\x02:display-message hi\r".to_vec(),
        _ => b"\x1b]11;rgb:0000/0000/0000\x1b\\\x1b[?62;22c".to_vec(),
    }
}

fn spawn_client(env: &Env, r: &mut Rng) -> Term {
    let size = Size::new(*r.pick(&[1, 2, 10, 80, 200]), *r.pick(&[1, 2, 5, 24, 60]));
    let target = r.pick(&["s0", "s1", "s2"]).to_string();
    let mut t = Term::spawn(env, &["attach-session", "-t", &target], size);
    t.pump(Duration::from_millis(20));
    t
}

/// `lif` run in the test's own directory: random keys typed into shells
/// create files where panes start, which is where their client ran.
fn run(env: &Env, args: &[&str]) -> std::process::Output {
    env.command().current_dir(&env.dir).args(args).output().expect("run lif")
}

fn ok(env: &Env, args: &[&str]) -> String {
    let o = run(env, args);
    assert!(o.status.success(), "lif {args:?} failed: {}", String::from_utf8_lossy(&o.stderr));
    String::from_utf8(o.stdout).unwrap()
}

fn alive(pid: u32) -> bool {
    // SAFETY: probing a pid this test recorded.
    unsafe { libc::kill(pid as i32, 0) == 0 }
}

#[test]
#[ignore]
fn random_stress() {
    let secs: u64 = std::env::var("LIF_STRESS_SECS").ok().and_then(|s| s.parse().ok()).unwrap_or(60);
    let seed: u64 = std::env::var("LIF_STRESS_SEED").ok().and_then(|s| s.parse().ok()).unwrap_or_else(|| {
        std::time::SystemTime::now().duration_since(std::time::UNIX_EPOCH).unwrap().as_nanos() as u64 | 1
    });
    eprintln!("stress: {secs} s, LIF_STRESS_SEED={seed}");
    let mut r = Rng(seed);
    let env = Env::new();
    ok(&env, &["new-session", "-d", "-s", "s0", "exec sh"]);
    ok(&env, &["set-option", "-g", "exit-empty", "off"]);
    // Memory the user asks for (scrollback of many flooding panes) is not
    // what this measures: keep it small, and the number of panes too.
    ok(&env, &["set-option", "-g", "scrollback-bytes", "1048576"]);
    ok(&env, &["set-option", "-g", "history-limit", "1000"]);
    ok(&env, &["new-session", "-d", "-s", "s1", PROGRAMS[2]]);
    ok(&env, &["new-session", "-d", "-s", "s2", PROGRAMS[3]]);
    let pid = env.server_pid();
    let rss_start = rss_kib(pid);
    let mut clients: Vec<Term> = Vec::new();
    let mut raws: Vec<UnixStream> = Vec::new();
    let (mut commands, mut failed, mut inputs, mut max_rss) = (0u64, 0u64, 0u64, rss_start);
    let end = Instant::now() + Duration::from_secs(secs);
    let mut last_check = Instant::now();
    while Instant::now() < end {
        // Keep something to work on: random commands kill sessions.
        if commands % 25 == 0 {
            for s in ["s0", "s1", "s2"] {
                if !run(&env, &["has-session", "-t", s]).status.success() {
                    let prog = r.pick(PROGRAMS).to_string();
                    let _ = run(&env, &["new-session", "-d", "-s", s, &prog]);
                    let _ = run(&env, &["split-window", "-d", "-t", s, "exec sh"]);
                }
            }
        }
        match r.below(10) {
            0..=3 => {
                let argv = random_command(&mut r);
                let o = env.command().current_dir(&env.dir).args(&argv).stdout(std::process::Stdio::null()).stderr(std::process::Stdio::null()).status().unwrap();
                commands += 1;
                if !o.success() {
                    failed += 1;
                }
            }
            4..=6 if !clients.is_empty() => {
                let i = r.below(clients.len() as u64) as usize;
                let bytes = random_input(&mut r);
                let t = &mut clients[i];
                let mut off = 0;
                let stop = Instant::now() + Duration::from_millis(200);
                while off < bytes.len() && Instant::now() < stop {
                    match t.pty.write(&bytes[off..]) {
                        Ok(n) => off += n,
                        Err(_) => {
                            t.pump(Duration::from_millis(2));
                        }
                    }
                }
                inputs += 1;
            }
            7 if !clients.is_empty() => {
                let i = r.below(clients.len() as u64) as usize;
                let size = Size::new(*r.pick(&[1, 2, 3, 40, 80, 300]), *r.pick(&[1, 2, 3, 24, 100]));
                clients[i].resize(size);
            }
            8 if !clients.is_empty() => {
                let i = r.below(clients.len() as u64) as usize;
                let cpid = clients[i].pty.pid() as i32;
                match r.below(4) {
                    // SAFETY: signals to client processes this test started.
                    0 => unsafe {
                        libc::kill(cpid, libc::SIGSTOP);
                    },
                    // SAFETY: as above.
                    1 => unsafe {
                        libc::kill(cpid, libc::SIGCONT);
                    },
                    // SAFETY: as above.
                    2 => unsafe {
                        libc::kill(cpid, libc::SIGTSTP);
                    },
                    _ => {
                        clients.swap_remove(i);
                    }
                }
            }
            _ => {
                if clients.len() < 3 && r.chance(60) {
                    clients.push(spawn_client(&env, &mut r));
                } else if let Ok(mut s) = UnixStream::connect(&env.sock) {
                    // Junk frames, sometimes after a valid Hello.
                    let mut b = Vec::new();
                    if r.chance(50) {
                        let _ = lif_proto::ClientMsg::Hello(lif_proto::Hello { protocol: 1, is_tty: true, ..Default::default() }).encode(&mut b);
                    }
                    b.extend((0..r.below(4096)).map(|_| r.next() as u8));
                    let _ = s.set_nonblocking(true);
                    let _ = s.write(&b);
                    raws.push(s);
                    if raws.len() > 20 {
                        raws.remove(0);
                    }
                }
            }
        }
        for t in clients.iter_mut() {
            t.pump(Duration::ZERO);
            // A client stopped by its own suspend has no shell to continue it.
            if r.chance(5) {
                // SAFETY: as above.
                unsafe { libc::kill(t.pty.pid() as i32, libc::SIGCONT) };
            }
        }
        clients.retain(|t| t.exit.is_none());
        if last_check.elapsed() > Duration::from_secs(5) {
            last_check = Instant::now();
            assert!(alive(pid), "the server died (seed {seed})");
            let t = Instant::now();
            let o = run(&env, &["show-options", "-gv", "exit-empty"]);
            assert_eq!(String::from_utf8_lossy(&o.stdout), "off\n", "server not answering (seed {seed}): {}", String::from_utf8_lossy(&o.stderr));
            let answer = t.elapsed();
            let panes = ok(&env, &["list-panes", "-a", "-F", "#{session_name}:#{window_index}"]);
            if panes.lines().count() > MAX_PANES {
                for w in panes.lines().skip(MAX_PANES / 2) {
                    let _ = run(&env, &["kill-window", "-t", w]);
                }
            }
            let rss = rss_kib(pid);
            max_rss = max_rss.max(rss);
            eprintln!(
                "stress: {commands} commands ({failed} failed), {inputs} inputs, {} clients, {} panes, answer {answer:?}, rss {rss} KiB",
                clients.len(),
                panes.lines().count(),
            );
        }
    }
    for t in clients.iter_mut() {
        // SAFETY: as above.
        unsafe { libc::kill(t.pty.pid() as i32, libc::SIGCONT) };
    }
    drop(clients);
    drop(raws);
    assert!(alive(pid), "the server died (seed {seed})");
    assert_eq!(env.server_pid(), pid);
    let t = Instant::now();
    assert_eq!(ok(&env, &["show-options", "-gv", "exit-empty"]), "off\n");
    assert!(t.elapsed() < Duration::from_secs(2), "slow answer: {:?}", t.elapsed());
    let rss = rss_kib(pid);
    eprintln!("stress done: {commands} commands ({failed} failed), {inputs} inputs; rss start {rss_start} KiB, max {max_rss} KiB, end {rss} KiB");
    assert!(max_rss < 512 * 1024, "server rss reached {max_rss} KiB (seed {seed})");
    ok(&env, &["kill-server"]);
}
