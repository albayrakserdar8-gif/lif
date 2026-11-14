Machine: Mac18,5 (Apple M6, 12 cores: 2 Super, 4 Performance, 6 Efficiency, 16 GiB), macOS 27.2 26B123. Client PTY 200x50. Runs: 2026-11-15T00:37:38, 2026-11-15T00:37:39, 2026-11-15T00:37:39.


### Throughput (MB/s: median [min-max], N runs)

From Enter on `cat FILE; touch MARKER` until MARKER exists. Client bytes: what the client received during the run. Tail: how long the client kept receiving output after the marker.

| | tmux 3.8 | lif 0.1.0 | lifbase 0.1.0 |
|---|---|---|---|
| 100 MB ASCII | 51.6 [51.5-51.6] N=3 | 214.1 [211.6-215.1] N=3 | 216.4 [211.3-216.4] N=3 |
| 100 MB ASCII: client bytes | 1.9 MB | 0.3 MB | 0.3 MB |
| 100 MB ASCII: tail | 118 ms | 2 ms | 4 ms |
| 100 MB SGR + UTF-8 | 21.1 [21.1-21.2] N=3 | 151.0 [150.6-151.8] N=3 | 151.8 [151.1-154.4] N=3 |
| 100 MB SGR + UTF-8: client bytes | 2.7 MB | 1.2 MB | 1.2 MB |
| 100 MB SGR + UTF-8: tail | 66 ms | 5 ms | 2 ms |
| 20 MB single line | 52.5 [51.5-53.2] N=3 | 151.2 [149.9-151.7] N=3 | 151.6 [149.2-153.0] N=3 |
| 20 MB single line: client bytes | 0.4 MB | 0.1 MB | 0.2 MB |
| 20 MB single line: tail | 47 ms | 8 ms | 5 ms |

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
| server, after 100 MB cat (default history) | 6.2 / 7.6 | 8.5 / 16.5 | 8.5 / 16.7 |
| lines retained after 100 MB cat (default history) | 1,818 | 5,688 | 5,688 |
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
| key echo, idle | 136 / 190 / 232 / 298 (N=600) | 119 / 169 / 206 / 311 (N=600) | 2,177 / 7,306 / 8,377 / 8,572 (N=600) |
| key echo, idle: median of each round | 136 137 136 | 129 116 119 | 2,154 2,140 2,360 |
| key echo, other pane runs `yes` | 264 / 331 / 345 / 372 (N=600) | 37 / 44 / 50 / 91 (N=600) | 4,225 / 7,222 / 7,233 / 7,279 (N=600) |
| key echo, other pane runs `yes`: median of each round | 265 262 266 | 37 38 37 | 4,220 4,231 3,849 |
| bare ESC, default config | 11,779 / 12,294 / 12,337 / 12,351 (N=200) | 6,402 / 6,462 / 6,492 / 6,611 (N=200) | 6,337 / 6,512 / 6,591 / 6,632 (N=200) |
| bare ESC, escape-time 0 | 1,444 / 1,509 / 1,558 / 1,977 (N=200) | - | - |

### Binary

| | tmux 3.8 | lif 0.1.0 | lifbase 0.1.0 |
|---|---|---|---|
| size | - | - | - |
| non-system dylibs | - | - | - |
| dylibs total | 0 | 0 | 0 |
