mod common;

use std::os::fd::AsRawFd;
use std::os::unix::process::ExitStatusExt;
use std::time::Instant;

use common::*;
use lif_pty::{ChildReaper, Pty, ReadOutcome, SpawnOptions};
use lif_types::{PixelSize, Size};

fn sh(script: &str) -> SpawnOptions {
    SpawnOptions::command(["/bin/sh", "-c", script])
}

#[test]
fn echo_round_trip() {
    let mut reaper = ChildReaper::new().unwrap();
    let mut pty = Pty::spawn(&SpawnOptions::command(["cat"])).unwrap();
    reaper.watch(pty.pid());
    let mut input: &[u8] = b"hello pty\n";
    while !input.is_empty() {
        let n = pty.write(input).unwrap();
        input = &input[n..];
    }
    // Once from the terminal's echo, once from cat.
    let out = read_until(&mut pty, b"hello pty\r\nhello pty\r\n");
    assert!(text(&out).contains("hello pty\nhello pty\n"));
    pty.write(&[4]).unwrap(); // ^D: cat sees EOF
    read_to_eof(&mut pty);
    assert_eq!(wait_exit(&mut reaper, pty.pid()).code(), Some(0));
}

#[test]
fn child_sees_size_and_resize() {
    let mut reaper = ChildReaper::new().unwrap();
    let mut opts = sh("stty size; echo READY; read x; stty size; echo DONE");
    opts.size = Size::new(91, 33);
    let mut pty = Pty::spawn(&opts).unwrap();
    reaper.watch(pty.pid());
    let out = read_until(&mut pty, b"READY");
    assert!(text(&out).contains("33 91\n"), "{:?}", text(&out));
    pty.resize(
        Size::new(120, 45),
        PixelSize {
            width: 1200,
            height: 900,
        },
    )
    .unwrap();
    pty.write(b"\n").unwrap();
    let out = read_until(&mut pty, b"DONE");
    assert!(text(&out).contains("45 120\n"), "{:?}", text(&out));
    read_to_eof(&mut pty);
    assert!(wait_exit(&mut reaper, pty.pid()).success());
}

#[test]
fn eof_and_exit_status() {
    let mut reaper = ChildReaper::new().unwrap();
    let mut pty = Pty::spawn(&sh("echo bye; exit 3")).unwrap();
    reaper.watch(pty.pid());
    let out = read_to_eof(&mut pty);
    assert_eq!(text(&out), "bye\n");
    // EOF is sticky.
    assert_eq!(pty.read(&mut [0u8; 16]).unwrap(), ReadOutcome::Eof);
    let status = wait_exit(&mut reaper, pty.pid());
    assert_eq!(status.code(), Some(3));
    assert!(reaper.is_empty());
}

#[test]
fn signalled_exit_status() {
    let mut reaper = ChildReaper::new().unwrap();
    let mut a = Pty::spawn(&sh("kill -TERM $$")).unwrap();
    reaper.watch(a.pid());
    read_to_eof(&mut a);
    assert_eq!(
        wait_exit(&mut reaper, a.pid()).signal(),
        Some(libc::SIGTERM)
    );

    let mut b = Pty::spawn(&SpawnOptions::command(["sleep", "30"])).unwrap();
    reaper.watch(b.pid());
    b.kill(libc::SIGKILL).unwrap();
    read_to_eof(&mut b);
    assert_eq!(
        wait_exit(&mut reaper, b.pid()).signal(),
        Some(libc::SIGKILL)
    );
}

#[test]
fn drop_hangs_up_child() {
    let mut reaper = ChildReaper::new().unwrap();
    let pty = Pty::spawn(&SpawnOptions::command(["sleep", "30"])).unwrap();
    let pid = pty.pid();
    reaper.watch(pid);
    drop(pty);
    assert_eq!(wait_exit(&mut reaper, pid).signal(), Some(libc::SIGHUP));
}

#[test]
fn cwd_env_and_term() {
    let dir = unique_dir("cwd-env");
    let mut reaper = ChildReaper::new().unwrap();
    let mut opts = sh(r#"pwd -P; echo "foo=$FOO"; echo "term=$TERM"; echo "home=${HOME-unset}""#);
    opts.cwd = Some(dir.clone());
    opts.env.push(("FOO".into(), "bar baz".into()));
    opts.env_remove.push("HOME".into());
    opts.term = "lif-test-term".into();
    let mut pty = Pty::spawn(&opts).unwrap();
    reaper.watch(pty.pid());
    let out = text(&read_to_eof(&mut pty));
    assert_eq!(
        out,
        format!(
            "{}\nfoo=bar baz\nterm=lif-test-term\nhome=unset\n",
            dir.display()
        )
    );
    assert!(wait_exit(&mut reaper, pty.pid()).success());
}

#[test]
fn missing_cwd_falls_back_to_home() {
    let home = unique_dir("fake-home");
    let mut reaper = ChildReaper::new().unwrap();
    let mut opts = sh("pwd -P");
    opts.cwd = Some("/nonexistent/lif/dir".into());
    opts.env.push(("HOME".into(), home.clone().into()));
    let mut pty = Pty::spawn(&opts).unwrap();
    reaper.watch(pty.pid());
    assert_eq!(
        text(&read_to_eof(&mut pty)),
        format!("{}\n", home.display())
    );
    wait_exit(&mut reaper, pty.pid());

    // No usable HOME either: "/".
    let mut opts = sh("pwd -P");
    opts.cwd = Some("/nonexistent/lif/dir".into());
    opts.env.push(("HOME".into(), "/nonexistent/home".into()));
    let mut pty = Pty::spawn(&opts).unwrap();
    reaper.watch(pty.pid());
    assert_eq!(text(&read_to_eof(&mut pty)), "/\n");
    wait_exit(&mut reaper, pty.pid());
}

#[test]
fn exec_failure_is_an_error() {
    let err = Pty::spawn(&SpawnOptions::command(["/nonexistent/program"])).unwrap_err();
    assert_eq!(err.kind(), std::io::ErrorKind::NotFound, "{err}");
    let err = Pty::spawn(&SpawnOptions::command(["lif-no-such-command-xyz"])).unwrap_err();
    assert_eq!(err.kind(), std::io::ErrorKind::NotFound, "{err}");
    assert!(err.to_string().contains("lif-no-such-command-xyz"), "{err}");
    // A directory is not executable.
    let err = Pty::spawn(&SpawnOptions::command(["/"])).unwrap_err();
    assert_eq!(err.kind(), std::io::ErrorKind::PermissionDenied, "{err}");
}

#[test]
fn child_has_controlling_tty() {
    let mut reaper = ChildReaper::new().unwrap();
    // Opening /dev/tty only works with a controlling terminal.
    let mut pty = Pty::spawn(&sh(
        "[ -t 0 ] && [ -t 1 ] && [ -t 2 ] && echo isatty; echo ctty >/dev/tty && tty",
    ))
    .unwrap();
    reaper.watch(pty.pid());
    let out = text(&read_to_eof(&mut pty));
    let lines: Vec<&str> = out.lines().collect();
    assert_eq!(lines.len(), 3, "{out:?}");
    assert_eq!(lines[0], "isatty");
    assert_eq!(lines[1], "ctty");
    assert!(lines[2].starts_with("/dev/"), "{out:?}");
    assert!(wait_exit(&mut reaper, pty.pid()).success());
}

#[test]
fn child_signal_state_and_fds_are_clean() {
    // A stray non-close-on-exec fd in the parent must not reach the child.
    // SAFETY: dup of stderr; closed below.
    let stray = unsafe { libc::dup(2) };
    assert!(stray > 2);
    let mut reaper = ChildReaper::new().unwrap();
    // The Rust runtime ignores SIGPIPE in this process; the child must not
    // inherit that, nor a blocked mask. ps shows the mask of the exec'd ps
    // (same pid); perl lists signals ignored on entry, except SIGFPE, which
    // perl ignores itself at startup.
    let mut pty = Pty::spawn(&sh(r#"exec ps -o sigmask= -p $$"#)).unwrap();
    reaper.watch(pty.pid());
    let mask = text(&read_to_eof(&mut pty));
    wait_exit(&mut reaper, pty.pid());
    let digits: String = mask.chars().filter(|c| !c.is_whitespace()).collect();
    assert!(
        !digits.is_empty() && digits.chars().all(|c| c == '0'),
        "{mask:?}"
    );
    let mut pty = Pty::spawn(&SpawnOptions::command([
        "perl",
        "-e",
        r#"for (sort keys %SIG) { print "$_ " if $_ ne "FPE" && defined $SIG{$_} && $SIG{$_} eq "IGNORE" } print "end\n""#,
    ]))
    .unwrap();
    reaper.watch(pty.pid());
    let ignored = text(&read_to_eof(&mut pty));
    wait_exit(&mut reaper, pty.pid());
    assert_eq!(ignored, "end\n");

    // Lists every fd from 3 up that is open in the exec'd program, without
    // opening any itself.
    let mut pty = Pty::spawn(&SpawnOptions::command([
        "perl",
        "-e",
        r#"for (3..4095) { print "$_ " if open(my $f, "<&=", $_) } print "end\n""#,
    ]))
    .unwrap();
    reaper.watch(pty.pid());
    let open = text(&read_to_eof(&mut pty));
    wait_exit(&mut reaper, pty.pid());
    // SAFETY: closing the fd dup'ed above.
    unsafe { libc::close(stray) };
    assert_eq!(open, "end\n");
}

#[test]
fn login_shell_by_default() {
    let mut reaper = ChildReaper::new().unwrap();
    let mut opts = SpawnOptions::default();
    opts.env.push(("SHELL".into(), "/bin/sh".into()));
    let mut pty = Pty::spawn(&opts).unwrap();
    reaper.watch(pty.pid());
    pty.write(b"echo \"arg0=$0\"; exit 0\n").unwrap();
    let out = text(&read_to_eof(&mut pty));
    assert!(out.contains("arg0=-sh\n"), "{out:?}");
    assert!(wait_exit(&mut reaper, pty.pid()).success());
}

#[test]
fn foreground_process_name_and_cwd() {
    let dir = unique_dir("fg");
    let mut reaper = ChildReaper::new().unwrap();
    let mut opts = SpawnOptions::command(["/bin/sh", "-i"]);
    opts.cwd = Some(dir.clone());
    opts.env.push(("PS1".into(), "$ ".into()));
    opts.env.push(("ENV".into(), "/dev/null".into()));
    let mut pty = Pty::spawn(&opts).unwrap();
    reaper.watch(pty.pid());
    read_until(&mut pty, b"$ ");
    let shell_name = pty.foreground_process_name().unwrap();
    assert!(
        ["sh", "bash", "dash"].contains(&shell_name.as_str()),
        "{shell_name}"
    );
    assert_eq!(pty.foreground_cwd().unwrap().canonicalize().unwrap(), dir);

    // An interactive shell gives the job its own foreground process group.
    pty.write(b"sleep 30\n").unwrap();
    let deadline = Instant::now() + TIMEOUT;
    let mut sink = [0u8; 4096];
    loop {
        if pty.foreground_process_name().as_deref() == Some("sleep") {
            break;
        }
        assert!(Instant::now() < deadline, "foreground never became sleep");
        let _ = pty.read(&mut sink);
        std::thread::sleep(std::time::Duration::from_millis(10));
    }
    assert_ne!(pty.foreground_pgrp(), Some(pty.pid()));
    let pid = pty.pid();
    drop(pty);
    wait_exit(&mut reaper, pid);
}

#[test]
fn would_block_when_idle() {
    let mut reaper = ChildReaper::new().unwrap();
    let mut pty = Pty::spawn(&SpawnOptions::command(["sleep", "30"])).unwrap();
    reaper.watch(pty.pid());
    assert_eq!(pty.read(&mut [0u8; 64]).unwrap(), ReadOutcome::WouldBlock);
    // SAFETY: F_GETFD/F_GETFL queries on the master fd.
    let (fd_flags, fl_flags) = unsafe {
        (
            libc::fcntl(pty.as_raw_fd(), libc::F_GETFD),
            libc::fcntl(pty.as_raw_fd(), libc::F_GETFL),
        )
    };
    assert!(fd_flags & libc::FD_CLOEXEC != 0);
    assert!(fl_flags & libc::O_NONBLOCK != 0);
    pty.kill(libc::SIGKILL).unwrap();
    wait_exit(&mut reaper, pty.pid());
}

#[test]
fn write_reports_would_block_when_full() {
    let mut reaper = ChildReaper::new().unwrap();
    // The child never reads, so the input queue fills up.
    let mut pty = Pty::spawn(&sh("stty -icanon -echo; echo READY; sleep 30")).unwrap();
    reaper.watch(pty.pid());
    read_until(&mut pty, b"READY");
    let chunk = [b'x'; 4096];
    let mut total = 0usize;
    let blocked = loop {
        match pty.write(&chunk) {
            Ok(n) => total += n,
            Err(e) if e.kind() == std::io::ErrorKind::WouldBlock => break true,
            Err(e) => panic!("{e}"),
        }
        if total > 64 << 20 {
            break false;
        }
    };
    assert!(blocked, "wrote {total} bytes without blocking");
    eprintln!("pty input queue accepted {total} bytes before EAGAIN");
    pty.kill(libc::SIGKILL).unwrap();
    wait_exit(&mut reaper, pty.pid());
}

#[test]
fn flood_50mb_in_order() {
    const LEN: usize = 50 << 20;
    let dir = unique_dir("flood");
    let path = dir.join("data.bin");
    let mut data = Vec::with_capacity(LEN);
    let mut x: u32 = 0x1234_5678;
    while data.len() < LEN {
        x ^= x << 13;
        x ^= x >> 17;
        x ^= x << 5;
        data.extend_from_slice(&x.to_le_bytes());
    }
    std::fs::write(&path, &data).unwrap();

    let mut reaper = ChildReaper::new().unwrap();
    // raw -opost: bytes pass through unchanged (no \n -> \r\n).
    let script = format!(
        "stty raw -echo -opost; echo READY; exec cat '{}'",
        path.display()
    );
    let mut pty = Pty::spawn(&sh(&script)).unwrap();
    reaper.watch(pty.pid());
    read_until(&mut pty, b"READY\n");
    let start = Instant::now();
    let deadline = start + std::time::Duration::from_secs(60);
    let mut out = Vec::with_capacity(LEN);
    let mut buf = vec![0u8; 64 * 1024];
    let mut reads = 0usize;
    let mut wakeups = 0usize;
    loop {
        match pty.read(&mut buf).unwrap() {
            ReadOutcome::Data(n) => {
                out.extend_from_slice(&buf[..n]);
                reads += 1;
            }
            ReadOutcome::WouldBlock => {
                wakeups += 1;
                assert!(
                    wait_readable(pty.as_raw_fd(), deadline),
                    "timed out after {} bytes",
                    out.len()
                );
            }
            ReadOutcome::Eof => break,
        }
    }
    let elapsed = start.elapsed();
    std::fs::remove_file(&path).unwrap();
    assert_eq!(out.len(), LEN);
    assert!(out == data, "data differs");
    assert!(wait_exit(&mut reaper, pty.pid()).success());
    eprintln!(
        "flood: {} MB in {:.3}s = {:.0} MB/s, {} reads ({} B avg), {} poll wakeups",
        LEN >> 20,
        elapsed.as_secs_f64(),
        (LEN as f64 / (1 << 20) as f64) / elapsed.as_secs_f64(),
        reads,
        LEN / reads.max(1),
        wakeups
    );
}
