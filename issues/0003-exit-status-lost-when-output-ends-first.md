# 0003: a pane's exit status is lost when its output ends before it is reaped

Status: fixed in the working tree · Found: 2026-11-15 · Severity: medium (`remain-on-exit failed` could keep a pane that exited with 0; `#{pane_dead_status}` could be empty for a moment)

## What happened

`display-popup -E 'exit 3'` made `lif` exit with 1 instead of 3. The server
log showed the program exiting with status 3 and the popup closing with
status 1:

```
0.652 pane %4026531841 pid 486 exited: ExitStatus(unix_wait_status(768))
0.652 overlay of client client2 ends, status 1
```

## Root cause

`Server::reap` (`crates/lif-server/src/server.rs`) removed the process from
`Rt::pids` and then drained what was left in its PTY with `read_pane`.
`read_pane`, on end of output, reports the pane gone itself when the
process is "not ours any more" (`!pids.contains_key`), which exists for
panes whose process was already collected. So when the PTY's end had been
read before the reap (a program that exits at once), the drain called
`pane_gone(id, None)` first, and only then did `reap` call
`pane_gone(id, Some(3))`.

For an ordinary pane the second call mostly repaired it: with
`remain-on-exit on` the dead pane got its status from the second call.
But the first call is the one that decides: with `remain-on-exit failed`
a status of `None` is "not 0", so a pane that exited with 0 could be kept,
and anything that acts on the first report (a popup's waiting command)
got the wrong status.

## Fix

`reap` keeps the pid in `Rt::pids` during the drain and removes it after,
so the end of the PTY's output during the drain is not mistaken for an
unknown process. The pane is reported gone once, with its status.

Test: `popup_status_and_closing` (`crates/lif/tests/overlay.rs`) runs
`display-popup -E 'exit 3'`, which hit the race every time.

## Not fixed

Nothing known.

## Cost

None.
