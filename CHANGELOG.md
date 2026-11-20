# Changelog

## 0.5.0 (unreleased)

- Security: a program could run lif commands (and through `run-shell`,
  shell commands) by setting its pane's title (OSC 0/2) to something like
  `a"; run-shell "..."`: prefix `T` offers the title in a prompt, and
  Enter put it back into `select-pane -T "%%"` unescaped. The default
  prompts that offer a name for editing (prefix `$`, `,`, `T` and Rename
  in the window and session menus) now use `%%%`, which escapes the
  answer. Bindings of your own that pass `#T` or another format through
  `command-prompt -I` need the same change.
- Fixed: an exiting server kept its socket until its panes had died (up
  to 500 ms), so a client arriving meanwhile had its connection dropped.
  The socket now goes first, and a new server starts as soon as the old
  one lets go of its lock (issues/0004, now explained).
- Fixed on Linux: a key's echo next to a pane flooding output took about
  4 ms (median). After a key the server now stops reading other panes for
  up to 2 ms (`ECHO_PAUSE`), since reading a flood keeps the kernel's tty
  work queue busy in front of the echo; the median is about 115 us and
  `echo_is_not_paced_next_to_a_flood` runs on Linux again (issues/0006).
  On macOS the same echo went from 39 to 23 us (p50).
- The README's benchmark table is re-measured with 0.5.0
  (`bench/results/2026-11-19-1543-0.5.0-summary.md`); an A/B run against
  0.4.0 is in `bench/results/2026-11-19-1539-0.5.0-ab-summary.md`.
- Fixed: `bench/run.sh` measured whatever `target/release/lif` held (a
  build that called itself 0.3.0 for the 0.4.0 table); it now builds lif into
  `target-bench` and measures that.
- `find-window -r` (a regular expression, with copy-mode search's
  syntax) and the format matches `#{m/r:regex,s}`, `#{m/i:glob,s}`
  and `#{m/ri:...}`, as in tmux. The regular expression engine moved
  from `lif-copy` to `lif-core` (`lif_copy::regex` still names it).
- `pipe-pane -I`: what the command prints is typed into the pane (with
  `-O` as well, both ways). Read only while the pane's input queue has
  room, so a program that does not read its input makes the command wait,
  not the server. A pipe now closes once its command has exited and all it
  printed was read.
- `command-prompt` has all of tmux's flags: `-i` (runs the command as
  the text changes, with `=text`, `+text` for C-s and `-text` for C-r),
  `-e` (BSpace on an empty line cancels), `-l` (no splitting at commas),
  `-b` and `-C`. Changed, as in tmux: without `-b` the commands after
  `command-prompt` or `confirm-before` wait until the prompt closes (after
  the answer's commands, their own waits included), and so does a client
  that ran them.
- `update-environment` (86 tmux options now), with tmux's default list,
  and `-E` of `new-session`, `attach-session` and `switch-client`: the
  variables it names are copied from the client into the session's
  environment (`show-environment` lists them; one the client lacks is
  `-NAME`). A list option, shown as `update-environment[0] DISPLAY`...;
  in `config.toml` a TOML array. The client now sends its environment in
  `Hello` (at most 512 variables); the server keeps only
  `ENV_ALLOW_LIST` and what `update-environment` names.
- Locking: `lock-client`, `lock-session`, `lock-server` and the options
  `lock-command` and `lock-after-time` (88 tmux options, 85 of the 92
  commands now). A locked client restores its terminal, runs
  `lock-command` on it and comes back when that exits. New in the
  protocol for it: `ServerMsg::Lock` and `Caps::LOCK` (an older client
  cannot be locked and says so). `lock-after-time` is a deadline that
  exists only while some session has the option set.
- `customize-mode` (`C-b C`, 86 of the 92 commands): tmux's headings
  (options by scope, hooks, environments, key tables), Enter edits the
  selected setting on the status line, `d` / `u` put the default back,
  and the mode stays up with the new value. The preview shows the scope
  and value of an option, not tmux's description.
- Linked windows and session groups (88 of the 92 commands): `link-window`,
  `unlink-window`, `new-session -t`. A window can now be in several
  sessions (and at two indexes of one); its panes show wherever it is,
  `kill-window` removes it everywhere, it takes the size the clients of
  all its sessions ask for, and alerts reach all of them. Group members
  share their window list. Real values for `window_linked`,
  `window_linked_sessions[_list]` and the `session_group*` formats.
  Model interface: `Window::links`, `Window::index_in`.
- Control mode: `lif -C` and `-CC` speak tmux's control-mode protocol
  (command answers in `%begin`/`%end`/`%error`, `%output`, and the
  session, window and layout notifications), with `refresh-client -C`.
  Pane output waits for a slow control client up to 1 MiB, then is
  paused (`%pause`/`%continue`). New in the protocol: `Caps::CONTROL`;
  in the model, `ControlEvent`, `Model::set_control_events` and
  `ClientInfo::control`.
- Floating panes (90 of the 92 commands): `new-pane` and `move-pane`
  with tmux's sizes, positions and depth, drawn in a box over the tiled
  panes; `resize-pane`, `split-window`, `swap-pane` and the mouse know
  them; `#{pane_floating_flag}`. Not yet: modal panes, `-A`, dragging,
  the `move` table. Model interface: `Window::floating`,
  `ClientView::floating`.
- Pane scrollbars: `pane-scrollbars` (`on`, `modal`),
  `pane-scrollbars-position`, `pane-scrollbars-style` (91 tmux options).
  `ClientView::scrollbars`, `Model::client_view_with`.

## 0.4.0

- Mouse keys: what the mouse does outside copy mode is bound in the root
  table under tmux 3.8's names (`MouseDown1Pane`, `MouseDrag1Border`,
  `WheelUpStatus`, `DoubleClick1Pane`, `C-MouseDown1Status`, ...) with
  tmux's default bindings, and can be bound, rebound and unbound
  (`bind -n`, `unbind -n`, `[keys.root]`). New for them: the target `=`
  (`{mouse}`), `send-keys -M`, `copy-mode -M`, `resize-pane -M`,
  `display-menu -x M -y M` / `-x W -y W` / `-O`, and the formats
  `mouse_x`, `mouse_y`, `mouse_word`, `mouse_line`, `mouse_any_flag`,
  `alternate_on`, `pane_mode`. A key left at its default is still handled
  by the server directly, so the default mouse costs what it did.
- New defaults that come with them, as in tmux: the middle button pastes,
  the right button opens the pane menu (on a window in the status line the
  window menu, on `status-left` a session menu), and the wheel over the
  status line changes window.
- The marked pane: `select-pane -m` / `-M` (`C-b m`, `C-b M`), the
  targets `{marked}` and `~`, the default source of `swap-pane`,
  `join-pane` and `swap-window`, `#{pane_marked}`, `#{pane_marked_set}`,
  `#{window_marked_flag}` and `M` in the window flags; the menus have
  tmux's "Swap Marked" and "Mark" items.
- Tree mode: `m` and `M` (mark), `S-Up` / `S-Down` (swap the window with
  its neighbour); all choosers: `C-h` or `F1` shows the keys.
- `[startup]` in the configuration file: `run = [...]`, commands the
  server runs once when it starts, after the settings (`new-session`,
  `run-shell`, `if-shell`, user options, ...). Checked by `check-config`;
  failures go to `show-messages`; `source-file` does not run them again.
- 15 more tmux options (85 of them now): `monitor-silence` with
  `silence-action` and `visual-silence` (a timer that exists only while a
  watched pane has printed since it last fired), `activity-action` and
  `visual-activity`, `destroy-unattached`, `aggressive-resize`,
  `focus-events` (on by default), and `popup-style`,
  `popup-border-style`, `popup-border-lines`, `menu-style`,
  `menu-selected-style`, `menu-border-style`, `menu-border-lines`.
  `#{window_silence_flag}`, `~` in the window flags, the `alert-silence`
  hook. Changed with them: activity in a watched window now rings the
  bell once (tmux's default `activity-action other`).
- `join-pane -f`.
- The README's benchmark table is re-measured against tmux 3.8 and zellij
  0.46.0 (`bench/results/2026-11-15-2221-0.4.0-dev-summary.md`).
- Fixed: `split-window -f -b` gave the new pane the smaller half of the
  window; tmux gives it the larger (issues/0005).
- `migrate-tmux-conf` translates mouse bindings of the root table, and
  the commands of a `.tmux.conf` that are not settings into `[startup]`
  (not `source-file`, and not shell commands that mention tmux, such as
  the line that starts TPM).
- Changed: a menu opened from a mouse key now behaves as in tmux (it is up
  while the button is held; `-O` keeps it), and `list-keys` lists the root
  table's mouse keys, so the root table always exists.
- Fixed: a command run right after `kill-server` could meet the exiting
  server and fail with `server closed the connection`; the client now
  asks again (issues/0004).
- Fixed: a client could exit with `lif: Broken pipe` (status 1) on a clean
  detach when the server ended the attachment while the client was still
  writing to it, e.g. `C-b d` (which the server reads from the terminal
  itself) typed right after the client was continued (issues/0007).
- Linux: the tests now run there (Ubuntu 26.04 arm64 in a Lima VM) and
  pass, with one set aside (issues/0006: the echo of a key next to a
  flooding pane takes about 4 ms on Linux). Found and fixed on the way:
  a client no longer waits for ever to connect to a stopped server whose
  listen queue is full (Linux blocks there; `connect_stream`), and
  `detach-client -P` and `attach -x` look up the client's parent before
  the client is told to leave (it could be gone by then). Test fixes for
  Linux's limits and for a shell that is called `sh`. A CI workflow builds,
  lints and tests on macOS arm64 and Linux arm64 and x86_64.
- A new icon (`docs/lif.svg`): the active pane with its prompt next to
  two others.
- Model interface: `KeyCode::mouse`, `KeyBindings::mouse_binding`,
  `MouseBinding`, `MouseAt`, `Model::set_mouse`, `Host::mouse_request`,
  `Model::status_ranges`, `Model::marked`, `PaneInfo::mouse_any` and
  `alternate`, `MenuState::release_chooses`, `ChooseScreen::help`.

## 0.3.0 (unreleased)

- Chooser modes: `choose-tree` (`C-b s`, `C-b w`), `choose-buffer`
  (`C-b =`), `choose-client` (`C-b D`) and `find-window` (`C-b f`), with
  tmux 3.8's list, tree, preview box, keys (move, expand, tag, search,
  filter, sort, kill, run a command for the tagged items), shortcuts and
  mouse. The list follows the server while it is open.
- Popups and menus: `display-popup` (tmux's sizes, positions and
  borders; `-E` and the program's exit status; the command waits for it)
  and `display-menu` (keys and mouse), with `C-b <` and `C-b >`. A popup's
  program is a pane only the server knows, so its lifetime follows the
  ordinary pane paths; the renderer is unchanged.
- Fixed: a pane's exit status could be lost (reported as a failure) when
  the end of its output was read before the process was reaped
  (issues/0003).
- `pipe-pane` (`-o`): a pane's output also goes to a shell command; a
  command that reads slowly never holds the pane (1 MiB waits, the rest is
  not piped).
- `clock-mode` (`C-b t`) with `clock-mode-style` and `clock-mode-colour`;
  its deadline exists only while a pane shows a clock.
- `resize-window`, `respawn-window`, `show-prompt-history`,
  `clear-prompt-history`, `split-window -f`, `select-pane -d` / `-e`
  (`#{pane_input_off}`), `#{pane_pipe}`.
- The default prefix bindings have tmux's notes (`list-keys -N`), and
  `C-b /`, `C-b ~` are bound as in tmux.
- Paste buffers remember when they were made (shown by `choose-buffer`).
- Linux: the workspace is checked, linted and linked for
  `aarch64-unknown-linux-gnu` by cross compiling from macOS
  (`tools/zigcc-aarch64-linux.sh` makes zig the linker). It has still not
  been run there.
- Model interface: `Host::choose`, `Host::popup`, `Host::menu`,
  `Host::pipe_pane`, `Host::clock_mode`, `Host::prompt_history`,
  `Model::choose_items`, `Model::chooser_closed`, `ChooseState`,
  `MenuState`, `PaneInfo::pipe`. 82 of tmux 3.8's 92 commands now exist.

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
