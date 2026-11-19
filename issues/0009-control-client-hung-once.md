# 0009: a control client hung once waiting for its Exit

Status: open (seen once, not reproduced) · Found: 2026-11-19 · Severity:
low (a `lif -CC` client run in a shell pipeline did not exit)

## What happened

During the 0.5.0 control-mode work, this shell line hung for 10 minutes
(it was then killed):

```
printf 'display -p hi\n\n' | lif -S $PWD/s -CC attach -t a | od -c | head -6
```

right after `lif -S $PWD/s new -d -s a` started a new server. `sample`
showed the client in `poll` in `lif_client::control_loop`, its standard
input at its end (so only the socket polled): the server neither sent the
`Exit` nor closed the connection. The server process was alive and idle.

## Expected

`%begin`/`hi`/`%end`, `%exit`, the client exits 0.

## Evidence

- Not reproduced since: the same line 5 times with a 5 s alarm, 200 runs of
  the same input through Python (`-CC`, piped standard input), and the
  control tests 20 times on Linux and repeatedly on macOS.
- A related failure that was found and fixed: on Linux the client could
  exit 1 because writing its `Detach` (end of input) hit the socket the
  server had just closed after the empty line (EPIPE). The client now
  reads what is left (the `Exit`) instead. That does not explain a hang.

## Hypotheses

1. The server stopped flushing the client's last bytes (`%exit`, `Exit`)
   and kept the connection: the `Detach` that followed went the way of a
   client that is not attached (`disconnect`), and a dead client's
   unwritten output is dropped; but then the socket would be closed and the
   client would see its end, not hang.
2. Something specific to that run (the binary of the moment was built
   while files were being edited).

## Root cause

Unknown.

## Fix

None besides the EPIPE one above.

## What is not fixed

The hang, if it is real. Next time: run the server with `-v` and keep its
log, and `sample` the server too.

## Measured cost

None.
