# Configuration reference

lif reads one TOML file. Each setting in it becomes an ordinary lif command at
start-up (`set-option -g ...`, `bind-key -T ...`, `unbind-key ...`), so a
setting means exactly what running that command means, and every value can
also be changed later with `lif set-option` or `lif bind-key`.

## Where the file is and when it is read

The server reads the file once, when it starts, from the first of:

1. the file given with `lif -f FILE` (only used when that command starts the server);
2. `$LIF_CONFIG`;
3. `$XDG_CONFIG_HOME/lif/config.toml` (only if `XDG_CONFIG_HOME` is an absolute path);
4. `~/.config/lif/config.toml`.

Empty variables count as unset. A missing default file is fine; a missing
`-f` file is reported. lif never reads `~/.tmux.conf`
(see [tmux-migration.md](tmux-migration.md#translating-a-tmuxconf)).

Mistakes never stop the server. Each problem is reported with file, line and
column, the setting is skipped, and everything else in the file still
applies. When the server starts, the reports go to the message log:

```sh
lif show-messages
```

To reload after editing: `lif source-file ~/.config/lif/config.toml` (or bind
it, `r = "source-file ~/.config/lif/config.toml"`). As in tmux, reloading
applies what the file sets; it does not reset options or bindings you removed
from the file. `source-file` only reads this TOML format: a file ending in
`.conf`, or one that consists of tmux commands, is refused with a pointer to
`lif migrate-tmux-conf`.

Three commands work on configuration files without a server:

| Command | What it does | Exit status |
|---|---|---|
| `lif check-config [FILE]` | Checks FILE (default: the file the server would read) and prints every problem; prints nothing when there are none | 1 if there is an error, the file is missing, or its name ends in `.conf`; else 0 |
| `lif print-config` | Prints the documented example file, in which every value is the default | 0 |
| `lif migrate-tmux-conf [FILE]` | Translates a `.tmux.conf` (default `~/.tmux.conf`) to TOML on stdout; untranslated lines on stderr | 0, or 1 if FILE cannot be read |

A good start is `lif print-config > ~/.config/lif/config.toml`, then delete
what you do not change.

## Layout of the file

```toml
prefix = "C-a"          # top-level settings
mouse = true

[status]                # the status line
position = "top"

[theme]                 # styles
status = { bg = "black", fg = "white" }

[keys]                  # bindings after the prefix ([keys.prefix] is the same table)
"|" = "split-window -h"

[keys.root]             # bindings without the prefix
"M-Left" = "select-pane -L"

[keys.copy-mode]        # copy mode, mode-keys = "emacs"
[keys.copy-mode-vi]     # copy mode, mode-keys = "vi"

[hooks]                 # commands run when something happens
after-split-window = "select-layout tiled"

[env]                   # environment of new panes and shell commands
EDITOR = "nvim"
```

Keys not mentioned keep their defaults. Names are lower case with dashes, the
same as the tmux option they set, except `scrollback` and `mouse-drag-end`
(tmux has no equivalent options) and the shorter names inside `[status]` and
`[theme]`.

Value types used below:

| Type | Written as |
|---|---|
| bool | `true` or `false` (not `"on"`) |
| integer | a TOML integer, not a string |
| string | a TOML string; formats use tmux format syntax |
| choice | one of the listed strings; choices that include `"on"` and `"off"` also take `true` / `false` |
| key | a key name (see [Key names](#key-names)), or `false` / `"none"` for no key |
| size | a number of cells (integer) or a percentage string such as `"50%"` |
| bytes | an integer, or a string with a unit: `"512KB"`, `"32MB"`, `"1GB"` (units B, K/KB/KiB, M/MB/MiB, G/GB/GiB, all powers of 1024, case-insensitive; no fractions) |
| style | a style table or a tmux style string (see [[theme]](#theme)) |

## Top-level settings

| Key | Type | Default | Effect | tmux option |
|---|---|---|---|---|
| `prefix` | key | `"C-b"` | The prefix key. Any other value also unbinds `C-b` and binds the new key to `send-prefix`, so pressing it twice sends it to the pane. | `prefix` |
| `prefix2` | key | `false` | A second prefix key. | `prefix2` |
| `repeat-time` | integer (0-32767) | `500` | Milliseconds in which a repeatable binding can be pressed again without the prefix; 0 disables repeat. | `repeat-time` |
| `mouse` | bool | `true` | Mouse support: select panes and windows, resize, scroll, select text. | `mouse` |
| `mouse-drag-end` | choice: `"copy-pipe-and-cancel"`, `"copy-pipe-no-clear"`, `"none"` | `"copy-pipe-and-cancel"` | What the end of a mouse selection (a drag, a double or a triple click) does: copy and leave copy mode (tmux's default), copy and stay with the selection shown, or only select. tmux sets this by binding `MouseDragEnd1Pane`. | `mouse-drag-end` (lif only) |
| `mode-keys` | choice: `"emacs"`, `"vi"` | `"emacs"`, or `"vi"` if `$VISUAL` (else `$EDITOR`) contains `vi` when the server starts | Which copy-mode key table is used. | `mode-keys` |
| `word-separators` | string | `` !"#$%&'()*+,-./:;<=>?@[\]^`{\|}~ `` | Characters that end a word in copy mode, besides spaces. | `word-separators` |
| `wrap-search` | bool | `true` | Searches in copy mode wrap around the ends of the history. | `wrap-search` |
| `set-clipboard` | choice: `"off"`, `"external"`, `"on"` | `"external"` | Copies from copy mode always become a paste buffer. `external`: they also set the outer terminal's clipboard (OSC 52), and OSC 52 from programs in panes is forwarded to it but not kept. `on`: as `external`, and what programs copy is also kept as a paste buffer. `off`: nothing reaches the terminal's clipboard and OSC 52 from programs is ignored. (tmux's `external` ignores programs' OSC 52 entirely.) | `set-clipboard` |
| `copy-command` | string | `""` | A command copies are piped to (standard input, run with `/bin/sh -c` in the pane's directory) when they come from `copy-pipe` without a command of its own: the default copy keys (`Enter` in vi, `M-w` in emacs) and mouse copies. For example `"pbcopy"`, `"wl-copy"`, `"xclip -selection clipboard"`. | `copy-command` |
| `scrollback` | bytes | `"10MB"` | Memory budget for each pane's scrollback; `history-limit` holds as well. A line of history counts about 1.7 KB at 200 columns. Memory is allocated in whole pages, so a small budget rounds up (`100KB` shows as 819200 bytes in `#{history_bytes}`). Applies to existing panes too. | `scrollback-bytes` (lif only) |
| `history-limit` | integer | `4000` | Lines of scrollback kept per pane (tmux's default is 2000); `scrollback` holds as well. Applies to existing panes too (tmux: only to new ones). 0 keeps none. | `history-limit` |
| `default-shell` | string (absolute path) | `$SHELL` of the server, else `/bin/sh` | Shell for new panes, started as a login shell. | `default-shell` |
| `default-command` | string | `""` | Command run instead of a login shell (through `default-shell -c`). | `default-command` |
| `display-time` | integer (ms) | `750` | How long messages stay on the status line. | `display-time` |
| `display-panes-time` | integer (ms, from 1) | `1000` | How long `display-panes` (`prefix q`) shows pane numbers. | `display-panes-time` |
| `escape-time` | integer (ms) | `5` | How long a lone Escape from a terminal using the legacy key encoding waits for the rest of a sequence (Alt+key, arrows); 0 never waits. With the kitty keyboard protocol (enabled when the terminal supports it) Escape never waits. tmux's default is 10. | `escape-time` |
| `exit-empty` | bool | `true` | Stop the server when the last session closes. | `exit-empty` |
| `buffer-limit` | integer (from 1) | `50` | How many automatically named paste buffers are kept. | `buffer-limit` |
| `clock-mode-colour` | string | `"blue"` | Color of the clock of `clock-mode`. | `clock-mode-colour` |
| `clock-mode-style` | choice: `"12"`, `"24"` | `"24"` | 12- or 24-hour clock. | `clock-mode-style` |
| `detach-on-destroy` | choice: `"off"`, `"on"`, `"no-detached"`, `"previous"`, `"next"` | `"on"` | What attached clients do when their session is destroyed: detach, or switch to another session. | `detach-on-destroy` |
| `base-index` | integer | `0` | Number of the first window. | `base-index` |
| `pane-base-index` | integer (0-65535) | `0` | Number of the first pane. | `pane-base-index` |
| `renumber-windows` | bool | `false` | Close gaps in window numbers when a window closes. | `renumber-windows` |
| `automatic-rename` | bool | `true` | Name windows after what runs in them. | `automatic-rename` |
| `automatic-rename-format` | string | `"#{?pane_in_mode,[lif],#{pane_current_command}}#{?pane_dead,[dead],}"` | The format used for automatic names. | `automatic-rename-format` |
| `remain-on-exit` | choice: `"off"`, `"on"`, `"failed"` | `"off"` | Keep a pane open after its program exits (`failed`: only on a non-zero status). | `remain-on-exit` |
| `synchronize-panes` | bool | `false` | Send what is typed (keys, pastes, `send-keys`) to every pane of the window. Usually set for one window: `set -w synchronize-panes on`. | `synchronize-panes` |
| `bell-action` | choice: `"none"`, `"any"`, `"current"`, `"other"` | `"any"` | Which windows' bells are passed on. | `bell-action` |
| `visual-bell` | choice: `"off"`, `"on"`, `"both"` | `"off"` | Show a bell as a message instead of (`on`) or as well as (`both`) ringing. | `visual-bell` |
| `monitor-bell` | bool | `true` | Flag windows that rang the bell. | `monitor-bell` |
| `monitor-activity` | bool | `false` | Flag windows with new output. | `monitor-activity` |
| `default-size` | string `"WxH"` | `"80x24"` | Size of sessions created without a client (`new-session -d`). | `default-size` |
| `window-size` | choice: `"largest"`, `"smallest"`, `"manual"`, `"latest"` | `"latest"` | How a window shown by clients of different sizes is sized. | `window-size` |
| `main-pane-width` | size | `80` | Main pane width for `main-vertical`. | `main-pane-width` |
| `main-pane-height` | size | `24` | Main pane height for `main-horizontal`. | `main-pane-height` |
| `other-pane-width` | size | `0` | Width of the other panes in `main-vertical` (0: what is left). | `other-pane-width` |
| `other-pane-height` | size | `0` | Height of the other panes in `main-horizontal`. | `other-pane-height` |
| `tiled-layout-max-columns` | integer | `0` | Most columns the `tiled` layout uses (0: no limit). | `tiled-layout-max-columns` |
| `pane-border-status` | choice: `"off"`, `"top"`, `"bottom"` | `"off"` | A line with each pane's `pane-border-format` on the border above or below it; the window gives up one row for it. | `pane-border-status` |
| `pane-border-format` | string | `'#{?pane_active,#[reverse],}#{pane_index}#[default] "#{pane_title}"'` | What that line shows (a format; `#[...]` styles work). | `pane-border-format` |
| `set-titles` | bool | `false` | Set the outer terminal's title. | `set-titles` |
| `set-titles-string` | string | `'#S:#I:#W - "#T"'` | The title, a format. | `set-titles-string` |
| `allow-passthrough` | choice: `"off"`, `"on"`, `"all"` | `"off"` | Unwrap tmux's passthrough escape (`ESC P tmux; ... ESC \`) from programs; the wrapped clipboard writes and notifications then reach the outer terminal. | `allow-passthrough` |
| `tmux-compat` | bool | `false` | New panes get `TMUX` and `TMUX_PANE` pointing at lif, and passthrough is unwrapped, for programs that only know tmux (see [claude-code.md](claude-code.md)). | `tmux-compat` (lif only) |

`status = false` (or `true`) at the top level is accepted as a shortcut for
`[status] enabled`, unless the file also has a `[status]` section (TOML
does not allow both).

Integers are checked against the option's range: `[status] interval = -5`
gives `value is too small: -5`.

## [status]

| Key | Type | Default | Effect | tmux option |
|---|---|---|---|---|
| `enabled` | bool | `true` | Show the status line. | `status` |
| `position` | choice: `"top"`, `"bottom"` | `"bottom"` | Where it is. | `status-position` |
| `justify` | choice: `"left"`, `"centre"`, `"right"`, `"absolute-centre"` | `"left"` | Where the window list goes. | `status-justify` |
| `interval` | integer (s) | `15` | Seconds between refreshes of a status line that uses `#(command)`, whose commands run again then; 0 never. (A clock is redrawn on the minute whatever this is.) | `status-interval` |
| `left` | string (format) | `"[#{session_name}] "` | Left part. | `status-left` |
| `left-length` | integer | `10` | Its maximum width in columns. | `status-left-length` |
| `right` | string (format) | `'#{?window_bigger,[#{window_offset_x}#,#{window_offset_y}] ,}"#{=21:pane_title}" %H:%M %d-%b-%y'` | Right part. | `status-right` |
| `right-length` | integer | `40` | Its maximum width. | `status-right-length` |
| `window-format` | string (format) | `"#I:#W#{?window_flags,#{window_flags}, }"` | How each window appears in the list. | `window-status-format` |
| `current-window-format` | string (format) | same as `window-format` | How the current window appears. | `window-status-current-format` |
| `separator` | string | `" "` | Between window entries. | `window-status-separator` |

Status formats expand strftime sequences (`%H:%M`) and can use
`#[fg=...,bg=...,bold]` to change style mid-string. `#(shell command)`
expands to the last line the command printed when it last ran (`/bin/sh
-c`; nothing until it has run once), and the command runs again every
`interval` seconds: `right = "#(uptime | sed 's/.*load/load/') %H:%M"`.
Alignment, fill, list and range directives
(`#[align=right]` and the like) are ignored in the status line and rejected
in `[theme]`.

## [theme]

lif has no named themes or theme files: `[theme]` sets the styles of the
interface, one key per element.

| Key | Default | What it styles | tmux option |
|---|---|---|---|
| `status` | `{ bg = "green", fg = "black" }` | The status line | `status-style` |
| `status-left` | `"default"` | Its left part | `status-left-style` |
| `status-right` | `"default"` | Its right part | `status-right-style` |
| `window` | `"default"` | Every window entry | `window-status-style` |
| `current-window` | `{ underline = true }` | The current window's entry | `window-status-current-style` |
| `last-window` | `"default"` | The previously current window's entry | `window-status-last-style` |
| `activity` | `{ reverse = true }` | Entries of windows with activity | `window-status-activity-style` |
| `bell` | `{ reverse = true }` | Entries of windows that rang the bell | `window-status-bell-style` |
| `border` | `"default"` | Pane borders | `pane-border-style` |
| `active-border` | `"fg=green"` | The border next to the active pane | `pane-active-border-style` |
| `selection` | `"bg=yellow,fg=black"` | Copy-mode selection and position indicator | `mode-style` |
| `message` | `"bg=yellow,fg=black"` | Messages and prompts | `message-style` |
| `match` | `"bg=cyan,fg=black"` | Copy-mode search matches | `copy-mode-match-style` |
| `current-match` | `"bg=magenta,fg=black"` | The match under the cursor | `copy-mode-current-match-style` |
| `pane` | `"default"` | Default colors of pane contents (only `fg` and `bg` are used) | `window-style` |
| `active-pane` | `"default"` | The same for the active pane, when not `"default"` | `window-active-style` |

A style is either a table or a tmux style string; both are equivalent:

```toml
[theme]
status = { fg = "#cdd6f4", bg = "#1e1e2e" }
current-window = "fg=black,bg=green,bold"
selection = { reverse = true, underline = "curly", underline-color = "red" }
```

Table keys:

| Key | Values |
|---|---|
| `fg`, `bg`, `underline-color` | `"black"`, `"red"`, `"green"`, `"yellow"`, `"blue"`, `"magenta"`, `"cyan"`, `"white"`, `"bright"` + any of those (`"brightred"`), `"colour0"`..`"colour255"` (or `color`), `"#rrggbb"`, `"default"` |
| `bold`, `dim`, `italic`, `blink`, `reverse`, `hidden`, `strikethrough`, `overline` | `true` / `false` |
| `underline` | `true`, `false`, `"single"`, `"double"`, `"curly"`, `"dotted"`, `"dashed"` |

In a style string, `fg=`, `bg=` and `us=` (underline color) take the same
colors, and the attributes are `bold`, `dim`, `italics`, `blink`,
`reverse`, `hidden`, `strikethrough`, `overline`, `underscore`,
`double-underscore`, `curly-underscore`, `dotted-underscore`,
`dashed-underscore`, `bright`, each also with a `no` prefix, plus `default`,
`none` and `noattr`. `"default"` (or `{}`) means the surrounding style: the
status line's for its parts, the terminal's for the status line itself.

## Key bindings

`[keys]` (or `[keys.prefix]`) binds keys pressed after the prefix,
`[keys.root]` keys pressed without it (they no longer reach programs),
`[keys.copy-mode]` and `[keys.copy-mode-vi]` keys in copy mode. Other table
names are rejected. Each entry is `KEY = VALUE`, where the value is:

| Value | Meaning | Becomes |
|---|---|---|
| `"split-window -h"` | a lif command string (tmux syntax) | `bind-key -T prefix KEY 'split-window -h'` |
| `["split-window -h", "select-layout even-horizontal"]` | several commands, run in order | `bind-key ... 'split-window -h ; select-layout even-horizontal'` |
| `{ command = "resize-pane -L 5", repeat = true }` | repeatable within `repeat-time` | `bind-key -r ...` |
| `{ command = "...", note = "text" }` | with a description, shown by `list-keys -N` | `bind-key -N text ...` |
| `false` or `"none"` | remove the default binding | `unbind-key -T TABLE KEY` |

Command strings are checked when the file is loaded: unknown commands, unknown
flags and commands lif does not implement are errors, reported at the
binding. Quote keys that are not plain words: `"|"`, `"-"`, `"C-a"`,
`"M-Left"`. Binding the same key twice in one table is an error, including
two spellings of one key (`"C-a"` and `"^A"`).

```toml
[keys]
"|" = "split-window -h -c '#{pane_current_path}'"
"-" = "split-window -v -c '#{pane_current_path}'"
h = { command = "select-pane -L", repeat = true }
j = { command = "select-pane -D", repeat = true }
k = { command = "select-pane -U", repeat = true }
l = { command = "select-pane -R", repeat = true }
x = "kill-pane"                                  # no confirmation
r = "source-file ~/.config/lif/config.toml"
"C-o" = false                                    # unbind rotate-window

[keys.root]
"M-Left" = "select-pane -L"
"M-Right" = "select-pane -R"

[keys.copy-mode-vi]
v = "send-keys -X begin-selection"
y = "send-keys -X copy-selection-and-cancel"
```

Mouse events cannot be bound; see [copy-mode.md](copy-mode.md#mouse).

### Key names

Key names are tmux's:

- A single character: `a`, `A`, `|`, `%`, `;`. Upper and lower case are
  different keys.
- Modifiers as prefixes, in any combination: `C-` (Ctrl), `M-` (Alt/Meta),
  `S-` (Shift): `C-a`, `M-Left`, `C-M-Up`, `S-F5`. `^A` is `C-a`.
  Modifier letters are case-insensitive.
- Named keys (case-insensitive): `Enter`, `Escape`, `Space`, `Tab`, `BTab`
  (Shift-Tab), `BSpace`, `Up`, `Down`, `Left`, `Right`, `Home`, `End`,
  `PPage`/`PageUp`/`PgUp`, `NPage`/`PageDown`/`PgDn`, `IC`/`Insert`,
  `DC`/`Delete`, `F1`..`F35`, keypad keys `KP0`..`KP9`, `KP/`, `KP*`,
  `KP-`, `KP+`, `KP.`, `KPEnter`.
- `C-i`, `C-m` and `C-[` are the same keys as `Tab`, `Enter` and `Escape`, as
  in tmux.

Modifier words such as `Ctrl-a` or `Alt-x` are rejected with a hint.

## Hooks

`[hooks]` sets the global hooks, what `set-hook -g` sets: commands lif runs
when something happens. Each entry is `HOOK = VALUE`:

| Value | Meaning | Becomes |
|---|---|---|
| `"select-layout tiled"` | a lif command string | `set-hook -g HOOK 'select-layout tiled'` |
| `["display-message one", "run-shell '~/bin/two'"]` | several, each run in turn | `set-hook -g HOOK ...`, then `set-hook -ga HOOK ...` for the others |
| `false` | no commands | `set-hook -gu HOOK` |

Hook names are tmux's (`lif show-hooks -g` and `lif show-hooks -gw` list
them); an unknown name is an error with a suggestion. The hooks lif runs,
and the `#{hook_...}` variables their commands can use, are listed in
[tmux-migration.md](tmux-migration.md#scripting). Reloading the file sets
each hook again from scratch.

```toml
[hooks]
after-split-window = "select-layout tiled"
session-created = "run-shell 'echo #{hook_session_name} >> ~/.lif-sessions'"
client-attached = ["display-message 'welcome back'", "refresh-client -S"]
```

## Environment

`[env]` sets variables for panes and shell commands started from then on
(`set-environment -g`). A string sets the variable; `false` removes it from
their environment (`set-environment -gr`). Other value types are errors, so
numbers are written as strings.

```toml
[env]
EDITOR = "nvim"
COLUMNS_HINT = "200"
SSH_ASKPASS = false
```

Variables set for one session (`lif set-environment -t work NAME value`)
and `-e NAME=value` of the commands that create panes come on top of these.

## Diagnostics

`lif check-config` prints one line per problem, `file:line:column: error:
message`, and the server writes the same lines to `show-messages` when it
loads the file. The column points at the offending key or value. A file with
typical mistakes:

```toml
prefix = "Ctrl-a"
mouse = "on"
scrollback = "1.5GB"
status-position = "top"
histroy-limit = 5000
mode-keys = "vim"

[status]
left-lenght = 20
justify = "center"

[theme]
status = { bg = "blak", fg = "white", bold = "yes" }
active-border = "fg=green,align=right"

[keys]
"|" = "split-window -h"
"\\" = "splt-window -h"
r = { command = "source-file ~/.config/lif/config.toml", repat = true }
"C-Lef" = "select-pane -L"

[keys.copy-mod]
v = "send-keys -X begin-selection"
```

```
$ lif check-config bad.toml
bad.toml:1:10: error: `prefix`: unknown key name `Ctrl-a`: write modifiers as C- (Ctrl), M- (Alt/Meta) and S- (Shift), e.g. `C-a`
bad.toml:2:9: error: `mouse`: expected true or false, found string "on" (write `mouse = true`)
bad.toml:3:14: error: `scrollback`: expected a size such as "32MB" (units B, KB, MB, GB; 1 KB = 1024 bytes), found string "1.5GB"
bad.toml:4:1: error: `status-position` is set as `position` in the [status] section
bad.toml:5:1: error: unknown option `histroy-limit`, did you mean `history-limit`?
bad.toml:6:13: error: `mode-keys`: unknown value "vim", did you mean "vi"? (expected one of: emacs, vi)
bad.toml:9:1: error: unknown key `left-lenght` in [status], did you mean `left-length`?
bad.toml:10:11: error: `status.justify`: unknown value "center", did you mean "centre"? (expected one of: left, centre, right, absolute-centre)
bad.toml:13:17: error: `theme.status`: unknown color "blak", did you mean "black"?
bad.toml:14:17: error: `theme.active-border`: unsupported style directive `align=` (lif styles take fg=, bg= and us=)
bad.toml:18:8: error: key `\` in [keys]: unknown command `splt-window`, did you mean `split-window`?
bad.toml:19:58: error: key `r` in [keys]: unknown binding field `repat`, did you mean `repeat`? (fields: command, repeat, note)
bad.toml:20:1: error: unknown key name `C-Lef`, did you mean `C-Left`?
bad.toml:22:7: error: unknown key table [keys.copy-mod], did you mean [keys.copy-mode]? (tables are: prefix, root, copy-mode, copy-mode-vi)
```

More examples:

```
$ lif check-config bad2.toml
bad2.toml:2:14: error: `base-index`: expected an integer, found string "1"
bad2.toml:3:1: error: `scrollback-bytes` is set as `scrollback` (bytes, or a size such as "32MB")
bad2.toml:5:16: error: `default-size`: invalid value: 80 by 24
bad2.toml:6:19: error: `main-pane-width`: invalid value: half
bad2.toml:8:2: error: unknown section [stauts], did you mean [status]?
bad2.toml:11:2: error: invalid TOML: duplicate key
bad2.toml:12:1: error: `status-left` is called `left` in [status]
bad2.toml:13:12: error: `status.interval`: value is too small: -5
bad2.toml:16:17: error: `theme.border`: unknown color "grey" (expected a color: a name (black, red, green, yellow, blue, magenta, cyan, white, or bright...), colour0 to colour255, #rrggbb, or default)
bad2.toml:17:51: error: `theme.selection`: `bold` expects true or false, found string "yes"
bad2.toml:18:32: error: `theme.current-window`: `underline` expects true, false, "single", "double", "curly", "dotted" or "dashed", found string "wavy"
bad2.toml:19:11: error: `theme.message`: expected a style table such as { fg = "black", bg = "green" } or a tmux style string, found integer 42
bad2.toml:25:1: error: key `^A` is bound twice in [keys] (it is the same key as the one on line 24)
bad2.toml:26:5: error: key `x` in [keys]: expected a command string, a list of commands, false to unbind, or { command = ..., repeat = true }, found integer 5
bad2.toml:27:5: error: key `z` in [keys]: a binding table needs `command`, e.g. { command = "resize-pane -L 5", repeat = true }
bad2.toml:28:5: error: key `v` in [keys]: empty list of commands
bad2.toml:29:1: error: unknown key name `Ctrl-x`: write modifiers as C- (Ctrl), M- (Alt/Meta) and S- (Shift), e.g. `C-x`
bad2.toml:30:1: error: unknown key name `Entr`, did you mean `Enter`?
bad2.toml:31:5: error: key `w` in [keys]: unsupported command: choose-tree
```

(`bad2.toml:11:2` is a second `status` key: line 4 set `status = "on"` at the
top level and line 11 opened a `[status]` section, which TOML does not allow.)

Broken TOML is reported with its position, and the settings on the damaged
line are not applied:

```
$ lif check-config broken.toml
broken.toml:1:14: error: invalid TOML: invalid basic string, expected `"`
```

Pointing `check-config`, `-f` or `source-file` at a tmux file:

```
$ lif check-config tmux.conf
tmux.conf: this looks like a tmux configuration file, but lif reads TOML; convert it with: lif migrate-tmux-conf tmux.conf
```
