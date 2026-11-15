//! Job control and the terminal the server uses directly: a stopped
//! client's terminal belongs to its shell again, never to the server.
mod common;

use std::ffi::OsString;
use std::os::fd::AsRawFd;
use std::time::Duration;

use common::*;
use lif_types::Size;

/// zsh with job control in a PTY, running a lif client.
fn shell_with_client(env: &Env, session: &str) -> Term {
    let argv: Vec<OsString> = vec!["/bin/zsh".into(), "-f".into(), "-i".into()];
    let mut t = Term::spawn_argv(env, argv, Size::new(80, 24), &[]);
    t.wait_for("zsh", secs(10), |t| !t.screen().trim().is_empty());
    t.type_str("PS1='ZSH%# '; unsetopt PROMPT_SP\r");
    t.wait_text("ZSH%");
    let cmd = format!("{} -S {} new-session -s {session}\r", BIN, env.sock.display());
    t.type_str(&cmd);
    t.wait_for("the pane's prompt", secs(10), |t| t.modes().alt_screen && t.screen().contains("$"));
    t.type_str("echo PANE''MARK\r");
    t.wait_text("PANEMARK");
    t.settle(Duration::from_millis(100));
    t
}

fn client_pid(env: &Env) -> i32 {
    env.ok(&["list-clients", "-F", "#{client_pid}"]).trim().parse().expect("one client")
}

fn signal(pid: i32, sig: i32) {
    // SAFETY: signalling the lif client this test started.
    unsafe { libc::kill(pid, sig) };
}

fn pane_text(env: &Env, target: &str) -> String {
    String::from_utf8_lossy(&env.run(&["capture-pane", "-p", "-t", target]).stdout).into_owned()
}

/// Waits for zsh to have the terminal again: its prompt after the job
/// report.
fn wait_shell_prompt(t: &mut Term) {
    t.wait_for("the shell's prompt", secs(10), |t| {
        let s = t.screen();
        s.contains("suspended") && s.trim_end().ends_with("ZSH%")
    });
    t.settle(Duration::from_millis(100));
}

/// Types a command at the shell and checks it ran there and that nothing
/// of it reached the pane.
fn shell_owns_the_keys(env: &Env, t: &mut Term, session: &str, n: u32) {
    let want = format!("SHELL{}", 40 + n);
    t.type_str(&format!("echo SHELL$((40+{n}))\r"));
    t.wait_text(&want);
    std::thread::sleep(Duration::from_millis(300));
    let pane = pane_text(env, session);
    assert!(!pane.contains("SHELL") && !pane.contains("40+"), "keys typed at the shell reached the pane:\n{pane}");
}

/// After fg the client takes the terminal back and the screen is lif's
/// again, all of it.
fn fg_resumes(env: &Env, t: &mut Term, session: &str) {
    t.type_str("fg\r");
    t.wait_for("lif again", secs(10), |t| t.modes().alt_screen && t.screen().contains("PANEMARK") && !t.screen().contains("ZSH%"));
    t.type_str("echo BACK''AGAIN\r");
    t.wait_text("BACKAGAIN");
    assert!(pane_text(env, session).contains("BACKAGAIN"));
    t.write(b"\x02d");
    t.wait_for("detached", secs(10), |t| t.screen().contains("[detached"));
}

/// SIGSTOP cannot be caught: the server notices the client lost the
/// terminal's foreground and stops reading and drawing on it.
#[test]
fn a_client_stopped_by_sigstop_does_not_take_the_shells_keys() {
    let env = Env::new();
    let mut t = shell_with_client(&env, "stop");
    let pid = client_pid(&env);
    signal(pid, libc::SIGSTOP);
    wait_shell_prompt(&mut t);
    shell_owns_the_keys(&env, &mut t, "stop", 2);
    // Output in the pane meanwhile is not drawn over the shell.
    env.ok(&["send-keys", "-t", "stop", "echo DRAWN''LATER", "Enter"]);
    std::thread::sleep(Duration::from_millis(300));
    t.settle(Duration::from_millis(100));
    assert!(!t.screen().contains("DRAWNLATER"), "the server drew over the shell:\n{}", t.screen());
    fg_resumes(&env, &mut t, "stop");
}

/// SIGTSTP (job control's stop): the client gives the terminal back
/// restored (main screen, cooked mode), then stops.
#[test]
fn a_client_stopped_by_sigtstp_restores_the_terminal() {
    let env = Env::new();
    let mut t = shell_with_client(&env, "tstp");
    let pid = client_pid(&env);
    signal(pid, libc::SIGTSTP);
    wait_shell_prompt(&mut t);
    assert!(!t.modes().alt_screen, "the terminal was not restored");
    shell_owns_the_keys(&env, &mut t, "tstp", 3);
    fg_resumes(&env, &mut t, "tstp");
}

/// suspend-client, bound to prefix C-z as in tmux.
#[test]
fn prefix_ctrl_z_suspends_the_client() {
    let env = Env::new();
    let mut t = shell_with_client(&env, "susp");
    t.write(b"\x02\x1a");
    wait_shell_prompt(&mut t);
    assert!(!t.modes().alt_screen, "the terminal was not restored");
    shell_owns_the_keys(&env, &mut t, "susp", 4);
    fg_resumes(&env, &mut t, "susp");
}

/// A client continued in the background (bg) does not take the terminal:
/// only fg gives it back.
#[test]
fn a_client_continued_in_the_background_waits_for_fg() {
    let env = Env::new();
    let mut t = shell_with_client(&env, "bg");
    let pid = client_pid(&env);
    signal(pid, libc::SIGTSTP);
    wait_shell_prompt(&mut t);
    t.type_str("bg\r");
    std::thread::sleep(Duration::from_millis(500));
    shell_owns_the_keys(&env, &mut t, "bg", 5);
    fg_resumes(&env, &mut t, "bg");
}

/// A client detached while it is stopped (the shell owns the terminal):
/// nothing is written over the shell, and when fg continues the client
/// it restores the terminal and exits.
#[test]
fn a_client_detached_while_stopped_restores_the_terminal_on_fg() {
    let env = Env::new();
    let mut t = shell_with_client(&env, "det");
    let pid = client_pid(&env);
    signal(pid, libc::SIGSTOP);
    wait_shell_prompt(&mut t);
    env.ok(&["detach-client", "-s", "det"]);
    std::thread::sleep(Duration::from_millis(300));
    t.settle(Duration::from_millis(100));
    assert!(!t.screen().contains("[detached"), "the server wrote over the shell:\n{}", t.screen());
    t.type_str("fg\r");
    t.wait_for("client exit", secs(10), |t| t.screen().contains("[detached") && t.screen().trim_end().ends_with("ZSH%"));
    // (Bracketed paste is zsh's own, on while it reads a command.)
    let m = t.modes();
    assert!(!m.alt_screen && !m.mouse_tracking && !m.focus_events, "terminal not restored: {m:?}");
}

/// One terminal, one server reader: a second client naming a terminal
/// another client already uses is relayed, and the first keeps working.
#[test]
fn two_clients_cannot_both_hand_over_one_terminal() {
    use lif_proto::{Caps, ClientMsg, Decoder, Hello, PROTOCOL_VERSION, ServerMsg};
    use std::io::Write;
    let env = Env::new();
    let mut t = Term::spawn(&env, &["new-session", "-s", "two"], Size::new(80, 24));
    t.wait_text("$");
    // The first client's terminal, by path and device.
    let client = client_pid(&env);
    let tty = String::from_utf8(std::process::Command::new("ps").args(["-o", "tty=", "-p", &client.to_string()]).output().unwrap().stdout).unwrap();
    let path = format!("/dev/{}", tty.trim());
    let dev = {
        use std::os::unix::fs::MetadataExt;
        std::fs::metadata(&path).unwrap().rdev()
    };
    let mut s = std::os::unix::net::UnixStream::connect(&env.sock).unwrap();
    let mut out = Vec::new();
    let hello = Hello { protocol: PROTOCOL_VERSION, caps: Caps::TTY, pid: std::process::id(), size: Size::new(80, 24), is_tty: true, ..Default::default() };
    ClientMsg::Hello(hello).encode(&mut out).unwrap();
    ClientMsg::Command { argv: vec!["attach-session".into(), "-t".into(), "two".into()] }.encode(&mut out).unwrap();
    ClientMsg::UseTty { path, dev: dev as u64 }.encode(&mut out).unwrap();
    s.write_all(&out).unwrap();
    s.set_read_timeout(Some(Duration::from_secs(5))).unwrap();
    let mut dec: Decoder<ServerMsg> = Decoder::new();
    let direct = loop {
        match dec.next().unwrap() {
            Some(ServerMsg::Tty { direct }) => break direct,
            Some(_) => continue,
            None => {
                assert!(dec.read_from(&mut s).unwrap() > 0, "server closed the connection");
            }
        }
    };
    assert!(!direct, "the server took a terminal another client uses");
    t.type_str("echo STILL''MINE\r");
    t.wait_text("STILLMINE");
    let _ = s.as_raw_fd();
}

/// A terminal that hangs up while the server uses it ends that client's
/// attachment only; the server and the session carry on.
#[test]
fn a_terminal_hanging_up_detaches_only_its_client() {
    let env = Env::new();
    let mut t = Term::spawn(&env, &["new-session", "-s", "hup"], Size::new(80, 24));
    t.wait_text("$");
    t.settle(Duration::from_millis(100));
    let pid = env.server_pid();
    // Close the terminal's master side: the terminal hangs up.
    let master = t.pty.as_raw_fd();
    // SAFETY: closes the PTY master this test owns; the Term is not used
    // to read or write it afterwards (only to reap the client).
    unsafe { libc::dup2(open_null(), master) };
    assert!(env.wait(secs(10), || env.ok(&["list-clients"]).is_empty()), "client still attached");
    assert_eq!(env.server_pid(), pid);
    assert_eq!(env.ok(&["list-sessions", "-F", "#{session_name}"]), "hup\n");
}

fn open_null() -> i32 {
    // SAFETY: valid path.
    unsafe { libc::open(c"/dev/null".as_ptr(), libc::O_RDWR) }
}

/// The server dying while it owns the terminal: the client notices and
/// restores the terminal itself.
#[test]
fn a_client_restores_the_terminal_when_the_server_is_killed() {
    let env = Env::new();
    let mut t = Term::spawn(&env, &["new-session", "-s", "kill"], Size::new(80, 24));
    t.wait_text("$");
    t.write(b"\x1b[200~"); // nothing: just make sure the server reads
    t.settle(Duration::from_millis(100));
    let pid = env.server_pid() as i32;
    signal(pid, libc::SIGKILL);
    let code = t.wait_exit();
    assert_eq!(code, 1);
    let m = t.modes();
    assert!(!m.alt_screen && !m.bracketed_paste && !m.mouse_tracking, "terminal not restored: {m:?}");
    let _ = std::fs::remove_file(&env.sock);
}
