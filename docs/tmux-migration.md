# Moving from tmux to lif

This guide is for someone who uses tmux today. Everything in it was checked
against lif 0.2.0's command table and by running the commands; the tmux side is
tmux 3.8 (`tmux list-commands`, `tmux list-keys` with an empty configuration).

In short: the commands, flags, targets, formats and default keys you use every
day are the same. The configuration file is not: lif reads TOML, and
`lif migrate-tmux-conf` translates the common part of a `.tmux.conf`. Shell
commands (`run-shell`, `if-shell`, `#()`), `wait-for` and hooks work as in
tmux, with the differences listed under [Scripting](#scripting).

## Commands

70 of tmux 3.8's 92 commands exist in lif (28 with the same flags, 42 with some
difference, usually missing flags); 22 do not. Command names, aliases and unambiguous prefixes
resolve as in tmux (`lif splitw`, `lif new`, `lif a`). An unimplemented
command fails with exit status 1:

```
$ lif choose-tree
unsupported command: choose-tree
```

A flag lif does not know fails the same way (`command split-window: unknown flag -f`).
A few tmux flags that lif cannot honor fail with an explicit `not supported`
error instead (`new-session -E`, `command-prompt -i`). The flags that are
accepted without changing anything are listed in
[Flags accepted without effect](#flags-accepted-without-effect).

| tmux command | lif | Differences |
|---|---|---|
| `attach-session` (`attach`) | supported with differences | no `-E`, `-f`, `-r` |
| `bind-key` (`bind`) | supported with differences | mouse key names (`MouseDown1Pane`, ...) are rejected: mouse handling is built in (see [Mouse](#mouse)) |
| `break-pane` (`breakp`) | supported with differences | no `-W`, `-x`, `-y`, `-X`, `-Y` (floating panes) |
| `capture-pane` (`capturep`) | supported with differences | has `-b -e -E -J -N -p -q -S -t`; no `-a`, `-C`, `-F`, `-H`, `-I`, `-L`, `-M`, `-P`, `-R`, `-T` |
| `choose-buffer` | not supported | |
| `choose-client` | not supported | |
| `choose-tree` | not supported | |
| `clear-history` (`clearhist`) | supported | `-H` changes nothing (see below) |
| `clear-prompt-history` (`clearphist`) | not supported | |
| `clock-mode` | not supported | |
| `command-prompt` | supported with differences | no `-b`, `-C`, `-e`, `-l`; `-i` fails with `not supported` (incremental prompts exist only for copy-mode searches); `-P`, `-T` change nothing (see below) |
| `confirm-before` (`confirm`) | supported with differences | `-b` changes nothing (see below) |
| `copy-mode` | supported with differences | no `-k`, `-s`, `-S` |
| `customize-mode` | not supported | |
| `delete-buffer` (`deleteb`) | supported | |
| `detach-client` (`detach`) | supported with differences | no `-E` |
| `display-menu` (`menu`) | not supported | |
| `display-message` (`display`) | supported with differences | has `-c -d -F -l -p -t`; no `-a`, `-C`, `-I`, `-j`, `-N`, `-v` |
| `display-panes` (`displayp`) | supported with differences | no `-k`, `-s`, `-Z` |
| `display-popup` (`popup`) | not supported | |
| `find-window` (`findw`) | not supported | |
| `has-session` (`has`) | supported | |
| `if-shell` (`if`) | supported | the shell command runs with `/bin/sh` |
| `join-pane` (`joinp`) | supported with differences | no `-f` (full-size split); `-p` percentage is also accepted |
| `kill-pane` (`killp`) | supported with differences | no `-f` (filter) |
| `kill-server` | supported | |
| `kill-session` | supported with differences | no `-f`, `-g` |
| `kill-window` (`killw`) | supported with differences | no `-f` |
| `last-pane` (`lastp`) | supported with differences | no `-d`, `-e` |
| `last-window` (`last`) | supported | |
| `link-window` (`linkw`) | not supported | |
| `list-buffers` (`lsb`) | supported with differences | no `-O` (sort order) |
| `list-clients` (`lsc`) | supported with differences | no `-O` |
| `list-commands` (`lscm`) | supported with differences | prints names and aliases only, no usage strings; with `-F`, `#{command_list_usage}` is built from the flag table (`[-deHMqu] [-t value]`), without tmux's argument names |
| `list-keys` (`lsk`) | supported with differences | no `-a`, `-F`, `-O`, `-r` |
| `list-panes` (`lsp`) | supported with differences | no `-O`, `-r` |
| `list-sessions` (`ls`) | supported with differences | no `-O`, `-r` |
| `list-windows` (`lsw`) | supported with differences | no `-O`, `-r` |
| `load-buffer` (`loadb`) | supported with differences | has `-b`, `-w` and `-` (standard input, at most 16 MiB); `-t` changes nothing |
| `lock-client` (`lockc`) | not supported | |
| `lock-server` (`lock`) | not supported | |
| `lock-session` (`locks`) | not supported | |
| `move-pane` (`movep`) | not supported | |
| `move-window` (`movew`) | supported | |
| `new-pane` (`newp`) | not supported | |
| `new-session` (`new`) | supported with differences | no `-f`, `-t` (no session groups); `-E` fails with `not supported` (lif has no `update-environment`) |
| `new-window` (`neww`) | supported with differences | no `-E` |
| `next-layout` (`nextl`) | supported | |
| `next-window` (`next`) | supported | |
| `paste-buffer` (`pasteb`) | supported with differences | no `-S` |
| `pipe-pane` (`pipep`) | not supported | |
| `previous-layout` (`prevl`) | supported | |
| `previous-window` (`prev`) | supported | |
| `refresh-client` (`refresh`) | supported with differences | only `-S` and `-t`; no control-mode or pan flags |
| `rename-session` (`rename`) | supported | |
| `rename-window` (`renamew`) | supported | |
| `resize-pane` (`resizep`) | supported with differences | no `-M` (mouse), `-T` |
| `resize-window` (`resizew`) | not supported | |
| `respawn-pane` (`respawnp`) | supported with differences | no `-E` |
| `respawn-window` (`respawnw`) | not supported | |
| `rotate-window` (`rotatew`) | supported | |
| `run-shell` (`run`) | supported with differences | no arguments after the shell command; it runs with `/bin/sh`; output of a command run from a key is shown as a message (one line) or in view mode |
| `save-buffer` (`saveb`) | supported | `-` writes to standard output |
| `select-layout` (`selectl`) | supported | |
| `select-pane` (`selectp`) | supported with differences | no `-d`, `-e` (input on/off), `-m`, `-M` (marked pane); `-T` takes the title literally (no format expansion) |
| `select-window` (`selectw`) | supported | |
| `send-keys` (`send`) | supported with differences | no `-c`, `-K`, `-M` |
| `send-prefix` | supported | |
| `server-access` | not supported | |
| `set-buffer` (`setb`) | supported with differences | no `-t`, `-w` |
| `set-environment` (`setenv`) | supported | see [Scripting](#scripting) for what the environment holds |
| `set-hook` | supported with differences | no `-B`, `-E`, `-T`; not every hook runs (see [Scripting](#scripting)) |
| `set-option` (`set`) | supported with differences | only the options listed below; no `-U`. `-p` sets window options per pane, as in tmux; the ones a pane's value changes are `remain-on-exit`, `allow-passthrough`, `window-style`, `window-active-style`, `pane-border-format` and the border styles of its `pane-border-status` line (border lines themselves keep the window's style) |
| `set-window-option` (`setw`) | supported | |
| `show-buffer` (`showb`) | supported | |
| `show-environment` (`showenv`) | supported | lists what `set-environment` set, not the whole environment |
| `show-hooks` | supported with differences | no `-B`, `-F` |
| `show-messages` (`showmsgs`) | supported | |
| `show-options` (`show`) | supported with differences | no `-A`, `-F`, `-H` |
| `show-prompt-history` (`showphist`) | not supported | |
| `show-window-options` (`showw`) | supported with differences | no `-F` |
| `source-file` (`source`) | supported with differences | reads lif TOML files only; a `.conf` file or a file of tmux commands is refused with a pointer to `lif migrate-tmux-conf` |
| `split-window` (`splitw`) | supported with differences | has `-b -c -d -e -F -h -l -p -P -t -v -Z`; no `-f`, `-I`, `-k`, `-m`, `-W`, `-B`, `-R`, `-s`, `-S`, `-T` |
| `start-server` (`start`) | supported | |
| `suspend-client` (`suspendc`) | supported | |
| `swap-pane` (`swapp`) | supported | |
| `swap-window` (`swapw`) | supported | |
| `switch-client` (`switchc`) | supported with differences | no `-E`, `-O`, `-r` |
| `switch-mode` | not supported | |
| `unbind-key` (`unbind`) | supported | |
| `unlink-window` (`unlinkw`) | not supported | |
| `wait-for` (`wait`) | supported with differences | has `-L -S -U`; no `-E`, `-l`, `-v`, `-F`, `-w` |

The commands `check-config`, `print-config` and `migrate-tmux-conf`
are lif's own; they run without a server (see [configuration.md](configuration.md)).
So is `install-tmux-shim [dir]` (see [claude-code.md](claude-code.md)).

### Flags accepted without effect

These parse and change nothing, because what they switch is how lif always
behaves (or something lif does not have):

- `confirm-before -b`: lif never makes the command that opened a prompt wait
  for the answer (tmux waits unless `-b` is given).
- `command-prompt -P`: tmux opens the prompt inside the pane; lif always uses
  the status line.
- `command-prompt -T`: the prompt type chooses Tab completions in tmux; lif's
  prompt has no completion.
- `lif -2`, `lif -u`: 256 colors and UTF-8 are always on.
- `clear-history -H`: in tmux it also removes the pane's hyperlinks; lif
  clears the history with or without it.

`display-panes -b` is not a tmux 3.8 flag and fails as unknown, as in tmux.

## Default prefix key table

Prefix `C-b` in both. 62 bindings are identical, 7 differ (6 only in
form, plus `?`), and 24 tmux bindings have no lif equivalent (they use
commands or flags lif lacks). Mouse
bindings are not in lif's tables at all (see [Mouse](#mouse)). Long tmux
commands are cut at 70 characters.

| Key | tmux 3.8 | lif 0.2.0 | |
|---|---|---|---|
| `Tab` | `new-pane -E -X 0 -Y 0 -x "75%" -y "30%" \; move-pane -P bottom-cent...` |  | missing in lif |
| `Space` | `next-layout` | `next-layout` | same |
| `!` | `break-pane` | `break-pane` | same |
| `"` | `split-window` | `split-window` | same |
| `#` | `list-buffers` | `list-buffers` | same |
| `$` | `command-prompt -I "#S" { rename-session "%%" }` | `command-prompt -I "#S" { rename-session "%%" }` | same |
| `%` | `split-window -h` | `split-window -h` | same |
| `&` | `confirm-before -p "kill-window #W? (y/n)" kill-window` | `confirm-before -p "kill-window #W? (y/n)" kill-window` | same |
| `'` | `command-prompt -p index { select-window -t ":%%" }` | `command-prompt -p index { select-window -t ":%%" }` | same |
| `(` | `switch-client -p` | `switch-client -p` | same |
| `)` | `switch-client -n` | `switch-client -n` | same |
| `*` | `new-pane` |  | missing in lif |
| `,` | `command-prompt -I "#W" { rename-window "%%" }` | `command-prompt -I "#W" { rename-window "%%" }` | same |
| `-` | `delete-buffer` | `delete-buffer` | same |
| `.` | `command-prompt { move-window -t "%%" }` | `command-prompt { move-window -t "%%" }` | same |
| `/` | `command-prompt -k -p key { list-keys -1N "%%" }` |  | missing in lif |
| `0` | `select-window -t :=0` | `select-window -t :=0` | same |
| `1` | `select-window -t :=1` | `select-window -t :=1` | same |
| `2` | `select-window -t :=2` | `select-window -t :=2` | same |
| `3` | `select-window -t :=3` | `select-window -t :=3` | same |
| `4` | `select-window -t :=4` | `select-window -t :=4` | same |
| `5` | `select-window -t :=5` | `select-window -t :=5` | same |
| `6` | `select-window -t :=6` | `select-window -t :=6` | same |
| `7` | `select-window -t :=7` | `select-window -t :=7` | same |
| `8` | `select-window -t :=8` | `select-window -t :=8` | same |
| `9` | `select-window -t :=9` | `select-window -t :=9` | same |
| `:` | `command-prompt` | `command-prompt` | same |
| `;` | `last-pane` | `last-pane` | same |
| `<` | `display-menu -T "#[align=centre]#{window_index}:#{window_name}" -x ...` |  | missing in lif |
| `=` | `choose-buffer -Z` |  | missing in lif |
| `>` | `display-menu -T "#[align=centre]#{pane_index} (#{pane_id})" -x P -y...` |  | missing in lif |
| `?` | `list-keys -N` | `list-keys` | different |
| `@` | `if-shell -F "#{pane_floating_flag}" { join-pane } { break-pane -W }` |  | missing in lif |
| `C` | `customize-mode -Z` |  | missing in lif |
| `D` | `choose-client -Z` |  | missing in lif |
| `E` | `select-layout -E` | `select-layout -E` | same |
| `L` | `switch-client -l` | `switch-client -l` | same |
| `M` | `select-pane -M` |  | missing in lif |
| `T` | `command-prompt -I "#T" { select-pane -T "%%" }` | `command-prompt -I "#T" { select-pane -T "%%" }` | same |
| `[` | `copy-mode` | `copy-mode` | same |
| `]` | `paste-buffer -p` | `paste-buffer -p` | same |
| `c` | `new-window` | `new-window` | same |
| `d` | `detach-client` | `detach-client` | same |
| `f` | `command-prompt { find-window -Z "%%" }` |  | missing in lif |
| `g` | `switch-client -T move` |  | missing in lif |
| `i` | `display-message` | `display-message` | same |
| `l` | `last-window` | `last-window` | same |
| `m` | `select-pane -m` |  | missing in lif |
| `n` | `next-window` | `next-window` | same |
| `o` | `select-pane -t :.+` | `select-pane -t :.+` | same |
| `p` | `previous-window` | `previous-window` | same |
| `q` | `display-panes` | `display-panes` | same |
| `r` | `refresh-client` | `refresh-client` | same |
| `s` | `choose-tree -Zs` |  | missing in lif |
| `t` | `clock-mode` |  | missing in lif |
| `w` | `choose-tree -Zw` |  | missing in lif |
| `x` | `confirm-before -p "kill-pane #P? (y/n)" kill-pane` | `confirm-before -p "kill-pane #P? (y/n)" kill-pane` | same |
| `z` | `resize-pane -Z` | `resize-pane -Z` | same |
| `{` | `swap-pane -U` | `swap-pane -U` | same |
| `}` | `swap-pane -D` | `swap-pane -D` | same |
| `~` | `show-messages` |  | missing in lif |
| `DC` | `[repeat] refresh-client -c` |  | missing in lif |
| `PPage` | `copy-mode -u` | `copy-mode -u` | same |
| `BTab` | `new-pane -E -X 0 -Y 0 -x "75%" -y "30%" \; move-pane -P bottom-cent...` |  | missing in lif |
| `Up` | `[repeat] select-pane -U` | `[repeat] select-pane -U` | same |
| `Down` | `[repeat] select-pane -D` | `[repeat] select-pane -D` | same |
| `Left` | `[repeat] select-pane -L` | `[repeat] select-pane -L` | same |
| `Right` | `[repeat] select-pane -R` | `[repeat] select-pane -R` | same |
| `M-1` | `select-layout even-horizontal` | `select-layout even-horizontal` | same |
| `M-2` | `select-layout even-vertical` | `select-layout even-vertical` | same |
| `M-3` | `select-layout main-horizontal` | `select-layout main-horizontal` | same |
| `M-4` | `select-layout main-vertical` | `select-layout main-vertical` | same |
| `M-5` | `select-layout tiled` | `select-layout tiled` | same |
| `M-6` | `select-layout main-horizontal-mirrored` | `select-layout main-horizontal-mirrored` | same |
| `M-7` | `select-layout main-vertical-mirrored` | `select-layout main-vertical-mirrored` | same |
| `M-n` | `next-window -a` | `next-window -a` | same |
| `M-o` | `rotate-window -D` | `rotate-window -D` | same |
| `M-p` | `previous-window -a` | `previous-window -a` | same |
| `M-Up` | `[repeat] if-shell -F "#{?floating_pane_flag}" { resize-pane -D -5 }...` | `[repeat] resize-pane -U 5` | different |
| `M-Down` | `[repeat] resize-pane -D 5` | `[repeat] resize-pane -D 5` | same |
| `M-Left` | `[repeat] if-shell -F "#{?floating_pane_flag}" { resize-pane -R -5 }...` | `[repeat] resize-pane -L 5` | different |
| `M-Right` | `[repeat] resize-pane -R 5` | `[repeat] resize-pane -R 5` | same |
| `C-b` | `send-prefix` | `send-prefix` | same |
| `C-o` | `rotate-window` | `rotate-window` | same |
| `C-z` | `suspend-client` | `suspend-client` | same |
| `C-Up` | `[repeat] if-shell -F "#{?floating_pane_flag}" { resize-pane -D -1 }...` | `[repeat] resize-pane -U` | different |
| `C-Down` | `[repeat] resize-pane -D --` | `[repeat] resize-pane -D` | different |
| `C-Left` | `[repeat] if-shell -F "#{?floating_pane_flag}" { resize-pane -R -1 }...` | `[repeat] resize-pane -L` | different |
| `C-Right` | `[repeat] resize-pane -R --` | `[repeat] resize-pane -R` | different |
| `S-Up` | `[repeat] refresh-client -U 10` |  | missing in lif |
| `S-Down` | `[repeat] refresh-client -D 10` |  | missing in lif |
| `S-Left` | `[repeat] refresh-client -L 10` |  | missing in lif |
| `S-Right` | `[repeat] refresh-client -R 10` |  | missing in lif |

The differing `C-`/`M-` arrow bindings do the same thing for ordinary panes;
tmux's versions only add a case for floating panes, which lif does not have.
`C-b ?` in lif shows every binding of every table (`list-keys`) in a
scrollable view; tmux shows the annotated summary (`list-keys -N`).

The root table is empty by default in both apart from tmux's mouse bindings.
The copy-mode tables are compared in [copy-mode.md](copy-mode.md#differences-from-tmux).

## Options

lif has 71 options: 68 with tmux's names, scopes and (except for colors that
tmux 3.8 expresses with theme colors, `pane-border-format`, and
`escape-time` and `history-limit`) tmux 3.8's defaults, plus three of its
own, `scrollback-bytes`, `mouse-drag-end` and `tmux-compat`. `set-option`, `set-window-option`, `show-options`
and `show-window-options` work on them as in tmux, including `-g`, `-s`,
`-w`, `-t`, `-u`, `-a`, `-q`, `-F` and toggling a flag by leaving
out the value. User options (`@name`) can be set and read in formats.
Setting any other tmux option fails with `invalid option: NAME` (`set -q`
silences it).

| Option | Scope | Values | Default | config.toml key |
|---|---|---|---|---|
| `allow-passthrough` | window | one of: off, on, all | `off` | `allow-passthrough` |
| `automatic-rename` | window | on/off | `on` | `automatic-rename` |
| `automatic-rename-format` | window | string | `#{?pane_in_mode,[lif],#{pane_current_command}}#{?pane_dead,[dead],}` | `automatic-rename-format` |
| `base-index` | session | number | `0` | `base-index` |
| `bell-action` | session | one of: none, any, current, other | `any` | `bell-action` |
| `buffer-limit` | server | number | `50` | `buffer-limit` |
| `copy-command` | server | string | (empty) | `copy-command` |
| `copy-mode-current-match-style` | window | string | `bg=magenta,fg=black` | `[theme] current-match` |
| `copy-mode-match-style` | window | string | `bg=cyan,fg=black` | `[theme] match` |
| `default-command` | session | string | (empty) | `default-command` |
| `default-shell` | session | string | `$SHELL` of the server, else `/bin/sh` | `default-shell` |
| `default-size` | session | WxH | `80x24` | `default-size` |
| `detach-on-destroy` | session | one of: off, on, no-detached, previous, next | `on` | `detach-on-destroy` |
| `display-panes-time` | window | number | `1000` | `display-panes-time` |
| `display-time` | session | number | `750` | `display-time` |
| `escape-time` | server | number (ms) | `5` (tmux: `10`) | `escape-time` |
| `exit-empty` | server | on/off | `on` | `exit-empty` |
| `history-limit` | session | number (lines) | `4000` (tmux: `2000`) | `history-limit` |
| `main-pane-height` | window | cells or N% | `24` | `main-pane-height` |
| `main-pane-width` | window | cells or N% | `80` | `main-pane-width` |
| `message-style` | session | string | `bg=yellow,fg=black` | `[theme] message` |
| `mode-keys` | window | one of: emacs, vi | `emacs` (`vi` if `$VISUAL` or `$EDITOR` contains `vi`) | `mode-keys` |
| `mode-style` | window | string | `bg=yellow,fg=black` | `[theme] selection` |
| `monitor-activity` | window | on/off | `off` | `monitor-activity` |
| `monitor-bell` | window | on/off | `on` | `monitor-bell` |
| `mouse` | session | on/off | `on` | `mouse` |
| `mouse-drag-end` (lif only) | session | one of: copy-pipe-and-cancel, copy-pipe-no-clear, none | `copy-pipe-and-cancel` | `mouse-drag-end` |
| `other-pane-height` | window | cells or N% | `0` | `other-pane-height` |
| `other-pane-width` | window | cells or N% | `0` | `other-pane-width` |
| `pane-active-border-style` | window | string | `fg=green` | `[theme] active-border` |
| `pane-base-index` | window | number | `0` | `pane-base-index` |
| `pane-border-format` | window | string | `#{?pane_active,#[reverse],}#{pane_index}#[default] "#{pane_title}"` (tmux adds mouse buttons) | `pane-border-format` |
| `pane-border-status` | window | one of: off, top, bottom | `off` | `pane-border-status` |
| `pane-border-style` | window | string | `default` | `[theme] border` |
| `prefix` | session | key | `C-b` | `prefix` |
| `prefix2` | session | key | `None` | `prefix2` |
| `remain-on-exit` | window | one of: off, on, failed | `off` | `remain-on-exit` |
| `renumber-windows` | session | on/off | `off` | `renumber-windows` |
| `repeat-time` | session | number | `500` | `repeat-time` |
| `scrollback-bytes` (lif only) | session | number (bytes) | `10485760` | `scrollback` |
| `set-clipboard` | server | one of: off, external, on | `external` | `set-clipboard` |
| `set-titles` | session | on/off | `off` | `set-titles` |
| `set-titles-string` | session | string | `#S:#I:#W - "#T"` | `set-titles-string` |
| `status` | session | one of: off, on | `on` | `[status] enabled` |
| `status-interval` | session | number | `15` | `[status] interval` |
| `status-justify` | session | one of: left, centre, right, absolute-centre | `left` | `[status] justify` |
| `status-left` | session | string | `[#{session_name}] ` | `[status] left` |
| `status-left-length` | session | number | `10` | `[status] left-length` |
| `status-left-style` | session | string | `default` | `[theme] status-left` |
| `status-position` | session | one of: top, bottom | `bottom` | `[status] position` |
| `status-right` | session | string | `#{?window_bigger,[#{window_offset_x}#,#{window_offset_y}] ,}"#{=21:pane_title}" %H:%M %d-%b-%y` | `[status] right` |
| `status-right-length` | session | number | `40` | `[status] right-length` |
| `status-right-style` | session | string | `default` | `[theme] status-right` |
| `status-style` | session | string | `bg=green,fg=black` | `[theme] status` |
| `synchronize-panes` | window | on/off | `off` | `synchronize-panes` |
| `tiled-layout-max-columns` | window | number | `0` | `tiled-layout-max-columns` |
| `tmux-compat` (lif only) | server | on/off | `off` | `tmux-compat` |
| `visual-bell` | session | one of: off, on, both | `off` | `visual-bell` |
| `window-active-style` | window | string | `default` | `[theme] active-pane` |
| `window-size` | window | one of: largest, smallest, manual, latest | `latest` | `window-size` |
| `window-status-activity-style` | window | string | `reverse` | `[theme] activity` |
| `window-status-bell-style` | window | string | `reverse` | `[theme] bell` |
| `window-status-current-format` | window | string | `#I:#W#{?window_flags,#{window_flags}, }` | `[status] current-window-format` |
| `window-status-current-style` | window | string | `underscore` | `[theme] current-window` |
| `window-status-format` | window | string | `#I:#W#{?window_flags,#{window_flags}, }` | `[status] window-format` |
| `window-status-last-style` | window | string | `default` | `[theme] last-window` |
| `window-status-separator` | window | string | ` ` | `[status] separator` |
| `window-status-style` | window | string | `default` | `[theme] window` |
| `window-style` | window | string | `default` | `[theme] pane` |
| `word-separators` | session | string | `` !"#$%&'()*+,-./:;<=>?@[\]^`{\|}~ `` | `word-separators` |
| `wrap-search` | window | on/off | `on` | `wrap-search` |

Some tmux options people often set and lif does not have: `default-terminal`,
`terminal-overrides`, `terminal-features`, `focus-events`,
`extended-keys`, `update-environment`,
`status-keys`, `aggressive-resize`, `allow-rename`, `message-command-style`,
`clock-mode-*`, `lock-*`. The behavior several of these exist to turn on
(true color, focus events, extended keys, OSC 52) is on or automatic in lif.

`allow-passthrough` unwraps tmux's `ESC P tmux; ... ESC \` wrapper and hands
the wrapped sequence to the pane's terminal, so what lif passes on anyway
(OSC 52, notifications) reaches the outer terminal; other wrapped sequences
are not written to it raw. `all` is the same as `on`. `pane-border-status`
takes one row from the window, as in tmux, and draws each pane's
`pane-border-format` on its border, two cells in from each end.

## Format variables

Formats are expanded wherever tmux expands them (`-F`, `display-message`,
`new-window -n`, `split-window -c`, the status line, `command-prompt -I`).
Supported syntax: `#{name}`, `#S #I #W #P #T #D #F #H #h ##`,
`#{?cond,a,b}` (with several conditions), `#{==:a,b}` `!=` `<` `>` `<=`
`>=` `||` `&&` `#{!:x}`, `#{m:glob,s}`, `#{e|op|:a,b}` (`+ - * / % ==`...),
and the modifiers `=N`, `=-N`, `=/N/marker`, `pN`, `t:`, `b:`, `d:`,
`l:`, `E:`, `T:`, `q:`, `n:`, `w:`, `s/from/to/:` (literal, not a regex).
`#[style]` is passed to the status line. `#(command)` expands to the last
line the shell command printed when it last ran (nothing the first time);
see [Scripting](#scripting).
An unknown variable expands to an empty string. As in tmux, a name that is not
a variable is looked up as an option (`#{status-left}`, `#{@myopt}`).

| Group | Variables |
|---|---|
| Server | `host`, `host_short`, `pid`, `socket_path`, `version` |
| Session | `session_name`, `session_id`, `session_windows`, `session_attached`, `session_many_attached`, `session_created`, `session_activity` (same value as `session_created`), `session_path`, `session_alerts`, `session_format`; `session_grouped` is always 0 and `session_group` empty |
| Window | `window_index`, `window_name`, `window_id`, `window_flags`, `window_raw_flags`, `window_active`, `window_last_flag`, `window_bell_flag`, `window_activity_flag`, `window_zoomed_flag`, `window_panes`, `window_width`, `window_height`, `window_layout`, `window_visible_layout`, `window_start_flag`, `window_end_flag`, `window_format`; `window_silence_flag`, `window_bigger`, `window_marked_flag`, `window_linked` are always 0 and `window_offset_x/y` 0 |
| Pane | `pane_index`, `pane_id`, `pane_title` (the host name until a program sets a title), `pane_current_command`, `pane_current_path`, `pane_width`, `pane_height`, `pane_active`, `pane_last`, `pane_pid`, `pane_dead`, `pane_dead_status`, `pane_in_mode`, `pane_start_command`, `pane_start_path`, `pane_left`, `pane_top`, `pane_right`, `pane_bottom`, `pane_at_left`, `pane_at_top`, `pane_at_right`, `pane_at_bottom`, `pane_synchronized`, `pane_format`; `pane_marked`, `pane_marked_set`, `pane_input_off`, `pane_pipe` are always 0 |
| History | `history_size` (lines), `history_bytes` (lif only: bytes used), `history_limit` |
| Client | `client_name`, `client_tty` (same as the name, e.g. `client-58982`), `client_termname`, `client_width`, `client_height`, `client_pid`, `client_session`, `client_last_session`, `client_prefix`, `client_key_table`, `client_flags`, `client_cwd` |
| Buffer | `buffer_name`, `buffer_size`, `buffer_sample` (in `list-buffers -F`) |
| Hook | `hook`, `hook_session`, `hook_session_name`, `hook_window`, `hook_window_name`, `hook_pane`, `hook_client` (in the commands of a hook) |

Not available, among others: `pane_tty`, `pane_mode`, `alternate_on`,
`cursor_x`/`cursor_y`, `mouse_*`, and the copy-mode variables
(`copy_cursor_x`, `copy_cursor_y`, `copy_cursor_word`, `scroll_position`,
`selection_present`, `search_present`, `pane_search_string`) in
`display-message`. The copy-mode key bindings that use `copy_cursor_word` and
`pane_search_string` are handled inside copy mode and work.

## Scripting

`run-shell`, `if-shell`, `wait-for`, `#()`, hooks and the environment
commands print what tmux 3.8 prints and exit as it does:

```
$ lif run-shell 'echo hi; exit 3'; echo "status $?"
hi
'echo hi; exit 3' returned 3
status 3
$ lif if-shell 'test -d /tmp' 'display -p yes' 'display -p no'
yes
$ lif wait-for -U nolock
channel nolock not locked
```

What is the same: a list waits for `run-shell` and `if-shell` (`run-shell
'sleep 1' \; display done` shows `done` after a second) unless `-b` is
given; the shell command is a format (`run-shell 'echo #{session_name}'`);
it runs in the directory of the client that ran it from a shell, else the
session's, or `-c`'s; `-d` delays in seconds with a fraction; `-C` runs lif
commands; `if-shell -F` tests a format; only the branch taken is parsed;
`wait-for -S` with nobody waiting is remembered for the next `wait-for`;
a hook's commands print as tmux parsed them (`after-new-window[0]
display-message -p hi`) and unknown hook names are `invalid option: NAME`.

What differs:

- **`/bin/sh`.** Shell commands run as `/bin/sh -c command`, as the
  server's user, with standard input from `/dev/null`. tmux uses
  `default-shell`. Their environment is the server's with
  `set-environment`'s variables, `LIF` (so `lif` inside the command
  talks to this server) and `LIF_PANE` for the target pane; `TMUX` and
  `TMUX_PANE` too when `tmux-compat` is on.
- **Limits.** At most 64 shell commands run at once (one more fails with
  `64 shell commands are still running; not running COMMAND`), 1 MiB of a
  command's output is kept, and at most 1024 command lists wait at a time.
  Commands still running when the server exits are hung up.
- **Output of commands run from keys.** One line is shown as a message;
  more open view mode in the active pane (as other commands' output does
  in lif). Output of `run-shell -b` from a shell is dropped.
- **`#()`.** Expands to the last line of the command's previous run, as in
  tmux, and does so in every format (tmux 3.8's `display-message -p
  '#(echo hi)'` printed an empty line each time). The command is run again when a format using it is expanded and
  its last run started `status-interval` seconds ago or more (at least
  one; tmux: one second). A status line with `#()` is redrawn every
  `status-interval`; lif remembers the 64 commands used most recently.
- **Hooks.** Every tmux 3.8 hook name can be set, globally (`-g`), on a
  session, or for window hooks on a window or a pane (`-p`). The ones lif
  runs are the `after-` hooks of the commands it has, `session-created`,
  `session-closed`, `session-renamed`, `session-window-changed`,
  `window-linked`, `window-unlinked`, `window-renamed` (for
  `rename-window`, not automatic names), `window-layout-changed`,
  `window-pane-changed`, `client-attached`, `client-detached`,
  `client-session-changed`, `client-resized`, `pane-exited`,
  `pane-died`, `alert-bell` and `alert-activity`. The others are kept and
  never run. A hook holds at most 64 commands. Commands run by a hook fire
  no hooks, as in tmux; a hook's error goes to `show-messages`.
- **Environment.** Panes get the server's environment (that of the client
  that started it) plus `SSH_AUTH_SOCK`, `DISPLAY` and a few more from the
  client that creates the session. `set-environment` adds to that or, with
  `-r`, removes from it, for panes and shell commands started afterwards:
  the global set (`-g`), then the session's, then `-e` of `new-session`,
  `new-window`, `split-window` or `respawn-pane`. `show-environment`
  lists only what was set this way (tmux lists everything, with the
  `update-environment` variables). Hidden variables (`-h`) are listed
  with `show-environment -h` and not passed on.
- **`synchronize-panes`.** Typed keys, pastes and `send-keys` go to every
  pane of the window that has the option on and is not in a mode; panes
  hidden by a zoomed pane are left alone. Mouse events go to one pane.
- **`join-pane`.** Without `-s` the source is the current pane (tmux: the
  marked pane, which lif does not have).

## Translating a .tmux.conf

lif never reads `~/.tmux.conf`. `lif migrate-tmux-conf [file]` (default
`~/.tmux.conf`) prints a `config.toml` on standard output and, on standard
error, one line per command it did not translate, with the reason. It
translates `set`/`set-option`/`setw` with `-g` or `-s` for options lif has,
`bind`/`bind-key` with `-n`, `-r`, `-T`, `-N`, `unbind`, a copy-mode table's
`MouseDragEnd1Pane` (as `mouse-drag-end`), `set-hook -g` (with `-a` and
`-u`, as `[hooks]`), `set-environment -g` (with `-r`, as `[env]`), and
comments directly above a translated line. It never guesses. It exits 0 even when
lines were left out.

Input, `tmux.conf`:

```tmux
# remap prefix to Ctrl-a
set -g prefix C-a
unbind C-b
bind C-a send-prefix

set -g mouse on
set -g base-index 1
setw -g pane-base-index 1
set -g history-limit 50000
set -sg escape-time 0
setw -g mode-keys vi
set -g default-terminal "tmux-256color"
set -ga terminal-overrides ",xterm-256color:Tc"

# status line
set -g status-position top
set -g status-style "bg=colour235,fg=colour250"
set -g status-left "#[bold]#S "
set -g status-right "%H:%M"
setw -g window-status-current-style "fg=black,bg=green"

# splits keep the current directory
bind | split-window -h -c "#{pane_current_path}"
bind - split-window -v -c "#{pane_current_path}"
bind -r h select-pane -L
bind -r j select-pane -D
bind -n M-Left select-pane -L
bind r source-file ~/.tmux.conf \; display "reloaded"
bind -T copy-mode-vi v send -X begin-selection
bind -T copy-mode-vi y send -X copy-pipe-and-cancel "pbcopy"
bind -T copy-mode-vi MouseDragEnd1Pane send -X copy-pipe-and-cancel "pbcopy"

set -g @plugin 'tmux-plugins/tpm'
run '~/.tmux/plugins/tpm/tpm'
if-shell "test -f ~/.tmux.local" "source ~/.tmux.local"
set-hook -g after-new-window 'display hi'
```

```
$ lif migrate-tmux-conf tmux.conf > ~/.config/lif/config.toml
tmux.conf:12: not translated (lif has no option default-terminal): set -g default-terminal "tmux-256color"
tmux.conf:13: not translated (set-option -a has no config.toml equivalent): set -ga terminal-overrides ",xterm-256color:Tc"
tmux.conf:28: not translated (source-file of a tmux configuration file: lif reloads its own config.toml (source-file ~/.config/lif/config.toml)): bind r source-file ~/.tmux.conf \; display "reloaded"
tmux.conf:31: not translated (lif pipes mouse copies to the copy-command option, which also serves copy-pipe keys without a command of their own (Enter in vi, M-w in emacs): add copy-command = "pbcopy" if that is what you want): bind -T copy-mode-vi MouseDragEnd1Pane send -X copy-pipe-and-cancel "pbcopy"
tmux.conf:33: not translated (user options (@name) have no config.toml equivalent): set -g @plugin 'tmux-plugins/tpm'
tmux.conf:34: not translated (run-shell at start-up has no config.toml equivalent (lif runs it in key bindings and [hooks])): run '~/.tmux/plugins/tpm/tpm'
tmux.conf:35: not translated (if-shell at start-up has no config.toml equivalent (lif runs it in key bindings and [hooks])): if-shell "test -f ~/.tmux.local" "source ~/.tmux.local"
```

The resulting `config.toml`:

```toml
# lif config.toml, translated from .tmux.conf.

# remap prefix to Ctrl-a
prefix = "C-a"
mouse = true
base-index = 1
pane-base-index = 1
history-limit = 50000
escape-time = 0
mode-keys = "vi"
mouse-drag-end = "copy-pipe-and-cancel"

[status]
# status line
position = "top"
left = "#[bold]#S "
right = "%H:%M"

[theme]
status = "bg=colour235,fg=colour250"
current-window = "fg=black,bg=green"

[keys]
# splits keep the current directory
"|" = 'split-window -h -c "#{pane_current_path}"'
"-" = 'split-window -v -c "#{pane_current_path}"'
h = { command = "select-pane -L", repeat = true }
j = { command = "select-pane -D", repeat = true }

[keys.root]
M-Left = "select-pane -L"

[keys.copy-mode-vi]
v = "send -X begin-selection"
y = "send -X copy-pipe-and-cancel pbcopy"

[hooks]
after-new-window = "display hi"
```

`lif check-config` reports no problems with it. Things to review by hand
after a migration:

- The usual prefix recipe (`unbind C-b`, `bind C-a send-prefix`) is folded
  into `prefix = "C-a"`, which does both.
- `history-limit = 50000` is carried over and enforced, but the byte budget
  `scrollback` (10 MiB by default) holds as well, and at 200 columns 50000
  lines count about 85 MB against it (compressed history still counts at its
  uncompressed size). Raise it too, for example `scrollback = "128MB"`.
- `copy-pipe-and-cancel pbcopy` runs `pbcopy` with the selection, as in
  tmux; the selection also goes to a lif buffer and, through OSC 52, to the
  terminal clipboard.
- The mouse's `MouseDragEnd1Pane` binding becomes `mouse-drag-end`; its
  `pbcopy` is reported because lif pipes mouse copies through
  `copy-command`, which also serves the default copy keys. Add
  `copy-command = "pbcopy"` to pipe every copy.
- Reload bindings: `bind r source-file ~/.config/lif/config.toml`.
- `run-shell` and `if-shell` lines are not translated: the configuration
  file has no commands that run at start-up. Both work in key bindings and
  hooks (`r = "run-shell '~/bin/x'"`, or a `session-created` hook for what
  should happen once a session exists). Plugins that are loaded this way
  (TPM) are not loaded.

## Behavior differences you will notice

- **Config file.** TOML, read once at server start from `$LIF_CONFIG`,
  `$XDG_CONFIG_HOME/lif/config.toml` or `~/.config/lif/config.toml`, or the
  file given with `lif -f`. `source-file` reads the same TOML format; it
  refuses `.conf` files and files of tmux commands with a pointer to
  `lif migrate-tmux-conf`. Errors in the file go to `show-messages`, and the
  rest of the file still applies.
- **Scrollback.** Two limits per pane, and both hold: `history-limit` in lines
  (default 4000, twice tmux's 2000) and `scrollback-bytes` in memory (default
  10 MiB, config key `scrollback`). The byte budget only matters for wide
  panes or long histories: a line of history counts about 1.7 KB at 200
  columns. After a 100 MB `cat` at 200 columns lif kept 3,784 lines (tmux
  1,818). The engine allocates whole pages, so a small budget rounds up: with
  `scrollback-bytes 100000`, `#{history_bytes}` showed 819200. Unlike tmux,
  where a new `history-limit` only applies to panes created afterwards,
  changing either limit also trims existing panes (`set -g history-limit 100`
  left an existing pane with 96 lines of `seq 1 3000`).
- **Environment inside panes.** `TERM=xterm-256color`, `COLORTERM=truecolor`,
  `TERM_PROGRAM=lif`, `TERM_PROGRAM_VERSION`, `LIF` and `LIF_PANE`; no
  `TMUX` or `TMUX_PANE`. Scripts that test `$TMUX` to detect a multiplexer
  need to test `$LIF` too.
- **lif inside a pane.** As tmux uses `$TMUX`, `lif` finds its server through
  `$LIF` (the socket path is its first comma-separated field) and knows which
  pane it runs in (`lif split-window` splits that pane). The socket is chosen
  in this order: `-S`, then `-L`, then `$LIF`, then `$LIF_SOCKET`, then the
  default socket (as tmux: `-S`, `-L`, `$TMUX`, then `$TMUX_TMPDIR` or the
  default). An empty variable counts as unset.
- **Escape key.** `escape-time` works as in tmux but defaults to 5 ms (tmux:
  10): a lone ESC from a terminal using the legacy encoding waits that long for
  the rest of a sequence, and 0 means no wait (a split Alt+key then arrives as
  Escape and the key, as in tmux). With a terminal that speaks the kitty
  keyboard protocol, which lif turns on when the terminal answers, Escape is
  unambiguous and never waits, whatever `escape-time` says. Measured (one
  run): 6.3 ms median with the defaults against tmux's 11.7 ms; 157 us with
  `escape-time 0` (tmux 1.4 ms); 162 us with the kitty protocol.
- **Clipboard.** `set-clipboard` defaults to `external`. Copies from copy
  mode always become a paste buffer and, unless the option is `off`, reach
  the terminal clipboard (OSC 52). OSC 52 from programs in panes: `on`
  forwards it to the terminal and keeps it as a buffer, `external` forwards
  it without keeping it, `off` ignores it. This `external` differs from
  tmux 3.8's, which ignores the program's OSC 52 entirely (checked: with
  `external`, tmux sent nothing to the terminal); use `off` if programs in
  panes must not set the clipboard. `copy-pipe` runs its command with the
  copied text on standard input (`/bin/sh -c`, in the pane's directory),
  and so does the `copy-command` option for `copy-pipe` without a command,
  as in tmux.
- **Notifications.** OSC 9, OSC 777 and OSC 99 desktop notifications from
  panes are forwarded to the attached terminals as OSC 9.
- **Mouse.** <a id="mouse"></a>On by default (as in tmux 3.8) and handled
  inside the server: click selects a pane, clicking a window in the status line
  selects it, dragging a border resizes, the wheel scrolls (entering copy mode,
  which it leaves at the bottom; in full-screen programs without mouse support
  it sends arrow keys), dragging in a pane selects text and copies it on
  release (then leaves copy mode, as tmux), double and triple click copy a
  word or a line, and programs that request the mouse get it. Differences:
  mouse events cannot be bound (`bind -n MouseDown1Pane ...` fails with
  `unknown key: MouseDown1Pane`); what tmux users change by rebinding
  `MouseDragEnd1Pane` is the `mouse-drag-end` option (`copy-pipe-no-clear`
  stays in copy mode with the selection shown, `none` only selects); after a
  double or triple click lif copies at once, where tmux shows the selection
  for 0.3 s first; there are no right-click menus.
- **Shell commands.** `run-shell`, `if-shell` and `#()` work; they run
  with `/bin/sh` rather than `default-shell`, and `#()` in the status line
  is refreshed every `status-interval` rather than every second. See
  [Scripting](#scripting).
- **Status line styles.** `fg=`, `bg=`, `us=` and attributes work;
  `align=`, `fill=`, `list=`, `range=` and theme colors (`themegreen`)
  are not supported.
- **`list-commands`** prints names and aliases, not usage lines; with `-F`,
  `#{command_list_usage}` is built from lif's flag table.
- **Prefix `?`** shows the complete `list-keys` output of all tables.
