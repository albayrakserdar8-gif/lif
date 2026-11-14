Machine: Mac18,5 (Apple M6, 12 cores: 2 Super, 4 Performance, 6 Efficiency, 16 GiB), macOS 27.2 26B123. Client PTY 200x50. Runs: 2026-11-15T01:30:36, 2026-11-15T01:30:37.


### Throughput (MB/s: median [min-max], N runs)

From Enter on `cat FILE; touch MARKER` until MARKER exists. Client bytes: what the client received during the run. Tail: how long the client kept receiving output after the marker.

| | tmux 3.8 | lif 0.1.0 |
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

| | tmux 3.8 | lif 0.1.0 |
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

### Idle: one attached session, ? s window

| | tmux 3.8 | lif 0.1.0 |
|---|---|---|
| server: CPU ms | - | - |
| server: idle wakeups | - | - |
| server: interrupt wakeups | - | - |
| client: CPU ms | - | - |
| client: idle wakeups | - | - |
| client: interrupt wakeups | - | - |

### Startup (ms: median [p25-p75] of warm runs, then the first run)

| | tmux 3.8 | lif 0.1.0 |
|---|---|---|
| create session + kill server (hyperfine) | 11.5 [11.4-11.7] N=30; first 11.9 | 5.5 [5.4-5.7] N=30; first 6.0 |
| attach, draw, detach (hyperfine; includes 20 ms quiet wait) | 30.3 [29.9-30.7] N=30; first 25.7 | 32.1 [27.5-33.3] N=30; first 24.0 |
| new session until a typed command runs (driver) | 52.6 [50.6-54.2] N=15 | 41.4 [40.1-42.1] N=15 |

### Input latency (us: p50 / p95 / p99 / max)

| | tmux 3.8 | lif 0.1.0 |
|---|---|---|
| key echo, idle | - | - |
| key echo, idle: median of each round | - | - |
| key echo, other pane runs `yes` | - | - |
| key echo, other pane runs `yes`: median of each round | - | - |
| bare ESC, default config | - | - |
| bare ESC, escape-time 0 | - | - |
| bare ESC, terminal with kitty keyboard protocol | - | - |

### Binary

| | tmux 3.8 | lif 0.1.0 |
|---|---|---|
| size | - | - |
| non-system dylibs | - | - |
| dylibs total | 0 | 0 |
