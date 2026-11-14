Machine: Mac18,5 (Apple M6, 12 cores: 2 Super, 4 Performance, 6 Efficiency, 16 GiB), macOS 27.2 26B123. Client PTY 200x50. Runs: 2026-11-15T00:24:46, 2026-11-15T00:24:48.


### Throughput (MB/s: median [min-max], N runs)

From Enter on `cat FILE; touch MARKER` until MARKER exists. Client bytes: what the client received during the run. Tail: how long the client kept receiving output after the marker.

| | tmux 3.8 | lifbase 0.1.0 |
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

| | tmux 3.8 | lifbase 0.1.0 |
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

| | tmux 3.8 | lifbase 0.1.0 |
|---|---|---|
| server: CPU ms | - | - |
| server: idle wakeups | - | - |
| server: interrupt wakeups | - | - |
| client: CPU ms | - | - |
| client: idle wakeups | - | - |
| client: interrupt wakeups | - | - |

### Startup (ms: median [p25-p75] of warm runs, then the first run)

| | tmux 3.8 | lifbase 0.1.0 |
|---|---|---|
| create session + kill server (hyperfine) | - | - |
| attach, draw, detach (hyperfine; includes 20 ms quiet wait) | - | - |
| new session until a typed command runs (driver) | - | - |

### Input latency (us: p50 / p95 / p99 / max)

| | tmux 3.8 | lifbase 0.1.0 |
|---|---|---|
| key echo, idle | 149 / 203 / 221 / 251 (N=300) | 2,091 / 6,819 / 8,353 / 8,402 (N=300) |
| key echo, idle: median of each round | 145 166 137 | 2,074 2,072 2,156 |
| key echo, other pane runs `yes` | 240 / 292 / 338 / 350 (N=300) | 4,227 / 7,228 / 7,235 / 7,340 (N=300) |
| key echo, other pane runs `yes`: median of each round | 263 217 211 | 4,230 4,220 4,229 |
| bare ESC, default config | 11,596 / 12,282 / 12,301 / 12,321 (N=100) | 6,413 / 6,541 / 6,580 / 6,639 (N=100) |
| bare ESC, escape-time 0 | 1,451 / 1,495 / 1,520 / 2,169 (N=100) | - |

### Binary

| | tmux 3.8 | lifbase 0.1.0 |
|---|---|---|
| size | - | - |
| non-system dylibs | - | - |
| dylibs total | 0 | 0 |
