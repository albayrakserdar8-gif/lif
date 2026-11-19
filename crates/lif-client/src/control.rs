//! Control mode (`lif -C`, tmux's `-C`): no terminal handling at all.
//! Lines read from standard input go to the server as `Input` (it runs
//! them as commands and answers with `%begin` ... `%end`), and what the
//! server sends (`Output`: the answers, `%output` of panes, the
//! notifications) is written to standard output as it is. The end of
//! standard input detaches; the server's `Exit` ends the loop. With
//! `-CC` the whole exchange is wrapped in a DCS sequence, as tmux does for
//! terminals that speak it (iTerm2).

use std::io::{self, Read, Write};
use std::os::fd::AsRawFd;

use lif_proto::{ClientMsg, ServerMsg};

use crate::conn::Connection;

/// tmux's DCS around a `-CC` session.
const DCS_START: &[u8] = b"\x1bP1000p";
const DCS_END: &[u8] = b"\x1b\\";

/// Runs control mode until the server ends it (`Exit`) or goes away.
pub fn control_loop(mut conn: Connection, dcs: bool) -> io::Result<()> {
    let mut stdout = io::stdout().lock();
    if dcs {
        stdout.write_all(DCS_START)?;
        stdout.flush()?;
    }
    let sock = conn.stream.as_raw_fd();
    let mut stdin_open = true;
    let mut buf = vec![0u8; 64 * 1024];
    let result = (|| -> io::Result<()> {
        loop {
            // Everything already decoded first: a reply may have come with
            // the `Attached` that started this loop.
            while let Some(frame) = conn.dec.next_frame().map_err(io::Error::other)? {
                match frame.decode::<ServerMsg>().map_err(io::Error::other)? {
                    ServerMsg::Output(b) => {
                        stdout.write_all(&b)?;
                        stdout.flush()?;
                    }
                    ServerMsg::Exit { .. } => return Ok(()),
                    _ => {}
                }
            }
            let mut fds = [libc::pollfd { fd: sock, events: libc::POLLIN, revents: 0 }, libc::pollfd { fd: 0, events: libc::POLLIN, revents: 0 }];
            let n = if stdin_open { 2 } else { 1 };
            // SAFETY: fds holds n valid pollfd structs.
            let r = unsafe { libc::poll(fds.as_mut_ptr(), n as libc::nfds_t, -1) };
            if r < 0 {
                let e = io::Error::last_os_error();
                if e.kind() == io::ErrorKind::Interrupted {
                    continue;
                }
                return Err(e);
            }
            // The server went away without an Exit.
            if fds[0].revents != 0 && conn.dec.read_from(&mut conn.stream)? == 0 {
                return Ok(());
            }
            if stdin_open && fds[1].revents != 0 {
                let got = match io::stdin().lock().read(&mut buf) {
                    Ok(n) => n,
                    Err(e) if e.kind() == io::ErrorKind::Interrupted => continue,
                    Err(e) => return Err(e),
                };
                let mut out = Vec::new();
                if got == 0 {
                    stdin_open = false;
                    ClientMsg::Detach.encode(&mut out).map_err(io::Error::other)?;
                } else {
                    ClientMsg::Input(buf[..got].to_vec()).encode(&mut out).map_err(io::Error::other)?;
                }
                match conn.stream.write_all(&out) {
                    Ok(()) => {}
                    // The server has ended it (an empty line, kill-server)
                    // and closed its side: what it sent before, its Exit
                    // included, is still to be read.
                    Err(e) if matches!(e.kind(), io::ErrorKind::BrokenPipe | io::ErrorKind::ConnectionReset) => stdin_open = false,
                    Err(e) => return Err(e),
                }
            }
        }
    })();
    if dcs {
        let _ = stdout.write_all(DCS_END);
        let _ = stdout.flush();
    }
    result
}
