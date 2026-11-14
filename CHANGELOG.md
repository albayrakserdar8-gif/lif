# Changelog

## 0.1.0 (unreleased)

First version.

- Client and server: a server per socket, started by the first client, with
  sessions, windows and panes that survive detaching. One server thread and
  event loop; the client only relays bytes.
- Terminal emulation in every pane by Ghostty's libghostty-vt, linked
  statically; the release binary depends only on system libraries.
- Rendering: a diff renderer that sends each client the newest state of its
  panes at most once per 8 ms, skipping intermediate frames instead of
  queueing output; synchronized output (mode 2026) when the terminal has it.
- 58 of tmux 3.8's commands with tmux's names, aliases, flags (most of them),
  targets, formats and error messages; the other 34 are recognized and
  reported as unsupported. See docs/tmux-migration.md.
- tmux 3.8's default prefix bindings for those commands (61 of 93 identical),
  and its copy-mode and copy-mode-vi tables.
- 60 tmux options with tmux's defaults, user options (`@name`), and
  `scrollback-bytes`, a per-pane scrollback budget in bytes.
- Layouts: even-horizontal, even-vertical, main-horizontal, main-vertical and
  their mirrored forms, tiled, and tmux layout strings.
- Copy mode with keyboard and mouse selection, rectangle selection, regular
  expression and literal search with match highlighting, jumps, marks, and
  paste buffers; copies also go to the terminal clipboard with OSC 52.
- Status line with tmux formats, styles and strftime; command prompt,
  confirmation prompt, `display-panes`, messages and message log.
- Mouse: pane and window selection, border resizing, wheel scrolling, drag
  selection; mouse reporting passed to programs that ask for it.
- Kitty keyboard protocol towards the outer terminal and towards programs in
  panes; no escape-time wait when it is active.
- OSC 52 clipboard and OSC 9 / 777 / 99 desktop notifications from programs
  passed through to the terminal.
- TOML configuration file (`~/.config/lif/config.toml`) with position-accurate
  diagnostics and suggestions; `lif check-config`, `lif print-config` and
  `lif migrate-tmux-conf`.
- Benchmark harness comparing lif with tmux and zellij (`bench/`).
- Tested on macOS on Apple Silicon only.
