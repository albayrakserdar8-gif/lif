//! A panic must not leave clients with a terminal in raw mode, on the
//! alternate screen with mouse reporting on. The loop keeps, per attached
//! client, the bytes that restore its terminal plus an `Exit` message; the
//! hook writes them, logs, and aborts.
//!
//! Limitation: if the panic interrupts a frame that was only partly
//! written to a socket, the client sees the restore bytes in the middle of
//! a protocol frame and drops the connection; its own raw-mode guard still
//! restores the tty modes.

use std::collections::HashMap;
use std::os::fd::RawFd;
use std::sync::Mutex;

static EMERGENCY: Mutex<Option<HashMap<RawFd, Vec<u8>>>> = Mutex::new(None);

pub(crate) fn set(fd: RawFd, bytes: Vec<u8>) {
    if let Ok(mut g) = EMERGENCY.lock() {
        g.get_or_insert_with(HashMap::new).insert(fd, bytes);
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
            for (fd, bytes) in m {
                crate::sys::write_all_timeout(*fd, bytes, 200);
            }
        }
        std::process::abort();
    }));
}
