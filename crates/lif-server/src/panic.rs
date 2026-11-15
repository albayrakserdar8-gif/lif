//! A panic must not leave clients with a terminal in raw mode, on the
//! alternate screen with mouse reporting on. The loop keeps, per attached
//! client, the bytes that restore its terminal plus an `Exit` message; the
//! hook writes them, logs, and aborts.
//!
//! The restore bytes are protocol frames, so they may only follow whole
//! frames. The loop also records, per client, whether the socket holds part
//! of a frame (a write stopped in the middle of the output buffer). For such
//! a client the hook closes the socket instead of writing: the client sees
//! the connection drop and restores its terminal by itself.

use std::collections::HashMap;
use std::os::fd::RawFd;
use std::sync::Mutex;

struct Emergency {
    bytes: Vec<u8>,
    /// Part of a frame is on the wire.
    mid_frame: bool,
}

static EMERGENCY: Mutex<Option<HashMap<RawFd, Emergency>>> = Mutex::new(None);

/// Pane processes not yet reaped (one per live pane, plus killed panes
/// still exiting). An abort closes their PTYs, which hangs them up, but
/// nothing would be left to SIGKILL the ones a hangup does not end
/// (`PANE_KILL_GRACE`), so the hook does it.
static PANES: Mutex<Vec<u32>> = Mutex::new(Vec::new());

/// How long the hook gives hung-up pane processes before SIGKILL. Shorter
/// than `PANE_KILL_GRACE`: clients are waiting for a crashed server to go.
const PANIC_GRACE_MS: u32 = 200;

pub(crate) fn pane_started(pid: u32) {
    if let Ok(mut g) = PANES.lock() {
        g.push(pid);
    }
}

pub(crate) fn pane_reaped(pid: u32) {
    if let Ok(mut g) = PANES.lock() {
        g.retain(|p| *p != pid);
    }
}

/// Hangs up every pane's process group, waits up to `PANIC_GRACE_MS` for
/// them to exit and SIGKILLs the rest. Only for the panic hook.
fn end_panes(pids: &[u32]) {
    for &pid in pids {
        crate::sys::kill_group(pid, libc::SIGHUP);
    }
    // waitpid, not kill(pid, 0): an exited child stays a zombie (and
    // signalable) for as long as this process does not reap it.
    let exited = |pid: u32| {
        let mut status = 0;
        // SAFETY: non-blocking wait for a child of this process.
        unsafe { libc::waitpid(pid as libc::pid_t, &mut status, libc::WNOHANG) != 0 }
    };
    let mut left: Vec<u32> = pids.to_vec();
    for _ in 0..PANIC_GRACE_MS / 5 {
        left.retain(|&pid| !exited(pid));
        if left.is_empty() {
            return;
        }
        std::thread::sleep(std::time::Duration::from_millis(5));
    }
    for pid in left {
        crate::sys::kill_group(pid, libc::SIGKILL);
    }
}

pub(crate) fn set(fd: RawFd, bytes: Vec<u8>) {
    if let Ok(mut g) = EMERGENCY.lock() {
        let m = g.get_or_insert_with(HashMap::new);
        let mid_frame = m.get(&fd).is_some_and(|e| e.mid_frame);
        m.insert(fd, Emergency { bytes, mid_frame });
    }
}

/// Whether `fd`'s socket holds part of a frame (see the module docs).
pub(crate) fn set_mid_frame(fd: RawFd, mid: bool) {
    if let Ok(mut g) = EMERGENCY.lock()
        && let Some(e) = g.as_mut().and_then(|m| m.get_mut(&fd))
    {
        e.mid_frame = mid;
    }
}

pub(crate) fn remove(fd: RawFd) {
    if let Ok(mut g) = EMERGENCY.lock()
        && let Some(m) = g.as_mut()
    {
        m.remove(&fd);
    }
}

pub(crate) fn install_hook() {
    std::panic::set_hook(Box::new(|info| {
        log!("PANIC: {info}");
        if let Ok(g) = EMERGENCY.try_lock()
            && let Some(m) = g.as_ref()
        {
            for (fd, e) in m {
                if e.mid_frame {
                    // SAFETY: shutdown on a socket fd this process owns; the
                    // process aborts right after, so nothing uses it again.
                    unsafe { libc::shutdown(*fd, libc::SHUT_RDWR) };
                } else {
                    crate::sys::write_all_timeout(*fd, &e.bytes, 200);
                }
            }
        }
        if let Ok(g) = PANES.try_lock() {
            end_panes(&g);
        }
        std::process::abort();
    }));
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn mid_frame_state_survives_new_restore_bytes() {
        set(9999, b"a".to_vec());
        set_mid_frame(9999, true);
        set(9999, b"b".to_vec());
        let g = EMERGENCY.lock().unwrap();
        let e = &g.as_ref().unwrap()[&9999];
        assert!(e.mid_frame);
        assert_eq!(e.bytes, b"b");
        drop(g);
        remove(9999);
        set_mid_frame(9999, true);
        assert!(EMERGENCY.lock().unwrap().as_ref().unwrap().get(&9999).is_none());
    }
}
