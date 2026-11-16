# 0007: a detach racing the client's own write ends in "Broken pipe"

Status: fixed in the client (0.4.0).

## What happened

The macOS job of CI (GitHub's `macos-15` runner, run 43756630819) failed in
`server_uses_the_terminal_directly` (`crates/lif/tests/latency.rs`):

```
assertion `left == right` failed
  left: 1
 right: 0
```

The test stops the client with SIGSTOP, types a command (the server reads
the terminal itself), continues the client with SIGCONT and at once types
`C-b d`. The client exited with status 1 instead of 0. The same test
passed on the Linux jobs and on this machine.

## Expected

The client prints `[detached (from session direct)]` and exits 0.

## Evidence

- 60 runs alone and 120 runs of the whole file (with and without
  `yes` loads on every CPU): no failure.
- The whole file under `taskpolicy -b` (throttled, efficiency cores) with
  one `yes` per CPU, the test changed to print the screen on failure:
  3 failures in 40 runs, each with

  ```
  lif: Broken pipe (os error 32)
  ```

## Root cause

After SIGCONT the client sends `Resume`. The server reads keys from the
terminal directly, so it can see `C-b d` before that `Resume`: it sends
the restore bytes and `Exit`, and closes the socket. A client slow to get
from its `poll` to its socket write (a loaded or throttled CPU) then
writes `Resume` to a closed socket, gets EPIPE and returned it as an
error, without reading the `Exit` that was already waiting for it.

Any client write can meet this (a resize, keys in relay mode), not only
`Resume`; a server-side detach is always possible while the client is
writing.

## Fix

`Relay::flush_sock` (`crates/lif-client/src/attach.rs`) takes EPIPE and
ECONNRESET on the socket as "the server closed it": it drops what it had
to send, stops writing and keeps reading. The `Exit` sent before the
close ends the loop as usual; a close with no `Exit` still ends in
"server closed the connection", as before.

Test: `server_closing_while_the_client_writes_still_exits_cleanly`
(`crates/lif-client/tests/client.rs`) makes the order certain. The fake
server fills the terminal, sends `Exit` and closes. A client blocked on
its terminal does not read the socket, so a SIGWINCH makes it write a
`Resize` to the closed socket before it reads the `Exit`. Without the
fix it fails at once with `BrokenPipe`. The throttled loop above ran 80
times with the fix and `server_uses_the_terminal_directly` did not fail.

## What is not fixed

Nothing known. Under `taskpolicy -b` the 3 ms bounds of the other
latency tests are missed now and then, which says more about the
throttling than about lif; they pass on CI.

## Measured cost

None: one more branch on a failed socket write.
