Machine: Mac18,5 (Apple M6, 12 cores: 2 Super, 4 Performance, 6 Efficiency, 16 GiB), macOS 27.2 26B123. Client PTY 200x50. Runs: 2026-11-15T21:12:26, 2026-11-15T21:12:26.


### Throughput (MB/s: median [min-max], N runs)

From Enter on `cat FILE; touch MARKER` until MARKER exists. Client bytes: what the client received during the run. Tail: how long the client kept receiving output after the marker.

| | lif 0.3.0 | lifbase 0.2.0 |
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

| | lif 0.3.0 | lifbase 0.2.0 |
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

| | lif 0.3.0 | lifbase 0.2.0 |
|---|---|---|
| server: CPU ms | 0.3 | 0.0 |
| server: idle wakeups | 0 | 0 |
| server: interrupt wakeups | 1 | 0 |
| client: CPU ms | 0.0 | 0.0 |
| client: idle wakeups | 0 | 0 |
| client: interrupt wakeups | 0 | 0 |

### Startup (ms: median [p25-p75] of warm runs, then the first run)

| | lif 0.3.0 | lifbase 0.2.0 |
|---|---|---|
| create session + kill server (hyperfine) | 5.9 [5.7-6.1] N=30; first 6.2 | 5.9 [5.6-6.1] N=30; first 6.9 |
| attach, draw, detach (hyperfine; includes 20 ms quiet wait) | 29.7 [25.6-32.7] N=30; first 23.9 | 31.0 [26.8-32.9] N=30; first 23.8 |
| new session until a typed command runs (driver) | 38.5 [36.8-39.4] N=15 | 37.2 [36.4-40.3] N=15 |

### Input latency (us: p50 / p95 / p99 / max)

| | lif 0.3.0 | lifbase 0.2.0 |
|---|---|---|
| key echo, idle | 111 / 163 / 197 / 235 (N=1000) | 108 / 159 / 194 / 242 (N=1000) |
| key echo, idle: median of each round | 112 111 111 110 112 | 107 108 108 109 109 |
| key echo, other pane runs `yes` | 39 / 48 / 59 / 123 (N=1000) | 39 / 48 / 61 / 102 (N=1000) |
| key echo, other pane runs `yes`: median of each round | 39 39 39 39 39 | 39 40 39 39 39 |
| bare ESC, default config | 6,250 / 6,467 / 6,508 / 6,683 (N=333) | 6,238 / 6,460 / 6,506 / 6,668 (N=333) |
| bare ESC, escape-time 0 | 155 / 216 / 280 / 411 (N=333) | 150 / 200 / 243 / 291 (N=333) |
| bare ESC, terminal with kitty keyboard protocol | 158 / 219 / 290 / 364 (N=333) | 156 / 214 / 256 / 943 (N=333) |

### Binary

| | lif 0.3.0 | lifbase 0.2.0 |
|---|---|---|
| size | 3.54 MB | 3.32 MB |
| non-system dylibs | - | - |
| dylibs total | 2 | 2 |
