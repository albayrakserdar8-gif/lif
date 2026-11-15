# 0006: on Linux an echo next to a flooding pane takes milliseconds

Status: open. Linux only; found the first time the tests ran on Linux
(Ubuntu 26.04, kernel 7.0, arm64, 6 CPUs, in a Lima VM on the M-series Mac).

## What happened

`echo_is_not_paced_next_to_a_flood` (`crates/lif/tests/latency.rs`) types
into a pane running `cat` while another pane of the same window runs
`yes`, and requires the median echo to arrive within 3 ms. On macOS the
median is about 40 us. On Linux:

```
echo next to a flood: median 3.738241ms, max 10.883815ms
echo next to a flood: median 4.206063ms, max 10.242122ms
echo next to a flood: median 3.988777ms, max 8.974653ms
```

The other three latency tests (idle echo, bare Escape) pass on Linux.

## Expected

An echo is drawn as soon as the pane prints it, whatever another pane
does: well under a millisecond.

## Evidence

Timestamps written by a patched server (in the VM only) for each key:

- the key is written to the pane's PTY at once (`input`);
- the pane's echo is read 0.15 to 3.9 ms later (`paneout`);
- the echo frame goes out 5 us after that (`echoframe`): 63 echo frames
  for 80 keys, the client's socket never full.

So the frame scheduler does what it does on macOS; the time passes between
writing the key to the PTY master and the echo becoming readable on it.
`Server::service` took more than 300 us only 151 times in 10 s, and
lowering `PANE_READ_BUDGET` from 256 KiB to 16 KiB changed nothing.

The same shows without lif. A Python script with `cat` on one PTY and
`yes` on another, measuring write-to-echo on the first:

| reader of the flood | median | p95 | max |
|---|---|---|---|
| another thread, blocked in `read` | 14 us | 51 us | 122 us |
| the measuring thread itself, `epoll_wait(0)` in a loop, 16 KiB a turn | 928 us | 2,061 us | 3,418 us |
| the same, 256 KiB a turn | 1,093 us | 3,629 us | 5,435 us |

(macOS, first row: 3 us.) A thread that never sleeps while it drains a
flooded PTY delays the echo of another PTY it reads. lif's server is such
a thread: with a pane flooding, every turn of its loop finds more to read.

## Hypotheses

1. The kernel work that moves a PTY's input through the line discipline
   (and produces the echo) is queued for the CPU the writer runs on, and
   waits there until the spinning server thread is preempted. Not
   confirmed: neither `sched_yield` nor a 1 us sleep before a
   zero-timeout poll changed the numbers.
2. Something in how epoll reports an edge-triggered PTY master to a
   thread that is already runnable. Not looked at.

## Root cause

Unknown.

## Fix

None. The test is ignored on Linux with a pointer here, so the rest of the
Linux run stays meaningful; it still runs on macOS.

## What is not fixed

The latency itself. Things to try: reading the flood on a schedule (a
budget per millisecond rather than per turn), `EPOLLEXCLUSIVE` or
level-triggered reads for panes, a second look with `perf sched` and
`ftrace` on the tty workqueue, and the same measurement on bare metal
(a VM's scheduler may be part of it).

## Measured cost

Median echo next to a flooding pane: about 4 ms on Linux in this VM,
against 40 us on macOS.
