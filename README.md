# lif

lif is a terminal multiplexer for macOS (and, untested, Linux) that keeps tmux's
commands, key bindings and target syntax, so most of what your hands and
scripts know about tmux works unchanged. It is a single-threaded Rust server
built on Ghostty's terminal engine (libghostty-vt), aimed at high output
throughput, low idle cost and a small self-contained binary.

Status: version 0.1.0. Enough of tmux is implemented for daily use; the list of
what is missing is below and in [docs/tmux-migration.md](docs/tmux-migration.md).

## Benchmarks

<!-- BEGIN BENCHMARKS: generated from bench/results/2026-11-15-0330-final-summary.md; replace this whole block when the numbers are re-measured -->

lif 0.1.0 against tmux 3.8, zellij 0.46.0 and a bare shell on the same PTY
("none"). One run on one machine (Mac18,5, Apple M6, 12 cores, 16 GiB,
macOS 27.2; 2026-11-15) with other work running (load average about 4.5),
client PTY 200x50, every tool with its default settings. Medians; spreads and
sample counts are in
[bench/results/2026-11-15-0330-final-summary.md](bench/results/2026-11-15-0330-final-summary.md),
the methodology in [bench/README.md](bench/README.md), and the history of how
the numbers moved in [bench/results/comparison.md](bench/results/comparison.md).
Absolute latencies vary several-fold between runs on this machine for every
tool at once; compare tools within a run.

| | none | tmux 3.8 | zellij 0.46.0 | lif |
|---|---|---|---|---|
| `cat` 100 MB ASCII (MB/s) | 209.2 | 49.8 | 76.4 | 210.9 |
| `cat` 100 MB SGR-colored UTF-8 (MB/s) | 213.1 | 19.0 | 75.7 | 142.7 |
| `cat` one 20 MB line (MB/s) | 196.6 | 53.5 | 14.4 | 204.1 |
| server memory, idle, 1 pane (MiB, footprint / RSS) | - | 3.8 / 5.2 | 25.1 / 41.7 | 3.4 / 5.2 |
| server memory, idle, 50 visible panes (MiB) | - | 4.2 / 5.7 | 37.7 / 54.7 | 9.6 / 11.6 |
| server memory, 100k colored lines of history (MiB) | - | 257.0 / 258.4 | 232.2 / 248.7 | 43.3 / 235.6 |
| server memory after a 100 MB `cat` (MiB), lines kept | - | 6.4 / 7.9, 1,818 | 44.5 / 61.0, 9,997 | 5.6 / 13.8, 3,784 |
| server over 30 s idle: CPU ms, interrupt wakeups | - | 0.7, 3 | 10.2, 363 | 0.1, 1 |
| create session + kill server (ms) | - | 13.0 | 61.9 | 6.9 |
| attach, draw, detach (ms, includes a 20 ms wait) | - | 28.6 | 69.6 | 25.2 |
| new session until a typed command runs (ms) | 11.5 | 24.1 | 108.7 | 33.3 |
| key echo, idle pane, p50 / p95 (us) | 2 / 4 | 21 / 26 | 97 / 138 | 17 / 23 |
| key echo while another pane runs `yes`, p50 / p95 (us) | - | 273 / 365 | 1,114 / 1,392 | 40 / 51 |
| bare ESC to the program, default settings, p50 (us) | 8 | 10,075 | 56,614 | 5,046 |
| bare ESC with `escape-time 0` / kitty keyboard protocol, p50 (us) | - | 1,283 / - | - | 24 / 24 |
| binary size (MB) | - | 1.19 + 4 Homebrew dylibs | 73.07 | 3.14, system libraries only |

Where lif still loses: each extra visible pane costs about 130 KiB (tmux:
about 9 KiB), so 50 idle panes take 9.6 MiB against tmux's 4.2 MiB. After
large output its RSS stays higher than tmux's (13.8 MiB against 7.9 after the
100 MB `cat`, and nearly as high as tmux's with 100k lines of history)
although its footprint is lower: compressed history leaves its old pages
resident until macOS reclaims them. A new session took 9 ms longer than
tmux's to run its first typed command in this run (in an earlier run lif was
10 ms faster; see comparison.md). The binary is 2.6 times tmux's.

<!-- END BENCHMARKS -->

## Install

There are no packages yet; build from source.

Requirements:

- Rust 1.96 or newer.
- Zig 0.17 on `PATH`. The terminal engine, libghostty-vt, is compiled from
  source by its build script.
- macOS on Apple Silicon is the only tested platform (see [Platforms](#platforms)).

```sh
git clone <this repository> lif && cd lif
cargo build --release -p lif
cp target/release/lif ~/.local/bin/   # or anywhere on PATH
```

The binary is self-contained: libghostty-vt is linked statically and it only
links system libraries (`otool -L` shows `libSystem` and `libiconv`). There
is no terminfo entry, runtime directory or plugin to install. A man page source
is in [docs/lif.1](docs/lif.1) (`man ./docs/lif.1`).

## Quick start

The five things a tmux user does first, with the same keys (prefix `C-b`):

```sh
lif                      # new session, attached (same as: lif new-session)
lif new -s work          # named session
```

| Do | Keys | Command |
|---|---|---|
| Split side by side / top and bottom | `C-b %` / `C-b "` | `split-window -h` / `split-window` |
| Move between panes | `C-b` + arrow, `C-b o`, `C-b ;` | `select-pane -L/-R/-U/-D` |
| New window, next / previous, by number | `C-b c`, `C-b n` / `C-b p`, `C-b 0`..`9` | `new-window`, `next-window`, `select-window -t :=N` |
| Scroll back and copy | `C-b [`, move, `Space`/`C-Space` to start, `Enter`/`M-w` to copy, `C-b ]` to paste | `copy-mode`, `paste-buffer` |
| Detach and come back | `C-b d`, then `lif attach` (or `lif a -t work`) | `detach-client`, `attach-session` |

`C-b z` zooms a pane, `C-b x` kills it after confirmation, `C-b :` opens the
command prompt, `C-b ?` lists the key bindings. `lif ls` lists sessions and
`lif kill-server` stops everything. Commands take the same flags and targets as
tmux, for example `lif split-window -h -t work:1.0 -c ~/src` or
`lif display-message -p '#{session_name}:#{window_index}'`.

## Same as tmux

- Prefix `C-b`, and tmux 3.8's default bindings for the commands lif has (61 of
  tmux's 93 default prefix bindings are identical; the rest are listed in the
  migration guide). The copy-mode tables (`copy-mode`, `copy-mode-vi`) are
  tmux 3.8's, minus the mouse entries and emacs `L`.
- Command names, aliases (`splitw`, `neww`, `a`... and unambiguous prefixes),
  flags, the command language (`;` separators, `{ }` blocks, quoting), and the
  error messages for unknown commands, options and targets.
- Targets: `session:window.pane`, `$id`, `@id`, `%id`, `=exact`, `{last}`,
  `{next}`, `+N`/`-N`, `{top}`/`{bottom}`/`{left}`/`{right}`, globs.
- Formats: `#{variable}`, `#S #I #W #P #T #D #F #H #h`, conditionals,
  comparisons and the common modifiers (`=N`, `pN`, `t:`, `b:`, `d:`, `l:`,
  `E:`, `T:`, `q:`, `n:`, `w:`, `s/a/b/:`, `m:`, `e|op|:`) and
  strftime in the status line.
- Option names, scopes (`-s`, `-g`, `-w`, per session and window) and the
  defaults of the 60 tmux options lif has (except `escape-time` and
  `history-limit`, see below), plus user options (`@name`).
- Layouts: `even-horizontal`, `even-vertical`, `main-horizontal`,
  `main-vertical`, their `-mirrored` variants, `tiled`, and tmux layout
  strings in `#{window_layout}`.
- A server per socket, started by the first client, sessions surviving detach.

## Deliberately different

- **Configuration is TOML**, at `~/.config/lif/config.toml` (or
  `$XDG_CONFIG_HOME/lif/config.toml`, or `$LIF_CONFIG`), not a file of tmux
  commands. lif does not read `~/.tmux.conf`. `lif migrate-tmux-conf`
  translates the common part of one and lists every line it could not
  translate; `lif check-config` reports mistakes with line, column and a
  suggestion. See [docs/configuration.md](docs/configuration.md).
- **Scrollback has two limits per pane**: `history-limit` in lines (default
  4000, tmux's is 2000) and a memory budget, `scrollback-bytes` (`scrollback =
  "10MB"` in the configuration file); both hold. Changing either applies to
  existing panes too (in tmux only to new ones). History a quiet pane holds is
  compressed after 2 s.
- **Clipboard and notifications pass through.** Copying in copy mode sets the
  outer terminal's clipboard with OSC 52 (`set-clipboard external`, the
  default), and OSC 52 from programs in panes is forwarded too (tmux 3.8's
  `external` drops it; `set-clipboard off` stops both). Desktop notifications from
  programs (OSC 9, OSC 777, OSC 99) reach the outer terminal as OSC 9.
- **Kitty keyboard protocol end to end.** When the outer terminal answers the
  kitty keyboard query, lif enables the protocol there (and restores it on
  detach); keys are then encoded for each pane according to the keyboard mode
  the program in that pane asked for, kitty or legacy.
- **`escape-time` defaults to 5 ms** (tmux: 10), and with the kitty protocol
  active there is no wait at all: the Escape key is unambiguous and is
  delivered at once. `escape-time 0` never waits, as in tmux.
- **TERM inside panes is `xterm-256color`**, with `COLORTERM=truecolor` and
  `TERM_PROGRAM=lif`. There is no `default-terminal` option and no
  `tmux-256color`-style terminfo entry to install.
- **`$LIF` instead of `$TMUX`.** Panes get `LIF=<socket>,<server pid>,<session id>`
  and `LIF_PANE=%N`; `TMUX` is not set (and is removed from the environment
  panes inherit). `lif` run inside a pane talks to that pane's server
  through `$LIF` unless `-S` or `-L` is given, and refuses to attach unless
  `$LIF` is unset, as tmux does with `$TMUX`.
- **Almost no shell commands from lif.** `#(command)` in formats expands to
  nothing and there is no `run-shell` or `if-shell`. The exception is copying:
  `copy-pipe-and-cancel "pbcopy"` runs `pbcopy` with the selection, and the
  `copy-command` option does the same for the default copy keys and mouse
  copies, as in tmux.
- **Mouse handling is built in**: mouse support is on by default (as in tmux
  3.8) and does what tmux's default mouse bindings do (a drag or a double or
  triple click copies and leaves copy mode), but mouse events cannot be bound.
  The `mouse-drag-end` option replaces rebinding `MouseDragEnd1Pane`.

## Not implemented

Checked against the command table and by running each one (they fail with
`unsupported command: NAME`):

- Running shell commands: `run-shell`, `if-shell`, `pipe-pane`, `#()` in
  formats.
- Hooks (`set-hook`, `show-hooks`) and `wait-for`.
- Popups and menus (`display-popup`, `display-menu`), floating panes
  (`new-pane`, `move-pane`).
- Tree and chooser modes: `choose-tree`, `choose-buffer`, `choose-client`,
  `customize-mode`, `find-window`, `clock-mode`, `switch-mode`.
- Session groups and linked windows (`new-session -t`, `link-window`,
  `unlink-window`), `join-pane`, `resize-window`, `respawn-window`.
- Environment commands (`set-environment`, `show-environment`), buffers to and
  from files (`load-buffer`, `save-buffer`), `clear-history`, locking,
  `server-access`, prompt history.
- Control mode (`-C`), marked panes, synchronized panes, pane titles set by
  `select-pane -T`, pane options (`set -p`), most of tmux's 204 options (lif
  has 60 of them, plus 2 of its own).
- Images (sixel, kitty graphics), plugins (TPM and anything that needs
  `run-shell`), Windows.

## Configuration in brief

```toml
# ~/.config/lif/config.toml
prefix = "C-a"          # also unbinds C-b and binds C-a to send-prefix
mode-keys = "vi"
scrollback = "64MB"
base-index = 1

[status]
position = "top"

[theme]
status = { bg = "#1e1e2e", fg = "#cdd6f4" }
current-window = { fg = "black", bg = "green", bold = true }

[keys]
"|" = "split-window -h -c '#{pane_current_path}'"
"-" = "split-window -v -c '#{pane_current_path}'"
h = { command = "select-pane -L", repeat = true }
"C-o" = false            # remove a default binding

[keys.copy-mode-vi]
v = "send-keys -X begin-selection"
```

The file is read when the server starts; `lif source-file ~/.config/lif/config.toml`
reloads it. Mistakes never stop the server: the rest of the file still
applies, and the errors go to `lif show-messages`. `lif print-config` prints
the documented default file. Full reference: [docs/configuration.md](docs/configuration.md).

## Architecture

- **One server thread, one event loop** (`mio`: kqueue on macOS, epoll on
  Linux). No thread per pane and no periodic timers: an idle server sleeps
  until there is I/O.
- **Terminal engine: libghostty-vt** (Ghostty's VT parser and screen model,
  linked statically). Each pane's output is parsed as fast as it can be read.
- **Diff renderer with frame skipping, not output queues.** Nothing is queued
  per client. When a client can take a frame, lif renders the newest state of
  its panes as a diff against what that client shows, at most one frame per
  8 ms for output nobody typed for; intermediate states are skipped. The echo
  of a key is drawn at once, in the same loop iteration, and only the echoing
  pane is refreshed for it, so typing stays fast next to a pane flooding output.
- **The server drives the terminal.** The client puts its terminal in raw mode
  and hands it over: the server opens the same terminal by path and reads
  keys from it and writes frames to it directly, so a keystroke costs the
  client nothing. The client only forwards resizes and signals. If the server
  cannot open the terminal, the client relays bytes instead.
- **Memory per pane follows what is shown.** A pane no client shows keeps only
  its terminal (no frame or render state); a pane that was resized is
  repacked once it is quiet; history is compressed after 2 s without output.

| Crate | Role |
|---|---|
| `lif` | The binary: command line, server start-up |
| `lif-server` | Event loop, panes, clients, frame scheduling |
| `lif-core` | Sessions, windows, panes, layouts, command language, options, key bindings, formats (no I/O) |
| `lif-vt` | `VtEngine` trait and the libghostty-vt backend |
| `lif-pty` | PTYs and child processes |
| `lif-input` | Parser for bytes from the outer terminal (keys, mouse, kitty protocol, replies) |
| `lif-render` | Compositor and diff renderer |
| `lif-client` | The attached client |
| `lif-proto` | Client-server protocol |
| `lif-copy` | Copy mode |
| `lif-config` | The TOML configuration file and the `.tmux.conf` translator |
| `lif-types` | Shared types |
| `bench/` | Benchmark harness against tmux and zellij |

## Platforms

- **macOS on Apple Silicon (arm64)**: the only platform it has been built,
  tested and benchmarked on.
- **Linux**: the code has Linux paths (epoll, `/proc`, PTY handling) but has
  not been built or run there. Expect problems.
- Other Unix systems: untested. Windows: not supported.

## License

MIT (as declared in `Cargo.toml`).

## Acknowledgements

- [Ghostty](https://ghostty.org)'s libghostty-vt is the terminal emulator inside
  every pane, used through the libghostty-rs bindings.
- [tmux](https://github.com/tmux/tmux) is the behavioral reference: its
  commands, key tables, options, formats, targets and messages are what lif
  reproduces, and tmux 3.8 is what lif was compared against.
