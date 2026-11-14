//! Where the server socket lives.

use std::ffi::OsString;
use std::fs::DirBuilder;
use std::io;
use std::os::unix::fs::{DirBuilderExt, MetadataExt};
use std::path::{Path, PathBuf};

/// A resolved socket location.
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct SocketPath {
    pub socket: PathBuf,
    /// The private directory that must exist (mode 0700, ours) before the
    /// socket is used. `None` when `$LIF_SOCKET` chose the path: the user
    /// picked that directory and owns its permissions.
    pub private_dir: Option<PathBuf>,
}

/// Pure resolution, so the rules can be tested without touching the
/// process environment: `$LIF_SOCKET`, else `$XDG_RUNTIME_DIR/lif/default`,
/// else `$TMPDIR/lif-<uid>/default` (`/tmp` without `$TMPDIR`). Empty
/// variables count as unset.
pub fn resolve_socket_path(lif_socket: Option<OsString>, xdg_runtime_dir: Option<OsString>, tmpdir: Option<OsString>, uid: u32) -> SocketPath {
    let set = |v: Option<OsString>| v.filter(|v| !v.is_empty());
    if let Some(s) = set(lif_socket) {
        return SocketPath { socket: s.into(), private_dir: None };
    }
    let dir = match set(xdg_runtime_dir) {
        Some(x) => Path::new(&x).join("lif"),
        None => {
            let base = set(tmpdir).map(PathBuf::from).unwrap_or_else(|| PathBuf::from("/tmp"));
            base.join(format!("lif-{uid}"))
        }
    };
    SocketPath { socket: dir.join("default"), private_dir: Some(dir) }
}

/// The socket path for this process, with its private directory created and
/// checked.
pub fn default_socket_path() -> io::Result<PathBuf> {
    // SAFETY: getuid cannot fail.
    let uid = unsafe { libc::getuid() };
    let p = resolve_socket_path(std::env::var_os("LIF_SOCKET"), std::env::var_os("XDG_RUNTIME_DIR"), std::env::var_os("TMPDIR"), uid);
    if let Some(dir) = &p.private_dir {
        secure_dir(dir)?;
    }
    Ok(p.socket)
}

/// Creates `dir` with mode 0700 if missing, then refuses it unless it is a
/// real directory (not a symlink) owned by us with no group/other access.
/// Another user who can write there could otherwise stand up a fake server.
pub fn secure_dir(dir: &Path) -> io::Result<()> {
    match DirBuilder::new().mode(0o700).create(dir) {
        Ok(()) => {}
        Err(e) if e.kind() == io::ErrorKind::AlreadyExists => {}
        Err(e) => return Err(e),
    }
    let md = std::fs::symlink_metadata(dir)?;
    // SAFETY: getuid cannot fail.
    let uid = unsafe { libc::getuid() };
    let bad = |why: &str| Err(io::Error::new(io::ErrorKind::PermissionDenied, format!("{}: {why}", dir.display())));
    if !md.file_type().is_dir() {
        return bad("not a directory");
    }
    if md.uid() != uid {
        return bad("directory is owned by another user");
    }
    if md.mode() & 0o077 != 0 {
        return bad("directory is accessible by group or others (needs mode 0700)");
    }
    Ok(())
}
