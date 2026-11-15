# Changelog

## 0.2.0 (unreleased)

Scripting: lif runs shell commands and hooks. 70 of tmux 3.8's 92 commands
now exist (9 more).

- `run-shell` (`-b`, `-C`, `-c`, `-d`, `-E`, `-t`): the command's output is
  the command's, a failing command prints `'cmd' returned N` and gives the
  client its exit status, and the commands after it in the list wait for
  it, as in tmux.
- `if-shell` (`-b`, `-F`, `-t`); the branch runs in place, before the
  commands that follow.
- `wait-for` with channels (`wait-for name`, `-S`) and locks (`-L`,
  `-U`).
- `#(command)` in formats: the last line of the command's output; a status
  line that uses it is redrawn every `status-interval`.
- Hooks: `set-hook` (`-a`, `-g`, `-p`, `-R`, `-u`, `-w`) and `show-hooks`
  with tmux 3.8's hook names and the `hook`, `hook_session`,
  `hook_session_name`, `hook_window`, `hook_window_name`, `hook_pane` and
  `hook_client` variables. The `after-` hooks and 17 event hooks run (the
  list is in the README).
- `set-environment`, `show-environment`, and `-e VARIABLE=value` on
  `new-session`, `new-window`, `split-window` and `respawn-pane`.
- `join-pane`, `clear-history`, and the `synchronize-panes` option (typed
  keys, pastes and `send-keys`), with `#{pane_synchronized}`.
- Configuration file: `[hooks]` and `[env]` sections and
  `synchronize-panes`; `lif migrate-tmux-conf` translates `set-hook -g`
  and `set-environment -g`.
- Shell commands run as `/bin/sh -c` in their own process group, at most 64
  at once, with 1 MiB of output kept; they are hung up when the server
  exits. Nothing is polled: a server whose status line has no `#()` still
  makes no wakeups while idle.
- Fixed: `lif new-window` typed in a pane's shell failed with
  `create window failed: index N in use` (issues/0002).
- Model interface: `Host::start_job`, `Host::start_timer`,
  `Host::clear_history`, `Model::job_done`, `Model::take_finished`,
  `Model::run_hooks`; `CmdOutput` has `status` and `pending`; `SpawnSpec`
  has `env`. The wire protocol is unchanged.

## 0.1.0 (unreleased)

First version.

- Client and server: a server per socket, started by the first client, with
  sessions, windows and panes that survive detaching. One server thread and
  event loop. The server opens the attached client's terminal itself and
  reads and writes it directly (protocol capability `TTY`); the client then
  only forwards resizes and signals, and relays bytes when the server cannot
  open the terminal.
- `lif` inside a pane finds its server through `$LIF`; the socket is chosen
  from `-S`, `-L`, `$LIF`, `$LIF_SOCKET` and the default, in that order.
- Terminal emulation in every pane by Ghostty's libghostty-vt, linked
  statically; the release binary depends only on system libraries.
- Rendering: a diff renderer that sends each client the newest state of its
  panes at most once per 8 ms, skipping intermediate frames instead of
  queueing output; the echo of typed input is drawn at once; synchronized
  output (mode 2026) when the terminal has it.
- 61 of tmux 3.8's commands with tmux's names, aliases, flags (most of them),
  targets, formats and error messages; the other 31 are recognized and
  reported as unsupported. See docs/tmux-migration.md.
- tmux 3.8's default prefix bindings for those commands (61 of 93 identical),
  and its copy-mode and copy-mode-vi tables.
- 60 tmux options with tmux's defaults (except `escape-time`, 5 ms, and
  `history-limit`, 4000 lines), user options (`@name`), and two of lif's own:
  `scrollback-bytes`, a per-pane scrollback budget in bytes, and
  `mouse-drag-end`. `history-limit` and `scrollback-bytes` both hold and
  apply to existing panes; quiet panes compress their history.
- Layouts: even-horizontal, even-vertical, main-horizontal, main-vertical and
  their mirrored forms, tiled, and tmux layout strings.
- Copy mode with keyboard and mouse selection, rectangle selection, regular
  expression and literal search with match highlighting, jumps, marks, and
  paste buffers; copies also go to the terminal clipboard with OSC 52, and
  `copy-pipe` and the `copy-command` option pipe them to a command.
  `send-keys` with key names drives a pane in copy mode through its key
  table.
- Status line with tmux formats, styles and strftime; command prompt,
  confirmation prompt, `display-panes`, messages and message log.
- Mouse: pane and window selection, border resizing, wheel scrolling, drag
  selection, double and triple click; a selection is copied and copy mode
  left, as with tmux's default bindings (`mouse-drag-end` changes it); mouse
  reporting passed to programs that ask for it.
- Kitty keyboard protocol towards the outer terminal and towards programs in
  panes; no escape-time wait when it is active, `escape-time` (default 5 ms)
  otherwise.
- OSC 52 clipboard and OSC 9 / 777 / 99 desktop notifications from programs
  passed through to the terminal.
- TOML configuration file (`~/.config/lif/config.toml`) with position-accurate
  diagnostics and suggestions; `lif check-config`, `lif print-config` and
  `lif migrate-tmux-conf`.
- Benchmark harness comparing lif with tmux and zellij (`bench/`).
- Claude Code support (docs/claude-code.md): `set-option -p` and
  `show-options -p` (window options per pane), `select-pane -T` and the
  prefix `T` binding, `pane-border-status` / `pane-border-format` lines,
  `window-style` / `window-active-style`, `set-titles` /
  `set-titles-string`, `allow-passthrough` (tmux's `ESC P tmux;` wrapper is
  unwrapped), `load-buffer` (from a file or standard input, `-w` for the
  clipboard) and `save-buffer`; lif's own `tmux-compat` option (`TMUX` and
  `TMUX_PANE` in new panes) and `lif install-tmux-shim` (a `tmux` that is
  lif inside lif panes and the real tmux elsewhere), so Claude Code's agent
  teams open their teammates in lif panes. Protocol: new client message
  `Stdin`.
- `pane_current_path` falls back to the OSC 7 directory as a path (it was
  the raw `file://` URL).
- Tested on macOS on Apple Silicon only.
