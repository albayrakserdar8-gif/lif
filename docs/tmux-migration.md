# Moving from tmux to lif

This guide is for someone who uses tmux today. Everything in it was checked
against lif 0.1.0's command table and by running the commands; the tmux side is
tmux 3.8 (`tmux list-commands`, `tmux list-keys` with an empty configuration).

In short: the commands, flags, targets, formats and default keys you use every
day are the same. The configuration file is not: lif reads TOML, and
`lif migrate-tmux-conf` translates the common part of a `.tmux.conf`. Anything
that runs shell commands (`run-shell`, `if-shell`, `#()`, hooks, plugins)
does not exist in lif.

## Commands

58 of tmux 3.8's 92 commands exist in lif (22 with the same flags, 36 with some
difference, usually missing flags); 34 do not. Command names, aliases and unambiguous prefixes
resolve as in tmux (`lif splitw`, `lif new`, `lif a`). An unimplemented
command fails with exit status 1:

```
$ lif run-shell 'echo hi'
unsupported command: run-shell
```

A flag lif does not know fails the same way (`command split-window: unknown flag -e`).
"Accepted and has no effect" below means the flag parses but the code does not
use it.

| tmux command | lif | Differences |
|---|---|---|
| `attach-session` (`attach`) | supported with differences | no `-E`, `-f`, `-r`, `-x` |
| `bind-key` (`bind`) | supported with differences | mouse key names (`MouseDown1Pane`, ...) are rejected: mouse handling is built in |
| `break-pane` (`breakp`) | supported with differences | no `-W`, `-x`, `-y`, `-X`, `-Y` (floating panes) |
| `capture-pane` (`capturep`) | supported with differences | has `-b -e -E -J -N -p -q -S -t`; no `-a`, `-C`, `-F`, `-H`, `-I`, `-L`, `-M`, `-P`, `-R`, `-T` |
| `choose-buffer` | not supported | |
| `choose-client` | not supported | |
| `choose-tree` | not supported | |
| `clear-history` (`clearhist`) | not supported | |
| `clear-prompt-history` (`clearphist`) | not supported | |
| `clock-mode` | not supported | |
| `command-prompt` | supported with differences | no `-b`, `-C`, `-e`, `-l` |
| `confirm-before` (`confirm`) | supported with differences | `-b` is accepted and has no effect |
| `copy-mode` | supported with differences | no `-k`, `-s`, `-S`; `-d` is accepted and has no effect |
| `customize-mode` | not supported | |
| `delete-buffer` (`deleteb`) | supported | |
| `detach-client` (`detach`) | supported with differences | no `-E`; `-P` is accepted and has no effect |
| `display-menu` (`menu`) | not supported | |
| `display-message` (`display`) | supported with differences | has `-c -d -F -l -p -t`; no `-a`, `-C`, `-I`, `-j`, `-N`, `-v` |
| `display-panes` (`displayp`) | supported with differences | no `-k`, `-s`, `-Z`; `-b` and `-N` are accepted and have no effect |
| `display-popup` (`popup`) | not supported | |
| `find-window` (`findw`) | not supported | |
| `has-session` (`has`) | supported | |
| `if-shell` (`if`) | not supported | |
| `join-pane` (`joinp`) | not supported | |
| `kill-pane` (`killp`) | supported with differences | no `-f` (filter) |
| `kill-server` | supported | |
| `kill-session` | supported with differences | no `-f`, `-g` |
| `kill-window` (`killw`) | supported with differences | no `-f` |
| `last-pane` (`lastp`) | supported with differences | no `-d`, `-e` |
| `last-window` (`last`) | supported | |
| `link-window` (`linkw`) | not supported | |
| `list-buffers` (`lsb`) | supported with differences | no `-O` (sort order) |
| `list-clients` (`lsc`) | supported with differences | no `-O` |
| `list-commands` (`lscm`) | supported with differences | prints names and aliases only, no usage strings; `-F` is accepted and ignored |
| `list-keys` (`lsk`) | supported with differences | no `-a`, `-F`, `-O`, `-r` |
| `list-panes` (`lsp`) | supported with differences | no `-O`, `-r` |
| `list-sessions` (`ls`) | supported with differences | no `-O`, `-r` |
| `list-windows` (`lsw`) | supported with differences | no `-O`, `-r` |
| `load-buffer` (`loadb`) | not supported | |
| `lock-client` (`lockc`) | not supported | |
| `lock-server` (`lock`) | not supported | |
| `lock-session` (`locks`) | not supported | |
| `move-pane` (`movep`) | not supported | |
| `move-window` (`movew`) | supported | |
| `new-pane` (`newp`) | not supported | |
| `new-session` (`new`) | supported with differences | no `-e`, `-f`, `-t` (no session groups); `-E` and `-X` are accepted and have no effect |
| `new-window` (`neww`) | supported with differences | no `-e`, `-E` |
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
| `respawn-pane` (`respawnp`) | supported with differences | no `-e`, `-E` |
| `respawn-window` (`respawnw`) | not supported | |
| `rotate-window` (`rotatew`) | supported | |
| `run-shell` (`run`) | not supported | |
| `save-buffer` (`saveb`) | not supported | |
| `select-layout` (`selectl`) | supported | |
| `select-pane` (`selectp`) | supported with differences | no `-d`, `-e` (input on/off), `-m`, `-M` (marked pane), `-T` (title) |
| `select-window` (`selectw`) | supported | |
| `send-keys` (`send`) | supported with differences | no `-c`, `-K`, `-M`; key names sent into a pane in copy mode do not drive copy mode reliably (use `-X`), see below |
| `send-prefix` | supported | |
| `server-access` | not supported | |
| `set-buffer` (`setb`) | supported with differences | no `-t`, `-w` |
| `set-environment` (`setenv`) | not supported | |
| `set-hook` | not supported | |
| `set-option` (`set`) | supported with differences | only the options listed below; no `-p` (pane options), `-U`; `-o` refuses only user options (`set -o -g status-right x` overwrites, tmux says `already set`) |
| `set-window-option` (`setw`) | supported | |
| `show-buffer` (`showb`) | supported | |
| `show-environment` (`showenv`) | not supported | |
| `show-hooks` | not supported | |
| `show-messages` (`showmsgs`) | supported | |
| `show-options` (`show`) | supported with differences | no `-A`, `-F`, `-H`, `-p` |
| `show-prompt-history` (`showphist`) | not supported | |
| `show-window-options` (`showw`) | supported with differences | no `-F` |
| `source-file` (`source`) | supported with differences | reads lif TOML files only; a `.conf` file or a file of tmux commands is refused with a pointer to `lif migrate-tmux-conf` |
| `split-window` (`splitw`) | supported with differences | has `-b -c -d -F -h -l -p -P -t -v -Z`; no `-e`, `-f`, `-I`, `-k`, `-m`, `-W`, `-B`, `-R`, `-s`, `-S`, `-T` |
| `start-server` (`start`) | supported | |
| `suspend-client` (`suspendc`) | not supported | |
| `swap-pane` (`swapp`) | supported | |
| `swap-window` (`swapw`) | supported | |
| `switch-client` (`switchc`) | supported with differences | no `-E`, `-O`, `-r`; `-Z` is accepted and has no effect |
| `switch-mode` | not supported | |
| `unbind-key` (`unbind`) | supported | |
| `unlink-window` (`unlinkw`) | not supported | |
| `wait-for` (`wait`) | not supported | |

The three commands `check-config`, `print-config` and `migrate-tmux-conf`
are lif's own; they run without a server (see [configuration.md](configuration.md)).

## Default prefix key table

Prefix `C-b` in both. 61 bindings are identical, 7 differ (6 only in
form, plus `?`), and 25 tmux bindings have no lif equivalent (they use
commands or flags lif lacks). Mouse
bindings are not in lif's tables at all (see [Mouse](#mouse)). Long tmux
commands are cut at 70 characters.

| Key | tmux 3.8 | lif 0.1.0 | |
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
| `T` | `command-prompt -I "#T" { select-pane -T "%%" }` |  | missing in lif |
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
| `C-z` | `suspend-client` |  | missing in lif |
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

lif has 60 options, all with tmux's names, scopes and (except for colors that
tmux 3.8 expresses with theme colors) tmux 3.8's defaults, plus one of its
own, `scrollback-bytes`. `set-option`, `set-window-option`, `show-options`
and `show-window-options` work on them as in tmux, including `-g`, `-s`,
`-w`, `-t`, `-u`, `-a`, `-q`, `-F` and toggling a flag by leaving
out the value. User options (`@name`) can be set and read in formats.
Setting any other tmux option fails with `invalid option: NAME` (`set -q`
silences it).

| Option | Scope | Values | Default | config.toml key |
|---|---|---|---|---|
| `automatic-rename` | window | on/off | `on` | `automatic-rename` |
| `automatic-rename-format` | window | string | `#{?pane_in_mode,[lif],#{pane_current_command}}#{?pane_dead,[dead],}` | `automatic-rename-format` |
| `base-index` | session | number | `0` | `base-index` |
| `bell-action` | session | one of: none, any, current, other | `any` | `bell-action` |
| `buffer-limit` | server | number | `50` | `buffer-limit` |
| `copy-mode-current-match-style` | window | string | `bg=magenta,fg=black` | `[theme] current-match` |
| `copy-mode-match-style` | window | string | `bg=cyan,fg=black` | `[theme] match` |
| `default-command` | session | string | (empty) | `default-command` |
| `default-shell` | session | string | `$SHELL` of the server, else `/bin/sh` | `default-shell` |
| `default-size` | session | WxH | `80x24` | `default-size` |
| `detach-on-destroy` | session | one of: off, on, no-detached, previous, next | `on` | `detach-on-destroy` |
| `display-panes-time` | window | number | `1000` | `display-panes-time` |
| `display-time` | session | number | `750` | `display-time` |
| `escape-time` (accepted, ignored) | server | number | `10` | `escape-time` |
| `exit-empty` | server | on/off | `on` | `exit-empty` |
| `history-limit` (shown, not enforced) | session | number | `2000` | `history-limit` |
| `main-pane-height` | window | cells or N% | `24` | `main-pane-height` |
| `main-pane-width` | window | cells or N% | `80` | `main-pane-width` |
| `message-style` | session | string | `bg=yellow,fg=black` | `[theme] message` |
| `mode-keys` | window | one of: emacs, vi | `emacs` (`vi` if `$VISUAL` or `$EDITOR` contains `vi`) | `mode-keys` |
| `mode-style` | window | string | `bg=yellow,fg=black` | `[theme] selection` |
| `monitor-activity` | window | on/off | `off` | `monitor-activity` |
| `monitor-bell` | window | on/off | `on` | `monitor-bell` |
| `mouse` | session | on/off | `on` | `mouse` |
| `other-pane-height` | window | cells or N% | `0` | `other-pane-height` |
| `other-pane-width` | window | cells or N% | `0` | `other-pane-width` |
| `pane-active-border-style` | window | string | `fg=green` | `[theme] active-border` |
| `pane-base-index` | window | number | `0` | `pane-base-index` |
| `pane-border-style` | window | string | `default` | `[theme] border` |
| `prefix` | session | key | `C-b` | `prefix` |
| `prefix2` | session | key | `None` | `prefix2` |
| `remain-on-exit` | window | one of: off, on, failed | `off` | `remain-on-exit` |
| `renumber-windows` | session | on/off | `off` | `renumber-windows` |
| `repeat-time` | session | number | `500` | `repeat-time` |
| `scrollback-bytes` | session | number | `10485760` | `scrollback` |
| `set-clipboard` | server | one of: off, external, on | `external` | `set-clipboard` |
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
| `tiled-layout-max-columns` | window | number | `0` | `tiled-layout-max-columns` |
| `visual-bell` | session | one of: off, on, both | `off` | `visual-bell` |
| `window-size` | window | one of: largest, smallest, manual, latest | `latest` | `window-size` |
| `window-status-activity-style` | window | string | `reverse` | `[theme] activity` |
| `window-status-bell-style` | window | string | `reverse` | `[theme] bell` |
| `window-status-current-format` | window | string | `#I:#W#{?window_flags,#{window_flags}, }` | `[status] current-window-format` |
| `window-status-current-style` | window | string | `underscore` | `[theme] current-window` |
| `window-status-format` | window | string | `#I:#W#{?window_flags,#{window_flags}, }` | `[status] window-format` |
| `window-status-last-style` | window | string | `default` | `[theme] last-window` |
| `window-status-separator` | window | string | ` ` | `[status] separator` |
| `window-status-style` | window | string | `default` | `[theme] window` |
| `word-separators` | session | string | `` !"#$%&'()*+,-./:;<=>?@[\]^`{\|}~ `` | `word-separators` |
| `wrap-search` | window | on/off | `on` | `wrap-search` |

Some tmux options people often set and lif does not have: `default-terminal`,
`terminal-overrides`, `terminal-features`, `focus-events`,
`allow-passthrough`, `update-environment`, `set-titles`,
`synchronize-panes`, `pane-border-status`, `pane-border-format`,
`status-keys`, `aggressive-resize`, `allow-rename`, `message-command-style`,
`window-style`/`window-active-style`, `clock-mode-*`, `lock-*`. The
behavior several of these exist to turn on (true color, focus events, OSC 52,
titles) is on or automatic in lif.

## Format variables

Formats are expanded wherever tmux expands them (`-F`, `display-message`,
`new-window -n`, `split-window -c`, the status line, `command-prompt -I`).
Supported syntax: `#{name}`, `#S #I #W #P #T #D #F #H #h ##`,
`#{?cond,a,b}` (with several conditions), `#{==:a,b}` `!=` `<` `>` `<=`
`>=` `||` `&&` `#{!:x}`, `#{m:glob,s}`, `#{e|op|:a,b}` (`+ - * / % ==`...),
and the modifiers `=N`, `=-N`, `=/N/marker`, `pN`, `t:`, `b:`, `d:`,
`l:`, `E:`, `T:`, `q:`, `n:`, `w:`, `s/from/to/:` (literal, not a regex).
`#[style]` is passed to the status line. `#(command)` expands to nothing.
An unknown variable expands to an empty string. As in tmux, a name that is not
a variable is looked up as an option (`#{status-left}`, `#{@myopt}`).

| Group | Variables |
|---|---|
| Server | `host`, `host_short`, `pid`, `socket_path`, `version` |
| Session | `session_name`, `session_id`, `session_windows`, `session_attached`, `session_many_attached`, `session_created`, `session_activity` (same value as `session_created`), `session_path`, `session_alerts`, `session_format`; `session_grouped` is always 0 and `session_group` empty |
| Window | `window_index`, `window_name`, `window_id`, `window_flags`, `window_raw_flags`, `window_active`, `window_last_flag`, `window_bell_flag`, `window_activity_flag`, `window_zoomed_flag`, `window_panes`, `window_width`, `window_height`, `window_layout`, `window_visible_layout`, `window_start_flag`, `window_end_flag`, `window_format`; `window_silence_flag`, `window_bigger`, `window_marked_flag`, `window_linked` are always 0 and `window_offset_x/y` 0 |
| Pane | `pane_index`, `pane_id`, `pane_title` (the host name until a program sets a title), `pane_current_command`, `pane_current_path`, `pane_width`, `pane_height`, `pane_active`, `pane_last`, `pane_pid`, `pane_dead`, `pane_dead_status`, `pane_in_mode`, `pane_start_command`, `pane_start_path`, `pane_left`, `pane_top`, `pane_right`, `pane_bottom`, `pane_at_left`, `pane_at_top`, `pane_at_right`, `pane_at_bottom`, `pane_format`; `pane_marked`, `pane_marked_set`, `pane_synchronized`, `pane_input_off`, `pane_pipe` are always 0 |
| History | `history_size` (lines), `history_bytes` (lif only: bytes used), `history_limit` |
| Client | `client_name`, `client_tty` (same as the name, e.g. `client-58982`), `client_termname`, `client_width`, `client_height`, `client_pid`, `client_session`, `client_last_session`, `client_prefix`, `client_key_table`, `client_flags`, `client_cwd` |
| Buffer | `buffer_name`, `buffer_size`, `buffer_sample` (in `list-buffers -F`) |

Not available, among others: `pane_tty`, `pane_mode`, `alternate_on`,
`cursor_x`/`cursor_y`, `mouse_*`, and the copy-mode variables
(`copy_cursor_x`, `copy_cursor_y`, `copy_cursor_word`, `scroll_position`,
`selection_present`, `search_present`, `pane_search_string`) in
`display-message`. The copy-mode key bindings that use `copy_cursor_word` and
`pane_search_string` are handled inside copy mode and work.

## Translating a .tmux.conf

lif never reads `~/.tmux.conf`. `lif migrate-tmux-conf [file]` (default
`~/.tmux.conf`) prints a `config.toml` on standard output and, on standard
error, one line per command it did not translate, with the reason. It
translates `set`/`set-option`/`setw` with `-g` or `-s` for options lif has,
`bind`/`bind-key` with `-n`, `-r`, `-T`, `-N`, `unbind`, and comments
directly above a translated line. It never guesses. It exits 0 even when
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
tmux.conf:32: not translated (user options (@name) have no config.toml equivalent): set -g @plugin 'tmux-plugins/tpm'
tmux.conf:33: not translated (unsupported command: run-shell): run '~/.tmux/plugins/tpm/tpm'
tmux.conf:34: not translated (unsupported command: if-shell): if-shell "test -f ~/.tmux.local" "source ~/.tmux.local"
tmux.conf:35: not translated (unsupported command: set-hook): set-hook -g after-new-window 'display hi'
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
```

`lif check-config` reports no problems with it. Things to review by hand
after a migration:

- The usual prefix recipe (`unbind C-b`, `bind C-a send-prefix`) is folded
  into `prefix = "C-a"`, which does both.
- `history-limit = 50000` is carried over but lif does not enforce it.
  Scrollback is limited by `scrollback` (bytes); set that instead, for example
  `scrollback = "64MB"`.
- `copy-pipe-and-cancel pbcopy` is translated, but lif ignores the command
  argument: the selection goes to a lif buffer and to the terminal clipboard
  through OSC 52. With a terminal that accepts OSC 52 the result is the same.
- `escape-time` is accepted and ignored.
- Reload bindings: `bind r source-file ~/.config/lif/config.toml`.
- Plugins, `run-shell`, `if-shell` and hooks have no replacement.

## Behavior differences you will notice

- **Config file.** TOML, read once at server start from `$LIF_CONFIG`,
  `$XDG_CONFIG_HOME/lif/config.toml` or `~/.config/lif/config.toml`, or the
  file given with `lif -f`. `source-file` reads the same TOML format; it
  refuses `.conf` files and files of tmux commands with a pointer to
  `lif migrate-tmux-conf`. Errors in the file go to `show-messages`, and the
  rest of the file still applies.
- **Scrollback.** Budgeted in bytes per pane (`scrollback-bytes`, default
  10 MiB) instead of lines. How many lines that holds depends on the pane
  width: in the benchmark, 5,688 lines at 200 columns after a 100 MB `cat`.
  The engine allocates in pages, so very small budgets round up (a budget of
  100000 bytes used 819200). As with tmux's `history-limit`, a new value
  applies to panes created afterwards. `history-limit` is not enforced:
  with `set -g history-limit 100`, a new pane kept 2,973 lines of `seq 1 3000`.
- **Environment inside panes.** `TERM=xterm-256color`, `COLORTERM=truecolor`,
  `TERM_PROGRAM=lif`, `TERM_PROGRAM_VERSION`, `LIF` and `LIF_PANE`; no
  `TMUX` or `TMUX_PANE`. Scripts that test `$TMUX` to detect a multiplexer
  need to test `$LIF` too.
- **lif inside a pane.** `lif` run in a pane knows which pane it is in
  (`lif split-window` splits that pane), but it finds the server through the
  default socket (or `-S`, `-L`, `$LIF_SOCKET`), not through `$LIF` as tmux
  does with `$TMUX`. For a server on a non-default socket, pass `-S` again.
- **Escape key.** No `escape-time`. With a terminal that speaks the kitty
  keyboard protocol, Escape is delivered immediately; otherwise a lone ESC
  waits 5 ms (fixed). Measured: 5.8 ms median, against 11 ms for tmux's default
  `escape-time 10`.
- **Clipboard.** `set-clipboard` defaults to `external`, as in tmux: copies
  from copy mode set the terminal clipboard (OSC 52), and OSC 52 from programs
  in panes is forwarded to the terminal but not stored as a lif buffer (`on`
  stores it too, `off` forwards nothing). `copy-pipe` commands are not run.
- **Notifications.** OSC 9, OSC 777 and OSC 99 desktop notifications from
  panes are forwarded to the attached terminals as OSC 9.
- **Mouse.** <a id="mouse"></a>On by default (as in tmux 3.8) and handled
  inside the server: click selects a pane, clicking a window in the status line
  selects it, dragging a border resizes, the wheel scrolls (entering copy mode,
  which it leaves at the bottom; in full-screen programs without mouse support
  it sends arrow keys), dragging in a pane selects text and copies it on
  release, and programs that request the mouse get it. Differences: mouse
  events cannot be bound (`bind -n MouseDown1Pane ...` fails with
  `unknown key: MouseDown1Pane`); after a drag-copy lif stays in copy mode
  with the selection shown (tmux exits); double- and triple-click select a word
  or line only when the pane is already in copy mode; there are no
  right-click menus.
- **No shell from lif.** `#()` in the status line is empty, so status bars
  built on shell commands (battery, load, git) need another approach.
- **Status line styles.** `fg=`, `bg=`, `us=` and attributes work;
  `align=`, `fill=`, `list=`, `range=` and theme colors (`themegreen`)
  are not supported.
- **`send-keys` into copy mode.** `send-keys -X command` works. Sending key
  names to a pane in copy mode (`send-keys k V Enter`) does not act like
  pressing those keys: in a test, `q` left copy mode but `V` and `Enter` did
  not select or copy. Use `-X` in scripts.
- **`list-commands`** prints names and aliases, not usage lines.
- **Prefix `?`** shows the complete `list-keys` output of all tables.
