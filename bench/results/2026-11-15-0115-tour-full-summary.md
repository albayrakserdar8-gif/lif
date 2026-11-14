Machine: Mac18,5 (Apple M6, 12 cores: 2 Super, 4 Performance, 6 Efficiency, 16 GiB), macOS 27.2 26B123. Client PTY 200x50. Runs: 2026-11-15T01:15:10, 2026-11-15T01:15:10, 2026-11-15T01:15:10, 2026-11-15T01:15:10, 2026-11-15T01:15:11.


### Throughput (MB/s: median [min-max], N runs)

From Enter on `cat FILE; touch MARKER` until MARKER exists. Client bytes: what the client received during the run. Tail: how long the client kept receiving output after the marker.

| | none | tmux 3.8 | zellij 0.46.0 | lif 0.1.0 | lifbase 0.1.0 |
|---|---|---|---|---|---|
| 100 MB ASCII | 225.2 [200.4-236.2] N=5 | 52.4 [49.9-54.0] N=5 | 79.2 [77.8-80.4] N=5 | 205.8 [201.3-217.8] N=5 | 208.1 [202.6-215.1] N=5 |
| 100 MB ASCII: client bytes | 101.0 MB | 1.9 MB | 1.3 MB | 0.3 MB | 0.3 MB |
| 100 MB ASCII: tail | 0 ms | 144 ms | 4 ms | 5 ms | 4 ms |
| 100 MB SGR + UTF-8 | 284.7 [283.0-298.4] N=5 | 21.3 [21.1-21.4] N=5 | 81.6 [81.2-87.4] N=5 | 151.8 [150.6-153.7] N=5 | 151.0 [150.2-153.2] N=5 |
| 100 MB SGR + UTF-8: client bytes | 100.3 MB | 2.7 MB | 3.1 MB | 1.2 MB | 1.2 MB |
| 100 MB SGR + UTF-8: tail | 0 ms | 87 ms | 4 ms | 2 ms | 4 ms |
| 20 MB single line | 155.1 [152.2-164.5] N=5 | 51.8 [51.8-52.4] N=5 | 17.6 [14.5-18.0] N=5 | 151.1 [149.7-158.9] N=5 | 151.5 [150.7-158.7] N=5 |
| 20 MB single line: client bytes | 20.0 MB | 0.4 MB | 1.0 MB | 0.1 MB | 0.1 MB |
| 20 MB single line: tail | 0 ms | 41 ms | 10 ms | 5 ms | 8 ms |

### Memory (MiB: phys_footprint / RSS)

Server: sum over all server-side processes of the tool. Pane shells are not counted.

| | none | tmux 3.8 | zellij 0.46.0 | lif 0.1.0 | lifbase 0.1.0 |
|---|---|---|---|---|---|
| server, idle, 1 pane | - | 3.7 / 5.2 | 26.5 / 43.1 | 3.3 / 5.1 | 3.4 / 5.2 |
| server, idle, 10 panes | - | 3.8 / 5.3 (10 panes) | 30.8 / 47.7 (10 panes) | 6.0 / 8.0 (10 panes) | 7.6 / 12.2 (10 panes) |
| server, idle, 50 panes | - | 4.2 / 5.7 (50 panes) | 38.2 / 55.2 (50 panes, 6 floating) | 9.5 / 11.4 (50 panes) | 24.8 / 44.2 (50 panes) |
| server, 100k colored lines in scrollback | - | 285.9 / 287.4 | 231.9 / 248.4 | 43.3 / 235.6 | 184.9 / 193.2 |
| server, cost per extra pane, 1 to 10 panes | - | 11 / 12 KiB | 492 / 528 KiB | 306 / 324 KiB | 482 / 795 KiB |
| server, cost per extra pane, 10 to 50 panes | - | 10 / 10 KiB | 189 / 193 KiB | 88 / 87 KiB | 440 / 819 KiB |
| server, cost per extra pane, 1 to 50 panes | - | 10 / 10 KiB | 245 / 254 KiB | 128 / 131 KiB | 448 / 815 KiB |
| 50 panes spread over | - | 1 window(s) | 1 window(s) | 1 window(s) | 1 window(s) |
| lines retained in scrollback after the 100k-line cat | - | 99,954 | 99,952 | 99,954 | 99,954 |
| server, after 100 MB cat (default history) | - | 6.1 / 7.6 | 44.8 / 61.4 | 5.6 / 13.7 | 8.5 / 16.7 |
| lines retained after 100 MB cat (default history) | - | 1,818 | 9,997 | 3,784 | 5,688 |
| client, idle | - | 3.3 / 4.9 | 12.0 / 26.1 | 1.4 / 2.2 | 1.4 / 2.3 |

### Idle: one attached session, 30 s window

| | none | tmux 3.8 | zellij 0.46.0 | lif 0.1.0 | lifbase 0.1.0 |
|---|---|---|---|---|---|
| server: CPU ms | - | 2.3 | 40.6 | 0.4 | 0.0 |
| server: idle wakeups | - | 1 | 155 | 0 | 0 |
| server: interrupt wakeups | - | 3 | 361 | 1 | 0 |
| client: CPU ms | 0.0 | 0.0 | 0.2 | 0.0 | 0.0 |
| client: idle wakeups | 0 | 0 | 1 | 0 | 0 |
| client: interrupt wakeups | 0 | 0 | 3 | 0 | 0 |

### Startup (ms: median [p25-p75] of warm runs, then the first run)

| | none | tmux 3.8 | zellij 0.46.0 | lif 0.1.0 | lifbase 0.1.0 |
|---|---|---|---|---|---|
| create session + kill server (hyperfine) | - | 11.0 [10.8-11.2] N=30; first 12.0 | 53.2 [52.7-53.5] N=30; first 50.6 | 6.1 [6.0-6.1] N=30; first 6.3 | 5.9 [5.7-6.2] N=30; first 6.6 |
| attach, draw, detach (hyperfine; includes 20 ms quiet wait) | - | 31.2 [29.5-34.0] N=30; first 25.8 | 63.9 [63.6-64.0] N=30; first 63.7 | 33.1 [27.6-34.4] N=30; first 24.1 | 46.5 [42.2-47.5] N=30; first 33.3 |
| new session until a typed command runs (driver) | 31.6 [29.3-32.8] N=15 | 51.4 [47.6-52.1] N=15 | 85.5 [83.3-88.3] N=15 | 41.9 [41.4-42.3] N=15 | 41.5 [40.3-42.4] N=15 |

### Input latency (us: p50 / p95 / p99 / max)

| | none | tmux 3.8 | zellij 0.46.0 | lif 0.1.0 | lifbase 0.1.0 |
|---|---|---|---|---|---|
| key echo, idle | 22 / 37 / 45 / 108 (N=1000) | 146 / 214 / 251 / 345 (N=1000) | 536 / 763 / 838 / 1,440 (N=1000) | 126 / 185 / 208 / 274 (N=1000) | 2,155 / 7,400 / 8,382 / 8,504 (N=1000) |
| key echo, idle: median of each round | 19 22 23 25 19 | 138 131 153 170 138 | 494 553 558 573 520 | 113 130 132 148 110 | 2,102 2,188 2,069 2,167 2,158 |
| key echo, other pane runs `yes` | - | 263 / 332 / 343 / 377 (N=1000) | 949 / 1,063 / 1,112 / 1,268 (N=1000) | 38 / 45 / 74 / 123 (N=1000) | 4,228 / 7,225 / 7,238 / 7,246 (N=1000) |
| key echo, other pane runs `yes`: median of each round | - | 262 266 260 266 264 | 954 957 946 946 937 | 38 38 38 38 38 | 4,233 4,229 4,218 3,834 4,233 |
| bare ESC, default config | 54 / 71 / 78 / 84 (N=333) | 11,696 / 12,284 / 12,315 / 12,344 (N=333) | 61,673 / 63,885 / 64,008 / 64,365 (N=333) | 6,323 / 6,469 / 6,495 / 6,519 (N=333) | 6,407 / 6,531 / 6,570 / 6,630 (N=332, 1 timeouts) |
| bare ESC, escape-time 0 | - | 1,444 / 1,506 / 1,540 / 2,127 (N=333) | - | 157 / 208 / 243 / 333 (N=333) | 6,438 / 6,521 / 6,553 / 6,673 (N=333) |
| bare ESC, terminal with kitty keyboard protocol | - | - | - | 162 / 215 / 247 / 358 (N=333) | 212 / 581 / 2,043 / 2,283 (N=333) |

### Binary

| | none | tmux 3.8 | zellij 0.46.0 | lif 0.1.0 | lifbase 0.1.0 |
|---|---|---|---|---|---|
| size | - | 1.19 MB | 73.07 MB | 3.06 MB | 2.84 MB |
| non-system dylibs | - | libutf8proc.3.dylib, libncursesw.6.dylib, libevent_core-2.1.7.dylib, libjemalloc.2.dylib | - | - | - |
| dylibs total | 0 | 6 | 23 | 2 | 2 |
