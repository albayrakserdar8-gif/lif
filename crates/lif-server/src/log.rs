//! The `-v` log. Off unless [`init`] ran; then every line goes to the file
//! with a timestamp relative to server start.

use std::fs::File;
use std::io::Write;
use std::os::unix::fs::OpenOptionsExt;
use std::path::Path;
use std::sync::Mutex;
use std::sync::atomic::{AtomicBool, Ordering};
use std::time::Instant;

static ENABLED: AtomicBool = AtomicBool::new(false);
static FILE: Mutex<Option<(File, Option<Instant>)>> = Mutex::new(None);

pub(crate) fn init(path: &Path) {
    // The log holds commands and names: the user's eyes only. Never
    // through a symlink someone planted at the path.
    if let Ok(f) = File::options().create(true).append(true).mode(0o600).custom_flags(libc::O_NOFOLLOW | libc::O_CLOEXEC).open(path)
        && let Ok(mut g) = FILE.lock()
    {
        *g = Some((f, Some(Instant::now())));
        ENABLED.store(true, Ordering::Relaxed);
    }
}

pub(crate) fn enabled() -> bool {
    ENABLED.load(Ordering::Relaxed)
}

pub(crate) fn write(args: std::fmt::Arguments<'_>) {
    // try_lock: the panic hook may log while the loop holds the lock.
    let Ok(mut g) = FILE.try_lock() else { return };
    if let Some((f, start)) = g.as_mut() {
        let t = start.map(|s| s.elapsed().as_secs_f64()).unwrap_or(0.0);
        let _ = writeln!(f, "{t:10.3} {args}");
    }
}

macro_rules! log {
    ($($t:tt)*) => {
        if $crate::log::enabled() {
            $crate::log::write(format_args!($($t)*));
        }
    };
}
