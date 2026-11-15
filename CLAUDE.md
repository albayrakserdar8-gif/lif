# CLAUDE.md

Guidance for Claude Code in this repository. The ground rules every agent
follows are in AGENTS.md and are imported here; read them first. This file
adds the practical detail: how to build, test, run and debug lif, where things
live, and the conventions the existing code follows.

@AGENTS.md

## What lif is

A terminal multiplexer that keeps tmux's commands, flags, targets, formats,
default key bindings (prefix `C-b`) and error messages, with a TOML config
instead of `.tmux.conf`. Its reason to exist is speed and a small footprint:
one server thread, one mio event loop, libghostty-vt as the terminal engine,
a diff renderer that skips frames instead of queueing output. Version 0.3.0,
tested on macOS / Apple Silicon only (Linux builds by cross compiling, see
below, but has never been run).

tmux 3.8 is the behavioral reference. When unsure how something should
behave, check what tmux does (`tmux -L scratch -f /dev/null ...` on a private
socket, never the user's own sessions) and match it, unless README.md's
"Deliberately different" section says otherwise.

## Environment

```sh
export PATH=/opt/homebrew/bin:$PATH   # cargo, zig 0.17, gh
cd /Volumes/extssd/Development/lif
```

- Toolchain: Rust >= 1.96 (Homebrew rustc), Zig 0.17 (libghostty-vt is built
  from source by its build script; the first build is slow).
- Everything lives on the external SSD. Build outputs must stay there:
  default `target/` is in the repo (on the SSD). When working in parallel with
  other agents use `CARGO_TARGET_DIR=/Volumes/extssd/Development/lif/target-<crate>`.
- `.cargo/config.toml` forces libghostty-vt to build ReleaseFast even in dev
  builds. Do not remove it: its Debug build is ~1000x slower at scrollback and
  makes tests time out.

## Build, test, lint

```sh
cargo build                              # whole workspace, dev profile (opt-level 1)
cargo build --release -p lif             # target/release/lif, fat LTO, ~3 MB, slow to link
cargo test -p <crate>                    # one crate
cargo test -p lif --test cli             # one integration-test file of the binary
cargo test -p lif --test cli -- some_test_name --nocapture
cargo clippy --all-targets -- -D warnings
```

The code is not rustfmt-formatted (lines run to about 150 columns and
`cargo fmt --check` reports differences in almost every file). Do not run
`cargo fmt` on the workspace: it would rewrite every file. Match the style
of the surrounding code by hand.

"Done" = build, test and clippy (`-D warnings`) pass for every crate touched.

Testing notes:

- Tests use real PTYs, real sockets and the real `lif` binary
  (`CARGO_BIN_EXE_lif`). Do not replace them with mocks.
- `crates/lif/tests/common/mod.rs` is the end-to-end harness: `Env::new()`
  gives each test a private socket dir under `CARGO_TARGET_TMPDIR`, a private
  `HOME` with a fixed `PS1='$ '`, `SHELL=/bin/sh`, and strips `LIF*`/`TMUX*`
  variables. A libghostty terminal stands in for the user's terminal so tests
  assert on what is actually visible.
- `Env::drop` fails a test whose pane processes outlive `kill-server` by 3 s.
  A test that SIGKILLs the server must call `env.track_panes()` first.
  Background: issues/0001-orphaned-pane-shells-spin.md (macOS `/bin/sh` is
  bash 3.2, which can deadlock in its SIGHUP handler; the server now escalates
  to SIGKILL after `PANE_KILL_GRACE`).
- After a test run, check nothing leaked:
  `ps -Ao pid,ppid,pcpu,etime,args | grep -- ' -sh$'` should print nothing
  from `crates/lif`.
- Some tests are timing-dependent (latency, stress). The machine is often
  loaded; a single flaky failure should be re-run once and, if it persists,
  investigated rather than loosened blindly. Wait on conditions (poll the
  screen until text appears, with a deadline), never on fixed sleeps.
- Pure crates (`lif-core`, `lif-copy`, `lif-input`, `lif-render`,
  `lif-config`, `lif-proto`, `lif-types`) have `#![forbid(unsafe_code)]`
  and fast unit tests; prefer adding coverage there when the logic is pure.

## Running it by hand

```sh
cargo build --release -p lif
./target/release/lif -L dev                 # private socket "dev", never the default one
./target/release/lif -L dev ls
./target/release/lif -L dev kill-server
```

- Always use `-L <name>` (or `-S <path>`) when experimenting so you never
  touch the user's own lif sessions on the default socket
  (`$TMPDIR/lif-<uid>/default`; overridable with `$LIF_SOCKET`).
- Inside a pane, `$LIF` (`<socket>,<server pid>,<session id>`) and
  `$LIF_PANE` (`%N`) are set; `lif` run there talks to that server. Unset
  `LIF` before starting a nested test server.
- Config is read at server start from `-f FILE`, `$LIF_CONFIG`,
  `$XDG_CONFIG_HOME/lif/config.toml`, `~/.config/lif/config.toml`. For
  experiments pass `-f` or `LIF_CONFIG`; do not edit the user's file.
- Config tools that need no server: `lif check-config [FILE]`,
  `lif print-config`, `lif migrate-tmux-conf [FILE]`. Errors from a
  running server's config load go to `lif show-messages`.
- An interactive client needs a real terminal; Claude Code's shell has none.
  To observe behavior from a script, drive the server with commands
  (`new-session -d`, `send-keys`, `capture-pane -p`, `display-message -p`)
  or write an end-to-end test using the harness.

## Architecture map

Data flow: PTY output -> `lif-pty` -> `VtEngine` (`lif-vt`, libghostty) ->
render state -> `lif-render` diff -> client's terminal. Keys: client's tty ->
`lif-input` parser -> `lif-core` key tables / commands -> encoded for the
pane (kitty or legacy) -> PTY.

| Crate | Where to look |
|---|---|
| `lif` | `src/main.rs` CLI parsing and socket choice, `src/start.rs` daemonize and server start; `tests/` end-to-end suites (`basic`, `choose`, `cli`, `commands`, `config`, `copy`, `features`, `files`, `jobs`, `latency`, `more`, `overlay`, `scripting`, `server_limits`, `stress`, `stress_fuzz`, `window_index`) |
| `lif-server` | `server.rs` event loop and deadlines (`next_deadline`), `pane.rs` PTY reading with budgets, `client.rs` attached clients and direct tty I/O, `frame.rs` frame scheduling (8 ms pacing, immediate echo frames), `host.rs` implements `lif-core`'s `Host` trait, `job.rs` shell commands for `run-shell` / `if-shell` / `#()`, `choose.rs` chooser modes in a pane, `overlay.rs` popups and menus (a popup is a pane only the server knows), `clock.rs`, `pipe.rs` `copy-pipe` and `pipe-pane`, `input.rs`, `mode.rs` copy mode glue, `prompt.rs`, `panic.rs` panic hook that kills pane groups |
| `lif-core` | Pure model: `model.rs` sessions/windows/panes, `cmd/` command lexer and table, `exec.rs`, `queue.rs` commands that wait (`run-shell`, `if-shell`, `wait-for`) and `#()`, `hooks.rs`, `environ.rs`, `choose.rs` (chooser lists and keys), `overlay.rs` (popup and menu geometry, menu state), `target.rs`, `format.rs` (`#{...}`), `options.rs`, `bindings.rs`, `layout.rs`, `status.rs` |
| `lif-vt` | `VtEngine` trait (`lib.rs`, a contract: do not change without being asked) and the libghostty backend (`ghostty.rs`, `ghostty/`) |
| `lif-pty` | `pty.rs` open/fork/exec (resets signals in the child), `spawn.rs` login-shell argv (`-sh`), `reaper.rs` |
| `lif-input` | Byte parser for the outer terminal: keys, mouse, kitty protocol, query replies (`probe.rs`) |
| `lif-render` | Compositor (`scene.rs`, `layout.rs`) and diff renderer (`render.rs`, `sgr.rs`, `moves.rs`) |
| `lif-client` | Attach loop, raw mode, signal restore, socket path resolution (`path.rs`), byte-relay fallback |
| `lif-proto` | Wire messages (`msg.rs`, a contract) and codec |
| `lif-copy` | Copy mode: motions, selection, search, regex, tmux-parity tests |
| `lif-config` | TOML schema, diagnostics with suggestions (`check.rs`, `suggest.rs`), `.tmux.conf` translator (`tmux.rs`), `config.example.toml` (source of `print-config`) |
| `lif-types` | Shared geometry, cells, frames, input events, capabilities (a contract) |

The libghostty-rs API source is under
`~/.cargo/git/checkouts/libghostty-rs-*/cd4d497/crates/libghostty-vt/src`.

## Performance rules in practice

- Hot paths (PTY read, VT feed, render, input parse) must not allocate per
  byte, cell or frame: reuse buffers held by the pane/client.
- Never add a periodic timer. A new deadline goes through
  `Server::next_deadline` and exists only while there is work for it. Idle
  server = zero wakeups (the bench measures this).
- Never queue output for a slow client; mark it dirty and render the newest
  state when it is writable. Every buffer has a stated bound (see
  `lif-core` limits and `server_limits.rs` tests).
- Optimizations need before/after numbers (hyperfine, `bench/run.sh --only ...`,
  or the crate `bench.rs` tests) in the report.

## Linux (cross-check only)

Nothing on this machine runs Linux, so Linux is checked by cross compiling.
Homebrew's rustc has only the host target; use the rustup toolchain, which
has `aarch64-unknown-linux-gnu` installed, and its own build directory:

```sh
export PATH=$(dirname $(rustup which rustc --toolchain stable)):/opt/homebrew/bin:$PATH
export CARGO_TARGET_DIR=/Volumes/extssd/Development/lif/target-linux
cargo check --workspace --all-targets --target aarch64-unknown-linux-gnu
cargo clippy --workspace --all-targets --target aarch64-unknown-linux-gnu -- -D warnings
CARGO_TARGET_AARCH64_UNKNOWN_LINUX_GNU_LINKER=$PWD/tools/zigcc-aarch64-linux.sh \
  cargo build -p lif --target aarch64-unknown-linux-gnu      # links an ELF binary
```

That toolchain's clippy is newer than Homebrew's and has more lints; code
must pass both. A change to `cfg(target_os)` code is not done until this
check passes. Running the tests on Linux needs a VM or container, which is
not installed.

## Benchmarks

```sh
bench/run.sh --tools lif --quick            # sanity check of the harness
bench/run.sh --tools none,tmux,zellij,lif   # full run, ~10+ minutes
bench/run.sh --only throughput,latency
```

Builds into `target-bench/`; results go to `bench/results/`. The harness uses
private sockets and HOMEs and only signals processes it started. The README's
benchmark table sits between `BEGIN BENCHMARKS` / `END BENCHMARKS` markers
and is regenerated from a summary file; replace the whole block, don't hand-
edit single numbers. Note the load average with any numbers you report: this
machine is often busy and absolute latencies vary several-fold between runs.

## Documentation that must stay in sync

When behavior changes, update in the same change:

- `README.md` (feature lists, "Deliberately different", "Not implemented")
- `docs/tmux-migration.md` (per-command support table and flag lists)
- `docs/configuration.md` and `crates/lif-config/config.example.toml`
- `docs/copy-mode.md`, `docs/lif.1` (man page)
- `docs/claude-code.md` when anything Claude Code relies on changes (the
  commands its tmux backend runs are exercised by
  `crates/lif/tests/claude_compat.rs`)
- `CHANGELOG.md` under the unreleased version

Every claim in these docs was checked by running lif; keep it that way.

## Issues

Bugs found while working go in `issues/NNNN-short-slug.md` with: status,
what happened (with evidence), expected behavior, hypotheses, root cause,
fix, what is not fixed, and measured cost. See `issues/0001-*` as the model.

## Code style

- Rust 2024; follow the existing formatting (see above). Comments say why, not what. Module docs (`//!`)
  explain each file's role; keep them accurate when moving code.
- Errors are plain enums/strings; no anyhow/thiserror. User-facing error text
  matches tmux's wording where tmux has the same error.
- `unsafe` only for OS/FFI calls, each block with `// SAFETY:`.
- No new dependencies without a one-sentence reason in the report.

## Git

- Do not commit, push or amend unless the user asks explicitly.
- The working tree often has uncommitted work from other sessions; do not
  revert or reformat files you were not asked to touch.
