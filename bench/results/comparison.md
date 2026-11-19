# 0.5.0 against 0.4.0 (2026-11-19)

Same-run A/B, `bench/run.sh --tools lifbase,lif --only throughput,latency
--tag 0.5.0-ab` (`lifbase`: the 0.4.0 commit built into
`bench/tmp/ab/lif-head`), load average 3.4 at the start, 1.8 at the end:
`results/2026-11-19-1539-0.5.0-ab-summary.md`.

| | 0.4.0 | 0.5.0 |
|---|---|---|
| `cat` 100 MB ASCII / SGR / one line (MB/s) | 190.7 / 154.4 / 180.6 | 190.7 / 155.3 / 180.5 |
| key echo, idle, p50 / p95 (us) | 110 / 155 | 112 / 168 |
| key echo next to `yes`, p50 / p99 (us) | 39 / 98 | 23 / 54 |
| bare ESC, escape-time 0, p50 (us) | 167 | 167 |

The echo next to a flood is the `ECHO_PAUSE` of issues/0006. Idle, the
same way (`--only idle`): 0.3 ms CPU and 1 interrupt wakeup in 30 s for
0.4.0, 0.0 and 0 for 0.5.0 (noise of the machine either way). The full
four-tool table of 0.5.0 is `results/2026-11-19-1543-0.5.0-summary.md`.

Earlier tables measured `target/release/lif` as it happened to be built
(the 0.4.0 table: a build that called itself 0.3.0); `bench/run.sh` now
builds the lif it measures into `target-bench`.

# lif after the performance tour vs tmux 3.8 vs zellij 0.46 vs none (2026-11-15)

One run, all tools, same code, metric-major with throughput and latency
repetitions interleaved across tools:
`bench/run.sh --tools none,tmux,zellij,lif,lifbase --tag tour-full` (01:15-01:28).
Raw data: `results/2026-11-15-0115-tour-full-*.json`. Methodology: `bench/README.md`.
"lif (after)" is `target/release/lif` with the changes of the performance tour
(3.06 MB); "lif HEAD (before)" is the unmodified HEAD build (`bench/tmp/ab/lif-head`,
2.84 MB) measured in the same run under the same load, so every before/after
pair below is a same-run comparison. Both with built-in defaults.

Machine: Mac18,5 (Apple M6, 12 cores: 2 Super, 4 Performance, 6 Efficiency, 16 GiB), macOS 27.2 26B123. Client PTY 200x50. Runs: 2026-11-15T01:15:10, 2026-11-15T01:15:10, 2026-11-15T01:15:10, 2026-11-15T01:15:10, 2026-11-15T01:15:11.

**Machine load**: 1-minute load average 3.8 during throughput and memory, 2.5-4.3
during idle, 2.3-2.6 during startup and latency (other work was running).

## Full table (medians; spread and N in each cell)

#### Throughput (MB/s: median [min-max], N runs)

From Enter on `cat FILE; touch MARKER` until MARKER exists. Client bytes: what the client received during the run. Tail: how long the client kept receiving output after the marker.

| | none | tmux 3.8 | zellij 0.46.0 | lif (after) | lif HEAD (before) |
|---|---|---|---|---|---|
| 100 MB ASCII | 225.2 [200.4-236.2] N=5 | 52.4 [49.9-54.0] N=5 | 79.2 [77.8-80.4] N=5 | 205.8 [201.3-217.8] N=5 | 208.1 [202.6-215.1] N=5 |
| 100 MB ASCII: client bytes | 101.0 MB | 1.9 MB | 1.3 MB | 0.3 MB | 0.3 MB |
| 100 MB ASCII: tail | 0 ms | 144 ms | 4 ms | 5 ms | 4 ms |
| 100 MB SGR + UTF-8 | 284.7 [283.0-298.4] N=5 | 21.3 [21.1-21.4] N=5 | 81.6 [81.2-87.4] N=5 | 151.8 [150.6-153.7] N=5 | 151.0 [150.2-153.2] N=5 |
| 100 MB SGR + UTF-8: client bytes | 100.3 MB | 2.7 MB | 3.1 MB | 1.2 MB | 1.2 MB |
| 100 MB SGR + UTF-8: tail | 0 ms | 87 ms | 4 ms | 2 ms | 4 ms |
| 20 MB single line | 155.1 [152.2-164.5] N=5 | 51.8 [51.8-52.4] N=5 | 17.6 [14.5-18.0] N=5 | 151.1 [149.7-158.9] N=5 | 151.5 [150.7-158.7] N=5 |
| 20 MB single line: client bytes | 20.0 MB | 0.4 MB | 1.0 MB | 0.1 MB | 0.1 MB |
| 20 MB single line: tail | 0 ms | 41 ms | 10 ms | 5 ms | 8 ms |

#### Memory (MiB: phys_footprint / RSS)

Server: sum over all server-side processes of the tool. Pane shells are not counted.

| | none | tmux 3.8 | zellij 0.46.0 | lif (after) | lif HEAD (before) |
|---|---|---|---|---|---|
| server, idle, 1 pane | - | 3.7 / 5.2 | 26.5 / 43.1 | 3.3 / 5.1 | 3.4 / 5.2 |
| server, idle, 10 panes | - | 3.8 / 5.3 (10 panes) | 30.8 / 47.7 (10 panes) | 6.0 / 8.0 (10 panes) | 7.6 / 12.2 (10 panes) |
| server, idle, 50 panes | - | 4.2 / 5.7 (50 panes) | 38.2 / 55.2 (50 panes, 6 floating) | 9.5 / 11.4 (50 panes) | 24.8 / 44.2 (50 panes) |
| server, 100k colored lines in scrollback | - | 285.9 / 287.4 | 231.9 / 248.4 | 43.3 / 235.6 | 184.9 / 193.2 |
| server, cost per extra pane, 1 to 10 panes | - | 11 / 12 KiB | 492 / 528 KiB | 306 / 324 KiB | 482 / 795 KiB |
| server, cost per extra pane, 10 to 50 panes | - | 10 / 10 KiB | 189 / 193 KiB | 88 / 87 KiB | 440 / 819 KiB |
| server, cost per extra pane, 1 to 50 panes | - | 10 / 10 KiB | 245 / 254 KiB | 128 / 131 KiB | 448 / 815 KiB |
| 50 panes spread over | - | 1 window(s) | 1 window(s) | 1 window(s) | 1 window(s) |
| lines retained in scrollback after the 100k-line cat | - | 99,954 | 99,952 | 99,954 | 99,954 |
| server, after 100 MB cat (default history) | - | 6.1 / 7.6 | 44.8 / 61.4 | 5.6 / 13.7 | 8.5 / 16.7 |
| lines retained after 100 MB cat (default history) | - | 1,818 | 9,997 | 3,784 | 5,688 |
| client, idle | - | 3.3 / 4.9 | 12.0 / 26.1 | 1.4 / 2.2 | 1.4 / 2.3 |

#### Idle: one attached session, 30 s window

| | none | tmux 3.8 | zellij 0.46.0 | lif (after) | lif HEAD (before) |
|---|---|---|---|---|---|
| server: CPU ms | - | 2.3 | 40.6 | 0.4 | 0.0 |
| server: idle wakeups | - | 1 | 155 | 0 | 0 |
| server: interrupt wakeups | - | 3 | 361 | 1 | 0 |
| client: CPU ms | 0.0 | 0.0 | 0.2 | 0.0 | 0.0 |
| client: idle wakeups | 0 | 0 | 1 | 0 | 0 |
| client: interrupt wakeups | 0 | 0 | 3 | 0 | 0 |

#### Startup (ms: median [p25-p75] of warm runs, then the first run)

| | none | tmux 3.8 | zellij 0.46.0 | lif (after) | lif HEAD (before) |
|---|---|---|---|---|---|
| create session + kill server (hyperfine) | - | 11.0 [10.8-11.2] N=30; first 12.0 | 53.2 [52.7-53.5] N=30; first 50.6 | 6.1 [6.0-6.1] N=30; first 6.3 | 5.9 [5.7-6.2] N=30; first 6.6 |
| attach, draw, detach (hyperfine; includes 20 ms quiet wait) | - | 31.2 [29.5-34.0] N=30; first 25.8 | 63.9 [63.6-64.0] N=30; first 63.7 | 33.1 [27.6-34.4] N=30; first 24.1 | 46.5 [42.2-47.5] N=30; first 33.3 |
| new session until a typed command runs (driver) | 31.6 [29.3-32.8] N=15 | 51.4 [47.6-52.1] N=15 | 85.5 [83.3-88.3] N=15 | 41.9 [41.4-42.3] N=15 | 41.5 [40.3-42.4] N=15 |

#### Input latency (us: p50 / p95 / p99 / max)

| | none | tmux 3.8 | zellij 0.46.0 | lif (after) | lif HEAD (before) |
|---|---|---|---|---|---|
| key echo, idle | 22 / 37 / 45 / 108 (N=1000) | 146 / 214 / 251 / 345 (N=1000) | 536 / 763 / 838 / 1,440 (N=1000) | 126 / 185 / 208 / 274 (N=1000) | 2,155 / 7,400 / 8,382 / 8,504 (N=1000) |
| key echo, idle: median of each round | 19 22 23 25 19 | 138 131 153 170 138 | 494 553 558 573 520 | 113 130 132 148 110 | 2,102 2,188 2,069 2,167 2,158 |
| key echo, other pane runs `yes` | - | 263 / 332 / 343 / 377 (N=1000) | 949 / 1,063 / 1,112 / 1,268 (N=1000) | 38 / 45 / 74 / 123 (N=1000) | 4,228 / 7,225 / 7,238 / 7,246 (N=1000) |
| key echo, other pane runs `yes`: median of each round | - | 262 266 260 266 264 | 954 957 946 946 937 | 38 38 38 38 38 | 4,233 4,229 4,218 3,834 4,233 |
| bare ESC, default config | 54 / 71 / 78 / 84 (N=333) | 11,696 / 12,284 / 12,315 / 12,344 (N=333) | 61,673 / 63,885 / 64,008 / 64,365 (N=333) | 6,323 / 6,469 / 6,495 / 6,519 (N=333) | 6,407 / 6,531 / 6,570 / 6,630 (N=332, 1 timeouts) |
| bare ESC, escape-time 0 | - | 1,444 / 1,506 / 1,540 / 2,127 (N=333) | - | 157 / 208 / 243 / 333 (N=333) | 6,438 / 6,521 / 6,553 / 6,673 (N=333) |
| bare ESC, terminal with kitty keyboard protocol | - | - | - | 162 / 215 / 247 / 358 (N=333) | 212 / 581 / 2,043 / 2,283 (N=333) |

#### Binary

| | none | tmux 3.8 | zellij 0.46.0 | lif (after) | lif HEAD (before) |
|---|---|---|---|---|---|
| size | - | 1.19 MB | 73.07 MB | 3.06 MB | 2.84 MB |
| non-system dylibs | - | libutf8proc.3.dylib, libncursesw.6.dylib, libevent_core-2.1.7.dylib, libjemalloc.2.dylib | - | - | - |
| dylibs total | 0 | 6 | 23 | 2 | 2 |

Notes: throughput N=5 per tool and file; memory one snapshot per scenario;
idle one 30 s window; startup N=30 warm + first; latency N=1000 echo samples
in 5 rounds of 200, N=333 ESC samples. New rows: `escape-time 0` and a terminal
speaking the kitty keyboard protocol (`lifbench-driver hold --kitty`) for lif;
tmux 3.8 and zellij 0.46 do not push kitty flags, so that row is lif only.
After the 100 MB cat lif keeps 3,784 lines (default `history-limit` is now 4000,
enforced), tmux 1,818, zellij 9,997.

## Reading

### Where lif wins (same run, vs tmux 3.8)

- **Keystroke echo, idle**: 126 / 185 / 208 us (p50 / p95 / p99) vs tmux 146 / 214 /
  251 us, zellij 536 / 763 / 838. Every round's median is below tmux's (113-148 vs
  131-170). Before: 2,155 / 7,400 / 8,382 us.
- **Keystroke echo while another pane runs `yes`**: 38 / 45 / 74 us vs tmux 263 / 332 /
  343, zellij 949 / 1,063. Before: 4,228 / 7,225 (a pacing timer, flat in every round).
  Faster than idle because the cores are awake.
- **Throughput**: unchanged by the tour (ASCII 205.8 vs HEAD 208.1 MB/s, SGR 151.8 vs
  151.0, long line 151.1 vs 151.5; the ranges overlap), still 3.9x / 7.1x / 2.9x tmux.
- **Idle**: 0 idle wakeups and 0.4 ms CPU in 30 s (tmux 1 / 2.3 ms, zellij 155 / 40.6 ms).
- **Bare ESC**: with `escape-time 0` 157 us (tmux with escape-time 0: 1,444 us); with a
  kitty-protocol terminal 162 us whatever escape-time says; with a legacy terminal
  and the default `escape-time` (5 ms) 6.3 ms vs tmux's 11.7 ms (default 10 ms).
- **Memory, small sessions**: 3.3 / 5.1 MiB for one idle pane (tmux 3.7 / 5.2); client
  1.4 / 2.2 (tmux 3.3 / 4.9).
- **Memory after the 100 MB cat (footprint)**: 5.6 MiB with 3,784 lines vs tmux 6.1 MiB
  with 1,818 (before: 8.5 MiB, 5,688 lines).
- **Memory with 100k colored lines (footprint)**: 43.3 MiB vs tmux 285.9, zellij 231.9
  (before: 184.9). Idle scrollback is now compressed completely (see below).
- **Startup**: create session + kill server 6.1 ms (tmux 11.0); new session until a
  typed command runs 41.9 ms (tmux 51.4).

### Where lif still loses, by how much, and why

- **Memory per pane**: 128 / 131 KiB (footprint / RSS) per extra pane from 1 to 50
  panes, 88 / 87 KiB from 10 to 50, vs tmux 10 / 10 KiB (zellij 245 / 254; before 448 /
  815). 50 idle panes: 9.5 / 11.4 MiB vs tmux 4.2 / 5.7 (before 24.8 / 44.2). In this
  scenario all 50 panes are visible; a pane nobody sees costs 46 / 51 KiB (50 hidden
  panes: 4.2 / 6.0 MiB, before 22.2 / 42.4; measured with no client attached). Why
  not 10 KiB: each visible pane holds libghostty's render state (about 80 bytes per
  cell) and lif's `PaneFrame` (32 bytes per cell) next to the terminal (16-30 KiB,
  of which one 16 KiB OS page of storage), and macOS malloc never returns freed
  small blocks (see "malloc" below), so the transient allocations of building a
  50-pane layout stay as about 60 KiB per pane of free-but-dirty heap. tmux keeps
  one compact grid per pane and nothing per client per cell.
- **RSS (not footprint) after large output**: after the 100 MB cat 13.7 MiB vs tmux
  7.6; with 100k lines 235.6 MiB vs tmux 287.4 (before 193.2). Retained history
  costs about 1.7 KB per 200-column line until the pane has been quiet for 2 s;
  then it is compressed (to 45-370 bytes per line, see below) and the footprint
  drops, but libghostty gives the pages' memory back with `madvise(MADV_FREE_REUSABLE)`,
  which on macOS leaves them resident ("reclaimable": counted in RSS, not in the
  footprint, taken back by the kernel first under memory pressure). RSS at or below
  tmux's with more history than tmux is not reachable without changing libghostty's
  row storage; the footprint is.
- **Attach, draw, detach**: 33.1 ms vs tmux 31.2 ms in this run (before 46.5 ms);
  two startup-only re-runs gave 32.1 vs 30.3 and 31.9 vs 32.9 (`results/*-tour-startup-*`).
  Within this machine's noise a tie; the driver's own timeline (first byte, last
  frame, detach) puts lif 1-2 ms ahead (last frame 6-7 ms vs 8-9 ms, detach 1.25 vs
  0.96 ms), and both tools drift by about 10 ms across one hyperfine series.
- **Binary**: 3.06 MB vs tmux 1.19 MB (+4 Homebrew dylibs). The tour added 0.22 MB:
  libghostty's snapshot codec, used to repack resized terminals.
- **New session until a command runs**: 41.9 ms vs "none" 31.6 ms. Unchanged by the
  tour (HEAD 41.5 in the same run); the previous report had 29.2 ms for an older
  build, so this moved before the tour (not investigated further).

## What changed (and what each fix measured)

1. **Keystroke echo.** Cause 1: frame pacing (8 ms) held any echo that came less than
   8 ms after the previous frame, and every echo under a flood. Now pacing applies
   only to output nobody typed for: input from a client gives the pane an "echo
   credit", its next output is refreshed and drawn in the same loop iteration, and
   follow-up output for 20 ms (until 64 KiB, which marks a flood) at most 1 ms apart.
   An echo frame refreshes only the echoing pane; other dirty panes keep their old
   frame (no dirty rows, so the renderer keeps showing it) until their paced frame.
   Synchronized-update holds became per pane, and only panes that changed are
   refreshed. Effect alone (same run, `results/*-tour-echo1-*`): idle 2,132 -> 168 us,
   flood 4,231 -> 50 us, tmux 142 / 260 us.
   Cause 2: the client relayed every key and every frame. Counted with
   `proc_pidinfo(PROC_PIDTASKINFO)` per sample (key + DEL): tmux's client makes 0
   syscalls (its server uses the terminal itself); lif's made 14 syscalls and 4
   context switches. Now the server opens the client's terminal itself (new
   capability `Caps::TTY`, messages `UseTty` / `Tty`; by path, so its O_NONBLOCK never
   reaches the user's shell; held until the client is in raw mode; `Exit` only
   after the terminal took every byte before it), and the client only forwards
   resizes and signals. Per sample now: client 0 syscalls / 0 switches, server 16
   syscalls / 2 switches (tmux 21 / 3), server CPU about a third below tmux's. Idle echo
   168 -> 115 us in the same measurement (tmux 145). Relaying stays as the fallback.
2. **Memory per pane.** Measured cause: a pane born large and shrunk by later splits
   (every pane of a tiled layout) held about 400 KiB of dirty libghostty page (a
   reflow recycles a page by zeroing all of it) plus up to 400 KiB of freed pages in
   its pool (resident on macOS), plus a render state sized for the largest viewport
   it saw (790 KiB at 200x49). A pane created at its final size costs 33 KiB. Now:
   after a width change, once the pane is quiet, `compact` rebuilds a small terminal
   (at most two pages) from a libghostty snapshot of itself, so only what it holds
   is touched (shrunk pane: 536 -> 35 KiB in `lif-vt/tests/memory.rs`; every
   observable checked equal by `compact_after_resize_keeps_every_observable`); panes
   no attached client shows drop their frame and render state; a frame that shrinks
   gives back its wider rows.
3. **Scrollback.** `history-limit` is enforced (libghostty's line limit next to the
   byte budget; both hold), both options apply to existing panes, and `compact` now
   continues on the next loop turns until compression is complete (it ran once per
   quiet period, about 12 of the 25 pages of a 10 MB history at 200 columns). Default `history-limit`
   is 4000 (twice tmux's); `scrollback-bytes` stays 10 MiB as a cap for wide panes.
4. **Attach.** The repaint after the terminal answers lif's capability queries was
   paced 8 ms after the first frame; it now goes out at once (last frame at about
   6 ms after spawn instead of 15).
5. **Bare ESC.** `escape-time` is honored (default 5 ms; 0 never waits, a split
   Alt+key then arrives as Escape and the key, as in tmux); with the kitty keyboard
   protocol there is no wait. Documented in `lif-core` options and `config.example.toml`.

## Diagnostics from the tour

**Scrollback compression** (`cargo test -p lif-vt --release --test memory -- --ignored`),
20,000 lines at 200 columns, 1.73 KB per line raw for every kind of content:

| content | footprint raw -> compacted | per line compacted | time |
|---|---|---|---|
| `ls -l` lines | 33.8 MB -> 1.5 MB | ~75 B | 9 ms |
| log lines | 33.7 MB -> 1.7 MB | ~86 B | 7 ms |
| colored build output (40k rows) | 67.4 MB -> 2.0 MB | ~51 B per row | 7 ms |
| random printable ASCII (the throughput file) | 33.8 MB -> 7.2 MB | ~370 B | 17 ms |
| `seq` | 33.7 MB -> 0.9 MB | ~45 B | 2 ms |

RSS does not drop (reclaimable pages, see above). libghostty counts a compressed
page at its raw size against the byte budget, so compressing during a flood would
keep no more lines and lower no RSS; it would only lower the footprint peak at a
CPU cost, so lif compacts when a pane is quiet (bounded steps per loop turn).

**malloc**: on this macOS, freed small blocks are never returned: 30 MB allocated in
14 KiB + 1 KiB blocks and then all freed stays 30 MB resident, and
`malloc_zone_pressure_relief` releases 0 bytes. A server's footprint therefore
follows the peak of its small allocations; the fixes above lower the peak.
Releasing render states on every shrink was tried and made it worse (fragmentation:
50 panes 9.6 -> 12.5 MiB); repacking with compressed history did not lower RSS.

**Where a 100 MB cat spends the server's time** (`sample`, optimized build with
symbols, 2 s in the middle of six back-to-back cats): ASCII: 70% waiting in kevent,
16% read(2), 10% libghostty parsing, 2% lif (frame fill, render, loop); the PTY
producer is the limit. SGR + UTF-8: 43% waiting, 9% read, 45% libghostty
(`vt_write_until`, simdutf UTF-8 decoding, printSlice, style ref-counting, SGR
parser), 2% lif. The server idles 43% of the time while reaching 55% of the bare
PTY, i.e. cat stalls on the small PTY buffer while lif parses a batch; parsing alone
would sustain about 330 MB/s, so a reader thread overlapping reads with parsing
could lift SGR toward the PTY's ~280 MB/s. That is evidence for a hypothesis, not a
measured win, so no thread was added.

## Findings of the previous report

1. Echo held back by pacing: fixed (echo frames are never paced).
2. Echo under flood 4.61 ms flat: fixed (38 us).
3. Bare ESC waits 5 ms, escape-time unused, docs wrong: escape-time is honored, 0
   means no wait, kitty mode never waits; docs corrected.
4. Config file not read: fixed before the tour (HEAD reads it).
5. scrollback options only for new panes: both now apply to existing panes.
6. history-limit not enforced: enforced (both limits hold).
7. `display-message -p '#{pid}'` empty: not addressed.
8. Per-pane memory: 820 -> 131 KiB RSS per visible pane, 51 KiB per hidden one.
9. Short default scrollback at wide sizes: default is now 4000 lines (3,784 kept
   after the 100 MB cat, 2.1x tmux) in less footprint than tmux.

## Before the performance tour

The previous report (2026-11-14, `--tag full4`, lif 0.1.0 2.49 MB built 23:22),
unchanged except for the heading levels:

One run, all four tools, same code, metric-major with throughput and latency
repetitions interleaved across tools: `bench/run.sh --tools none,tmux,zellij,lif --tag full4`
(23:26-23:37). Raw data: `results/2026-11-14-2326-full4-*.json`. Methodology:
`bench/README.md`. Machine, OS and tool configs as in `results/baseline.md`; lif is
`target/release/lif` (0.1.0, 2.49 MB, built 23:22) with built-in defaults.

**Machine load**: other builds were running. The 1-minute load average was 6.6
when the run started (throughput phase), 3.4-4.0 during memory, 2.1-2.9 during
idle and startup, 2.4-3.8 during latency.

### Full table (medians; spread and N in each cell)

#### Throughput (MB/s: median [min-max], N runs)

From Enter on `cat FILE; touch MARKER` until MARKER exists. Client bytes: what the client received during the run. Tail: how long the client kept receiving output after the marker.

| | none | tmux 3.8 | zellij 0.46.0 | lif 0.1.0 |
|---|---|---|---|---|
| 100 MB ASCII | 231.9 [223.1-253.5] N=5 | 55.6 [52.9-57.6] N=5 | 79.2 [74.7-81.6] N=5 | 210.9 [206.1-238.6] N=5 |
| 100 MB ASCII: client bytes | 101.0 MB | 1.8 MB | 1.3 MB | 0.3 MB |
| 100 MB ASCII: tail | 0 ms | 96 ms | 4 ms | 4 ms |
| 100 MB SGR + UTF-8 | 279.5 [271.0-282.8] N=5 | 21.2 [21.1-21.3] N=5 | 80.9 [75.8-81.4] N=5 | 147.3 [146.9-170.1] N=5 |
| 100 MB SGR + UTF-8: client bytes | 100.3 MB | 2.7 MB | 3.1 MB | 1.2 MB |
| 100 MB SGR + UTF-8: tail | 0 ms | 74 ms | 4 ms | 5 ms |
| 20 MB single line | 154.7 [153.0-160.2] N=5 | 52.4 [50.7-53.0] N=5 | 17.4 [15.8-18.3] N=5 | 151.3 [149.3-159.6] N=5 |
| 20 MB single line: client bytes | 20.0 MB | 0.4 MB | 1.0 MB | 0.2 MB |
| 20 MB single line: tail | 0 ms | 41 ms | 18 ms | 6 ms |

#### Memory (MiB: phys_footprint / RSS)

Server: sum over all server-side processes of the tool. Pane shells are not counted.

| | none | tmux 3.8 | zellij 0.46.0 | lif 0.1.0 |
|---|---|---|---|---|
| server, idle, 1 pane | - | 3.7 / 5.2 | 25.2 / 41.8 | 3.3 / 4.8 |
| server, idle, 10 panes | - | 3.9 / 5.4 (10 panes) | 30.6 / 47.5 (10 panes) | 7.9 / 12.2 (10 panes) |
| server, idle, 50 panes | - | 4.2 / 5.7 (50 panes) | 37.8 / 54.8 (50 panes, 6 floating) | 25.1 / 44.2 (50 panes) |
| server, 100k colored lines in scrollback | - | 270.8 / 272.3 | 231.6 / 248.1 | 184.8 / 192.8 |
| server, cost per extra pane, 1 to 10 panes | - | 25 / 27 KiB | 612 / 647 KiB | 524 / 839 KiB |
| server, cost per extra pane, 10 to 50 panes | - | 7 / 7 KiB | 186 / 189 KiB | 440 / 820 KiB |
| server, cost per extra pane, 1 to 50 panes | - | 10 / 10 KiB | 264 / 273 KiB | 456 / 823 KiB |
| 50 panes spread over | - | 1 window(s) | 1 window(s) | 1 window(s) |
| lines retained in scrollback after the 100k-line cat | - | 99,954 | 99,952 | 99,954 |
| server, after 100 MB cat (default history) | - | 6.4 / 7.9 | 44.5 / 61.1 | 8.3 / 16.2 |
| lines retained after 100 MB cat (default history) | - | 1,818 | 9,997 | 5,688 |
| client, idle | - | 3.3 / 4.9 | 11.9 / 26.1 | 1.3 / 2.2 |

#### Idle: one attached session, 30 s window

| | none | tmux 3.8 | zellij 0.46.0 | lif 0.1.0 |
|---|---|---|---|---|
| server: CPU ms | - | 3.3 | 40.7 | 0.4 |
| server: idle wakeups | - | 2 | 177 | 0 |
| server: interrupt wakeups | - | 3 | 364 | 1 |
| client: CPU ms | 0.0 | 0.0 | 0.2 | 0.1 |
| client: idle wakeups | 0 | 0 | 2 | 0 |
| client: interrupt wakeups | 0 | 0 | 3 | 0 |

#### Startup (ms: median [p25-p75] of warm runs, then the first run)

| | none | tmux 3.8 | zellij 0.46.0 | lif 0.1.0 |
|---|---|---|---|---|
| create session + kill server (hyperfine) | - | 11.0 [10.8-11.2] N=30; first 11.6 | 52.6 [52.0-53.5] N=30; first 50.2 | 6.0 [5.9-6.1] N=30; first 6.7 |
| attach, draw, detach (hyperfine; includes 20 ms quiet wait) | - | 29.5 [27.9-30.5] N=30; first 27.0 | 66.0 [65.7-66.2] N=30; first 64.9 | 33.0 [32.6-33.3] N=30; first 31.8 |
| new session until a typed command runs (driver) | 30.5 [28.3-31.3] N=15 | 49.0 [35.3-50.9] N=15 | 88.6 [86.4-98.0] N=15 | 29.2 [28.8-29.6] N=15 |

#### Input latency (us: p50 / p95 / p99 / max)

| | none | tmux 3.8 | zellij 0.46.0 | lif 0.1.0 |
|---|---|---|---|---|
| key echo, idle | 16 / 26 / 33 / 81 (N=1000) | 115 / 198 / 247 / 3,281 (N=1000) | 387 / 534 / 659 / 10,128 (N=1000) | 1,299 / 5,798 / 6,464 / 6,921 (N=1000) |
| key echo, idle: median of each round | 21 6 16 16 15 | 162 103 112 111 114 | 129 414 412 382 412 | 1,291 1,253 1,227 1,310 1,347 |
| key echo, other pane runs `yes` | - | 262 / 330 / 358 / 430 (N=1000) | 949 / 1,062 / 1,147 / 2,136 (N=1000) | 4,611 / 8,098 / 8,112 / 8,210 (N=1000) |
| key echo, other pane runs `yes`: median of each round | - | 262 261 261 268 220 | 951 951 945 952 949 | 4,611 4,612 4,610 4,609 4,610 |
| bare ESC, default config | 48 / 75 / 117 / 503 (N=333) | 10,960 / 11,239 / 11,339 / 15,987 (N=333) | 56,202 / 58,520 / 58,800 / 61,174 (N=333) | 5,786 / 5,837 / 10,188 / 14,718 (N=333) |
| bare ESC, escape-time 0 | - | 1,284 / 1,337 / 1,363 / 1,490 (N=333) | - | - |

#### Binary

| | none | tmux 3.8 | zellij 0.46.0 | lif 0.1.0 |
|---|---|---|---|---|
| size | - | 1.19 MB | 73.07 MB | 2.49 MB |
| non-system dylibs | - | libutf8proc.3.dylib, libncursesw.6.dylib, libevent_core-2.1.7.dylib, libjemalloc.2.dylib | - | - |
| dylibs total | 0 | 6 | 23 | 2 |

Notes: throughput N=5 per tool and file; memory one snapshot per scenario;
idle one 30 s window; startup N=30 warm + first; latency N=1000 echo samples
in 5 rounds of 200, N=333 ESC samples. 0 timeouts and 0 stray samples in every
latency scenario for every tool. All 50 panes are in one window for every
tool (tmux and lif: `split-window` + `select-layout tiled`; zellij: 44 tiled +
6 floating in one tab, because it refuses a 45th tiled pane at 200x50). Every
tool kept the full 100k colored lines in the scrollback test (99,952-99,954
lines above the viewport; lif needed `scrollback-bytes` raised to 1 GiB). With
default settings after the 100 MB cat the tools keep very different amounts
of history (tmux 1,818 lines, lif 5,688, zellij 9,997), so that row is not a
like-for-like memory comparison.

Idle-echo per-round medians show the machine's fast mode (see baseline.md) in
round 1 for none (6 us), tmux (103-162 us) and zellij (129 vs ~410 us); lif's
rounds are flat at 1.2-1.3 ms because its delay is a timer, not a wakeup cost.

### Diagnostic: keystroke latency vs typing gap

The standard protocol types a key, waits for its echo, types DEL to erase it,
then waits a random 10-20 ms before the next key. Re-run with 300 samples per
tool, idle pane, varying only that gap (23:39-23:41, same machine state):

| gap between keys | none | tmux | zellij | lif |
|---|---|---|---|---|
| 10-20 ms (standard) p50 / p95 / p99 | 16 / 22 / 26 us | 113 / 148 / 158 us | 324 / 2,295 / 27,721 us | 1,241 / 6,405 / 7,433 us |
| 30-60 ms p50 / p95 / p99 | 22 / 34 / 39 us | 159 / 217 / 259 us | 499 / 745 / 3,808 us | **167 / 247 / 281 us** |

lif's echo is as fast as tmux's when the previous frame is more than 8 ms old,
and up to 8 ms late otherwise; see finding 1. Pane size (80x24 vs 200x50) and
the status line made no difference to lif's 1.2-1.3 ms median (measured too).
