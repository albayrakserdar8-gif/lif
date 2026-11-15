Machine: Mac18,5 (Apple M6, 12 cores: 2 Super, 4 Performance, 6 Efficiency, 16 GiB), macOS 27.2 26B123. Client PTY 200x50. Runs: 2026-11-15T21:59:47, 2026-11-15T21:59:47.


### Throughput (MB/s: median [min-max], N runs)

From Enter on `cat FILE; touch MARKER` until MARKER exists. Client bytes: what the client received during the run. Tail: how long the client kept receiving output after the marker.

| | lif 0.3.0 | lifbase 0.3.0 |
|---|---|---|
| 100 MB ASCII | - | - |
| 100 MB ASCII: client bytes | - | - |
| 100 MB ASCII: tail | - | - |
| 100 MB SGR + UTF-8 | - | - |
| 100 MB SGR + UTF-8: client bytes | - | - |
| 100 MB SGR + UTF-8: tail | - | - |
| 20 MB single line | - | - |
| 20 MB single line: client bytes | - | - |
| 20 MB single line: tail | - | - |

### Memory (MiB: phys_footprint / RSS)

Server: sum over all server-side processes of the tool. Pane shells are not counted.

| | lif 0.3.0 | lifbase 0.3.0 |
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
| server, after 100 MB cat (default history) | - | - |
| lines retained after 100 MB cat (default history) | - | - |
| client, idle | - | - |

### Idle: one attached session, 30 s window

| | lif 0.3.0 | lifbase 0.3.0 |
|---|---|---|
| server: CPU ms | 0.3 | 0.0 |
| server: idle wakeups | 0 | 0 |
| server: interrupt wakeups | 1 | 0 |
| client: CPU ms | 0.0 | 0.0 |
| client: idle wakeups | 0 | 0 |
| client: interrupt wakeups | 0 | 0 |

### Startup (ms: median [p25-p75] of warm runs, then the first run)

| | lif 0.3.0 | lifbase 0.3.0 |
|---|---|---|
| create session + kill server (hyperfine) | 5.7 [5.4-5.9] N=30; first 6.2 | 5.7 [5.5-5.9] N=30; first 6.6 |
| attach, draw, detach (hyperfine; includes 20 ms quiet wait) | 31.2 [26.1-34.2] N=30; first 23.4 | 32.0 [26.9-33.7] N=30; first 24.0 |
| new session until a typed command runs (driver) | 40.0 [39.6-41.1] N=15 | 39.7 [38.7-40.9] N=15 |

### Input latency (us: p50 / p95 / p99 / max)

| | lif 0.3.0 | lifbase 0.3.0 |
|---|---|---|
| key echo, idle | 113 / 167 / 197 / 843 (N=1000) | 112 / 166 / 196 / 239 (N=1000) |
| key echo, idle: median of each round | 107 115 118 115 109 | 110 114 109 114 111 |
| key echo, other pane runs `yes` | 38 / 50 / 116 / 283 (N=1000) | 39 / 49 / 119 / 183 (N=1000) |
| key echo, other pane runs `yes`: median of each round | 38 38 39 39 39 | 39 39 39 39 39 |
| bare ESC, default config | 5,787 / 5,846 / 5,913 / 6,093 (N=333) | 5,784 / 5,852 / 5,880 / 6,032 (N=333) |
| bare ESC, escape-time 0 | 160 / 226 / 271 / 400 (N=333) | 160 / 224 / 289 / 431 (N=333) |
| bare ESC, terminal with kitty keyboard protocol | 165 / 230 / 264 / 467 (N=333) | 160 / 217 / 263 / 301 (N=333) |

### Binary

| | lif 0.3.0 | lifbase 0.3.0 |
|---|---|---|
| size | - | - |
| non-system dylibs | - | - |
| dylibs total | 0 | 0 |
