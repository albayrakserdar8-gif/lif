Machine: Mac18,5 (Apple M6, 12 cores: 2 Super, 4 Performance, 6 Efficiency, 16 GiB), macOS 27.2 26B123. Client PTY 200x50. Runs: 2026-11-15T14:39:36, 2026-11-15T14:39:36.


### Throughput (MB/s: median [min-max], N runs)

From Enter on `cat FILE; touch MARKER` until MARKER exists. Client bytes: what the client received during the run. Tail: how long the client kept receiving output after the marker.

| | lifbase 0.1.0 | lif 0.1.0 |
|---|---|---|
| 100 MB ASCII | 191.5 [167.6-206.0] N=5 | 194.3 [168.6-210.6] N=5 |
| 100 MB ASCII: client bytes | 0.3 MB | 0.3 MB |
| 100 MB ASCII: tail | 5 ms | 6 ms |
| 100 MB SGR + UTF-8 | 149.9 [129.5-151.6] N=5 | 149.7 [132.4-150.6] N=5 |
| 100 MB SGR + UTF-8: client bytes | 1.2 MB | 1.3 MB |
| 100 MB SGR + UTF-8: tail | 6 ms | 7 ms |
| 20 MB single line | 143.7 [130.5-152.2] N=5 | 153.7 [135.6-160.1] N=5 |
| 20 MB single line: client bytes | 0.2 MB | 0.2 MB |
| 20 MB single line: tail | 4 ms | 5 ms |

### Memory (MiB: phys_footprint / RSS)

Server: sum over all server-side processes of the tool. Pane shells are not counted.

| | lifbase 0.1.0 | lif 0.1.0 |
|---|---|---|
| server, idle, 1 pane | - | - |
| server, idle, 10 panes | - | - |
| server, idle, 50 panes | - | - |
| server, 100k colored lines in scrollback | - | - |
| server, cost per extra pane, 1 to 10 panes | - | - |
| server, cost per extra pane, 10 to 50 panes | - | - |
| server, cost per extra pane, 1 to 50 panes | - | - |
| 50 panes spread over | - | - |
| lines retained in scrollback after the 100k-line cat | - | - |
| server, after 100 MB cat (default history) | 5.7 / 13.7 | 5.6 / 13.8 |
| lines retained after 100 MB cat (default history) | 3,784 | 3,784 |
| client, idle | - | - |

### Idle: one attached session, 20 s window

| | lifbase 0.1.0 | lif 0.1.0 |
|---|---|---|
| server: CPU ms | 0.0 | 0.3 |
| server: idle wakeups | 0 | 1 |
| server: interrupt wakeups | 0 | 1 |
| client: CPU ms | 0.0 | 0.0 |
| client: idle wakeups | 0 | 0 |
| client: interrupt wakeups | 0 | 0 |

### Startup (ms: median [p25-p75] of warm runs, then the first run)

| | lifbase 0.1.0 | lif 0.1.0 |
|---|---|---|
| create session + kill server (hyperfine) | 5.8 [5.6-6.1] N=30; first 7.5 | 5.7 [5.6-6.2] N=30; first 6.2 |
| attach, draw, detach (hyperfine; includes 20 ms quiet wait) | 32.4 [24.7-33.5] N=30; first 24.1 | 30.6 [24.9-31.8] N=30; first 23.8 |
| new session until a typed command runs (driver) | 33.5 [31.1-38.1] N=15 | 37.5 [32.4-38.1] N=15 |

### Input latency (us: p50 / p95 / p99 / max)

| | lifbase 0.1.0 | lif 0.1.0 |
|---|---|---|
| key echo, idle | - | - |
| key echo, idle: median of each round | - | - |
| key echo, other pane runs `yes` | - | - |
| key echo, other pane runs `yes`: median of each round | - | - |
| bare ESC, default config | - | - |
| bare ESC, escape-time 0 | - | - |
| bare ESC, terminal with kitty keyboard protocol | - | - |

### Binary

| | lifbase 0.1.0 | lif 0.1.0 |
|---|---|---|
| size | - | - |
| non-system dylibs | - | - |
| dylibs total | 0 | 0 |
