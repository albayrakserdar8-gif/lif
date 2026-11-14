# Baseline: none, tmux 3.8, zellij 0.46 (2026-11-14)

The numbers lif will be judged against. Produced by `bench/run.sh`; methodology in
`bench/README.md`. Raw data (every sample) is in the JSON files next to this one.

## Setup

| | |
|---|---|
| Machine | Mac mini (Mac18,5), Apple M6, 12 cores (2 Super, 4 Performance, 6 Efficiency), 16 GB, AC power |
| OS | macOS 27.2 (26B123), Darwin 27.2.0 |
| Conditions | a desktop in normal use, not an isolated host: WindowServer, a browser, other people's builds; 1-minute load average 1.9 to 5.0 during the runs (recorded per metric in the JSON) |
| tmux | 3.8 (Homebrew), 1.19 MB, links libutf8proc, libncursesw, libevent_core, libjemalloc + 2 system libs |
| zellij | 0.46.0 (Homebrew), 73.07 MB, links only system libraries/frameworks (23) |
| Harness | lifbench-driver (release build, rustc 1.99.1), hyperfine 2.0.0, Python 3.15.2 |
| Client terminal | the driver's PTY, 200x50, TERM=xterm-256color, LANG=en_US.UTF-8; answers DA/DSR/color/size queries like a basic xterm |
| Pane shell | /bin/sh, empty private HOME (no rc files) |
| tmux config | `-f /dev/null`: built-in defaults (escape-time 10, history-limit 2000, status on, status-interval 15). Scrollback scenario: `set -g history-limit 200000`. ESC scenario 2: `set -sg escape-time 0` |
| zellij config | `show_startup_tips false`, `show_release_notes false`, first-run onboarding marked dismissed; otherwise defaults, including the default layout with tab bar and status bar. Scrollback scenario: `scroll_buffer_size 200000` |

Runs used: **C** (22:45) and **D** (22:54), two complete runs of the same code,
and **E** (23:04), latency only, to confirm D's latency mode. Tables below are
run D; the stability table compares C and D.

## Results (run D)

### Throughput (MB/s: median [min-max], N runs)

From Enter on `cat FILE; touch MARKER` until MARKER exists. Client bytes: what the client received during the run. Tail: how long the client kept receiving output after the marker.

| | none | tmux 3.8 | zellij 0.46.0 |
|---|---|---|---|
| 100 MB ASCII | 225.8 [223.2-240.1] N=5 | 53.0 [52.3-54.9] N=5 | 78.8 [78.3-79.1] N=5 |
| 100 MB ASCII: client bytes | 101.0 MB | 1.9 MB | 1.4 MB |
| 100 MB ASCII: tail | 0 ms | 160 ms | 4 ms |
| 100 MB SGR + UTF-8 | 276.3 [270.4-280.4] N=5 | 21.1 [21.1-21.2] N=5 | 81.0 [80.8-81.3] N=5 |
| 100 MB SGR + UTF-8: client bytes | 100.3 MB | 2.7 MB | 3.4 MB |
| 100 MB SGR + UTF-8: tail | 0 ms | 59 ms | 4 ms |
| 20 MB single line | 161.1 [157.7-168.1] N=5 | 52.4 [51.1-52.7] N=5 | 16.4 [14.3-17.2] N=5 |
| 20 MB single line: client bytes | 20.0 MB | 0.4 MB | 1.1 MB |
| 20 MB single line: tail | 0 ms | 43 ms | 13 ms |

### Memory (MiB: phys_footprint / RSS)

Server: sum over all server-side processes of the tool. Pane shells are not counted.

| | none | tmux 3.8 | zellij 0.46.0 |
|---|---|---|---|
| server, idle, 1 pane | - | 3.7 / 5.2 | 25.1 / 41.7 |
| server, idle, 10 panes | - | 3.8 / 5.2 (10 panes) | 30.2 / 47.1 (10 panes) |
| server, idle, 50 panes | - | 4.2 / 5.7 (50 panes) | 36.8 / 53.8 (50 panes, 6 floating) |
| server, 100k colored lines in scrollback | - | 289.3 / 290.8 | 232.6 / 249.1 |
| server, after 100 MB cat (default history) | - | 5.4 / 6.9 | 45.0 / 61.6 |
| client, idle | - | 3.3 / 4.9 | 11.8 / 25.9 |

### Idle: one attached session, 30 s window

| | none | tmux 3.8 | zellij 0.46.0 |
|---|---|---|---|
| server: CPU ms | - | 3.4 | 36.2 |
| server: idle wakeups | - | 0 | 136 |
| server: interrupt wakeups | - | 3 | 363 |
| client: CPU ms | 0.0 | 0.0 | 0.2 |
| client: idle wakeups | 0 | 0 | 1 |
| client: interrupt wakeups | 0 | 0 | 3 |

### Startup (ms: median [p25-p75] of warm runs, then the first run)

| | none | tmux 3.8 | zellij 0.46.0 |
|---|---|---|---|
| create session + kill server (hyperfine) | - | 11.3 [11.2-11.5] N=30; first 11.3 | 53.1 [52.6-53.9] N=30; first 51.2 |
| attach, draw, detach (hyperfine; includes 20 ms quiet wait) | - | 29.9 [29.0-31.2] N=30; first 25.6 | 64.5 [64.3-64.9] N=30; first 63.5 |
| new session until a typed command runs (driver) | 31.1 [29.5-31.6] N=15 | 49.8 [46.2-51.1] N=15 | 85.1 [84.7-86.6] N=15 |

### Input latency (us: p50 / p95 / p99 / max)

| | none | tmux 3.8 | zellij 0.46.0 |
|---|---|---|---|
| key echo, idle | 18 / 27 / 31 / 142 (N=1000) | 128 / 179 / 219 / 1,103 (N=1000) | 459 / 597 / 694 / 1,052 (N=1000) |
| key echo, idle: median of each round | 18 18 18 18 19 | 133 131 122 126 131 | 462 450 464 459 466 |
| key echo, other pane runs `yes` | - | 263 / 331 / 366 / 650 (N=1000) | 951 / 1,080 / 1,183 / 1,497 (N=1000) |
| key echo, other pane runs `yes`: median of each round | - | 266 261 260 285 265 | 955 949 946 959 945 |
| bare ESC, default config | 49 / 65 / 74 / 87 (N=333) | 11,582 / 12,251 / 12,285 / 12,537 (N=333) | 60,720 / 63,783 / 64,038 / 65,263 (N=333) |
| bare ESC, escape-time 0 | - | 1,430 / 1,483 / 1,511 / 2,164 (N=333) | - |

### Binary

| | none | tmux 3.8 | zellij 0.46.0 |
|---|---|---|---|
| size | - | 1.19 MB | 73.07 MB |
| non-system dylibs | - | libutf8proc.3.dylib, libncursesw.6.dylib, libevent_core-2.1.7.dylib, libjemalloc.2.dylib | - |
| dylibs total | 0 | 6 | 23 |

Notes on these tables:

- Throughput N=5 per file per tool, each in a fresh session, repetitions
  interleaved across tools. "none" is the fastest everywhere, as it must be.
- Memory is one snapshot per scenario (N=1). Server = all server-side processes
  (one process for both tools; zellij runs plugins as threads). The client is
  `tmux attach` / `zellij attach`. RSS from rusage matched `ps -o rss` exactly in every snapshot.
- zellij refuses a 45th tiled pane at 200x50 ("No room for a new pane", with or
  without `--stacked`), so its 50-pane figure has 44 tiled and 6 floating panes.
- Idle is one 30 s window (N=1 per run).
- Latency: 1000 samples per tool and scenario in 5 interleaved rounds, 0
  timeouts and 0 stray hits in every scenario. ESC: 333 samples.
- Startup hyperfine rows: N=30 warm runs plus the first run.

## Stability: run C vs run D

| metric | tool | C median | D median | diff | C range | D range |
|---|---|---|---|---|---|---|
| throughput ascii MB/s | none | 222.3 | 225.8 | +2% | 217.8-225.1 | 223.2-240.1 |
| throughput ascii MB/s | tmux | 52.4 | 53.0 | +1% | 52.0-53.5 | 52.3-54.9 |
| throughput ascii MB/s | zellij | 79.2 | 78.8 | -0% | 79.1-81.3 | 78.3-79.1 |
| throughput sgr MB/s | none | 278.0 | 276.3 | -1% | 265.3-281.5 | 270.4-280.4 |
| throughput sgr MB/s | tmux | 21.1 | 21.1 | +0% | 21.0-21.2 | 21.1-21.2 |
| throughput sgr MB/s | zellij | 81.1 | 81.0 | -0% | 59.6-83.0 | 80.8-81.3 |
| throughput longline MB/s | none | 156.3 | 161.1 | +3% | 153.3-158.0 | 157.7-168.1 |
| throughput longline MB/s | tmux | 52.3 | 52.4 | +0% | 51.7-54.5 | 51.1-52.7 |
| throughput longline MB/s | zellij | 14.8 | 16.4 | +11% | 14.4-18.5 | 14.3-17.2 |
| latency echo_idle p50 us | none | 10.1 | 18.1 | +79% | 5.0-20.1 | 16.1-21.0 |
| latency echo_idle p50 us | tmux | 59.4 | 128.1 | +116% | 28.8-121.7 | 116.4-142.7 |
| latency echo_idle p50 us | zellij | 245.1 | 459.2 | +87% | 143.6-405.0 | 426.2-495.5 |
| latency echo_flood p50 us | tmux | 263.9 | 263.4 | -0% | 206.1-274.0 | 208.3-277.9 |
| latency echo_flood p50 us | zellij | 957.0 | 951.0 | -1% | 920.8-1,005.7 | 916.8-992.0 |
| latency esc_default p50 us | none | 50.5 | 49.4 | -2% | 44.5-56.8 | 42.8-56.2 |
| latency esc_default p50 us | tmux | 11,101.4 | 11,581.6 | +4% | 10,461.2-11,200.0 | 10,666.9-12,194.8 |
| latency esc_default p50 us | zellij | 57,061.5 | 60,720.2 | +6% | 54,812.1-57,998.2 | 56,191.8-62,854.4 |
| latency esc_time_0 p50 us | tmux | 1,304.5 | 1,429.8 | +10% | 1,288.9-1,324.8 | 1,414.0-1,448.6 |
| startup create_kill ms | tmux | 11.4 | 11.3 | -1% | 11.2-11.6 | 11.2-11.5 |
| startup create_kill ms | zellij | 53.4 | 53.1 | -0% | 52.7-54.2 | 52.6-53.9 |
| startup attach_detach ms | tmux | 31.1 | 29.9 | -4% | 28.5-33.2 | 29.0-31.2 |
| startup attach_detach ms | zellij | 64.2 | 64.5 | +0% | 63.8-65.0 | 64.3-64.9 |
| startup new_session_to_command ms | none | 30.7 | 31.1 | +1% | 28.0-32.9 | 29.5-31.6 |
| startup new_session_to_command ms | tmux | 51.1 | 49.8 | -3% | 49.1-53.0 | 46.2-51.1 |
| startup new_session_to_command ms | zellij | 87.5 | 85.1 | -3% | 84.4-90.6 | 84.7-86.6 |

For latency the table shows p25-p75. Throughput, startup and the
latency-under-flood and ESC medians agree within 0-4% (zellij long line 11%,
tmux ESC with escape-time 0 10%), each well inside or next to the other run's
spread. The idle echo latency does not agree, see below.

Other metrics across four runs: C, D, and two earlier full runs at 22:10 and
22:28 whose files were not kept, because the harness changed afterwards
(latency rounds, zellij floating panes) in ways that do not affect these
metrics:

| | tmux | zellij |
|---|---|---|
| server footprint, idle 1 pane | 3.7-3.8 MiB | 25.1-26.1 MiB |
| server footprint, 100k colored lines retained | 230-303 MiB | 232-233 MiB |
| server CPU in 30 s idle | 0.9-3.5 ms | 11.9-39.8 ms |
| server idle wakeups in 30 s | 0-1 | 16-170 |
| server interrupt wakeups in 30 s | 3 | 360-366 |

tmux's scrollback footprint moves by 30% between identical runs; we have not
established why (the snapshot is taken 1 s after output stops plus 2 s, and
tmux links jemalloc, whose purging is time-based, which is a plausible cause).
Treat it as 230-300 MiB. zellij's idle wakeup count also varies by 10x
between runs while its interrupt wakeups and CPU time stay within a factor
of 3.

## Keystroke latency has two machine-wide modes

Per-round medians (us) of the idle echo test:

| run | none | tmux | zellij |
|---|---|---|---|
| C | 8 4 5 21 20 | 47 23 29 148 120 | 191 117 223 427 426 |
| D | 18 18 18 18 19 | 133 131 122 126 131 | 462 450 464 459 466 |
| E | 21 18 18 17 18 | 132 128 123 129 121 | 459 443 455 452 438 |

In most runs (D, E, and three earlier latency runs) the medians are steady at
about 18 / 125 / 455 us. In some stretches (the first three rounds of C, and
two earlier runs) every tool, including "none", is 2-4x faster at the same
time, so it is a property of the machine, not of a tool. Load average was
not higher in C's fast rounds (2.5-2.9) than in its slow ones (4.0-5.0), and
a 45 s rest before the metric did not prevent it; the cause is unknown.
Total CPU use per round is recorded from run E on, to help find it. The baseline is the steady mode (run D, confirmed
by E): **none 18 us, tmux 128 us, zellij 459 us** median. When comparing lif,
use rounds that agree with each other, and compare tools within one run.

The latency-under-flood numbers do not show the two modes (C and D agree
within 1%).

## Observations

- Throughput: zellij parses faster than tmux on 100 MB of plain ASCII (79 vs
  53 MB/s) and much faster on SGR-heavy UTF-8 (81 vs 21 MB/s: tmux loses 60%
  on colored UTF-8 text, zellij nothing). On one 20 MB line zellij drops to
  15-16 MB/s, tmux keeps 52 MB/s. The bare PTY ("none") carries 156-278 MB/s
  through the same driver, so neither multiplexer is bounded by the PTY.
- Redraw volume: for 100 MB of input the tmux client received 1.9-2.7 MB and
  the zellij client 1.3-3.4 MB, i.e. both coalesce aggressively. tmux keeps
  sending for 40-160 ms after the pane's command exits, zellij 4-13 ms.
- Keystrokes: tmux sends the client 4 bytes per typed character plus erase,
  the minimum; zellij about 960 bytes per pair.
- With a second pane printing `yes`, keystroke echo takes 263 us in tmux and
  950 us in zellij (median); during those 1000 samples (about 16 s) the tmux
  client received 44 MB and the zellij client 18 MB.
- Bare ESC reaches the pane after 11.5 ms in tmux (escape-time 10 ms), 1.4 ms
  with escape-time 0 (not 0: some other delay remains, about 1.3 ms above
  the echo path), and 60 ms in zellij, which has no setting for it.
- Idle: tmux makes essentially no wakeups (0-1 idle, 3 interrupt wakeups per
  30 s with its status line on). zellij uses 12-40 ms CPU and ~360 interrupt
  wakeups per 30 s (~12/s) while nothing happens.
- Memory: tmux 3.7 MiB for one idle pane, +10 KiB per extra idle pane; zellij
  25 MiB plus a 12 MiB client, +0.25 MiB per extra pane. Both hold 100k
  colored lines of scrollback in about 230 MiB (tmux 230-300).
- Startup: tmux creates a session and kills its server in 11 ms, zellij in
  53 ms; until a typed command runs in a new attached session: tmux 50 ms,
  zellij 85 ms, a bare shell 31 ms.

## What could not be measured, or only with caveats

- **powermetrics** needs root; it was not used. Wakeups come from
  `proc_pid_rusage` (the counters `top` shows as IDLEW), which needs no root.
- **Cold start**: dropping the file cache needs root (`purge`). "first" in the
  startup table is only the first of 31 runs and is no slower than the rest,
  so it is not a cold start.
- **Create + kill** times stop when the kill command returns; both servers
  finish exiting after that (the harness waits for it, untimed).
- **Attach/detach** includes a fixed 20 ms wait for the client to stop
  drawing; subtract it for the bare cost (tmux ~10 ms, zellij ~45 ms).
- **Memory** is a single snapshot per scenario and run, not a peak.
- **Input latency** depends on the machine mode described above; the absolute
  numbers will differ on another machine, or on this one at another time.
- The driver is a minimal terminal. A full terminal emulator answering more
  queries (kitty keyboard protocol, more DECRQM modes) may make zellij or tmux
  take different code paths.
