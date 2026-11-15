# 0004: `a_stale_socket_is_replaced` failed once

Status: handled in the client (0.4.0); the server side is not explained.

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

Partly known. `kill-server` returns when the server has answered, not when
it is gone, so the next client can find the socket of a server that is on
its way out: its connection is taken and dropped. That explains the two
`kill-server` sightings. Why a server just started on a stale socket would
drop its first client (the other two) is not known.

## Fix

The client treats a connection that is dropped during the handshake
(end of file, reset, broken pipe) as no answer and tries again for up to
2 s (`crates/lif/src/main.rs`): the exiting server's socket is gone by
then and the usual path starts a new one, and a server that is up answers
the second time. 300 rounds of `new-session; kill-server; new-session`
gave no failure afterwards (none before either with a release build: the
window is narrow).

## What is not fixed

The server dropping a first client after a stale socket was replaced is
only papered over by the retry. To catch it: run the start-up tests of
`server_limits.rs` in a loop with `-v` and keep `lif-server-*.log`.

## Measured cost

None on the usual path: one more branch on a failed connect.
