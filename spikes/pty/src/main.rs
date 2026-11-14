//! Spike: does kqueue / poll(2) report readability and hangup correctly on a
//! PTY master on macOS? tmux disables both there and falls back to select.
use std::ffi::CString;
use std::time::Instant;

unsafe fn spawn(cmd: &str) -> (i32, i32) {
    unsafe {
        let mut master = 0;
        let mut ws: libc::winsize = std::mem::zeroed();
        ws.ws_col = 120;
        ws.ws_row = 40;
        let pid = libc::forkpty(&mut master, std::ptr::null_mut(), std::ptr::null_mut(), &mut ws);
        assert!(pid >= 0);
        if pid == 0 {
            let sh = CString::new("/bin/sh").unwrap();
            let c = CString::new("-c").unwrap();
            let arg = CString::new(cmd).unwrap();
            libc::execl(sh.as_ptr(), sh.as_ptr(), c.as_ptr(), arg.as_ptr(), std::ptr::null::<libc::c_char>());
            libc::_exit(127);
        }
        let fl = libc::fcntl(master, libc::F_GETFL);
        libc::fcntl(master, libc::F_SETFL, fl | libc::O_NONBLOCK);
        (master, pid)
    }
}

/// Drain until EAGAIN. Returns (bytes, eof).
unsafe fn drain(fd: i32, buf: &mut [u8], max_read: &mut usize) -> (usize, bool) {
    let mut total = 0;
    loop {
        let n = unsafe { libc::read(fd, buf.as_mut_ptr().cast(), buf.len()) };
        if n > 0 {
            total += n as usize;
            *max_read = (*max_read).max(n as usize);
        } else if n == 0 {
            return (total, true);
        } else {
            let e = std::io::Error::last_os_error();
            return match e.raw_os_error() {
                Some(libc::EAGAIN) => (total, false),
                Some(libc::EINTR) => continue,
                _ => (total, true), // EIO on Linux == child side closed
            };
        }
    }
}

fn run(name: &str, cmd: &str, use_kqueue: bool) {
    unsafe {
        let (master, pid) = spawn(cmd);
        let mut buf = vec![0u8; 256 * 1024];
        let (mut bytes, mut wakeups, mut max_read) = (0usize, 0usize, 0usize);
        let t = Instant::now();
        let kq = if use_kqueue { libc::kqueue() } else { -1 };
        if use_kqueue {
            let mut ev: libc::kevent = std::mem::zeroed();
            ev.ident = master as usize;
            ev.filter = libc::EVFILT_READ;
            ev.flags = libc::EV_ADD;
            let r = libc::kevent(kq, &ev, 1, std::ptr::null_mut(), 0, std::ptr::null());
            if r < 0 {
                println!("{name}: kevent register FAILED: {}", std::io::Error::last_os_error());
                libc::kill(pid, libc::SIGKILL);
                libc::waitpid(pid, std::ptr::null_mut(), 0);
                return;
            }
        }
        let mut timed_out = false;
        loop {
            let ready = if use_kqueue {
                let mut out: libc::kevent = std::mem::zeroed();
                let ts = libc::timespec { tv_sec: 3, tv_nsec: 0 };
                libc::kevent(kq, std::ptr::null(), 0, &mut out, 1, &ts)
            } else {
                let mut p = libc::pollfd { fd: master, events: libc::POLLIN, revents: 0 };
                libc::poll(&mut p, 1, 3000)
            };
            if ready == 0 {
                timed_out = true;
                break;
            }
            if ready < 0 {
                continue;
            }
            wakeups += 1;
            let (n, eof) = drain(master, &mut buf, &mut max_read);
            bytes += n;
            if eof {
                break;
            }
        }
        let dt = t.elapsed().as_secs_f64();
        libc::kill(pid, libc::SIGKILL);
        libc::waitpid(pid, std::ptr::null_mut(), 0);
        libc::close(master);
        if kq >= 0 {
            libc::close(kq);
        }
        println!(
            "{name:<22} {:>9} bytes {:>7} wakeups max_read={:>6} {:>7.1} ms {:>6.0} MB/s{}",
            bytes,
            wakeups,
            max_read,
            dt * 1e3,
            bytes as f64 / 1e6 / dt.max(1e-9),
            if timed_out { "  ** TIMED OUT (no EOF/hangup event) **" } else { "" }
        );
    }
}

fn main() {
    let small = "echo hello";
    let big = "head -c 50000000 /dev/zero | tr '\\0' 'x' | fold -w 100";
    for (label, kq) in [("poll", false), ("kqueue", true)] {
        run(&format!("{label}/small+eof"), small, kq);
        run(&format!("{label}/50MB flood"), big, kq);
    }
}
