Machine: Mac18,5 (Apple M6, 12 cores: 2 Super, 4 Performance, 6 Efficiency, 16 GiB), macOS 27.2 26B123. Client PTY 200x50. Runs: 2026-11-15T14:41:34, 2026-11-15T14:41:34.


### Throughput (MB/s: median [min-max], N runs)

From Enter on `cat FILE; touch MARKER` until MARKER exists. Client bytes: what the client received during the run. Tail: how long the client kept receiving output after the marker.

| | lif 0.1.0 | lifbase 0.1.0 |
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

| | lif 0.1.0 | lifbase 0.1.0 |
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

| | lif 0.1.0 | lifbase 0.1.0 |
|---|---|---|
| server: CPU ms | 0.4 | 0.0 |
| server: idle wakeups | 0 | 0 |
| server: interrupt wakeups | 1 | 0 |
| client: CPU ms | 0.0 | 0.0 |
| client: idle wakeups | 0 | 0 |
| client: interrupt wakeups | 0 | 0 |

### Startup (ms: median [p25-p75] of warm runs, then the first run)

| | lif 0.1.0 | lifbase 0.1.0 |
|---|---|---|
| create session + kill server (hyperfine) | - | - |
| attach, draw, detach (hyperfine; includes 20 ms quiet wait) | - | - |
| new session until a typed command runs (driver) | - | - |

### Input latency (us: p50 / p95 / p99 / max)

| | lif 0.1.0 | lifbase 0.1.0 |
|---|---|---|
| key echo, idle | - | - |
| key echo, idle: median of each round | - | - |
| key echo, other pane runs `yes` | - | - |
| key echo, other pane runs `yes`: median of each round | - | - |
| bare ESC, default config | - | - |
| bare ESC, escape-time 0 | - | - |
| bare ESC, terminal with kitty keyboard protocol | - | - |

### Binary

| | lif 0.1.0 | lifbase 0.1.0 |
|---|---|---|
| size | - | - |
| non-system dylibs | - | - |
| dylibs total | 0 | 0 |
