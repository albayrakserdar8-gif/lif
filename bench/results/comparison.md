# lif 0.1.0 vs tmux 3.8 vs zellij 0.46 vs none (2026-11-14)

One run, all four tools, same code, metric-major with throughput and latency
repetitions interleaved across tools: `bench/run.sh --tools none,tmux,zellij,lif --tag full4`
(23:26-23:37). Raw data: `results/2026-11-14-2326-full4-*.json`. Methodology:
`bench/README.md`. Machine, OS and tool configs as in `results/baseline.md`; lif is
`target/release/lif` (0.1.0, 2.49 MB, built 23:22) with built-in defaults.

**Machine load**: other builds were running. The 1-minute load average was 6.6
when the run started (throughput phase), 3.4-4.0 during memory, 2.1-2.9 during
idle and startup, 2.4-3.8 during latency.

## Full table (medians; spread and N in each cell)

### Throughput (MB/s: median [min-max], N runs)

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

### Memory (MiB: phys_footprint / RSS)

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

### Idle: one attached session, 30 s window

| | none | tmux 3.8 | zellij 0.46.0 | lif 0.1.0 |
|---|---|---|---|---|
| server: CPU ms | - | 3.3 | 40.7 | 0.4 |
| server: idle wakeups | - | 2 | 177 | 0 |
| server: interrupt wakeups | - | 3 | 364 | 1 |
| client: CPU ms | 0.0 | 0.0 | 0.2 | 0.1 |
| client: idle wakeups | 0 | 0 | 2 | 0 |
| client: interrupt wakeups | 0 | 0 | 3 | 0 |

### Startup (ms: median [p25-p75] of warm runs, then the first run)

| | none | tmux 3.8 | zellij 0.46.0 | lif 0.1.0 |
|---|---|---|---|---|
| create session + kill server (hyperfine) | - | 11.0 [10.8-11.2] N=30; first 11.6 | 52.6 [52.0-53.5] N=30; first 50.2 | 6.0 [5.9-6.1] N=30; first 6.7 |
| attach, draw, detach (hyperfine; includes 20 ms quiet wait) | - | 29.5 [27.9-30.5] N=30; first 27.0 | 66.0 [65.7-66.2] N=30; first 64.9 | 33.0 [32.6-33.3] N=30; first 31.8 |
| new session until a typed command runs (driver) | 30.5 [28.3-31.3] N=15 | 49.0 [35.3-50.9] N=15 | 88.6 [86.4-98.0] N=15 | 29.2 [28.8-29.6] N=15 |

### Input latency (us: p50 / p95 / p99 / max)

| | none | tmux 3.8 | zellij 0.46.0 | lif 0.1.0 |
|---|---|---|---|---|
| key echo, idle | 16 / 26 / 33 / 81 (N=1000) | 115 / 198 / 247 / 3,281 (N=1000) | 387 / 534 / 659 / 10,128 (N=1000) | 1,299 / 5,798 / 6,464 / 6,921 (N=1000) |
| key echo, idle: median of each round | 21 6 16 16 15 | 162 103 112 111 114 | 129 414 412 382 412 | 1,291 1,253 1,227 1,310 1,347 |
| key echo, other pane runs `yes` | - | 262 / 330 / 358 / 430 (N=1000) | 949 / 1,062 / 1,147 / 2,136 (N=1000) | 4,611 / 8,098 / 8,112 / 8,210 (N=1000) |
| key echo, other pane runs `yes`: median of each round | - | 262 261 261 268 220 | 951 951 945 952 949 | 4,611 4,612 4,610 4,609 4,610 |
| bare ESC, default config | 48 / 75 / 117 / 503 (N=333) | 10,960 / 11,239 / 11,339 / 15,987 (N=333) | 56,202 / 58,520 / 58,800 / 61,174 (N=333) | 5,786 / 5,837 / 10,188 / 14,718 (N=333) |
| bare ESC, escape-time 0 | - | 1,284 / 1,337 / 1,363 / 1,490 (N=333) | - | - |

### Binary

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

## Diagnostic: keystroke latency vs typing gap

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

## Reading: where lif wins, where it loses

Wins:
- **Throughput**: 3.8x tmux and 2.7x zellij on plain ASCII (211 vs 56 / 79 MB/s),
  7x tmux and 1.8x zellij on SGR-colored UTF-8 (147 vs 21 / 81 MB/s), 2.9x tmux
  and 8.7x zellij on one 20 MB line (151 vs 52 / 17 MB/s). On ASCII and the long
  line lif reaches 91% and 98% of the bare PTY ("none"); on SGR text 53%.
  It also sends its client the least (0.2-1.2 MB per run vs 0.4-3.1 MB) and
  finishes drawing within 4-6 ms of the command ending (tmux 41-96 ms).
- **Idle**: 0.4 ms CPU, 0 idle wakeups and 1 interrupt wakeup in 30 s (tmux 3.3
  ms / 2 / 3, zellij 40.7 ms / 177 / 364). The client used 0.1 ms CPU.
- **Startup**: create session + kill server 6.0 ms (tmux 11.0, zellij 52.6);
  new session until a typed command runs 29.2 ms, the same as a bare shell on
  the PTY (30.5 ms; tmux 49.0, zellij 88.6).
- **Memory, small sessions**: 3.3 MiB footprint for one idle pane (tmux 3.7,
  zellij 25.2), client 1.3 MiB (tmux 3.3, zellij 11.9).
- **Memory, 100k colored ~100-column lines retained**: 185 MiB (tmux 271,
  zellij 232). lif stores scrollback rows at full width (~1.7 KB per 200-column
  row whatever the content), so for short lines this flips: 232k lines of `seq`
  output took 393 MB of lif's budget.
- **Bare ESC**: 5.8 ms vs tmux 11.0 ms (default escape-time 10) and zellij 56 ms;
  tmux with escape-time 0 is faster (1.3 ms).
- **Binary**: 2.49 MB with only system libraries (tmux 1.19 MB + 4 Homebrew
  libraries, zellij 73 MB).

Loses:
- **Keystroke echo**: median 1.30 ms, p95 5.8 ms, vs tmux 0.115 / 0.198 ms and
  zellij 0.387 / 0.534 ms in the standard protocol: 11x tmux at the median,
  29x at p95. Only typing within 8 ms of the previous frame is affected (gap
  diagnostic above).
- **Keystroke echo while another pane floods output**: 4.61 ms in every round
  (p95 8.1 ms) vs tmux 0.262 ms and zellij 0.949 ms: 18x tmux, 5x zellij.
- **Memory per pane**: 456 KiB footprint / 823 KiB RSS per extra idle pane,
  vs tmux 10 / 10 KiB and zellij 264 / 273 KiB. 50 idle panes: lif 25.1 MiB,
  tmux 4.2, zellij 37.8.
- **Attach + draw + detach**: 33.0 ms vs tmux 29.5 ms (both include a 20 ms
  quiet wait, so about 13 vs 9.5 ms of work); zellij 66.0 ms.

## Findings (lif 0.1.0)

No crash, hang, or failed measurement in any metric. After cat of the 100 MB
ASCII and 100 MB SGR files, `capture-pane -p` showed the same final screen in
lif as in tmux (checked for the last lines of each file).

1. **Keystroke echo held back by frame pacing.** A key whose echo would be
   rendered less than 8 ms after the previous frame to that client waits until
   8 ms have passed (`MIN_FRAME_INTERVAL` in `crates/lif-server/src/lib.rs`; the
   doc comment says pacing "never shows", but it shows for typing). In the
   benchmark the DEL echo is the previous frame, so the next key, 10-20 ms later,
   often lands inside the window: median 1.3 ms, p95 5.8 ms; with 30-60 ms gaps,
   median 167 us. Reproduce: `bench/run.sh --tools tmux,lif --only latency`.
2. **Echo while another pane produces output: 4.61 ms, flat.** Every round's
   median is 4,609-4,612 us, i.e. the key waits for a timer while the `yes`
   pane keeps the session dirty, although the client received only 36 KB in
   1000 samples (the `yes` screen does not change, so nothing else is sent).
   Reproduce: as above (scenario "key echo, other pane runs yes").
3. **A bare ESC waits 5 ms** (`DEFAULT_ESC_TIMEOUT` in
   `crates/lif-input/src/parser.rs`), while `config.example.toml` and
   `lif-core/src/options.rs` say lif never waits after Escape and `escape-time` is
   unused. It cannot be set to 0. Measured 5,786 us median.
4. **The config file is not read.** `lif-config` is not a dependency of the `lif` or
   `lif-server` crates. Reproduce: write `scrollback = "1GB"` to a file, start
   `LIF_CONFIG=file lif -S /tmp/x.sock new-session -d`, then
   `lif -S /tmp/x.sock show-options -g scrollback-bytes` prints 10485760.
5. **`set-option -g scrollback-bytes` only affects panes created afterwards.**
   After setting it to 400 MB, the existing pane still had its old budget and
   kept 5,784 lines of `seq 1 300000`; a new window kept 232,122. (tmux's
   `history-limit` behaves the same way, so this may be intended.)
6. **`history-limit` is not enforced**: with history-limit 200000, list-panes
   showed `[history 232122/200000, 393297687 bytes]`.
7. **`display-message -p '#{pid}'` prints an empty line** (tmux prints the server
   pid). The harness finds the server as the process with parent 1 whose command
   line names the socket.
8. **Per-pane memory is high**: about 820 KiB RSS per idle 200x50-tiled pane
   (tmux 10 KiB). Each new pane also shows a 409,600-byte scrollback figure in
   `list-panes` before any output.
9. **Default scrollback is short at wide sizes**: the 10 MB default kept 5,688
   lines after the 100 MB cat at 200 columns (rows cost ~1.7 KB each); at 80
   columns it would keep more.
