//! Exercises the driver binary end to end on real PTYs.

use std::io::{BufRead, BufReader, Write};
use std::process::{Child, ChildStdin, ChildStdout, Command, Stdio};
use std::time::{Duration, Instant};

const BIN: &str = env!("CARGO_BIN_EXE_lifbench-driver");

fn tmpdir(name: &str) -> String {
    let d = format!("{}/{name}-{}", env!("CARGO_TARGET_TMPDIR"), std::process::id());
    let _ = std::fs::remove_dir_all(&d);
    std::fs::create_dir_all(&d).unwrap();
    d
}

struct Hold {
    child: Child,
    stdin: ChildStdin,
    out: BufReader<ChildStdout>,
}

impl Hold {
    fn start(argv: &[&str]) -> (Hold, String) {
        let mut child = Command::new(BIN)
            .args(["hold", "--cols", "100", "--rows", "30", "--"])
            .args(argv)
            .env("PS1", "$ ")
            .env("TERM", "xterm-256color")
            .stdin(Stdio::piped())
            .stdout(Stdio::piped())
            .spawn()
            .unwrap();
        let stdin = child.stdin.take().unwrap();
        let mut out = BufReader::new(child.stdout.take().unwrap());
        let mut hello = String::new();
        out.read_line(&mut hello).unwrap();
        (Hold { child, stdin, out }, hello)
    }

    fn cmd(&mut self, line: &str) -> String {
        writeln!(self.stdin, "{line}").unwrap();
        self.stdin.flush().unwrap();
        let mut r = String::new();
        self.out.read_line(&mut r).unwrap();
        assert!(!r.is_empty(), "driver closed stdout on {line}");
        r
    }
}

fn hex(s: &str) -> String {
    s.bytes().map(|b| format!("{b:02x}")).collect()
}

/// Pulls a numeric field out of a one-line JSON reply.
fn num(json: &str, key: &str) -> f64 {
    let k = format!("\"{key}\":");
    let i = json.find(&k).unwrap_or_else(|| panic!("{key} missing in {json}")) + k.len();
    json[i..].chars().take_while(|c| c.is_ascii_digit() || *c == '.').collect::<String>().parse().unwrap()
}

fn samples(json: &str) -> Vec<f64> {
    let i = json.find("\"samples_us\":[").unwrap() + 14;
    let j = json[i..].find(']').unwrap() + i;
    json[i..j].split(',').filter(|s| !s.is_empty()).map(|s| s.parse().unwrap()).collect()
}

#[test]
fn shell_session_ready_run_and_latency() {
    let dir = tmpdir("shell");
    let (mut h, hello) = Hold::start(&["/bin/sh"]);
    assert!(hello.contains("\"pid\":"), "{hello}");

    let r = h.cmd(&format!("ready {dir}/ready 10000 200"));
    assert!(r.contains("\"ok\":true"), "{r}");

    // 200000 lines of output: the marker must only appear after all of it.
    let marker = format!("{dir}/done");
    let r = h.cmd(&format!("run {marker} 60000 100 {}", hex(&format!("seq 1 200000; touch {marker}\r"))));
    assert!(r.contains("\"ok\":true"), "{r}");
    let bytes = num(&r, "bytes_at_done");
    // seq 1 200000 prints 1288895 bytes; the tty turns each \n into \r\n.
    assert!(bytes >= 1_288_895.0 + 200_000.0, "only {bytes} bytes before the marker: {r}");
    assert!(num(&r, "secs") > 0.0);

    // cat with tty echo: every sample must be seen, and quickly.
    h.cmd(&format!("type {}", hex("exec cat\r")));
    h.cmd("quiet 200 5000");
    let r = h.cmd("latency echo 100 7b7d7e5e60 2 2000");
    assert!(r.contains("\"timeouts\":0"), "{r}");
    let mut s = samples(&r);
    assert_eq!(s.len(), 100);
    s.sort_by(f64::total_cmp);
    assert!(s[50] < 5_000.0, "median echo {} us on a bare PTY", s[50]);

    let st = h.cmd("stats");
    assert!(st.contains("\"alive\":true"), "{st}");
    let q = h.cmd("quit 2000");
    assert!(q.contains("\"ok\":true"), "{q}");
    assert!(h.child.wait().unwrap().success());
}

#[test]
fn esc_probe_reports_bare_escape() {
    let dir = tmpdir("esc");
    let (mut h, _) = Hold::start(&["/bin/sh"]);
    h.cmd(&format!("ready {dir}/ready 10000 200"));
    let probe_ready = format!("{dir}/probe");
    let r = h.cmd(&format!("run {probe_ready} 10000 100 {}", hex(&format!("exec {BIN} escprobe {probe_ready}\r"))));
    assert!(r.contains("\"ok\":true"), "{r}");
    let r = h.cmd("latency esc 30 7e 5 2000");
    assert!(r.contains("\"timeouts\":0"), "{r}");
    assert_eq!(samples(&r).len(), 30);
    h.cmd("quit 2000");
    h.child.wait().unwrap();
}

#[test]
fn run_reports_timeout() {
    let dir = tmpdir("timeout");
    let (mut h, _) = Hold::start(&["/bin/sh"]);
    h.cmd(&format!("ready {dir}/ready 10000 200"));
    let r = h.cmd(&format!("run {dir}/never 300 50 {}", hex("sleep 5\r")));
    assert!(r.contains("\"ok\":false") && r.contains("timeout"), "{r}");
    h.cmd("quit 500");
    h.child.wait().unwrap();
}

#[test]
fn answers_terminal_queries() {
    // The client asks for device attributes and only finishes once answered.
    let script = "stty raw -echo; printf '\\033[c'; dd bs=1 count=9 2>/dev/null | od -c > \"$0\"; stty sane";
    let dir = tmpdir("answer");
    let out = format!("{dir}/reply");
    let (mut h, _) = Hold::start(&["/bin/sh", "-c", script, &out]);
    let t = Instant::now();
    while t.elapsed() < Duration::from_secs(5) && !h.cmd("stats").contains("\"alive\":false") {
        std::thread::sleep(Duration::from_millis(20));
    }
    let reply = std::fs::read_to_string(&out).unwrap();
    assert!(reply.contains("[   ?   6   2   ;   2   2   c"), "{reply}");
    h.cmd("quit 500");
    h.child.wait().unwrap();
}

#[test]
fn attach_detach_sends_keys_after_first_output() {
    // A stand-in client: draws, then exits once it reads the "detach" line.
    let st = Command::new(BIN)
        .args(["attach-detach", "--keys", &hex("q\r"), "--quiet-ms", "10", "--timeout-ms", "5000", "--"])
        .args(["/bin/sh", "-c", "printf 'drawn'; read x; [ \"$x\" = q ]"])
        .status()
        .unwrap();
    assert!(st.success());

    // A client that never reads its input times out and fails.
    let st = Command::new(BIN)
        .args(["attach-detach", "--keys", "71", "--timeout-ms", "500", "--", "/bin/sh", "-c", "printf x; sleep 5"])
        .status()
        .unwrap();
    assert!(!st.success());
}

#[test]
fn rusage_converts_cpu_time() {
    let mut child = Command::new("/bin/sh")
        .args(["-c", "i=0; while [ $i -lt 400000 ]; do i=$((i+1)); done; sleep 2"])
        .spawn()
        .unwrap();
    std::thread::sleep(Duration::from_millis(1500));
    let out = Command::new(BIN).args(["rusage", &child.id().to_string()]).output().unwrap();
    let ps = Command::new("ps").args(["-o", "time=", "-p", &child.id().to_string()]).output().unwrap();
    child.kill().unwrap();
    child.wait().unwrap();
    let j = String::from_utf8(out.stdout).unwrap();
    assert!(j.contains("\"ok\":true"), "{j}");
    let cpu_s = (num(&j, "user_ns") + num(&j, "sys_ns")) / 1e9;
    // ps prints [m]m:ss.cc
    let ps = String::from_utf8(ps.stdout).unwrap();
    let (m, s) = ps.trim().split_once(':').unwrap();
    let ps_s = m.parse::<f64>().unwrap() * 60.0 + s.parse::<f64>().unwrap();
    assert!(cpu_s > 0.05, "busy loop used only {cpu_s} s");
    assert!((cpu_s - ps_s).abs() < 0.05 + 0.1 * ps_s, "rusage {cpu_s} s vs ps {ps_s} s");
    assert!(num(&j, "rss") > 100_000.0 && num(&j, "footprint") > 0.0, "{j}");
}

#[test]
fn waitfile_waits() {
    let dir = tmpdir("waitfile");
    let p = format!("{dir}/f");
    let p2 = p.clone();
    let t = std::thread::spawn(move || {
        std::thread::sleep(Duration::from_millis(100));
        std::fs::write(p2, b"").unwrap();
    });
    assert!(Command::new(BIN).args(["waitfile", &p, "5000"]).status().unwrap().success());
    t.join().unwrap();
    assert!(!Command::new(BIN).args(["waitfile", &format!("{dir}/nope"), "50"]).status().unwrap().success());
}
