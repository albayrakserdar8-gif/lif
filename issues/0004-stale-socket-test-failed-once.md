# 0004: `a_stale_socket_is_replaced` failed once

Status: fixed (0.5.0): explained; the server removes its socket before it waits for its panes.

## What happened

One run of `cargo test -p lif --test server_limits` (all 15 tests of the
file in parallel, load average about 4.9) failed in
`a_stale_socket_is_replaced`:

```
lif ["new-session", "-d", "-s", "x"] failed: lif: error connecting to
.../target/tmp/e35824-2/s (server closed the connection)
```

The test binds a socket at the server's path, drops the listener (leaving
the file) and runs `new-session -d`, which has to replace the stale socket
and start a server.

## Expected

The client starts a server on the stale path and the command succeeds.

## Evidence

- The same test alone: 5 of 5 passes, then 200 of 200 (8 at a time).
- The whole file again: 6 of 6 passes; the full workspace run after it
  passed twice.
- It was seen while the mouse-key work of 0.4.0 was in the tree; nothing in
  that work touches start-up, the socket or the client's connect path.

A second sighting, on Linux (the first full run there, 2026-11-15):
`dash_f_reaches_a_new_server_only` in `crates/lif/tests/config.rs`
failed once in a run of the whole workspace and passed alone afterwards.
That test runs `kill-server` and at once `new-session`, which again has
a client meet a socket whose server is going away or gone.

A third, with a loop on Linux (`a_stale_socket_is_replaced` together
with the four other start-up tests of the file, 25 times): one failure,

```
lif ["new-session", "-d", "-s", "x"] failed: lif: error connecting to
.../target/tmp/e74205-2/s (Connection reset by peer (os error 104))
```

So the client does reach a server, which drops the connection during the
handshake. The server's wait for its first client is 10 s
(`FIRST_CLIENT_TIMEOUT`), so it is not a server that gave up waiting.

## Hypotheses

1. The client connected to a server that was still starting and that closed
   the connection (a start-up race that only shows under load).
2. Two clients of the test raced to start the server (the test runs one
   command, so this needs a retry inside the client).

Neither is confirmed: the failure left no server log (the test does not run
with `-v`).

A fourth, on macOS, in `dash_f_reaches_a_new_server_only` again
(`server closed the connection` from the `new-session` right after
`kill-server`).

## Root cause

Two causes, both now explained.

1. After `kill-server` (the two `dash_f_reaches_a_new_server_only`
   sightings): `kill-server` returns when the server has answered, not
   when it is gone. `Server::finish` then delivered what was pending,
   hung up the panes and waited up to `PANE_KILL_GRACE` (500 ms) for them,
   and only then removed the socket. A client arriving in that window
   connected to a listener nobody would accept from again; the connection
   was dropped when the process exited.
2. The stale socket (the two `a_stale_socket_is_replaced` sightings) is
   the test's own doing, not the server's. The test binds a listener in
   the test process and drops it; another test thread of the same binary
   that forks at that moment (`Term::spawn` forks a client onto a PTY)
   gives its child a copy of the listener's descriptor, which stays open
   until the child execs or exits. In that window the "stale" socket still
   listens: `connect` succeeds into its queue and the client then reads
   end of file (macOS) or ECONNRESET (Linux) when the copy closes. No lif
   server is involved at all.

   Reproduced without lif: a Python thread forking children that wait
   3 ms before exiting while another thread binds, closes and connects to
   400 sockets: 398 `ECONNREFUSED`, 2 "connected, then EOF".

## Fix

The client treats a connection that is dropped during the handshake
(end of file, reset, broken pipe) as no answer and tries again for up to
2 s (`crates/lif/src/main.rs`): the exiting server's socket is gone by
then and the usual path starts a new one, and a server that is up answers
the second time. 300 rounds of `new-session; kill-server; new-session`
gave no failure afterwards (none before either with a release build: the
window is narrow).

The server (0.5.0) removes its socket first in `Server::finish`, before
it delivers the last messages and kills the panes. A client arriving
meanwhile finds no socket and goes the start-up path, which waits for the
exiting server's lock; that wait now polls every 2 ms instead of 10 ms
(`crates/lif/src/start.rs`). The client's retry stays: it still covers a
connection that was already in the queue, and the test artifact above.

Test: `an_exiting_server_removes_its_socket_first` in
`crates/lif/tests/server_limits.rs` (a pane that ignores SIGHUP keeps the
server in its grace period; the socket must be gone within 300 ms of
`kill-server`). It fails without the fix and passes with it.

## What is not fixed

The test artifact of cause 2 is not removed (the retry handles it, and any
test that drops a listener while other threads fork can meet it).

## Measured cost

6 parallel loops of 100 `new-session -d; kill-server` (debug build, load
average about 1.8): 1.84 s per loop before, 2.28 s with the socket removed
first and the 10 ms lock poll, 1.97 s with the 2 ms poll (about 1.3 ms more
per back-to-back restart, now spent waiting for the old server's lock
instead of on a dropped connection). No failures in any run. Nothing on the
usual path.
