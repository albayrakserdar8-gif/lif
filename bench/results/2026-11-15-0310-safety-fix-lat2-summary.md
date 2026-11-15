Machine: Mac18,5 (Apple M6, 12 cores: 2 Super, 4 Performance, 6 Efficiency, 16 GiB), macOS 27.2 26B123. Client PTY 200x50. Runs: 2026-11-15T03:10:15, 2026-11-15T03:10:16, 2026-11-15T03:10:16.


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
| key echo, idle | 130 / 185 / 214 / 275 (N=1000) | 116 / 176 / 211 / 2,238 (N=1000) | 108 / 160 / 190 / 256 (N=1000) |
| key echo, idle: median of each round | 131 129 133 130 129 | 116 116 115 111 132 | 110 108 108 106 109 |
| key echo, other pane runs `yes` | 263 / 331 / 350 / 372 (N=1000) | 39 / 47 / 91 / 128 (N=1000) | 38 / 46 / 101 / 123 (N=1000) |
| key echo, other pane runs `yes`: median of each round | 261 261 265 267 263 | 40 39 39 38 39 | 38 38 38 38 38 |
| bare ESC, default config | 11,689 / 12,290 / 12,327 / 12,388 (N=333) | 6,327 / 6,461 / 6,482 / 6,521 (N=333) | 6,390 / 6,469 / 6,514 / 6,648 (N=333) |
| bare ESC, escape-time 0 | 1,437 / 1,495 / 1,536 / 1,971 (N=333) | 171 / 229 / 267 / 323 (N=333) | 154 / 208 / 247 / 269 (N=333) |
| bare ESC, terminal with kitty keyboard protocol | - | 164 / 222 / 255 / 319 (N=333) | 157 / 211 / 247 / 320 (N=333) |

### Binary

| | tmux 3.8 | lif 0.1.0 | lifbase 0.1.0 |
|---|---|---|---|
| size | - | - | - |
| non-system dylibs | - | - | - |
| dylibs total | 0 | 0 | 0 |
