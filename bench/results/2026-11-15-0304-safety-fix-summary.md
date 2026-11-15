Machine: Mac18,5 (Apple M6, 12 cores: 2 Super, 4 Performance, 6 Efficiency, 16 GiB), macOS 27.2 26B123. Client PTY 200x50. Runs: 2026-11-15T03:04:11, 2026-11-15T03:04:12, 2026-11-15T03:04:12.


### Throughput (MB/s: median [min-max], N runs)

From Enter on `cat FILE; touch MARKER` until MARKER exists. Client bytes: what the client received during the run. Tail: how long the client kept receiving output after the marker.

| | tmux 3.8 | lif 0.1.0 | lifbase 0.1.0 |
|---|---|---|---|
| 100 MB ASCII | 51.5 [42.3-52.2] N=5 | 177.7 [150.3-188.9] N=5 | 177.2 [174.3-187.4] N=5 |
| 100 MB ASCII: client bytes | 2.0 MB | 0.4 MB | 0.4 MB |
| 100 MB ASCII: tail | 109 ms | 1 ms | 5 ms |
| 100 MB SGR + UTF-8 | 20.3 [20.0-20.6] N=5 | 172.2 [150.0-174.9] N=5 | 173.1 [168.3-174.4] N=5 |
| 100 MB SGR + UTF-8: client bytes | 2.8 MB | 1.1 MB | 1.1 MB |
| 100 MB SGR + UTF-8: tail | 74 ms | 5 ms | 1 ms |
| 20 MB single line | 54.4 [51.4-56.3] N=5 | 154.9 [149.1-180.9] N=5 | 151.7 [150.1-182.8] N=5 |
| 20 MB single line: client bytes | 0.4 MB | 0.2 MB | 0.1 MB |
| 20 MB single line: tail | 57 ms | 4 ms | 5 ms |

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
| server, after 100 MB cat (default history) | 6.1 / 7.5 | 5.6 / 13.8 | 5.6 / 13.7 |
| lines retained after 100 MB cat (default history) | 1,818 | 3,784 | 3,784 |
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
| create session + kill server (hyperfine) | - | - | - |
| attach, draw, detach (hyperfine; includes 20 ms quiet wait) | - | - | - |
| new session until a typed command runs (driver) | - | - | - |

### Input latency (us: p50 / p95 / p99 / max)

| | tmux 3.8 | lif 0.1.0 | lifbase 0.1.0 |
|---|---|---|---|
| key echo, idle | 134 / 193 / 224 / 269 (N=1000) | 117 / 172 / 199 / 227 (N=1000) | 115 / 168 / 193 / 1,365 (N=1000) |
| key echo, idle: median of each round | 131 135 133 138 136 | 116 119 114 120 118 | 112 110 128 112 115 |
| key echo, other pane runs `yes` | 262 / 329 / 345 / 444 (N=1000) | 39 / 47 / 65 / 123 (N=1000) | 38 / 45 / 69 / 126 (N=1000) |
| key echo, other pane runs `yes`: median of each round | 208 266 264 263 258 | 39 40 38 39 38 | 39 39 38 37 37 |
| bare ESC, default config | 11,741 / 12,290 / 12,320 / 12,374 (N=333) | 6,293 / 6,467 / 6,493 / 6,533 (N=333) | 6,328 / 6,461 / 6,507 / 6,591 (N=333) |
| bare ESC, escape-time 0 | 1,438 / 1,506 / 1,543 / 2,171 (N=333) | 165 / 224 / 259 / 368 (N=333) | 152 / 214 / 240 / 296 (N=333) |
| bare ESC, terminal with kitty keyboard protocol | - | 164 / 215 / 259 / 296 (N=333) | 164 / 224 / 264 / 338 (N=333) |

### Binary

| | tmux 3.8 | lif 0.1.0 | lifbase 0.1.0 |
|---|---|---|---|
| size | - | - | - |
| non-system dylibs | - | - | - |
| dylibs total | 0 | 0 | 0 |
