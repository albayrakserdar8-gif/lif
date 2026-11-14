# lif benchmark harness

Compares terminal multiplexers on one machine: lif, tmux, zellij, and "none"
(a plain shell on the same PTY, the upper bound). One command runs everything:

```sh
bench/run.sh                                   # none,tmux,zellij,lif (lif skipped until built)
bench/run.sh --tools tmux,zellij --tag mytest  # chosen tools
bench/run.sh --quick                           # 2 reps / 100 samples: checks the harness, not for numbers
bench/run.sh --only throughput,latency         # subset: binary,throughput,memory,idle,startup,latency
python3 bench/lifbench.py report bench/results/X-tmux.json bench/results/X-zellij.json -o table.md
```

Output: `bench/results/<date>[-tag]-<tool>.json` (every raw sample, plus the
machine, versions, configs and load average per metric) and
`<date>[-tag]-summary.md` (merged table). A full run of none+tmux+zellij takes
about 10 minutes. `bench/results/baseline.md` holds the reference numbers.

## Layout

| Path | What |
|---|---|
| `run.sh` | builds the driver (into `$CARGO_TARGET_DIR`, default `target-bench/`), runs `lifbench.py run` |
| `lifbench.py` | orchestration, adapters, statistics, report (python3, stdlib only) |
| `driver/` | `lifbench-driver`, the timing-sensitive part (Rust, libc only, its own workspace) |
| `tmp/` | generated data, private HOME, sockets, session dirs (gitignored) |
| `results/` | JSON results and markdown summaries |

## How a measurement runs

Each multiplexer client runs on a PTY owned by `lifbench-driver hold` at a fixed
200x50. The driver reads and discards client output as fast as it arrives
(poll + non-blocking reads until EAGAIN), counts the bytes, and executes
commands that python sends on its stdin (`ready`, `run`, `latency`, `quiet`,
`stats`, `quit`). All timestamps are taken inside the driver with a monotonic
clock, next to the read or write they refer to; python only orchestrates.

The driver behaves like a minimal real terminal: it answers the queries
clients send at startup (DA1, DA2, XTVERSION, DSR, OSC 10/11 colors, the
`?996n` theme query, `14t/16t/18t` sizes, DECRQM with 2026 reported as
supported). Without answers, clients wait on timeouts: tmux 3.8 held a bare ESC
for 500 ms while its DA request was outstanding, and zellij's attach-then-detach took
1.05 s instead of under 70 ms. No user with a real terminal sees those, so they
would be artifacts of the harness.

Every session starts fresh: a new server, a new client, a new work dir. A
session is "ready" when the shell in its first pane has run a typed
`touch MARKER` (the driver re-types it every 250 ms because zellij drops keys sent
before its pane exists), and then the client has been quiet for 300 ms.

Pane shell: `/bin/sh` (via `SHELL`) for every tool, HOME pointed at an empty
private dir, so no user rc files run. TERM=xterm-256color, LANG=en_US.UTF-8.

### Isolation and safety

- Every session gets its own socket dir under `bench/tmp/s/` (TMPDIR, TMUX_TMPDIR,
  XDG_RUNTIME_DIR) and a private HOME/XDG dirs under `bench/tmp/home`. tmux always
  runs as `tmux -L lifbench -f <conf>` inside that socket dir; zellij sessions
  are named `lifbench-*` and use ZELLIJ_CONFIG_DIR in the work dir. The harness
  strips TMUX/ZELLIJ variables, so it is safe to run from inside a tmux or
  zellij session.
- Shutdown uses the tool's own command (`kill-server`, `kill-session`). Only if a
  server pid the harness recorded is still alive 5 s later, and its command
  line still names the private socket dir, does it get SIGTERM, then SIGKILL.
  Nothing else is ever signalled.
- zellij 0.46 opens a floating "Welcome to Zellij" setup plugin on first run,
  which takes input focus. The harness pre-seeds
  `tmp/home/Library/Application Support/org.Zellij-Contributors.Zellij/desktop_integration.json`
  with the "dismissed" flags, the state a user is in after closing it once.

## Metrics

All statistics: median and spread (min-max for few samples, p25-p75 or
p95/p99 for many). N is printed next to every number.

### 1. Throughput

Files (generated once with a fixed seed into `tmp/data.noindex`):
(a) 100,000,000 bytes of printable ASCII, lines of 80-120 columns;
(b) 100,000,000 bytes of SGR-colored UTF-8 lines (256-color, truecolor, bold,
Latin-extended, box drawing and CJK double-width text, at most 120 columns);
(c) a single 20,000,000-byte line with no newline.

The driver types `cat FILE; touch MARKER` into the client and starts the clock
when it writes the final Enter; it stats MARKER every millisecond while
draining output, so completion is detected to ±1 ms. Reported: MB/s
(10^6 bytes / s), the bytes the client received during the run (how much the
multiplexer redrew), and "tail": how long the client kept receiving output
after the marker appeared (a multiplexer may finish parsing before it finishes
drawing). N=5 per file per tool, each in a fresh session, with repetitions
interleaved across tools (rep 1 of every tool, then rep 2, ...) so drift in
machine load affects all tools alike. A run that does not finish within 600 s
(300 s for the long line) is reported as failed and not repeated.

macOS returns at most 1024 bytes per read from a PTY master, so every reader
(the multiplexer server reading the pane, and the driver reading "none")
needs ~100k reads per 100 MB. That bounds everyone; "none" shows the ceiling.
When `cat` exits, up to one PTY buffer of its output may still be unread by the
server; that is under 0.1% of the file.

### 2. Memory

From `proc_pid_rusage` (driver subcommand `rusage`; works without root for own
processes): `ri_phys_footprint` (the number Activity Monitor and `footprint(1)`
report) and `ri_resident_size` (RSS), cross-checked against `ps -o rss` (kept
in the JSON). "Server" is the sum over all server-side processes of the tool
(tmux: the server pid from `display -p '#{pid}'`; zellij: processes whose
command is `zellij --server <our socket dir>`; lif: processes naming our socket
that are not `attach` clients). Pane shells are not counted. The client is
reported separately.

Scenarios, each in a fresh session and sampled after 1 s of client silence
plus 2-3 s: idle with 1 pane; with 10 and with 50 panes, each an idle shell
(tmux: `split-window` + `select-layout tiled` after each split, so 50 fit in
200x50; zellij: `action new-pane`, which refuses a 45th tiled pane at 200x50
("No room for a new pane", also with `--stacked`), so panes 45-50 are floating
panes; the actual pane count is recorded); one pane
after `cat` of 100,000 SGR-colored lines with the history limit raised to
200,000 (tmux `history-limit`, zellij `scroll_buffer_size`) so all are
retained; and one pane after the 100 MB ASCII cat with default history (taken
from the first throughput repetition).

### 3. Idle CPU and wakeups

One attached session, shell prompt idle, 3 s settle, then two rusage samples
30 s apart. Reported: CPU time (user+sys) and `ri_pkg_idle_wkups` /
`ri_interrupt_wkups` deltas for server and client. `ps` cputime deltas are also
recorded but only have 10 ms resolution. `powermetrics` needs root and `top`
reports the same kernel counters, so neither is used. The driver itself
blocks in `poll(-1)` during this window, so it adds no wakeups to the client.

### 4. Startup

- **create session + kill server** (hyperfine, 31 runs, no warmup; the first
  run is reported separately as "first", the other 30 as warm): tmux
  `new-session -d && kill-server`, zellij `attach -b && kill-session`. Both kill
  commands return before the server has fully exited; an untimed hyperfine
  `--prepare` waits for that so runs never overlap.
- **attach, draw, detach** (hyperfine, 31 runs): `lifbench-driver attach-detach`
  starts the client on a PTY, waits for its first output and then for 20 ms of
  silence (the frame is drawn: 1.7 KB for tmux, 13 KB for zellij), sends the
  detach keys (tmux `C-b d`, zellij `Ctrl-o d`) and waits for the client to exit.
  The time includes that fixed 20 ms wait. The run fails unless the client
  exits 0 and the session is still alive afterwards.
- **new session until a typed command runs** (driver, 15 sessions): from the
  start of session creation until the first pane's shell has executed a typed
  command, with the client attached. This is what a user waits for.

"Cold" start cannot be measured honestly without root (no way to drop the
file cache), so "first" is only the first run of the suite.

### 5. Input latency

The pane runs `cat` (the pane's tty echoes). Each sample: the driver writes one
byte to the client PTY and timestamps the read in which that byte appears in
the client's output as printable text. A small VT scanner skips escape
sequences so a byte inside a CSI/OSC/DCS sequence does not count. The byte
rotates through `{`, `}`, `~`, `^` and backtick (in no status bar, not in `yes` output); after each
sample a DEL erases it again, then the driver waits a random 10-20 ms (to
decorrelate from frame timers) while still draining. A "stray" counter records
any sample whose byte showed up before it was typed (would mean a
contaminated sample). Reported: p50/p95/p99/max in microseconds, and the
median of each round.

Keystroke latency on this machine has two modes that are a property of the
machine, not of the tools: in most runs the medians are steady (none ~18 us,
tmux ~125 us, zellij ~450 us), but in some stretches every tool, including
"none", is 2-4x faster (none 2-8 us, tmux 23-50 us, zellij 109-220 us) for a
minute or so. It is not explained by load average or total CPU use (both
recorded per round). The latency metric starts after 45 s of rest
(`--latency-rest`), which did not prevent the fast mode, so the per-round
medians are reported: a run whose rounds disagree was taken across a mode
change and should be compared round by round, or repeated. Samples are taken in 5 rounds; each round runs, for each tool in turn, a
fresh session with 20 discarded warm-up samples and then 200 kept ones (1000
per tool and scenario). Interleaving keeps the comparison fair within a run.

The same is repeated (again in interleaved rounds) while a second pane runs `yes` (focus stays on the
`cat` pane; tmux `split-window -d`, zellij `run` then `focus-pane-id terminal_0`).

Bare ESC: the pane runs `lifbench-driver escprobe`, which sets raw mode and prints
`~` (then erases it) for every ESC byte it receives. The driver sends a lone
0x1b and times the `~`. 333 samples, 15-30 ms apart. tmux runs twice: default
config (`escape-time` is 10 ms in 3.8) and `set -sg escape-time 0`.

### 6. Binary

Size of the resolved binary and `otool -L` output; "non-system" means outside
`/usr/lib` and `/System`.

## Known limitations and noise

- This is a desktop machine, not an isolated benchmark host: WindowServer,
  browsers and other people's builds run at the same time. Load average is
  recorded per metric in the JSON. Interleaving and metric-major ordering
  make the comparison fair between tools in one run, but absolute numbers
  move between runs; compare tools within one run, and look at the spread.
- A single keystroke crosses 4-5 processes (driver, client, server, pane tty,
  and back), each a wakeup whose cost depends on which core it lands on and
  that core's power state; macOS offers no CPU affinity to pin this down. It
  is the main source of run-to-run spread in latency, including the two
  modes described under "Input latency".
- The driver discards output instead of rendering it; a real terminal is
  slower to consume, which matters for "none" (it bounds the bare-PTY case
  at what the driver can read) and for multiplexers that pace their output on
  client backpressure.
- The driver answers terminal queries like a basic xterm-compatible terminal.
  Clients that adapt to terminal capabilities (kitty keyboard protocol, sync
  output 2026) may behave differently with a full-featured terminal.
- Throughput completion is "cat exited", i.e. the server has read the bytes;
  drawing may lag (see "tail").
- zellij's floating onboarding plugin is suppressed as described above; its
  default tab-bar and status-bar plugins stay on, because that is what users
  run.
- Memory is a snapshot, not a peak. The sum over server processes excludes
  pane shells and processes the server spawns (zellij runs its plugins as
  threads in the server, so they are included).

## Adding a tool

Write one subclass of `Adapter` in `lifbench.py` and add it to `ADAPTERS`:

| Method | Must |
|---|---|
| `binary_name()`, `available()`, `version()` | locate the binary; the tool is skipped when unavailable |
| `start()` | create the session detached at 200x50, or do nothing if the client creates it |
| `client_argv()` | argv of an attaching client (run on the driver's PTY) |
| `server_pids()` | pids of all server-side processes (also recorded for cleanup) |
| `split(cmd)` | add a pane running cmd (or a shell) without moving input focus |
| `pane_count()` | number of panes, for checking the 10/50 pane scenarios |
| `stop()` | shut the server down with the tool's own command, then `kill_tracked` leftovers |
| `hf_create_kill()` | (prepare, command, cleanup) for the hyperfine create+kill benchmark |
| `detach_keys` | bytes that detach the client |
| `config_text()` | the configuration in effect, recorded in the results |

Everything must live in `self.sock_dir` / `self.work` (use `self.env()`) and honor
`self.opts` (`history`: scrollback lines; `escape_time`: ms, where supported).

The `lif` adapter drives `target/release/lif` with `-S <private socket>` and the
tmux verbs `new-session -d -x 200 -y 50`, `attach -t`, `split-window -d -t [CMD]`
(+ `select-layout tiled`), `list-panes -s`, `list-windows`, `has-session`,
`kill-server`; detach is `C-b d`. The server is a daemon forked from the first
client: its command line is that client's (so it names the private socket)
and its parent is launchd, which is how its pid is found (ppid 1 and socket
path in the command line). lif 0.1.0 does not read a config file yet, so the
scrollback scenario sets `scrollback-bytes` (1 GiB) and `history-limit` with
`set-option -g` after the server starts, then opens a fresh window and kills
the first one, because options only apply to panes created afterwards.
Retained scrollback lines are read back from every tool (`#{history_size}`
for tmux and lif, a `dump-screen --full` line count for zellij) and reported.

## Driver tests

```sh
export PATH=/opt/homebrew/bin:$PATH CARGO_TARGET_DIR=/Volumes/extssd/Development/lif/target-bench
cd bench/driver && cargo test && cargo clippy --all-targets -- -D warnings
```

The tests run the driver on real PTYs: a shell session (ready, a 200,000-line
run whose byte count is checked, echo latency through `cat`), the ESC probe, a
run timeout, query answering, attach-detach against a stand-in client, rusage
CPU time against `ps`, and `waitfile`.
