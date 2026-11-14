# Working on lif

lif is a terminal multiplexer (a tmux alternative) whose reason to exist is
speed and a small footprint. Read this file before changing anything.

## Ground rules

- Rust 2024, workspace at the repo root, one crate per directory in `crates/`.
- Shell setup for every command: `export PATH=/opt/homebrew/bin:$PATH`.
  Zig 0.17 must be on PATH: `lif-vt` builds libghostty-vt from source.
- When several people work in parallel each uses its own build directory:
  `export CARGO_TARGET_DIR=/Volumes/extssd/Development/lif/target-<your-crate>`.
  Never build into the internal disk.
- Stay inside the crate(s) you were given. Do not edit the root `Cargo.toml`,
  `lif-types`, the message definitions in `lif-proto/src/msg.rs`, the
  `VtEngine` trait, or another crate. If a contract is wrong or missing
  something, finish what you can and report the exact change you need.
- Dependencies are deliberately few: `libc`, `mio`, `libghostty-vt`, and the
  workspace crates. No async runtime, no serde, no clap, no anyhow/thiserror.
  Adding a dependency needs a reason you can state in one sentence, and goes
  in your report.
- `unsafe` only where the OS or FFI demands it, each block with a
  `// SAFETY:` comment. Crates that need none keep `#![forbid(unsafe_code)]`.
- Done means: `cargo build`, `cargo test` and
  `cargo clippy --all-targets -- -D warnings` pass for your crate, and the
  tests exercise real behavior (real PTYs, real sockets), not mocks of it.
- Do not commit. Do not touch anything outside this repository. Never kill
  or attach to tmux/zellij sessions you did not start yourself.
- Comments say why, not what. No TODOs for things you were asked to do.

## Performance rules (the product)

- The server is one thread with one event loop (mio: kqueue on macOS, epoll
  on Linux). No thread per pane, no timers while idle: an idle server makes
  zero wakeups.
- Never queue bytes for a slow consumer. Queue state: parse everything into
  the pane's terminal, and render the newest state when the consumer can
  take it. Every buffer has a stated bound.
- No allocation per byte, per cell or per frame on hot paths; reuse buffers.
- Measure before and after any optimization and put the numbers in your report.

## Facts established by the spikes (`spikes/`)

- libghostty-vt 1.0.1 links statically from Rust (binary 1.5 MB with LTO).
  Parsing: about 1800 MB/s plain ASCII, about 400 MB/s SGR-heavy UTF-8 (M-series).
- `RenderState` gives per-row dirty flags; when the snapshot reports
  `Dirty::Full`, `next_dirty()` yields nothing, so iterate every row.
- On macOS both `poll` and `kqueue` report data and hangup correctly on a
  PTY master. The kernel returns at most 1024 bytes per `read` from a PTY, so
  drain until `EAGAIN` on every wakeup (with a budget, so one pane cannot
  starve the others).
- On macOS `TIOCSWINSZ` on a PTY master fails with ENOTTY until the slave has
  been opened once; `lif-pty` sets the initial size on the slave.
- A PTY's input queue takes about 1 KiB before `EAGAIN`: writes to a pane are
  always partial-write aware and wait for writability.
- `.cargo/config.toml` forces libghostty-vt to build optimized even in dev
  builds (its Debug build is about 1000x slower at scrollback).
- The libghostty-rs API lives in
  `~/.cargo/git/checkouts/libghostty-rs-*/cd4d497/crates/libghostty-vt/src`
  (`terminal.rs`, `render.rs`, `effect.rs`, `key.rs`, `mouse.rs`, `fmt.rs`).

## Layout

| Crate | Role |
|---|---|
| `lif-types` | Engine-neutral shared types: geometry, cells, frames, input events |
| `lif-proto` | Wire protocol: message types and codec |
| `lif-vt` | `VtEngine` trait and the libghostty-vt backend |
| `lif-pty` | PTY and child processes |
| `lif-client` | Thin client: raw mode, byte relay |
| `lif-core` | Sessions, windows, panes, layouts, command language, key bindings (pure, no I/O) |
| `lif-input` | Parser for bytes arriving from the client's terminal |
| `lif-render` | Compositor and diff renderer onto the client's terminal |
| `bench/` | Benchmark harness comparing lif with tmux and zellij |
