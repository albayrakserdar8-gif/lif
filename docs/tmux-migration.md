# Moving from tmux to lif

This guide is for someone who uses tmux today. Everything in it was checked
against lif 0.5.0's command table and by running the commands; the tmux side is
tmux 3.8 (`tmux list-commands`, `tmux list-keys` with an empty configuration).

In short: the commands, flags, targets, formats and default keys you use every
day are the same. The configuration file is not: lif reads TOML, and
`lif migrate-tmux-conf` translates the common part of a `.tmux.conf`. Shell
commands (`run-shell`, `if-shell`, `#()`), `wait-for` and hooks work as in
tmux, with the differences listed under [Scripting](#scripting).

## Commands

90 of tmux 3.8's 92 commands exist in lif (38 with the same flags, 52 with some
difference, usually missing flags); 2 do not. Command names, aliases and unambiguous prefixes
resolve as in tmux (`lif splitw`, `lif new`, `lif a`). An unimplemented
command fails with exit status 1:

```
$ lif switch-mode
unsupported command: switch-mode
```

A flag lif does not know fails the same way (`command split-window: unknown flag -f`).
A few tmux flags that lif cannot honor fail with an explicit `not supported`
error instead (`new-session -E`). The flags that are
accepted without changing anything are listed in
[Flags accepted without effect](#flags-accepted-without-effect).

| tmux command | lif | Differences |
|---|---|---|
| `attach-session` (`attach`) | supported with differences | no `-f`, `-r` |
| `bind-key` (`bind`) | supported with differences | mouse keys are bound in the root table; in the copy-mode tables they are accepted and never used (see [Mouse](#mouse)) |
| `break-pane` (`breakp`) | supported with differences | no `-W`, `-x`, `-y`, `-X`, `-Y` (floating panes) |
| `capture-pane` (`capturep`) | supported with differences | has `-b -e -E -J -N -p -q -S -t`; no `-a`, `-C`, `-F`, `-H`, `-I`, `-L`, `-M`, `-P`, `-R`, `-T` |
| `choose-buffer` | supported with differences | has `-F -f -N -O -r -t -Z`; no `-k`, `-K`; no `e` (edit) key |
| `choose-client` | supported with differences | has `-F -f -N -O -r -t -Z`; no `-h`, `-i`, `-k`, `-K`; the preview shows the client's active pane |
| `choose-tree` | supported with differences | has `-F -f -N -O -r -s -t -w -Z`; no `-G`, `-h`, `-k`, `-K`; see [Chooser modes](#chooser-modes) |
| `clear-history` (`clearhist`) | supported | `-H` changes nothing (see below) |
| `clear-prompt-history` (`clearphist`) | supported | lif keeps one history, the command prompt's |
| `clock-mode` | supported | |
| `command-prompt` | supported with differences | all of tmux's flags; `-C`, `-P`, `-T` change nothing (see below); without `-b` the commands after it wait for the prompt, as in tmux |
| `confirm-before` (`confirm`) | supported | without `-b` the commands after it wait for the answer, as in tmux |
| `copy-mode` | supported with differences | no `-k`, `-s`, `-S` |
| `customize-mode` | supported with differences | tmux's headings, keys and editing (Enter, `d`, `u`); the preview shows the scope and value of an option but not tmux's description of it; `-F` and `-f` change nothing |
| `delete-buffer` (`deleteb`) | supported | |
| `detach-client` (`detach`) | supported with differences | no `-E` |
| `display-menu` (`menu`) | supported with differences | has `-b -c -C -H -M -O -s -S -t -T -x -y`; positions `M` and `W` are the centre unless a mouse key opened the menu; `#[...]` styles in names and titles are dropped; see [Popups and menus](#popups-and-menus) |
| `display-message` (`display`) | supported with differences | has `-c -d -F -l -p -t`; no `-a`, `-C`, `-I`, `-j`, `-N`, `-v` |
| `display-panes` (`displayp`) | supported with differences | no `-k`, `-s`, `-Z` |
| `display-popup` (`popup`) | supported with differences | has `-B -b -C -c -d -e -E -h -k -s -S -t -T -w -x -y`; no `-N`; see [Popups and menus](#popups-and-menus) |
| `find-window` (`findw`) | supported | `-r` takes the regular expressions of copy-mode search (see [copy-mode.md](copy-mode.md)) |
| `has-session` (`has`) | supported | |
| `if-shell` (`if`) | supported | the shell command runs with `/bin/sh` |
| `join-pane` (`joinp`) | supported with differences | `-p` percentage is also accepted |
| `kill-pane` (`killp`) | supported with differences | no `-f` (filter) |
| `kill-server` | supported | |
| `kill-session` | supported with differences | no `-f`, `-g` |
| `kill-window` (`killw`) | supported with differences | no `-f` |
| `last-pane` (`lastp`) | supported with differences | no `-d`, `-e` |
| `last-window` (`last`) | supported | |
| `link-window` (`linkw`) | supported | the window is the same in both places, as in tmux; see [Linked windows and session groups](#linked-windows-and-session-groups) |
| `list-buffers` (`lsb`) | supported with differences | no `-O` (sort order) |
| `list-clients` (`lsc`) | supported with differences | no `-O` |
| `list-commands` (`lscm`) | supported with differences | prints names and aliases only, no usage strings; with `-F`, `#{command_list_usage}` is built from the flag table (`[-deHMqu] [-t value]`), without tmux's argument names |
| `list-keys` (`lsk`) | supported with differences | no `-a`, `-F`, `-O`, `-r` |
| `list-panes` (`lsp`) | supported with differences | no `-O`, `-r` |
| `list-sessions` (`ls`) | supported with differences | no `-O`, `-r` |
| `list-windows` (`lsw`) | supported with differences | no `-O`, `-r` |
| `load-buffer` (`loadb`) | supported with differences | has `-b`, `-w` and `-` (standard input, at most 16 MiB); `-t` changes nothing |
| `lock-client` (`lockc`) | supported | `lock-command` runs with `/bin/sh` on the client's terminal |
| `lock-server` (`lock`) | supported | |
| `lock-session` (`locks`) | supported | |
| `move-pane` (`movep`) | supported with differences | as `join-pane`, or with `-D -L -P -R -U -X -Y -z` it moves a floating pane; no `-M` (see [Floating panes](#floating-panes)) |
| `move-window` (`movew`) | supported | |
| `new-pane` (`newp`) | supported with differences | has `-b -c -d -e -F -L -P -t -T -x -X -y -Y` and the command; `-A -C -D -I -K -M -O -W` fail with `not supported`; `-B -m -R -s -S -k -l -p -f -h -v -Z` change nothing without `-L` (see [Floating panes](#floating-panes)) |
| `new-session` (`new`) | supported with differences | no `-f` |
| `new-window` (`neww`) | supported with differences | no `-E` |
| `next-layout` (`nextl`) | supported | |
| `next-window` (`next`) | supported | |
| `paste-buffer` (`pasteb`) | supported with differences | no `-S` |
| `pipe-pane` (`pipep`) | supported with differences | all of tmux's flags; the command runs with `/bin/sh`; at most 1 MiB waits for a command that reads slowly, the rest is not piped; with `-I` the command's output is read only while the pane's input queue (1 MiB) has room |
| `previous-layout` (`prevl`) | supported | |
| `previous-window` (`prev`) | supported | |
| `refresh-client` (`refresh`) | supported with differences | `-C` (a control client's size), `-S`, `-t`; no `-A`, `-B`, `-f` (control-mode subscriptions and flags) or pan flags |
| `rename-session` (`rename`) | supported | |
| `rename-window` (`renamew`) | supported | |
| `resize-pane` (`resizep`) | supported with differences | no `-T` |
| `resize-window` (`resizew`) | supported | |
| `respawn-pane` (`respawnp`) | supported with differences | no `-E` |
| `respawn-window` (`respawnw`) | supported with differences | no `-E` |
| `rotate-window` (`rotatew`) | supported | |
| `run-shell` (`run`) | supported with differences | no arguments after the shell command; it runs with `/bin/sh`; output of a command run from a key is shown as a message (one line) or in view mode |
| `save-buffer` (`saveb`) | supported | `-` writes to standard output |
| `select-layout` (`selectl`) | supported | |
| `select-pane` (`selectp`) | supported with differences | `-d` stops what clients type and paste (not `send-keys`); `-T` takes the title literally (no format expansion) |
| `select-window` (`selectw`) | supported | |
| `send-keys` (`send`) | supported with differences | no `-c`, `-K` |
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
| `show-prompt-history` (`showphist`) | supported | only the command prompt has a history; the other types are empty |
| `show-window-options` (`showw`) | supported with differences | no `-F` |
| `source-file` (`source`) | supported with differences | reads lif TOML files only; a `.conf` file or a file of tmux commands is refused with a pointer to `lif migrate-tmux-conf` |
| `split-window` (`splitw`) | supported with differences | has `-b -c -d -e -f -F -h -l -p -P -t -v -Z`; no `-I`, `-k`, `-m`, `-W`, `-B`, `-R`, `-s`, `-S`, `-T` |
| `start-server` (`start`) | supported | |
| `suspend-client` (`suspendc`) | supported | |
| `swap-pane` (`swapp`) | supported | |
| `swap-window` (`swapw`) | supported | |
| `switch-client` (`switchc`) | supported with differences | no `-O`, `-r` |
| `switch-mode` | not supported | |
| `unbind-key` (`unbind`) | supported | |
| `unlink-window` (`unlinkw`) | supported | |
| `wait-for` (`wait`) | supported with differences | has `-L -S -U`; no `-E`, `-l`, `-v`, `-F`, `-w` |

The commands `check-config`, `print-config` and `migrate-tmux-conf`
are lif's own; they run without a server (see [configuration.md](configuration.md)).
So is `install-tmux-shim [dir]` (see [claude-code.md](claude-code.md)).

### Flags accepted without effect

These parse and change nothing, because what they switch is how lif always
behaves (or something lif does not have):

- `command-prompt -C`: tmux keeps updating panes while the prompt is up
  only with it; lif always does.
- `command-prompt -P`: tmux opens the prompt inside the pane; lif always uses
  the status line.
- `command-prompt -T`: the prompt type chooses Tab completions in tmux; lif's
  prompt has no completion.
- `lif -2`, `lif -u`: 256 colors and UTF-8 are always on.
- `clear-history -H`: in tmux it also removes the pane's hyperlinks; lif
  clears the history with or without it.

`display-panes -b` is not a tmux 3.8 flag and fails as unknown, as in tmux.

## Default prefix key table

Prefix `C-b` in both. 70 bindings are identical, 12 differ (6 only in
form; `$`, `,` and `T`, whose prompts put the answer back escaped,
`%%%` where tmux has `%%`, so that a name or a pane title holding `"` or
`;` stays one argument and is never run as commands; plus `?` and the two
menus, which leave out what lif lacks, and whose Rename entries use `%%%`
too), and 11
tmux bindings have no lif equivalent (they use
commands or flags lif lacks). The mouse keys of the root table are listed
under [Mouse](#mouse). Long tmux
commands are cut at 70 characters.

| Key | tmux 3.8 | lif 0.5.0 | |
|---|---|---|---|
| `Tab` | `new-pane -E -X 0 -Y 0 -x "75%" -y "30%" \; move-pane -P bottom-cent...` |  | missing in lif |
| `Space` | `next-layout` | `next-layout` | same |
| `!` | `break-pane` | `break-pane` | same |
| `"` | `split-window` | `split-window` | same |
| `#` | `list-buffers` | `list-buffers` | same |
| `$` | `command-prompt -I "#S" { rename-session "%%" }` | `command-prompt -I "#S" { rename-session "%%%" }` | different |
| `%` | `split-window -h` | `split-window -h` | same |
| `&` | `confirm-before -p "kill-window #W? (y/n)" kill-window` | `confirm-before -p "kill-window #W? (y/n)" kill-window` | same |
| `'` | `command-prompt -p index { select-window -t ":%%" }` | `command-prompt -p index { select-window -t ":%%" }` | same |
| `(` | `switch-client -p` | `switch-client -p` | same |
| `)` | `switch-client -n` | `switch-client -n` | same |
| `*` | `new-pane` |  | missing in lif |
| `,` | `command-prompt -I "#W" { rename-window "%%" }` | `command-prompt -I "#W" { rename-window "%%%" }` | different |
| `-` | `delete-buffer` | `delete-buffer` | same |
| `.` | `command-prompt { move-window -t "%%" }` | `command-prompt { move-window -t "%%" }` | same |
| `/` | `command-prompt -k -p key { list-keys -1N "%%" }` | `command-prompt -k -p key { list-keys -1N "%%" }` | same |
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
| `<` | `display-menu -T "#[align=centre]#{window_index}:#{window_name}" -x ...` | `display-menu -T "#[align=centre]#{window_index}:#{window_name}" -x W -y...` | different |
| `=` | `choose-buffer -Z` | `choose-buffer -Z` | same |
| `>` | `display-menu -T "#[align=centre]#{pane_index} (#{pane_id})" -x P -y...` | `display-menu -T "#[align=centre]#{pane_index} (#{pane_id})" -x P -y P...` | different |
| `?` | `list-keys -N` | `list-keys` | different |
| `@` | `if-shell -F "#{pane_floating_flag}" { join-pane } { break-pane -W }` |  | missing in lif |
| `C` | `customize-mode -Z` | `customize-mode -Z` | same |
| `D` | `choose-client -Z` | `choose-client -Z` | same |
| `E` | `select-layout -E` | `select-layout -E` | same |
| `L` | `switch-client -l` | `switch-client -l` | same |
| `M` | `select-pane -M` | `select-pane -M` | same |
| `T` | `command-prompt -I "#T" { select-pane -T "%%" }` | `command-prompt -I "#T" { select-pane -T "%%%" }` | different |
| `[` | `copy-mode` | `copy-mode` | same |
| `]` | `paste-buffer -p` | `paste-buffer -p` | same |
| `c` | `new-window` | `new-window` | same |
| `d` | `detach-client` | `detach-client` | same |
| `f` | `command-prompt { find-window -Z "%%" }` | `command-prompt { find-window -Z "%%" }` | same |
| `g` | `switch-client -T move` |  | missing in lif |
| `i` | `display-message` | `display-message` | same |
| `l` | `last-window` | `last-window` | same |
| `m` | `select-pane -m` | `select-pane -m` | same |
| `n` | `next-window` | `next-window` | same |
| `o` | `select-pane -t :.+` | `select-pane -t :.+` | same |
| `p` | `previous-window` | `previous-window` | same |
| `q` | `display-panes` | `display-panes` | same |
| `r` | `refresh-client` | `refresh-client` | same |
| `s` | `choose-tree -Zs` | `choose-tree -Zs` | same |
| `t` | `clock-mode` | `clock-mode` | same |
| `w` | `choose-tree -Zw` | `choose-tree -Zw` | same |
| `x` | `confirm-before -p "kill-pane #P? (y/n)" kill-pane` | `confirm-before -p "kill-pane #P? (y/n)" kill-pane` | same |
| `z` | `resize-pane -Z` | `resize-pane -Z` | same |
| `{` | `swap-pane -U` | `swap-pane -U` | same |
| `}` | `swap-pane -D` | `swap-pane -D` | same |
| `~` | `show-messages` | `show-messages` | same |
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

lif has 94 options: 91 with tmux's names, scopes and (except for colors that
tmux 3.8 expresses with theme colors, `pane-border-format`, `focus-events`,
and `escape-time` and `history-limit`) tmux 3.8's defaults, plus three of its
own, `scrollback-bytes`, `mouse-drag-end` and `tmux-compat`. `set-option`, `set-window-option`, `show-options`
and `show-window-options` work on them as in tmux, including `-g`, `-s`,
`-w`, `-t`, `-u`, `-a`, `-q`, `-F` and toggling a flag by leaving
out the value. User options (`@name`) can be set and read in formats.
Setting any other tmux option fails with `invalid option: NAME` (`set -q`
silences it).

| Option | Scope | Values | Default | config.toml key |
|---|---|---|---|---|
| `activity-action` | session | one of: none, any, current, other | `other` | `activity-action` |
| `aggressive-resize` | window | on/off | `off` | `aggressive-resize` |
| `allow-passthrough` | window | one of: off, on, all | `off` | `allow-passthrough` |
| `automatic-rename` | window | on/off | `on` | `automatic-rename` |
| `automatic-rename-format` | window | string | `#{?pane_in_mode,[lif],#{pane_current_command}}#{?pane_dead,[dead],}` | `automatic-rename-format` |
| `base-index` | session | number | `0` | `base-index` |
| `bell-action` | session | one of: none, any, current, other | `any` | `bell-action` |
| `buffer-limit` | server | number | `50` | `buffer-limit` |
| `clock-mode-colour` | window | string | `blue` | `clock-mode-colour` |
| `clock-mode-style` | window | one of: 12, 24 | `24` | `clock-mode-style` |
| `copy-command` | server | string | (empty) | `copy-command` |
| `copy-mode-current-match-style` | window | string | `bg=magenta,fg=black` | `[theme] current-match` |
| `copy-mode-match-style` | window | string | `bg=cyan,fg=black` | `[theme] match` |
| `default-command` | session | string | (empty) | `default-command` |
| `default-shell` | session | string | `$SHELL` of the server, else `/bin/sh` | `default-shell` |
| `default-size` | session | WxH | `80x24` | `default-size` |
| `destroy-unattached` | session | one of: off, on, keep-last, keep-group | `off` | `destroy-unattached` |
| `detach-on-destroy` | session | one of: off, on, no-detached, previous, next | `on` | `detach-on-destroy` |
| `display-panes-time` | window | number | `1000` | `display-panes-time` |
| `display-time` | session | number | `750` | `display-time` |
| `escape-time` | server | number (ms) | `5` (tmux: `10`) | `escape-time` |
| `exit-empty` | server | on/off | `on` | `exit-empty` |
| `focus-events` | server | on/off | `on` | `focus-events` |
| `history-limit` | session | number (lines) | `4000` (tmux: `2000`) | `history-limit` |
| `lock-after-time` | session | number (seconds; 0: never) | `0` | `lock-after-time` |
| `lock-command` | session | string | `lock -np` | `lock-command` |
| `main-pane-height` | window | cells or N% | `24` | `main-pane-height` |
| `main-pane-width` | window | cells or N% | `80` | `main-pane-width` |
| `menu-border-lines` | window | one of: single, rounded, double, heavy, simple, padded, none | `single` | `menu-border-lines` |
| `menu-border-style` | window | string | `default` | `[theme] menu-border` |
| `menu-selected-style` | window | string | `bg=yellow,fg=black` | `[theme] menu-selected` |
| `menu-style` | window | string | `default` | `[theme] menu` |
| `message-style` | session | string | `bg=yellow,fg=black` | `[theme] message` |
| `mode-keys` | window | one of: emacs, vi | `emacs` (`vi` if `$VISUAL` or `$EDITOR` contains `vi`) | `mode-keys` |
| `mode-style` | window | string | `bg=yellow,fg=black` | `[theme] selection` |
| `monitor-activity` | window | on/off | `off` | `monitor-activity` |
| `monitor-bell` | window | on/off | `on` | `monitor-bell` |
| `monitor-silence` | window | number | `0` | `monitor-silence` |
| `mouse` | session | on/off | `on` | `mouse` |
| `mouse-drag-end` (lif only) | session | one of: copy-pipe-and-cancel, copy-pipe-no-clear, none | `copy-pipe-and-cancel` | `mouse-drag-end` |
| `other-pane-height` | window | cells or N% | `0` | `other-pane-height` |
| `other-pane-width` | window | cells or N% | `0` | `other-pane-width` |
| `pane-active-border-style` | window | string | `fg=green` | `[theme] active-border` |
| `pane-base-index` | window | number | `0` | `pane-base-index` |
| `pane-border-format` | window | string | `#{?pane_active,#[reverse],}#{pane_index}#[default] "#{pane_title}"` (tmux adds mouse buttons) | `pane-border-format` |
| `pane-border-status` | window | one of: off, top, bottom | `off` | `pane-border-status` |
| `pane-border-style` | window | string | `default` | `[theme] border` |
| `pane-scrollbars` | window | one of: off, modal, on | `off` | `pane-scrollbars` |
| `pane-scrollbars-position` | window | one of: right, left | `right` | `pane-scrollbars-position` |
| `pane-scrollbars-style` | window | string (a style plus `width=N`, `pad=N`) | `bg=brightblack,fg=white,width=1,pad=0` (tmux: theme colors) | `pane-scrollbars-style` |
| `popup-border-lines` | window | one of: single, rounded, double, heavy, simple, padded, none | `single` | `popup-border-lines` |
| `popup-border-style` | window | string | `default` | `[theme] popup-border` |
| `popup-style` | window | string | `default` | `[theme] popup` |
| `prefix` | session | key | `C-b` | `prefix` |
| `prefix2` | session | key | `None` | `prefix2` |
| `remain-on-exit` | window | one of: off, on, failed | `off` | `remain-on-exit` |
| `renumber-windows` | session | on/off | `off` | `renumber-windows` |
| `repeat-time` | session | number | `500` | `repeat-time` |
| `scrollback-bytes` (lif only) | session | number (bytes) | `10485760` | `scrollback` |
| `set-clipboard` | server | one of: off, external, on | `external` | `set-clipboard` |
| `set-titles` | session | on/off | `off` | `set-titles` |
| `set-titles-string` | session | string | `#S:#I:#W - "#T"` | `set-titles-string` |
| `silence-action` | session | one of: none, any, current, other | `other` | `silence-action` |
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
| `update-environment` | session | list of names (`set -a` adds one; shown as `update-environment[0] DISPLAY`...) | tmux's: `DISPLAY KRB5CCNAME MSYSTEM SSH_ASKPASS SSH_AUTH_SOCK SSH_AGENT_PID SSH_CONNECTION WINDOWID XAUTHORITY` | `update-environment` (a TOML array) |
| `visual-activity` | session | one of: off, on, both | `off` | `visual-activity` |
| `visual-bell` | session | one of: off, on, both | `off` | `visual-bell` |
| `visual-silence` | session | one of: off, on, both | `off` | `visual-silence` |
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
`terminal-overrides`, `terminal-features`,
`extended-keys`,
`status-keys`, `allow-rename`, `message-command-style`,
`pane-border-lines`, `pane-border-indicators`. The behavior
several of these exist to turn on (true color, extended keys, OSC 52) is on
or automatic in lif.

Notes on some that lif has:

- `focus-events` is on by default (tmux: off): a program that asks for
  focus reports gets them.
- `monitor-silence`, `monitor-activity` and `monitor-bell` flag a window
  that is not the current one (`~`, `#`, `!` in its flags) and run the
  `alert-silence`, `alert-activity` and `alert-bell` hooks; the
  `*-action` options choose whose clients are told and the `visual-*`
  options how (a bell, a message such as `Silence in window 2`, or both),
  as in tmux. The silence timer exists only while the option is set and
  the pane has printed since it last fired.
- `destroy-unattached`: `on` destroys a session as soon as nobody is
  attached to it, including one made with `new-session -d`. `keep-last`
  and `keep-group` are about session groups, which lif does not have, and
  keep every session (as they do in tmux for a session in no group).
- `aggressive-resize` sizes a window for the clients that are showing it:
  with it a window that is not current keeps its size when the client's
  terminal changes, until it is selected. (tmux's other use, clients of
  several sessions sharing a window, needs linked windows.)
- The `popup-*` and `menu-*` options are the defaults of
  `display-popup -b -s -S` and `display-menu -b -s -H -S`. Their style
  defaults are the terminal's colors and a yellow selected line; tmux's
  are theme colors.

`allow-passthrough` unwraps tmux's `ESC P tmux; ... ESC \` wrapper and hands
the wrapped sequence to the pane's terminal, so what lif passes on anyway
(OSC 52, notifications) reaches the outer terminal; other wrapped sequences
are not written to it raw. `all` is the same as `on`. `pane-border-status`
takes one row from the window, as in tmux, and draws each pane's
`pane-border-format` on its border, two cells in from each end.

## Floating panes

`new-pane` makes a floating pane: drawn over the tiled panes in a box,
in the window's pane list after them (`#{pane_floating_flag}` is 1), and
not part of the layout. As in tmux, `-x`/`-y` give its size and
`-X`/`-Y` its position, border included (cells or `N%` of the window);
without them it is half the window wide and a quarter high, cascaded from
the top left. `move-pane` moves one (`-P` with tmux's positions, `-X`,
`-Y`, `-D`/`-U`/`-L`/`-R` by lines or columns, `-z` its depth, 0 in
front), `resize-pane -x`/`-y` and `-L`/`-R`/`-U`/`-D` resize one,
`split-window` of one makes another below it, `swap-pane` trades places
with a tiled pane of the same window. A click selects the pane on top.
Not in lif: modal panes (`-O -K -C -D`), floating panes over a zoomed
pane (`-A`; they are hidden while a pane is zoomed), dragging them
(`-M`), the `move` key table and the floating pane keys of the default
bindings. Differences: the last tiled pane of a window closes the window
with its floating panes (tmux keeps the window); `rotate-window` turns
the tiled panes only; a floating pane cannot be zoomed or swapped with a
pane of another window.

`pane-scrollbars` (`on`, or `modal`: only in copy mode),
`pane-scrollbars-position` and `pane-scrollbars-style` (`width=N`,
`pad=N`) work as in tmux: `on` takes the columns from the pane, `modal`
draws over it. The slider shows where the screen (or copy mode's view) is
in the history. Not in lif: `pane-scrollbars-timeout` and the mouse on
the scrollbar.

## Control mode

`lif -C` (and `-CC`, wrapped in tmux's DCS for terminals such as iTerm2)
speaks tmux's control-mode protocol: each line on standard input is a
command list, answered with `%begin` ... `%end` (`%error` on failure;
commands that wait, such as `run-shell`, are answered when they finish,
in order); the panes of the client's session send `%output %N data`
(bytes below space and `\` in octal); and `%session-changed`,
`%sessions-changed`, `%session-renamed`, `%session-window-changed`,
`%window-add`, `%window-close`, `%unlinked-window-add`,
`%unlinked-window-close`, `%window-renamed`, `%window-pane-changed` and
`%layout-change` tell what changed, after the answer of the command that
changed it. An empty line or the end of input detaches (`%exit`). The
client's size counts for its windows only after `refresh-client -C WxH`,
and it has no status line. Differences: no `refresh-client -A`, `-B`,
`-f` (subscriptions, `pause-after`, `no-output`), no
`%extended-output`, `%pane-mode-changed`, `%client-*` or
`%subscription-changed`. A client that does not keep up gets
`%pause %N` for a pane (at most 1 MiB waits for it) and `%continue %N`
once it has caught up; tmux does that only with `pause-after` and
otherwise buffers without a bound.

## Linked windows and session groups

As in tmux, a window can be in several sessions at once: `link-window`
puts it at an index of another session (or a second index of the same
one), `new-session -t target` makes a session that shares all of
`target`'s windows (a session group). It is one window either way: its
panes are shown wherever it is linked, `kill-window` removes it from every
session and `unlink-window` from one (its last place only with `-k`).
The members of a group share their window list (a window made, moved or
killed in one is in all of them) and keep their own current window,
options and clients. The window takes the size the clients of all the
sessions it is in ask for (`window-size`). A group is named after the
session it was made from and keeps that name; `#{session_group}`,
`#{session_group_list}` and `#{window_linked_sessions_list}` list them.

## Format variables

Formats are expanded wherever tmux expands them (`-F`, `display-message`,
`new-window -n`, `split-window -c`, the status line, `command-prompt -I`).
Supported syntax: `#{name}`, `#S #I #W #P #T #D #F #H #h ##`,
`#{?cond,a,b}` (with several conditions), `#{==:a,b}` `!=` `<` `>` `<=`
`>=` `||` `&&` `#{!:x}`, `#{m:glob,s}` (`m/i:` ignores case, `m/r:` takes a regular expression, `m/ri:` both), `#{e|op|:a,b}` (`+ - * / % ==`...),
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
| Session | `session_name`, `session_id`, `session_windows`, `session_attached`, `session_many_attached`, `session_created`, `session_activity` (same value as `session_created`), `session_path`, `session_alerts`, `session_format`, `session_grouped`, `session_group`, `session_group_size`, `session_group_list`, `session_group_attached`, `session_group_many_attached` |
| Window | `window_index`, `window_name`, `window_id`, `window_flags`, `window_raw_flags`, `window_active`, `window_last_flag`, `window_bell_flag`, `window_activity_flag`, `window_zoomed_flag`, `window_panes`, `window_width`, `window_height`, `window_layout`, `window_visible_layout`, `window_start_flag`, `window_end_flag`, `window_marked_flag`, `window_format`; `window_silence_flag`, `window_linked`, `window_linked_sessions`, `window_linked_sessions_list`; `window_bigger` is always 0 and `window_offset_x/y` 0 |
| Pane | `pane_index`, `pane_id`, `pane_title` (the host name until a program sets a title), `pane_current_command`, `pane_current_path`, `pane_width`, `pane_height`, `pane_active`, `pane_last`, `pane_pid`, `pane_dead`, `pane_dead_status`, `pane_in_mode`, `pane_start_command`, `pane_start_path`, `pane_left`, `pane_top`, `pane_right`, `pane_bottom`, `pane_at_left`, `pane_at_top`, `pane_at_right`, `pane_at_bottom`, `pane_synchronized`, `pane_input_off`, `pane_pipe`, `pane_marked`, `pane_marked_set`, `pane_floating_flag`, `pane_format`; `pane_mode` is `copy-mode` in any mode; `mouse_any_flag`, `alternate_on` |
| Mouse (in a mouse key's commands) | `mouse_x`, `mouse_y` (in the pane under the mouse), `mouse_word`, `mouse_line` |
| History | `history_size` (lines), `history_bytes` (lif only: bytes used), `history_limit` |
| Client | `client_name`, `client_tty` (same as the name, e.g. `client-58982`), `client_termname`, `client_width`, `client_height`, `client_pid`, `client_session`, `client_last_session`, `client_prefix`, `client_key_table`, `client_flags`, `client_cwd` |
| Buffer | `buffer_name`, `buffer_size`, `buffer_sample` (in `list-buffers -F`) |
| Hook | `hook`, `hook_session`, `hook_session_name`, `hook_window`, `hook_window_name`, `hook_pane`, `hook_client` (in the commands of a hook) |

Not available, among others: `pane_tty`, `cursor_x`/`cursor_y`,
`mouse_hyperlink`, `mouse_status_line`, and the copy-mode variables
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
  `new-window`, `split-window` or `respawn-pane`. `update-environment`
  copies the variables it names from the client into the session's set
  when the client creates, attaches or switches to the session (unless
  `-E`); one the client lacks is set to be removed (`-NAME`), as in tmux.
  `show-environment` lists only what was set this way (tmux lists
  everything). Hidden variables (`-h`) are listed with
  `show-environment -h` and not passed on.
- **`synchronize-panes`.** Typed keys, pastes and `send-keys` go to every
  pane of the window that has the option on and is not in a mode; panes
  hidden by a zoomed pane are left alone. Mouse events go to one pane.
- **`join-pane`.** Without `-s` the source is the current pane (tmux: the
  marked pane, which lif does not have).

## Mouse

Mouse support is on by default, as in tmux 3.8. What the mouse does
outside copy mode is bound in the root table under tmux's key names: an
event (`MouseDown1`-`3`, `MouseUp1`-`3`, `MouseDrag1`-`3`,
`MouseDragEnd1`-`3`, `SecondClick1`-`3`, `DoubleClick1`-`3`,
`TripleClick1`-`3`, `WheelUp`, `WheelDown`) followed by where it happened
(`Pane`, `Border`, `Status` for a window's entry in the status line,
`StatusLeft`, `StatusRight`, `StatusDefault` for the rest of it), with
`C-`, `M-` and `S-` for the modifiers held. A button that goes down and
comes up in place ends with `MouseUp`; one that moves first is
`MouseDrag` at the first motion and `MouseDragEnd` when it comes up.

```
$ lif bind -n MouseDown2Pane display-message -t = 'pane #{pane_index} at #{mouse_x},#{mouse_y}: #{mouse_word}'
$ lif unbind -n MouseDown3Pane          # no menu on the right button
$ lif bind -n C-MouseDown1Status kill-window -t =
```

In a mouse key's commands the target `=` (or `{mouse}`) is the pane or
window under the mouse (the window's active pane on a border, the current
window beside the window list), the `mouse_x`, `mouse_y`, `mouse_word` and
`mouse_line` formats describe the cell it is on, `send-keys -M` passes the
event on to the pane's mode or its program, `copy-mode -M` starts a
selection from it, `resize-pane -M` makes the drag resize, and
`display-menu -x M -y M` opens a menu there. Elsewhere `=` fails with
`no mouse target`.

The default bindings (`lif list-keys -T root`) are tmux's:

| Key | Does |
|---|---|
| `MouseDown1Pane` | selects the pane |
| `MouseDown1Status` | selects the window |
| `MouseDown1Border`, `MouseDrag1Border` | resize by dragging the border |
| `MouseDrag1Pane` | copy mode, selecting; the end of the drag copies (see `mouse-drag-end`) |
| `DoubleClick1Pane`, `TripleClick1Pane` | copy the word, the line |
| `WheelUpPane` | copy mode, scrolling (left again at the bottom); arrow keys in a full-screen program without mouse support |
| `WheelUpStatus`, `WheelDownStatus` | previous, next window |
| `MouseDown2Pane` | pastes the newest buffer |
| `MouseDown3Pane` | the pane menu of `C-b >`, at the mouse |
| `MouseDown3Status` | the window menu of `C-b <`, at the window's entry |
| `MouseDown3StatusLeft` | a session menu (renumber, rename, detach, new session, new window) |

A program that asked for the mouse gets every event in its pane, as in
tmux, unless you bound the key yourself.

Differences:

- The copy-mode tables have no mouse entries. Once a pane is in copy mode
  or a chooser the mouse there is built in (see
  [copy-mode.md](copy-mode.md#mouse)), and no mouse key runs for it. Mouse
  keys bound in `copy-mode` or `copy-mode-vi` are accepted and never used. What tmux
  users change by rebinding `MouseDragEnd1Pane` there is the
  `mouse-drag-end` option (`copy-pipe-no-clear` stays in copy mode with
  the selection shown, `none` only selects).
- A key left at its default for selecting, resizing, dragging, clicking
  twice or thrice, or `WheelUpPane`, is handled by the server directly;
  the commands `list-keys` shows for it are tmux's and are what runs if
  you bind them yourself. One visible difference: after a double or triple
  click lif copies at once, where the commands show the selection for
  0.3 s first.
- `WheelDownPane` has no binding, as in tmux, and still sends arrow keys
  to a full-screen program without mouse support; binding it replaces
  that.
- Clicks are counted at once: the second press on a cell within the click
  time is `SecondClick` and then `DoubleClick`, the third `TripleClick`
  (tmux waits to see whether a third press follows before it runs
  `DoubleClick`). A double or triple click nothing is bound to counts as
  `MouseDown` again. A drag that starts on a border that resizes, or
  inside a selection lif makes itself, has no `MouseUp` or
  `MouseDragEnd` key.
- No keys for what lif does not have: scrollbars (`...ScrollbarUp`,
  `...ScrollbarSlider`, `...ScrollbarDown`), floating panes
  (`...Control7`-`9`, `...Empty`), `MouseMove`, and tmux's `M-` and `C-`
  defaults that move or create floating panes. They fail with
  `unknown key`.
- The menus leave out what lif has no command for; the session menu has no
  "Switch To" entries.
- The marked pane has no reversed border; the window's flags show `M`.

## Popups and menus

`display-popup` runs a program in a box over the panes; `display-menu`
shows a menu there. Sizes, positions, borders and exit statuses are tmux
3.8's:

```
$ lif display-popup -E 'exit 3'; echo "status $?"
status 3
$ lif display-popup -w 0 true
width too small
```

- A popup is half the client's screen unless `-w` and `-h` say otherwise
  (cells or `N%`), centred where tmux centres it. `-x` is its left column
  and `-y` the row below it; `C` is the centre, `R` the right edge, `P`
  the target pane, `S` next to the status line. It is moved to fit.
- The command that opened it waits until it closes. With `-E` it closes
  when its program exits and the program's status is the command's (`-EE`:
  only on status 0); without, it stays until Escape or C-c (any key with
  `-k`) and the status is 0. `-C` closes the client's popup.
- While it is up, keys, pastes and the mouse (if the program asked for it)
  go to its program; the prefix key too. A client has one popup or menu at
  a time: a second `display-popup` does nothing, as in tmux.
- `-b` takes `single`, `rounded`, `double`, `heavy`, `simple`, `padded`
  and `none`; `-B` is no border. `-T` puts a title on the border, `-s`
  and `-S` set the styles, `-d` the directory, `-e` variables.
- A menu's arguments are `name key command` triples; an empty name is a
  separator, a name that starts with `-` cannot be chosen, and a name
  that expands to nothing leaves the item out. Up and Down (`k`, `j`)
  move, Enter or the item's key chooses, `q` or Escape closes; moving the
  mouse over an item selects it, a click chooses, a click beside the menu
  closes it. The command runs for the pane the menu was opened for.
- A menu opened by a mouse key is up while the button is held, as in
  tmux: letting go on an item chooses it, anywhere else closes the menu
  (so a quick click only flashes it). `-O` keeps it open after the
  button is let go; then a click chooses. `-x M -y M` centres such a menu
  on the mouse with its top border on the mouse's line; `-x W -y W` puts
  it above the status line where the window's entry starts.
- `C-b <` and `C-b >` open tmux's window and pane menus without the items
  lif has no command for (floating panes, the copy-mode toggles, the
  items built from the word under the mouse).

Differences:

- A popup's program gets `LIF` and the `LIF_PANE` of the pane it was
  opened for; it is not a pane (no target reaches it, `capture-pane` does
  not see it) and it ends with its client.
- No `display-popup -N`, no moving or resizing a popup with the mouse; a
  popup that no longer fits after the terminal shrank is closed.
- `display-menu` returns at once instead of waiting.
- Styles written into names (`#[underscore]`) are dropped; without a
  mouse key the `-x`/`-y` forms `M` (mouse) and `W` (window list) mean
  the centre.

## Chooser modes

`choose-tree` (`C-b s`, `C-b w`), `choose-buffer` (`C-b =`), `choose-client`
(`C-b D`) and `find-window` (`C-b f`) look and work as in tmux 3.8: the
list with a shortcut on each line, the tree with `+` and `-`, and below it
a box with a preview of what is selected.

```
(0) - alpha: 2 windows (attached)
(1) ├─→ + 0: zsh*
(2) └─→ + 1: second: "host.local"
(3) + beta: 1 windows

┌ 0 (sort: index) (view: preview) ───────────────────────────────┐
```

Keys, as in tmux: `Enter` chooses, the shortcut on a line (`0`-`9`,
`M-a`-`M-z`) chooses that line, `Up`/`Down` (`k`/`j`), `PPage`/`NPage`,
`Home`/`End` (`g`/`G`) move, `Right`/`+` and `Left`/`-` expand and collapse
(`M-+`, `M--` all), `t` tags (`T` none, `C-t` all), `C-s` searches (`n`,
`N` again), `f` filters with a format (`c` clears), `O` changes the sort
order and `r` reverses it, `v` hides the preview, `H` goes back to where
the mode started, `:` runs a command for the tagged items, `q` leaves.
In the tree `x` kills the selected session, window or pane and `X` the
tagged ones, after a `y`; in the buffer list `p`/`P` paste and `d`/`D`
delete; in the client list `d`/`D` detach, `x`/`X` detach and hang up,
`z`/`Z` suspend. In the tree `m` marks the selected item's pane and `M`
clears the mark, and `S-Up`/`S-Down` swap the selected window with the
one before or after it in its session (a current window stays current).
`C-h` or `F1` shows the keys in a box; the next key takes it down. A
click selects a line, a double click chooses it, the wheel moves. The list
follows the server: a window that closes disappears from it.

Differences:

- Prompts (search, filter, the kill question) are on the status line; tmux
  draws them at the bottom of the pane.
- No `<`/`>` to scroll the previews, no `i` view, no `e` to edit a
  buffer, no `-K` key formats, no session groups (`-G`). The help lists
  the keys lif has.
- A mark set with `m` stays on its pane when `S-Up`/`S-Down` move the
  window (tmux loses it).
- The preview is drawn when the selection or the server changes, not
  continuously, and a pane no client is showing is previewed from its top
  left corner (tmux centers on its cursor).
- More panes or windows than fit side by side at 16 columns each are left
  out of a preview (tmux scrolls them).
- `choose-buffer` with no buffers opens nothing.
- Text uses the terminal's default colors and the selected line
  `mode-style`; tmux's theme colors for the tree are not there.

## Translating a .tmux.conf

lif never reads `~/.tmux.conf`. `lif migrate-tmux-conf [file]` (default
`~/.tmux.conf`) prints a `config.toml` on standard output and, on standard
error, one line per command it did not translate, with the reason. It
translates `set`/`set-option`/`setw` with `-g` or `-s` for options lif has,
`bind`/`bind-key` with `-n`, `-r`, `-T`, `-N`, `unbind` (mouse keys of the
root table included), a copy-mode table's
`MouseDragEnd1Pane` (as `mouse-drag-end`), `set-hook -g` (with `-a` and
`-u`, as `[hooks]`), `set-environment -g` (with `-r`, as `[env]`), and
comments directly above a translated line. Other commands lif has
(`new-session`, `if-shell`, `run-shell`, `set -g @user-option`, ...) become
`[startup]` commands, which the server runs when it starts, as tmux runs
them when it reads the file; not `source-file` (what it names is a tmux
file) and not a `run-shell` or `if-shell` that mentions tmux. It never
guesses. It exits 0 even when lines were left out.

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
tmux.conf:34: not translated (run-shell runs something written for tmux, which would talk to tmux and not to lif: add it to run in [startup] yourself if it works without tmux): run '~/.tmux/plugins/tpm/tpm'
tmux.conf:35: not translated (it runs source-file on a tmux file: translate that file as well and add what it gives to this file): if-shell "test -f ~/.tmux.local" "source ~/.tmux.local"
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

[startup]
run = [
    "set-option -g @plugin tmux-plugins/tpm",
]
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
- Plugins are not loaded. The line that starts TPM is left out on
  purpose: a plugin is a shell script that calls `tmux`, so run from lif
  it would talk to a tmux server (starting one), not to lif. The `@plugin`
  option is carried over as a `[startup]` command and changes nothing by
  itself. A script that only needs a few tmux commands can work through
  the tmux shim (see [claude-code.md](claude-code.md)); try it by hand
  before adding it to `[startup]`.
- `[startup]` commands run once, when the server starts; reloading the
  file does not run them again, where tmux runs every line of a
  `.tmux.conf` each time it is sourced.

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
- **Mouse.** On by default (as in tmux 3.8), with tmux's mouse keys and
  default bindings in the root table; the mouse inside copy mode is built
  in. See [Mouse](#mouse).
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
