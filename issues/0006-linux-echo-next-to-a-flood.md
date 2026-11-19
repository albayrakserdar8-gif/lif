# 0006: on Linux an echo next to a flooding pane takes milliseconds

Status: fixed (0.5.0). Linux only; found the first time the tests ran on Linux
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

Reading the flood is what delays the echo. Every read of the flooded PTY
frees room, so its program writes more, and the kernel moves that output
through the tty layer as queued work; the key's echo, and later the frame
that carries it to the client's terminal, are queued behind it. A thread
that reads the flood without a break keeps that queue full. (Hypothesis 1
was close: it is not the CPU the reader runs on but the work its reads
generate. That is why neither `sched_yield` nor a 1 us sleep helped: the
loop still read the flood on every turn.)

The Python reproduction, in the VM, with the reader stopping reading the
flood after the key until the echo arrives (only the typed PTY in epoll):

| after the key | median | p95 | max |
|---|---|---|---|
| keep reading the flood (`epoll_wait(0)` loop) | 891 us | 3,516 us | 6,414 us |
| wait up to 200 us / 1 ms / 5 ms for the echo alone | 18 to 21 us | 74 to 83 us | 91 us |

Blocking in `epoll_wait` with a timeout while the flood stays registered
changes nothing (about 1 ms): the flood keeps reporting new edges.

## Fix

`ECHO_PAUSE` (`crates/lif-server/src/lib.rs`): when a client's input is
written to a pane (`Rt::write_input`), the server stops reading every
other pane for 2 ms. A paused pane that becomes readable gets `read_more`
(the edge is not reported again) and `Server::next_deadline` gives the end
of the pause instead of a zero timeout. The pause is not ended by the echo:
ended there, the flood was read again while the echo's frame went to the
client's terminal, and that frame waited instead (end to end, median
0.9 to 3.1 ms against 140 to 380 us with the full pause).

The same code runs on macOS, where it was not needed and costs nothing
measurable. `echo_is_not_paced_next_to_a_flood` runs on Linux again.

## What is not fixed

The key's way in: until the server has read the key from the client's
terminal it does not know it is coming, so that read still waits behind
the flood. This is what is left in the maximum (3 to 9 ms on Linux).
Input from the mouse pauses floods too; a program asking for every mouse
motion can make another pane's flood wait up to 2 ms per motion event.

## Measured cost

Linux VM (6 CPUs), `echo_is_not_paced_next_to_a_flood`, 3 runs each:

| | test median | max | server key-to-echo-read median / p90 / max |
|---|---|---|---|
| before | 2.4 to 4.2 ms | 6.8 to 11 ms | 397 to 867 / 896 to 3,031 / 2,276 to 6,148 us |
| after | 112 to 119 us | 3.1 to 9.4 ms | 29 / 58 to 81 / 93 to 106 us |

macOS, `bench/run.sh --tools lifbase,lif --only throughput,latency`, the
0.4.0 commit (`lifbase`) against 0.5.0 in one interleaved run
(`bench/results/2026-11-19-1539-0.5.0-ab-summary.md`, load average 3.4 to
1.8): throughput 190.7 / 154.4 / 180.6 MB/s before, 190.7 / 155.3 / 180.5
after (ASCII / SGR / one line); key echo next to `yes` p50 / p99 39 / 98 us
before, 23 / 54 us after; idle key echo p50 110 before, 112 after. (A first
measurement earlier the same day is not valid: the harness measured a stale
`target/release/lif` instead of the tree, which `bench/run.sh` now builds.)

macOS, same test: median 60 us after (40 to 84 us before; the machine
load varies more than that). A flood next to someone typing loses at most
2 ms of reading per key; a flood nobody types next to is not affected.
