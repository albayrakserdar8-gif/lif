Machine: Mac18,5 (Apple M6, 12 cores: 2 Super, 4 Performance, 6 Efficiency, 16 GiB), macOS 27.2 26B123. Client PTY 200x50. Runs: 2026-11-15T00:28:33, 2026-11-15T00:28:34, 2026-11-15T00:28:34.


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
| create session + kill server (hyperfine) | - | - | - |
| attach, draw, detach (hyperfine; includes 20 ms quiet wait) | - | - | - |
| new session until a typed command runs (driver) | - | - | - |

### Input latency (us: p50 / p95 / p99 / max)

| | tmux 3.8 | lif 0.1.0 | lifbase 0.1.0 |
|---|---|---|---|
| key echo, idle | 142 / 197 / 213 / 238 (N=300) | 168 / 235 / 275 / 313 (N=300) | 2,132 / 6,861 / 7,680 / 8,384 (N=300) |
| key echo, idle: median of each round | 159 134 135 | 161 165 181 | 2,164 2,106 2,124 |
| key echo, other pane runs `yes` | 260 / 328 / 358 / 379 (N=300) | 50 / 63 / 80 / 82 (N=300) | 4,231 / 7,227 / 7,244 / 7,476 (N=300) |
| key echo, other pane runs `yes`: median of each round | 261 261 259 | 50 51 50 | 4,232 4,211 4,233 |
| bare ESC, default config | 11,976 / 12,296 / 12,328 / 12,354 (N=100) | 6,377 / 6,494 / 6,524 / 6,550 (N=100) | 6,313 / 6,515 / 6,568 / 6,646 (N=99, 1 timeouts) |
| bare ESC, escape-time 0 | 1,443 / 1,524 / 1,687 / 2,158 (N=100) | - | - |

### Binary

| | tmux 3.8 | lif 0.1.0 | lifbase 0.1.0 |
|---|---|---|---|
| size | - | - | - |
| non-system dylibs | - | - | - |
| dylibs total | 0 | 0 | 0 |
