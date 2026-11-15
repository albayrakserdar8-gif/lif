# Pane shells from the test suite outlive their server and spin at 100% CPU

Status: fixed in the working tree (not committed) · Found: 2026-11-15 · Severity: high (two cores pinned for ~7 h)

## What happened

On 2026-11-15 at 10:11 two orphaned `-sh` processes were using about 98% CPU
each. They had been running for about 6 h 48 min and were killed by hand:

| PID | PPID | Started | argv | TTY | cwd |
|---|---|---|---|---|---|
| 32803 | 1 | 03:23:08 | `-sh` (bash) | ttys006, fds 0/1/2/255 still open | `crates/lif` |
| 43141 | 1 | 03:24:02 | `-sh` (bash) | none, fds 0/1/2/255 **(revoked)** | `crates/lif` |

Neither had any child processes. Machine load average stayed around 4.2 for
the whole period. It also made the `2026-11-15-0330-final` benchmark run
noisy: the README notes "load average about 4.5".

## Why we think lif started them

- argv[0] is `-sh`. That is how `lif-pty` starts the default shell as a login
  shell (`crates/lif-pty/src/spawn.rs:154`: `'-'` + basename of the shell).
- `SHELL=/bin/sh` is set by the test harness (`crates/lif/tests/common/mod.rs:49`)
  and the bench (`bench/lifbench.py:233`). The user's own login shell is zsh.
- The cwd is `crates/lif`, which is where `cargo test -p lif` runs the
  integration tests. The start times, 03:23 and 03:24, fall in the last test
  and benchmark pass before the 03:30 final benchmark.
- Their parent is launchd (PPID 1). So the server, or whatever forked them,
  exited without the shells getting a hangup they acted on.

## Expected

When a server exits, crashes or is killed (`kill-server`, a test's
`Drop`/timeout, SIGKILL), every pane's session should get SIGHUP and exit.
No pane process should outlive its server for more than a moment.

## Hypotheses to check

1. **The shell survives the PTY hangup and busy-loops.** In 43141 the tty was
   revoked: the master was closed and the slave was hung up. Yet bash kept
   running, which suggests it spins on EIO/EOF from a revoked terminal. This
   could happen if SIGHUP went to a process group other than the shell's, or
   if the shell had SIGHUP ignored or blocked from the parent. Check the
   signal mask and dispositions the child inherits in `pty.rs` after
   `fork()` (does the server block or ignore SIGHUP/SIGPIPE before forking
   without resetting them in the child?).
2. **The master fd stayed open somewhere.** 32803 still had a live
   `/dev/ttys006`, so for that one the master was apparently *not* closed.
   That points to a leaked master fd, for example in another process forked
   while it was open (`start.rs` daemonize forks, the test harness, the
   bench driver), or a test that drops its handle without killing the server.
3. **The test harness does not clean up on failure or timeout.** A test that
   panics or times out may leave its server, and so its panes, behind. The
   harness should kill the server's process group and every pane session in
   `Drop`.

## Repro idea

```sh
cargo test -p lif   # in a loop, with some tests forced to time out or panic
ps -Ao pid,ppid,pcpu,etime,args | grep -- ' -sh$'   # nothing should remain
```

Then try the same with `kill -9 <server pid>` while panes are running.

## Suggested fixes

- In the child after `fork()`, reset every signal to SIG_DFL and clear the
  signal mask before `exec`.
- On server shutdown, send SIGHUP explicitly to each pane's process group
  (`killpg`), and SIGKILL after a short grace period. Do not rely only on the
  master close.
- Have the test harness (`tests/common`) and `bench/lifbench.py` check for
  surviving pane processes when a test or run ends, and fail if any remain.
- Add an end-to-end test: start a server with a pane, SIGKILL the server,
  and assert the pane's shell is gone within 1 s.

## Root cause (2026-11-15)

Four more orphans from the same night were still alive (idle, not spinning)
and could be inspected: pids 14274, 36854, 58047, 33704, all session leaders
with PPID 1 and cwd `crates/lif`.

- Hypothesis 1 is wrong. `lif-pty` already resets every signal to SIG_DFL
  and clears the mask in the child (`pty.rs`, `child()`); `ps` showed nothing
  blocked or pending in the survivors.
- Hypothesis 2 is wrong. No process held the masters of the two live ttys
  (`lsof /dev/ptmx`). The master was closed; the slave was just not revoked.
- The hangup is delivered, and the shell does not survive it cleanly.
  `/bin/sh` on macOS is bash 3.2, whose signal handlers are not
  async-signal-safe. `sample` of 33704: main thread in `putc` (holding the
  stdio lock) → `_sigtramp` → bash's handler → `tputs` → `putc` →
  `flockfile` → `__psynch_mutexwait`. It deadlocks on itself when the signal
  lands while it writes, which the suite provokes by killing servers whose
  panes are printing (33704 had written 500 KB). 14274 waited on the same
  lock. The two that spun were killed before anyone looked at their stacks;
  they are assumed to be the same class (stuck after the hangup).
- The defect in lif: it sent SIGHUP and closed the PTY, but never checked
  that the process went away and never escalated.
- Hypothesis 3 is right: the harness neither checked nor cleaned up.

## Fix

- `lif-server`: a hung-up pane's process group gets SIGKILL when it has not
  been reaped after `PANE_KILL_GRACE` (500 ms). `kill-pane` arms a deadline
  in the event loop (no timer unless a killed pane is still alive). On exit
  the server waits for its panes, at most the grace period, and SIGKILLs
  the rest. The panic hook does the same with a 200 ms grace.
- Only the group lif started is killed. A background job in a group of its
  own that ignores SIGHUP keeps running, as under tmux.
- `crates/lif/tests/common`: `Env::drop` fails the test when a pane process
  outlives `kill-server` by 3 s, and kills it. Tests that SIGKILL the server
  call `track_panes()` so their panes are killed too.
- `bench/lifbench.py`: `LifAdapter.stop()` warns about and kills pane
  processes that outlive the server.
- Tests: `kill_pane_kills_a_process_that_ignores_sighup`,
  `kill_server_kills_a_process_that_ignores_sighup`,
  `kill_server_does_not_wait_for_panes_that_exit` (`tests/cli.rs`). The first
  two fail without the server change.

## Not fixed

A server that dies by SIGKILL (or any death that skips the panic hook)
cannot escalate: its panes get the kernel's hangup and nothing more, so a
shell that hangs in its handler stays. Closing that would take a watchdog
process per server. The panic hook path has no end-to-end test (nothing
makes a server panic on demand).

## Cost

Release build, M-series, load average about 3.5, 200 runs of create+kill
(hyperfine) and 100 runs of kill-server until the server process is gone:

| | before | after |
|---|---|---|
| create + kill-server, mean | 6.3 to 6.7 ms | 6.2 to 7.2 ms |
| kill-server to server gone, fresh shell, mean | 4.6 ms | 4.9 ms |
| kill-server to server gone, shell at its prompt, mean | 10.2 ms | 10.6 ms |
