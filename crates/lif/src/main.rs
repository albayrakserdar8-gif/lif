//! `lif`: the command-line client, and the server it starts.
//!
//! `lif [-2uvV] [-L socket-name] [-S socket-path] [command [flags...]]`.
//! No command means `new-session`, as in tmux.

mod start;

use std::io::{self, Write};
use std::path::{Path, PathBuf};
use std::process::exit;

use lif_client::{CommandOutcome, attach_loop, connect_with, hello_from_env, resolve_socket_path, run_command, secure_dir};
use lif_proto::{ExitReason, Hello};

const USAGE: &str = "usage: lif [-2uvV] [-L socket-name] [-S socket-path] [command [flags]]";

struct Cli {
    socket: Option<PathBuf>,
    label: Option<String>,
    verbose: bool,
    argv: Vec<String>,
}

fn usage() -> ! {
    eprintln!("{USAGE}");
    exit(1)
}

fn parse_cli(args: Vec<String>) -> Cli {
    let mut cli = Cli { socket: None, label: None, verbose: false, argv: Vec::new() };
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
                'V' => {
                    println!("lif {}", env!("CARGO_PKG_VERSION"));
                    exit(0);
                }
                'v' => cli.verbose = true,
                // tmux's -2 (256 colors) and -u (UTF-8) are always on in lif.
                '2' | 'u' => {}
                f @ ('S' | 'L') => {
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
                    if f == 'S' {
                        cli.socket = Some(PathBuf::from(value));
                    } else {
                        cli.label = Some(value);
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
    lif_client::default_socket_path()
}

/// `Hello` for this process, plus the pane we run in when `$LIF` names
/// this server (so `lif split-window` inside a pane splits that pane).
fn hello(socket: &Path) -> Hello {
    let mut h = hello_from_env();
    if let (Ok(lif), Ok(pane)) = (std::env::var("LIF"), std::env::var("LIF_PANE")) {
        let sock = lif.split(',').next().unwrap_or("");
        if Path::new(sock) == socket {
            h.env.push(("LIF_PANE".into(), pane));
        }
    }
    h
}

fn main() {
    let cli = parse_cli(std::env::args().skip(1).collect());
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
    let conn = match connect_with(&socket, hello(&socket)) {
        Ok(c) => c,
        Err(e) if matches!(e.kind(), io::ErrorKind::NotFound | io::ErrorKind::ConnectionRefused) => {
            if !may_start {
                eprintln!("no server running on {}", socket.display());
                exit(1);
            }
            if let Err(e) = start::start_server(&socket, cli.verbose) {
                eprintln!("lif: server failed to start: {e}");
                exit(1);
            }
            match connect_with(&socket, hello(&socket)) {
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
    match run_command(&mut conn, cli.argv.iter().cloned()) {
        Ok(CommandOutcome::Done { status, stdout, stderr }) => {
            let _ = io::stdout().write_all(&stdout);
            let _ = io::stdout().flush();
            let _ = io::stderr().write_all(&stderr);
            exit(status);
        }
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
