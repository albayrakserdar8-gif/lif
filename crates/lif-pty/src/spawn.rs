use std::ffi::{CString, OsStr, OsString};
use std::io;
use std::os::unix::ffi::{OsStrExt, OsStringExt};
use std::path::{Path, PathBuf};

use libc::c_char;
use lif_types::{PixelSize, Size};

use crate::sys::{self, FdCloser};

/// What to run in a new PTY.
#[derive(Clone, Debug)]
pub struct SpawnOptions {
    /// Program and arguments, looked up in the child's `PATH` when the
    /// program has no '/'. Empty starts the user's shell (`$SHELL`, else the
    /// password database, else /bin/sh) as a login shell: argv[0] is the
    /// shell's base name prefixed with '-', as tmux does.
    pub argv: Vec<OsString>,
    /// Working directory. `None` or a directory that does not exist falls
    /// back to `$HOME`, then `/`.
    pub cwd: Option<PathBuf>,
    /// Variables set in the child on top of the server's environment.
    pub env: Vec<(OsString, OsString)>,
    /// Variables removed from the inherited environment (before `env`).
    pub env_remove: Vec<OsString>,
    /// `TERM` in the child; overrides `env`.
    pub term: OsString,
    pub size: Size,
    pub pixels: PixelSize,
}

impl Default for SpawnOptions {
    fn default() -> Self {
        Self {
            argv: Vec::new(),
            cwd: None,
            env: Vec::new(),
            env_remove: Vec::new(),
            term: OsString::from("xterm-256color"),
            size: Size::new(80, 24),
            pixels: PixelSize::default(),
        }
    }
}

impl SpawnOptions {
    /// Options running `argv` with every other field at its default.
    pub fn command<I, S>(argv: I) -> Self
    where
        I: IntoIterator<Item = S>,
        S: Into<OsString>,
    {
        Self {
            argv: argv.into_iter().map(Into::into).collect(),
            ..Self::default()
        }
    }
}

/// Everything the forked child touches, built before fork so the child only
/// makes async-signal-safe calls.
pub(crate) struct Prepared {
    /// Paths to try with execve, in order (the PATH search, done up front).
    pub(crate) programs: Vec<CString>,
    pub(crate) argv: Vec<*const c_char>,
    pub(crate) envp: Vec<*const c_char>,
    /// Directories to try with chdir, in order.
    pub(crate) dirs: Vec<CString>,
    pub(crate) closer: FdCloser,
    /// RLIMIT_NOFILE soft limit to give the child (0: leave it).
    pub(crate) nofile: u64,
    /// Shown in error messages.
    pub(crate) display_name: String,
    // Own the strings the pointer arrays above point into.
    _argv: Vec<CString>,
    _envp: Vec<CString>,
}

fn cstring(bytes: impl Into<Vec<u8>>, what: &str) -> io::Result<CString> {
    CString::new(bytes).map_err(|_| {
        io::Error::new(
            io::ErrorKind::InvalidInput,
            format!("{what} contains a NUL byte"),
        )
    })
}

fn lookup<'a>(env: &'a [(OsString, OsString)], key: &str) -> Option<&'a OsStr> {
    env.iter()
        .find(|(k, _)| k == key)
        .map(|(_, v)| v.as_os_str())
}

fn set(env: &mut Vec<(OsString, OsString)>, key: &OsStr, value: &OsStr) {
    match env.iter_mut().find(|(k, _)| k == key) {
        Some(entry) => entry.1 = value.to_owned(),
        None => env.push((key.to_owned(), value.to_owned())),
    }
}

fn is_dir(p: &Path) -> bool {
    std::fs::metadata(p).map(|m| m.is_dir()).unwrap_or(false)
}

fn usable_shell(path: &OsStr) -> Option<CString> {
    if !path.as_bytes().starts_with(b"/") {
        return None;
    }
    let c = CString::new(path.as_bytes()).ok()?;
    sys::is_executable(&c).then_some(c)
}

/// See [`set_child_fd_limit`].
static CHILD_NOFILE: std::sync::atomic::AtomicU64 = std::sync::atomic::AtomicU64::new(0);

/// The RLIMIT_NOFILE soft limit children get (`None`: whatever this
/// process has). A server that raised its own limit to serve many panes
/// gives programs back the limit they would have had: some size tables by
/// it, or close every descriptor up to it.
pub fn set_child_fd_limit(soft: Option<u64>) {
    CHILD_NOFILE.store(soft.unwrap_or(0), std::sync::atomic::Ordering::Relaxed);
}

pub(crate) fn prepare(opts: &SpawnOptions) -> io::Result<Prepared> {
    let mut env: Vec<(OsString, OsString)> = std::env::vars_os().collect();
    env.retain(|(k, _)| !opts.env_remove.contains(k));
    for (k, v) in &opts.env {
        if k.is_empty() || k.as_bytes().contains(&b'=') {
            return Err(io::Error::new(
                io::ErrorKind::InvalidInput,
                format!("invalid environment variable name {k:?}"),
            ));
        }
        set(&mut env, k, v);
    }
    set(&mut env, OsStr::new("TERM"), &opts.term);

    let passwd = sys::passwd_entry();

    let (program, argv): (CString, Vec<CString>) = if opts.argv.is_empty() {
        let shell = lookup(&env, "SHELL")
            .and_then(usable_shell)
            .or_else(|| {
                passwd
                    .as_ref()
                    .and_then(|(shell, _)| usable_shell(OsStr::from_bytes(shell.as_bytes())))
            })
            .unwrap_or_else(|| c"/bin/sh".to_owned());
        let base = shell
            .as_bytes()
            .rsplit(|&b| b == b'/')
            .next()
            .unwrap_or(b"sh");
        let mut arg0 = Vec::with_capacity(base.len() + 1);
        arg0.push(b'-');
        arg0.extend_from_slice(base);
        let arg0 = cstring(arg0, "shell name")?;
        (shell, vec![arg0])
    } else {
        let argv = opts
            .argv
            .iter()
            .map(|a| cstring(a.as_bytes(), "argument"))
            .collect::<io::Result<Vec<_>>>()?;
        (argv[0].clone(), argv)
    };
    let display_name = String::from_utf8_lossy(program.as_bytes()).into_owned();
    if program.as_bytes().is_empty() {
        return Err(io::Error::new(
            io::ErrorKind::InvalidInput,
            "empty program name",
        ));
    }

    let programs = if program.as_bytes().contains(&b'/') {
        vec![program]
    } else {
        let path = lookup(&env, "PATH")
            .map(|p| p.as_bytes())
            .unwrap_or(b"/usr/bin:/bin");
        path.split(|&b| b == b':')
            .map(|dir| {
                let dir: &[u8] = if dir.is_empty() { b"." } else { dir };
                let mut full = Vec::with_capacity(dir.len() + 1 + program.as_bytes().len());
                full.extend_from_slice(dir);
                full.push(b'/');
                full.extend_from_slice(program.as_bytes());
                cstring(full, "PATH entry")
            })
            .collect::<io::Result<Vec<_>>>()?
    };

    let home: Option<PathBuf> = lookup(&env, "HOME").map(PathBuf::from).or_else(|| {
        passwd
            .as_ref()
            .map(|(_, dir)| PathBuf::from(OsStr::from_bytes(dir.as_bytes())))
    });
    let mut dir_paths: Vec<PathBuf> = Vec::with_capacity(3);
    for candidate in [opts.cwd.clone(), home] {
        if let Some(d) = candidate
            && d.is_absolute()
            && is_dir(&d)
        {
            dir_paths.push(d);
        }
    }
    dir_paths.push(PathBuf::from("/"));
    if !opts.env.iter().any(|(k, _)| k == "PWD") {
        set(&mut env, OsStr::new("PWD"), dir_paths[0].as_os_str());
    }
    let dirs = dir_paths
        .into_iter()
        .map(|d| cstring(d.into_os_string().into_vec(), "directory"))
        .collect::<io::Result<Vec<_>>>()?;

    let envp_strings = env
        .into_iter()
        .map(|(k, v)| {
            let mut kv = k.into_vec();
            kv.push(b'=');
            kv.extend_from_slice(v.as_bytes());
            cstring(kv, "environment variable")
        })
        .collect::<io::Result<Vec<_>>>()?;

    let argv_ptrs = argv
        .iter()
        .map(|s| s.as_ptr())
        .chain([std::ptr::null()])
        .collect();
    let envp_ptrs = envp_strings
        .iter()
        .map(|s| s.as_ptr())
        .chain([std::ptr::null()])
        .collect();

    Ok(Prepared {
        programs,
        argv: argv_ptrs,
        envp: envp_ptrs,
        dirs,
        closer: FdCloser::new(),
        nofile: CHILD_NOFILE.load(std::sync::atomic::Ordering::Relaxed),
        display_name,
        _argv: argv,
        _envp: envp_strings,
    })
}
