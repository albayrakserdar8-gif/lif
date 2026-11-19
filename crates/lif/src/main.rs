//! `lif`: the command-line client, and the server it starts.
//!
//! `lif [-2uvV] [-f file] [-L socket-name] [-S socket-path] [command [flags...]]`.
//! No command means `new-session`, as in tmux.
//!
//! Four commands are the binary's own and need no server:
//! `check-config [file]`, `print-config`, `migrate-tmux-conf [file]` and
//! `install-tmux-shim [dir]`.
//!
//! Invoked as `tmux` (through the shim `install-tmux-shim` makes), lif
//! serves programs that only know tmux: inside a lif pane it is lif,
//! finding its server through `$TMUX` too and answering `-V` with the tmux
//! version it follows; anywhere else it runs the real tmux.

mod start;

use std::io::{self, Write};
use std::path::{Path, PathBuf};
use std::process::exit;

use lif_client::{CommandOutcome, attach_loop, connect_with, hello_from_env, resolve_socket_path, run_command, secure_dir};
use lif_proto::{ExitReason, Hello};

const USAGE: &str = "usage: lif [-2CuvV] [-f file] [-L socket-name] [-S socket-path] [command [flags]]";

/// The tmux version lif's commands follow, for `tmux -V`.
const TMUX_VERSION: &str = "3.8";

struct Cli {
    /// Invoked as `tmux`.
    as_tmux: bool,
    socket: Option<PathBuf>,
    label: Option<String>,
    /// `-f`: the configuration file a server started now reads.
    config: Option<PathBuf>,
    verbose: bool,
    /// `-C` given once (1) or twice (2, `-CC`): control mode.
    control: u8,
    argv: Vec<String>,
}

fn usage() -> ! {
    eprintln!("{USAGE}");
    exit(1)
}

fn parse_cli(args: Vec<String>, as_tmux: bool) -> Cli {
    let mut cli = Cli { as_tmux, socket: None, label: None, config: None, verbose: false, control: 0, argv: Vec::new() };
    let mut i = 0;
    while i < args.len() {
        let a = &args[i];
        if a == "--" {
            i += 1;
            break;
        }
        if !a.starts_with('-') || a == "-" {
            break;
        }
        let flags: Vec<char> = a[1..].chars().collect();
        let mut j = 0;
        while j < flags.len() {
            match flags[j] {
                'V' if as_tmux => {
                    println!("tmux {TMUX_VERSION} (lif {})", env!("CARGO_PKG_VERSION"));
                    exit(0);
                }
                'V' => {
                    println!("lif {}", env!("CARGO_PKG_VERSION"));
                    exit(0);
                }
                'v' => cli.verbose = true,
                'C' => cli.control = cli.control.saturating_add(1).min(2),
                // tmux's -2 (256 colors) and -u (UTF-8) are always on in lif.
                '2' | 'u' => {}
                f @ ('S' | 'L' | 'f') => {
                    let rest: String = flags[j + 1..].iter().collect();
                    let value = if !rest.is_empty() {
                        rest
                    } else {
                        i += 1;
                        match args.get(i) {
                            Some(v) => v.clone(),
                            None => usage(),
                        }
                    };
                    match f {
                        'S' => cli.socket = Some(PathBuf::from(value)),
                        'f' => cli.config = Some(PathBuf::from(value)),
                        _ => cli.label = Some(value),
                    }
                    j = flags.len();
                    continue;
                }
                'h' => {
                    println!("{USAGE}");
                    exit(0);
                }
                _ => usage(),
            }
            j += 1;
        }
        i += 1;
    }
    cli.argv = args[i..].to_vec();
    if cli.argv.is_empty() {
        cli.argv.push("new-session".into());
    }
    cli
}

fn socket_path(cli: &Cli) -> io::Result<PathBuf> {
    if let Some(s) = &cli.socket {
        return Ok(s.clone());
    }
    if let Some(label) = &cli.label {
        if label.is_empty() || label.contains('/') {
            return Err(io::Error::new(io::ErrorKind::InvalidInput, format!("invalid socket name: {label}")));
        }
        // SAFETY: getuid cannot fail.
        let uid = unsafe { libc::getuid() };
        let p = resolve_socket_path(None, std::env::var_os("XDG_RUNTIME_DIR"), std::env::var_os("TMPDIR"), uid);
        let dir = p.private_dir.unwrap_or_else(|| PathBuf::from("/tmp"));
        secure_dir(&dir)?;
        return Ok(dir.join(label));
    }
    // Inside a pane, the pane's own server, as tmux uses `$TMUX`: it comes
    // before `$LIF_SOCKET` the way `$TMUX` comes before `$TMUX_TMPDIR`.
    // As `tmux`, `$TMUX` itself (set in panes by `tmux-compat`) too.
    if cli.as_tmux
        && let Some(s) = lif_client::socket_from_lif_env(std::env::var("TMUX").ok().as_deref())
    {
        return Ok(s);
    }
    if let Some(s) = lif_client::socket_from_lif_env(std::env::var("LIF").ok().as_deref()) {
        return Ok(s);
    }
    lif_client::default_socket_path()
}

/// `Hello` for this process, plus the pane we run in when `$LIF` names
/// this server (so `lif split-window` inside a pane splits that pane).
fn hello(socket: &Path, control: bool) -> Hello {
    let mut h = hello_from_env();
    if control {
        // No terminal: the server neither draws for it nor reads keys from
        // it, and its size counts only after refresh-client -C.
        h.caps = lif_proto::Caps(lif_proto::Caps::CONTROL.0 | lif_proto::Caps::OUTPUT_STREAM.0);
        h.size = Default::default();
        h.pixels = Default::default();
    }
    for (var, pane_var) in [("LIF", "LIF_PANE"), ("TMUX", "TMUX_PANE")] {
        if let (Ok(env), Ok(pane)) = (std::env::var(var), std::env::var(pane_var)) {
            let sock = env.split(',').next().unwrap_or("");
            if Path::new(sock) == socket {
                h.env.push(("LIF_PANE".into(), pane));
                break;
            }
        }
    }
    h
}

/// Whether this process runs inside a pane of some lif server.
fn in_lif_pane() -> bool {
    std::env::var_os("LIF").is_some_and(|v| !v.is_empty())
}

/// As `tmux` outside lif: run the real tmux, the first `tmux` on `PATH`
/// that is not this program. Does not return.
fn exec_real_tmux(args: &[String]) -> ! {
    use std::os::unix::process::CommandExt;
    let me = std::env::current_exe().ok().and_then(|p| std::fs::canonicalize(p).ok());
    let path = std::env::var_os("PATH").unwrap_or_default();
    for dir in std::env::split_paths(&path) {
        let cand = dir.join("tmux");
        let Ok(real) = std::fs::canonicalize(&cand) else { continue };
        if Some(&real) == me.as_ref() || !is_executable(&real) {
            continue;
        }
        let err = std::process::Command::new(&cand).arg0("tmux").args(args).exec();
        eprintln!("lif: {}: {err}", cand.display());
        exit(127);
    }
    eprintln!("lif: tmux: not found (outside lif panes `tmux` runs the real tmux, and there is none on PATH)");
    exit(127)
}

fn is_executable(p: &Path) -> bool {
    use std::os::unix::fs::PermissionsExt;
    std::fs::metadata(p).is_ok_and(|m| m.is_file() && m.permissions().mode() & 0o111 != 0)
}

/// `install-tmux-shim [dir]`: `dir/tmux` as a link to this program.
fn install_tmux_shim(args: &[String]) -> i32 {
    let dir = match args {
        [] => home_file(".local/share/lif/shim"),
        [d] => PathBuf::from(d),
        _ => {
            eprintln!("usage: lif install-tmux-shim [dir]");
            return 1;
        }
    };
    let me = match std::env::current_exe().and_then(std::fs::canonicalize) {
        Ok(p) => p,
        Err(e) => {
            eprintln!("lif: cannot find this program: {e}");
            return 1;
        }
    };
    let link = dir.join("tmux");
    let made = std::fs::create_dir_all(&dir).and_then(|_| {
        match std::fs::symlink_metadata(&link) {
            // Replace only a link (an earlier shim), never a real file.
            Ok(m) if m.file_type().is_symlink() => std::fs::remove_file(&link)?,
            Ok(_) => return Err(io::Error::new(io::ErrorKind::AlreadyExists, "exists and is not a link; not replaced")),
            Err(_) => {}
        }
        std::os::unix::fs::symlink(&me, &link)
    });
    if let Err(e) = made {
        eprintln!("lif: {}: {e}", link.display());
        return 1;
    }
    println!("{} -> {}", link.display(), me.display());
    println!("Put {} first on PATH. Inside lif panes `tmux` is then lif; elsewhere it is the real tmux.", dir.display());
    println!("For programs that need `$TMUX` (Claude Code agent teams), also set: tmux-compat = true");
    0
}

/// The client's standard input when a command reads it (`load-buffer -`).
fn command_stdin(list: &lif_core::CommandList) -> Option<Vec<u8>> {
    use std::io::Read;
    let wants = list.commands().iter().any(|c| c.name() == "load-buffer" && c.args().value(0) == Some("-"));
    if !wants {
        return None;
    }
    let mut data = Vec::new();
    if let Err(e) = io::stdin().take(lif_client::STDIN_LIMIT as u64 + 1).read_to_end(&mut data) {
        eprintln!("lif: standard input: {e}");
        exit(1);
    }
    if data.len() > lif_client::STDIN_LIMIT {
        eprintln!("lif: standard input too large (at most {} bytes)", lif_client::STDIN_LIMIT);
        exit(1);
    }
    Some(data)
}

/// `~/.tmux.conf`, or a relative name if `HOME` is not set.
fn home_file(name: &str) -> PathBuf {
    match std::env::var_os("HOME") {
        Some(h) if !h.is_empty() => PathBuf::from(h).join(name),
        _ => PathBuf::from(name),
    }
}

/// The commands that work on configuration files without a server.
/// Returns the exit status, or `None` if `argv` is not one of them.
fn config_command(argv: &[String]) -> Option<i32> {
    let (name, args) = argv.split_first()?;
    let path_arg = |args: &[String]| -> Result<Option<PathBuf>, i32> {
        match args {
            [] => Ok(None),
            [p] => Ok(Some(PathBuf::from(p))),
            _ => {
                eprintln!("usage: lif {name} [file]");
                Err(1)
            }
        }
    };
    Some(match name.as_str() {
        "print-config" => {
            if !args.is_empty() {
                eprintln!("usage: lif print-config");
                return Some(1);
            }
            print!("{}", lif_config::example());
            0
        }
        "check-config" => {
            let path = match path_arg(args) {
                Ok(p) => p.or_else(lif_config::default_path),
                Err(code) => return Some(code),
            };
            let Some(path) = path else {
                eprintln!("lif: no configuration file to check (set LIF_CONFIG or HOME)");
                return Some(1);
            };
            if lif_config::looks_like_tmux_conf(&path.to_string_lossy()) {
                eprintln!(
                    "{}: this looks like a tmux configuration file, but lif reads TOML; convert it with: lif migrate-tmux-conf {}",
                    path.display(),
                    path.display()
                );
                return Some(1);
            }
            if !path.exists() {
                eprintln!("{}: No such file or directory", path.display());
                return Some(1);
            }
            let cfg = lif_config::Config::load_file(&path);
            print!("{}", cfg.report());
            if cfg.has_errors() { 1 } else { 0 }
        }
        "migrate-tmux-conf" => {
            let path = match path_arg(args) {
                Ok(p) => p.unwrap_or_else(|| home_file(".tmux.conf")),
                Err(code) => return Some(code),
            };
            let text = match lif_config::read_file_limited(&path) {
                Ok(b) => String::from_utf8_lossy(&b).into_owned(),
                Err(e) => {
                    eprintln!("{}: {e}", path.display());
                    return Some(1);
                }
            };
            let m = lif_config::migrate_tmux_conf(&text);
            print!("{}", m.toml);
            for u in &m.untranslated {
                eprintln!("{}:{}: not translated ({}): {}", path.display(), u.line, u.reason, u.text);
            }
            0
        }
        "install-tmux-shim" => install_tmux_shim(args),
        _ => return None,
    })
}

fn main() {
    let argv0 = std::env::args_os().next().unwrap_or_default();
    let as_tmux = Path::new(&argv0).file_name().is_some_and(|n| n == "tmux");
    let args: Vec<String> = std::env::args().skip(1).collect();
    if as_tmux && !in_lif_pane() {
        exec_real_tmux(&args);
    }
    let cli = parse_cli(args, as_tmux);
    if let Some(code) = config_command(&cli.argv) {
        let _ = io::stdout().flush();
        exit(code);
    }
    let list = match lif_core::parse_list(&cli.argv) {
        Ok(l) => l,
        Err(e) => {
            eprintln!("{e}");
            exit(1);
        }
    };
    let attaches = list.commands().iter().any(|c| c.name() == "attach-session" || (c.name() == "new-session" && !c.args().has('d')));
    if attaches && std::env::var_os("LIF").is_some_and(|v| !v.is_empty()) {
        eprintln!("sessions should be nested with care, unset $LIF to force");
        exit(1);
    }
    let may_start = list.commands().iter().any(|c| matches!(c.name(), "new-session" | "start-server"));
    let socket = match socket_path(&cli) {
        Ok(s) => s,
        Err(e) => {
            eprintln!("lif: {e}");
            exit(1);
        }
    };
    // A server that is exiting (after kill-server, or when its last session
    // closed) may still take a connection and drop it. That is no answer:
    // ask again, until its socket is gone or another server answers.
    let connect = || {
        let end = std::time::Instant::now() + std::time::Duration::from_secs(2);
        loop {
            match connect_with(&socket, hello(&socket, cli.control > 0)) {
                Err(e)
                    if matches!(e.kind(), io::ErrorKind::UnexpectedEof | io::ErrorKind::ConnectionReset | io::ErrorKind::BrokenPipe)
                        && std::time::Instant::now() < end =>
                {
                    std::thread::sleep(std::time::Duration::from_millis(5));
                }
                r => return r,
            }
        }
    };
    let conn = match connect() {
        Ok(c) => c,
        Err(e) if matches!(e.kind(), io::ErrorKind::NotFound | io::ErrorKind::ConnectionRefused) => {
            if !may_start {
                if e.kind() == io::ErrorKind::ConnectionRefused && start::server_holds_lock(&socket) {
                    eprintln!("lif: {}", start::not_responding(&socket));
                } else {
                    eprintln!("no server running on {}", socket.display());
                }
                exit(1);
            }
            match start::start_server(&socket, cli.verbose, cli.config.as_deref()) {
                Ok(()) => {}
                Err(e) if e.kind() == io::ErrorKind::TimedOut => {
                    eprintln!("lif: {e}");
                    exit(1);
                }
                Err(e) => {
                    eprintln!("lif: server failed to start: {e}");
                    exit(1);
                }
            }
            match connect() {
                Ok(c) => c,
                Err(e) => {
                    eprintln!("lif: error connecting to {} ({e})", socket.display());
                    exit(1);
                }
            }
        }
        Err(e) => {
            eprintln!("lif: error connecting to {} ({e})", socket.display());
            exit(1);
        }
    };
    let mut conn = conn;
    if let Some(data) = command_stdin(&list)
        && let Err(e) = conn.send_stdin(data)
    {
        eprintln!("lif: {e}");
        exit(1);
    }
    match run_command(&mut conn, cli.argv.iter().cloned()) {
        Ok(CommandOutcome::Done { status, stdout, stderr }) => {
            let _ = io::stdout().write_all(&stdout);
            let _ = io::stdout().flush();
            let _ = io::stderr().write_all(&stderr);
            exit(status);
        }
        Ok(CommandOutcome::Attached) if cli.control > 0 => match lif_client::control_loop(conn, cli.control > 1) {
            Ok(()) => exit(0),
            Err(e) => {
                eprintln!("lif: {e}");
                exit(1);
            }
        },
        Ok(CommandOutcome::Attached) => match attach_loop(conn) {
            Ok(info) => exit(if info.reason == ExitReason::Error { 1 } else { 0 }),
            Err(e) => {
                eprintln!("lif: {e}");
                exit(1);
            }
        },
        Err(e) => {
            eprintln!("lif: {e}");
            exit(1);
        }
    }
}
