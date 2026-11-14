Machine: Mac18,5 (Apple M6, 12 cores: 2 Super, 4 Performance, 6 Efficiency, 16 GiB), macOS 27.2 26B123. Client PTY 200x50. Runs: 2026-11-15T01:03:07, 2026-11-15T01:03:08, 2026-11-15T01:03:08.


### Throughput (MB/s: median [min-max], N runs)

From Enter on `cat FILE; touch MARKER` until MARKER exists. Client bytes: what the client received during the run. Tail: how long the client kept receiving output after the marker.

| | tmux 3.8 | lif 0.1.0 | lifbase 0.1.0 |
|---|---|---|---|
| 100 MB ASCII | - | - | - |
| 100 MB ASCII: client bytes | - | - | - |
| 100 MB ASCII: tail | - | - | - |
| 100 MB SGR + UTF-8 | - | - | - |
| 100 MB SGR + UTF-8: client bytes | - | - | - |
| 100 MB SGR + UTF-8: tail | - | - | - |
| 20 MB single line | - | - | - |
| 20 MB single line: client bytes | - | - | - |
| 20 MB single line: tail | - | - | - |

### Memory (MiB: phys_footprint / RSS)

Server: sum over all server-side processes of the tool. Pane shells are not counted.

| | tmux 3.8 | lif 0.1.0 | lifbase 0.1.0 |
|---|---|---|---|
| server, idle, 1 pane | - | - | - |
| server, idle, 10 panes | - | - | - |
| server, idle, 50 panes | - | - | - |
| server, 100k colored lines in scrollback | - | - | - |
| server, cost per extra pane, 1 to 10 panes | - | - | - |
| server, cost per extra pane, 10 to 50 panes | - | - | - |
| server, cost per extra pane, 1 to 50 panes | - | - | - |
| 50 panes spread over | - | - | - |
| lines retained in scrollback after the 100k-line cat | - | - | - |
| server, after 100 MB cat (default history) | - | - | - |
| lines retained after 100 MB cat (default history) | - | - | - |
| client, idle | - | - | - |

### Idle: one attached session, ? s window

| | tmux 3.8 | lif 0.1.0 | lifbase 0.1.0 |
|---|---|---|---|
| server: CPU ms | - | - | - |
| server: idle wakeups | - | - | - |
| server: interrupt wakeups | - | - | - |
| client: CPU ms | - | - | - |
| client: idle wakeups | - | - | - |
| client: interrupt wakeups | - | - | - |

### Startup (ms: median [p25-p75] of warm runs, then the first run)

| | tmux 3.8 | lif 0.1.0 | lifbase 0.1.0 |
|---|---|---|---|
| create session + kill server (hyperfine) | 11.7 [11.6-11.8] N=30; first 11.9 | 5.5 [5.2-5.8] N=30; first 6.7 | 5.8 [5.6-5.9] N=30; first 13.0 |
| attach, draw, detach (hyperfine; includes 20 ms quiet wait) | 30.8 [30.3-31.9] N=30; first 26.0 | 43.1 [41.7-44.4] N=30; first 33.5 | 48.1 [44.5-49.3] N=30; first 31.8 |
| new session until a typed command runs (driver) | 52.9 [52.1-55.3] N=15 | 41.5 [41.0-42.8] N=15 | 41.3 [40.8-42.1] N=15 |

### Input latency (us: p50 / p95 / p99 / max)

| | tmux 3.8 | lif 0.1.0 | lifbase 0.1.0 |
|---|---|---|---|
| key echo, idle | 131 / 183 / 214 / 282 (N=300) | 110 / 169 / 189 / 286 (N=300) | 2,161 / 6,916 / 8,370 / 8,488 (N=300) |
| key echo, idle: median of each round | 127 134 134 | 109 106 117 | 2,172 2,144 2,156 |
| key echo, other pane runs `yes` | 264 / 296 / 336 / 338 (N=300) | 38 / 46 / 82 / 119 (N=300) | 4,230 / 7,226 / 7,239 / 7,317 (N=300) |
| key echo, other pane runs `yes`: median of each round | 263 267 262 | 38 38 38 | 4,227 4,233 3,951 |
| bare ESC, default config | 11,771 / 12,270 / 12,332 / 12,488 (N=100) | 6,298 / 6,462 / 6,535 / 6,542 (N=100) | 6,438 / 6,530 / 6,580 / 6,640 (N=99, 1 timeouts) |
| bare ESC, escape-time 0 | 1,437 / 1,500 / 1,613 / 2,067 (N=100) | 150 / 210 / 235 / 263 (N=100) | 6,441 / 6,517 / 6,548 / 6,568 (N=100) |
| bare ESC, terminal with kitty keyboard protocol | - | 156 / 220 / 269 / 273 (N=100) | 209 / 691 / 2,127 / 2,189 (N=100) |

### Binary

| | tmux 3.8 | lif 0.1.0 | lifbase 0.1.0 |
|---|---|---|---|
| size | - | - | - |
| non-system dylibs | - | - | - |
| dylibs total | 0 | 0 | 0 |
