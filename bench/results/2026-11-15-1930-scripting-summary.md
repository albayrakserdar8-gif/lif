Machine: Mac18,5 (Apple M6, 12 cores: 2 Super, 4 Performance, 6 Efficiency, 16 GiB), macOS 27.2 26B123. Client PTY 200x50. Runs: 2026-11-15T19:30:48, 2026-11-15T19:30:48.


### Throughput (MB/s: median [min-max], N runs)

From Enter on `cat FILE; touch MARKER` until MARKER exists. Client bytes: what the client received during the run. Tail: how long the client kept receiving output after the marker.

| | lif 0.2.0 | lifbase 0.1.0 |
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

| | lif 0.2.0 | lifbase 0.1.0 |
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

| | lif 0.2.0 | lifbase 0.1.0 |
|---|---|---|
| server: CPU ms | 0.2 | 0.0 |
| server: idle wakeups | 1 | 0 |
| server: interrupt wakeups | 1 | 0 |
| client: CPU ms | 0.0 | 0.0 |
| client: idle wakeups | 0 | 0 |
| client: interrupt wakeups | 0 | 0 |

### Startup (ms: median [p25-p75] of warm runs, then the first run)

| | lif 0.2.0 | lifbase 0.1.0 |
|---|---|---|
| create session + kill server (hyperfine) | 5.7 [5.5-5.8] N=30; first 13.5 | 7.1 [6.4-8.8] N=30; first 7.5 |
| attach, draw, detach (hyperfine; includes 20 ms quiet wait) | 29.1 [26.0-31.3] N=30; first 24.4 | 30.6 [25.9-31.5] N=30; first 28.3 |
| new session until a typed command runs (driver) | 38.9 [33.5-39.6] N=15 | 32.6 [32.1-33.0] N=15 |

### Input latency (us: p50 / p95 / p99 / max)

| | lif 0.2.0 | lifbase 0.1.0 |
|---|---|---|
| key echo, idle | 120 / 201 / 329 / 6,399 (N=1000) | 122 / 188 / 269 / 457 (N=1000) |
| key echo, idle: median of each round | 102 130 125 107 143 | 108 96 106 143 144 |
| key echo, other pane runs `yes` | 38 / 47 / 86 / 130 (N=1000) | 38 / 47 / 74 / 169 (N=1000) |
| key echo, other pane runs `yes`: median of each round | 38 38 38 39 38 | 38 38 38 38 38 |
| bare ESC, default config | 5,788 / 5,876 / 5,937 / 7,805 (N=333) | 5,791 / 5,879 / 6,054 / 6,539 (N=333) |
| bare ESC, escape-time 0 | 189 / 262 / 332 / 435 (N=333) | 178 / 246 / 288 / 383 (N=333) |
| bare ESC, terminal with kitty keyboard protocol | 174 / 254 / 303 / 1,054 (N=333) | 177 / 241 / 321 / 7,804 (N=333) |

### Binary

| | lif 0.2.0 | lifbase 0.1.0 |
|---|---|---|
| size | 3.32 MB | 3.19 MB |
| non-system dylibs | - | - |
| dylibs total | 2 | 2 |
