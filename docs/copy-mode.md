# Copy mode

Copy mode freezes a pane's view so you can scroll back through its history,
search it, select text and copy it. lif's copy mode uses tmux's commands
(`send-keys -X`) and tmux 3.8's default key tables, so the keys you know from
tmux work the same way.

## Entering and leaving

| How | What it does |
|---|---|
| `prefix [` | `copy-mode`: enter at the bottom of the history |
| `prefix PPage` | `copy-mode -u`: enter and scroll one page up |
| Mouse wheel up over a pane | enter and scroll up; leaves copy mode when you scroll back to the bottom (`copy-mode -e`) |
| Mouse drag in a pane | enter and start a selection where the button went down; on release copy it and leave (see [Mouse](#mouse)) |
| Double / triple click in a pane | copy the word / line under the pointer (entering and leaving copy mode) |
| `lif copy-mode -t TARGET` | from a script or the command prompt; `-u` page up, `-d` page down, `-e` exit at the bottom, `-H` hide the position indicator, `-q` leave any mode |
| `q`, `C-c` (vi and emacs), `Escape` (emacs) | leave copy mode |

While a pane is in copy mode its window name shows `[lif]` (the
`automatic-rename-format` default) and the top right corner shows the
position, `[offset/history]` (`P` toggles it). Output that arrives meanwhile
is kept; the view stays where it is.

Which key table applies is set by the `mode-keys` option: `emacs` (the
`copy-mode` table) by default, or `vi` (the `copy-mode-vi` table) if
`$VISUAL` or `$EDITOR` contains `vi` when the server starts. Set it
explicitly in the configuration file with `mode-keys = "vi"`.

## Moving

| | vi | emacs |
|---|---|---|
| Character, line | `h` `j` `k` `l`, arrows | `C-b` `C-f` `C-p` `C-n`, arrows |
| Word forward / back / end | `w` / `b` / `e` (`W` `B` `E` for space-separated words) | `M-f` (to word end) / `M-b` |
| Start / end of line, first non-blank | `0` / `$` / `^` | `C-a` / `C-e` / `M-m` |
| Page up / down | `C-b` / `C-f` | `M-v` or `PPage` / `C-v`, `Space` or `NPage` |
| Half page up / down | `C-u` / `C-d` | `M-Up` / `M-Down` |
| Scroll one line without moving the cursor | `C-y` `K` / `C-e` `J` | `C-Up` / `C-Down` |
| Top / bottom of history | `g` / `G` | `M-<` / `M->` |
| Top, middle, bottom of screen | `H` `M` `L` | `M-R`, `M-r` |
| Paragraph back / forward | `{` / `}` | `M-{` / `M-}` |
| Matching bracket | `%` | `C-M-b` / `C-M-f` |
| Jump to a character (forward, back, before, after) | `f` `F` `t` `T`, repeat `;`, reverse `,` | same keys |
| Go to line | `:` | `g` |
| Set mark / jump to it | `X` / `M-x` | `X` / `M-x` |
| Previous / next shell prompt | (unbound) | `C-M-Up` / `C-M-Down` |
| Repeat count | `1`..`9` | `M-1`..`M-9` |

`previous-prompt` and `next-prompt` rely on the shell marking its prompts
(OSC 133), as in tmux.

## Selecting

| | vi | emacs |
|---|---|---|
| Start selection | `Space` | `C-Space` |
| Select the whole line | `V` | |
| Rectangle selection | `v` or `C-v` (toggle) | `R` (toggle) |
| Move the other end | `o` | |
| Clear selection | `Escape` | `C-g` |

## Searching

| | vi | emacs |
|---|---|---|
| Search down / up | `/` / `?` | `C-s` / `C-r` (incremental: matches update as you type) |
| Next / previous match | `n` / `N` | `n` / `N` |
| Search for the word under the cursor | `*` (down), `#` (up) | |

All matches in view are highlighted (`[theme] match`), the current one with
`[theme] current-match`. Searches wrap around unless `wrap-search = false`.
`search-forward` / `search-backward` treat the pattern as a regular
expression (`l.ne [34]` finds `line 3`); the `-text` variants search for the
literal text. Long histories are searched in bounded steps, so a search never
blocks the server; pressing a key cancels a search still running.

## Copying and pasting

| | vi | emacs |
|---|---|---|
| Copy the selection and leave | `Enter` or `C-j` | `M-w` or `C-w` |
| Append the selection to the newest buffer and leave | `A` | |
| Copy to the end of the line and leave | `D` | `C-k` |
| Paste the newest buffer (outside copy mode) | `prefix ]` | `prefix ]` |

A copy goes to a new paste buffer (`buffer0000`, `buffer0001`, ...; at
most `buffer-limit` are kept) and, unless `set-clipboard` is `off`, to the
outer terminal's clipboard with OSC 52. lif sends it to every attached
terminal whether or not it detected OSC 52 support; a terminal that does not
support it (or has it disabled) ignores it, and the text is still in the lif
buffer.

To hand copies to a program as well (the system clipboard through `pbcopy`,
`wl-copy` or `xclip`), use `copy-pipe` with a command, as in tmux:

```toml
[keys.copy-mode-vi]
y = "send-keys -X copy-pipe-and-cancel pbcopy"
```

or set `copy-command = "pbcopy"` (`set -s copy-command pbcopy`): it receives
every copy made by `copy-pipe` without a command of its own, which includes
the default copy keys (`Enter`, `C-j`, `D` in vi; `M-w`, `C-w`, `C-k` in
emacs) and mouse copies. `copy-selection` and the other `copy-*` commands do
not pipe. The command runs as `/bin/sh -c COMMAND` in the pane's directory
with the text on its standard input and its output discarded; the server
does not wait for it (at most 8 such commands may still be reading at once;
more are refused with a message).

Buffers: `lif list-buffers`, `lif show-buffer [-b NAME]`, `lif set-buffer
[-a] [-b NAME] [-n NEWNAME] DATA`, `lif delete-buffer [-b NAME]` (`prefix -`),
`lif paste-buffer [-b NAME] [-d] [-p] [-r] [-s SEP] [-t PANE]`. The default
`prefix ]` pastes with `-p`: bracketed if the program in the pane asked for
bracketed paste. `prefix #` lists the buffers. `capture-pane` copies a
pane's text into a buffer without copy mode (`lif capture-pane -p -S -` prints
the whole history).

## Mouse

<a id="mouse"></a>With `mouse` on (the default):

- Wheel up over a pane enters copy mode and scrolls 5 lines per notch; wheel
  down scrolls back, and copy mode entered with the wheel ends when it reaches
  the bottom (until you click or use a key that is not a scroll). In a full-screen
  program that does not use the mouse (for example `less`), the wheel sends
  Up/Down arrow keys instead (3 per notch).
- Dragging with the left button selects text, entering copy mode if needed.
  Holding the pointer at the top or bottom edge scrolls. On release the
  selection is copied (buffer, OSC 52 and `copy-command`) and copy mode
  ends, as in tmux.
- A double click selects and copies a word, a triple click a line, in copy
  mode or not (outside it, the pane enters copy mode for the selection and
  leaves it again).
- The option `mouse-drag-end` changes what the end of a selection does:
  `copy-pipe-and-cancel` (the default) copies and leaves copy mode,
  `copy-pipe-no-clear` copies and stays with the selection shown, `none`
  only selects (copy with the keyboard). It is what tmux users set by
  rebinding `MouseDragEnd1Pane`.
- Programs that turn on mouse reporting (vim, htop, ...) get the mouse events
  instead.

Outside copy mode these are tmux's mouse keys in the root table
(`WheelUpPane`, `MouseDrag1Pane`, `DoubleClick1Pane`, ...), which can be
bound, rebound and unbound; see the
[migration guide](tmux-migration.md#mouse). Inside copy mode the mouse is
built into the server: the `copy-mode` and `copy-mode-vi` tables have no
mouse entries, and a mouse key bound there is never used.

## Commands

Every key above runs a `send-keys -X` command, which you can also bind or
run yourself, for example `lif send-keys -t work:1 -X history-top`. lif
implements these (tmux's names):

`append-selection`, `append-selection-and-cancel`, `back-to-indentation`, `begin-selection`, `bottom-line`, `cancel`, `clear-selection`, `copy-end-of-line`, `copy-end-of-line-and-cancel`, `copy-line`, `copy-line-and-cancel`, `copy-selection`, `copy-selection-no-clear`, `copy-selection-and-cancel`, `cursor-down`, `cursor-down-and-cancel`, `cursor-left`, `cursor-right`, `cursor-up`, `cursor-centre-vertical`, `cursor-centre-horizontal`, `end-of-line`, `goto-line`, `halfpage-down`, `halfpage-down-and-cancel`, `halfpage-up`, `history-bottom`, `history-top`, `jump-again`, `jump-backward`, `jump-forward`, `jump-reverse`, `jump-to-backward`, `jump-to-forward`, `jump-to-mark`, `middle-line`, `next-matching-bracket`, `next-paragraph`, `next-prompt`, `next-word`, `next-word-end`, `next-space`, `next-space-end`, `other-end`, `page-down`, `page-down-and-cancel`, `page-up`, `previous-matching-bracket`, `previous-paragraph`, `previous-prompt`, `previous-word`, `previous-space`, `rectangle-on`, `rectangle-off`, `rectangle-toggle`, `recentre-top-bottom`, `refresh-now`, `scroll-bottom`, `scroll-down`, `scroll-down-and-cancel`, `scroll-exit-on`, `scroll-exit-off`, `scroll-exit-toggle`, `scroll-middle`, `scroll-top`, `scroll-up`, `search-again`, `search-backward`, `search-backward-incremental`, `search-backward-text`, `search-forward`, `search-forward-incremental`, `search-forward-text`, `search-reverse`, `select-line`, `select-word`, `set-mark`, `start-of-line`, `stop-selection`, `toggle-position`, `top-line`, and the `copy-pipe` family
(`copy-pipe`, `copy-pipe-no-clear`, `copy-pipe-and-cancel`,
`copy-pipe-line`, `copy-pipe-line-and-cancel`, `copy-pipe-end-of-line`,
`copy-pipe-end-of-line-and-cancel`), which copy like the matching
`copy-*` command and then pipe the text to their command argument, or
without one to the `copy-command` option (see above).

Key names sent with `send-keys` (without `-X`) to a pane in copy mode are
looked up in the copy-mode key table, as if typed: with vi keys,
`lif send-keys -t work V Enter` selects the line and copies it.

`send-keys -X` on a pane that is not in a mode fails with `not in a mode`.

## Default key tables

Generated from `lif list-keys -T copy-mode-vi` and `lif list-keys -T copy-mode`
(lif 0.4.0). Change them in the `[keys.copy-mode-vi]` and `[keys.copy-mode]`
sections of the configuration file or with `bind-key -T`.

### copy-mode-vi (mode-keys vi)

| Key | Command |
|---|---|
| `Enter` | `send-keys -X copy-pipe-and-cancel` |
| `Escape` | `send-keys -X clear-selection` |
| `Space` | `send-keys -X begin-selection` |
| `#` | `send-keys -FX search-backward -- "#{copy_cursor_word}"` |
| `$` | `send-keys -X end-of-line` |
| `%` | `send-keys -X next-matching-bracket` |
| `*` | `send-keys -FX search-forward -- "#{copy_cursor_word}"` |
| `,` | `send-keys -X jump-reverse` |
| `/` | `command-prompt -P -T search -p "(search down)" { send-keys -X search-forward -- "%%" }` |
| `0` | `send-keys -X start-of-line` |
| `1` | `command-prompt -NP -I 1 -p (repeat) { send-keys -N "%%" }` |
| `2` | `command-prompt -NP -I 2 -p (repeat) { send-keys -N "%%" }` |
| `3` | `command-prompt -NP -I 3 -p (repeat) { send-keys -N "%%" }` |
| `4` | `command-prompt -NP -I 4 -p (repeat) { send-keys -N "%%" }` |
| `5` | `command-prompt -NP -I 5 -p (repeat) { send-keys -N "%%" }` |
| `6` | `command-prompt -NP -I 6 -p (repeat) { send-keys -N "%%" }` |
| `7` | `command-prompt -NP -I 7 -p (repeat) { send-keys -N "%%" }` |
| `8` | `command-prompt -NP -I 8 -p (repeat) { send-keys -N "%%" }` |
| `9` | `command-prompt -NP -I 9 -p (repeat) { send-keys -N "%%" }` |
| `:` | `command-prompt -P -p "(goto line)" { send-keys -X goto-line -- "%%" }` |
| `;` | `send-keys -X jump-again` |
| `?` | `command-prompt -P -T search -p "(search up)" { send-keys -X search-backward -- "%%" }` |
| `A` | `send-keys -X append-selection-and-cancel` |
| `B` | `send-keys -X previous-space` |
| `D` | `send-keys -X copy-pipe-end-of-line-and-cancel` |
| `E` | `send-keys -X next-space-end` |
| `F` | `command-prompt -1P -p "(jump backward)" { send-keys -X jump-backward -- "%%" }` |
| `G` | `send-keys -X history-bottom` |
| `H` | `send-keys -X top-line` |
| `J` | `send-keys -X scroll-down` |
| `K` | `send-keys -X scroll-up` |
| `L` | `send-keys -X bottom-line` |
| `M` | `send-keys -X middle-line` |
| `N` | `send-keys -X search-reverse` |
| `P` | `send-keys -X toggle-position` |
| `T` | `command-prompt -1P -p "(jump to backward)" { send-keys -X jump-to-backward -- "%%" }` |
| `V` | `send-keys -X select-line` |
| `W` | `send-keys -X next-space` |
| `X` | `send-keys -X set-mark` |
| `^` | `send-keys -X back-to-indentation` |
| `b` | `send-keys -X previous-word` |
| `e` | `send-keys -X next-word-end` |
| `f` | `command-prompt -1P -p "(jump forward)" { send-keys -X jump-forward -- "%%" }` |
| `g` | `send-keys -X history-top` |
| `h` | `send-keys -X cursor-left` |
| `j` | `send-keys -X cursor-down` |
| `k` | `send-keys -X cursor-up` |
| `l` | `send-keys -X cursor-right` |
| `n` | `send-keys -X search-again` |
| `o` | `send-keys -X other-end` |
| `q` | `send-keys -X cancel` |
| `r` | `send-keys -X refresh-now` |
| `t` | `command-prompt -1P -p "(jump to forward)" { send-keys -X jump-to-forward -- "%%" }` |
| `v` | `send-keys -X rectangle-toggle` |
| `w` | `send-keys -X next-word` |
| `z` | `send-keys -X scroll-middle` |
| `{` | `send-keys -X previous-paragraph` |
| `}` | `send-keys -X next-paragraph` |
| `BSpace` | `send-keys -X cursor-left` |
| `Home` | `send-keys -X start-of-line` |
| `End` | `send-keys -X end-of-line` |
| `NPage` | `send-keys -X page-down` |
| `PPage` | `send-keys -X page-up` |
| `Up` | `send-keys -X cursor-up` |
| `Down` | `send-keys -X cursor-down` |
| `Left` | `send-keys -X cursor-left` |
| `Right` | `send-keys -X cursor-right` |
| `M-x` | `send-keys -X jump-to-mark` |
| `C-b` | `send-keys -X page-up` |
| `C-c` | `send-keys -X cancel` |
| `C-d` | `send-keys -X halfpage-down` |
| `C-e` | `send-keys -X scroll-down` |
| `C-f` | `send-keys -X page-down` |
| `C-h` | `send-keys -X cursor-left` |
| `C-j` | `send-keys -X copy-pipe-and-cancel` |
| `C-u` | `send-keys -X halfpage-up` |
| `C-v` | `send-keys -X rectangle-toggle` |
| `C-y` | `send-keys -X scroll-up` |
| `C-Up` | `send-keys -X scroll-up` |
| `C-Down` | `send-keys -X scroll-down` |

### copy-mode (mode-keys emacs)

| Key | Command |
|---|---|
| `Escape` | `send-keys -X cancel` |
| `Space` | `send-keys -X page-down` |
| `,` | `send-keys -X jump-reverse` |
| `;` | `send-keys -X jump-again` |
| `F` | `command-prompt -1P -p "(jump backward)" { send-keys -X jump-backward -- "%%" }` |
| `N` | `send-keys -X search-reverse` |
| `P` | `send-keys -X toggle-position` |
| `R` | `send-keys -X rectangle-toggle` |
| `T` | `command-prompt -1P -p "(jump to backward)" { send-keys -X jump-to-backward -- "%%" }` |
| `X` | `send-keys -X set-mark` |
| `f` | `command-prompt -1P -p "(jump forward)" { send-keys -X jump-forward -- "%%" }` |
| `g` | `command-prompt -P -p "(goto line)" { send-keys -X goto-line -- "%%" }` |
| `n` | `send-keys -X search-again` |
| `q` | `send-keys -X cancel` |
| `r` | `send-keys -X refresh-now` |
| `t` | `command-prompt -1P -p "(jump to forward)" { send-keys -X jump-to-forward -- "%%" }` |
| `Home` | `send-keys -X start-of-line` |
| `End` | `send-keys -X end-of-line` |
| `NPage` | `send-keys -X page-down` |
| `PPage` | `send-keys -X page-up` |
| `Up` | `send-keys -X cursor-up` |
| `Down` | `send-keys -X cursor-down` |
| `Left` | `send-keys -X cursor-left` |
| `Right` | `send-keys -X cursor-right` |
| `M-1` | `command-prompt -NP -I 1 -p (repeat) { send-keys -N "%%" }` |
| `M-2` | `command-prompt -NP -I 2 -p (repeat) { send-keys -N "%%" }` |
| `M-3` | `command-prompt -NP -I 3 -p (repeat) { send-keys -N "%%" }` |
| `M-4` | `command-prompt -NP -I 4 -p (repeat) { send-keys -N "%%" }` |
| `M-5` | `command-prompt -NP -I 5 -p (repeat) { send-keys -N "%%" }` |
| `M-6` | `command-prompt -NP -I 6 -p (repeat) { send-keys -N "%%" }` |
| `M-7` | `command-prompt -NP -I 7 -p (repeat) { send-keys -N "%%" }` |
| `M-8` | `command-prompt -NP -I 8 -p (repeat) { send-keys -N "%%" }` |
| `M-9` | `command-prompt -NP -I 9 -p (repeat) { send-keys -N "%%" }` |
| `M-<` | `send-keys -X history-top` |
| `M->` | `send-keys -X history-bottom` |
| `M-R` | `send-keys -X top-line` |
| `M-b` | `send-keys -X previous-word` |
| `M-f` | `send-keys -X next-word-end` |
| `M-l` | `send-keys -X cursor-centre-horizontal` |
| `M-m` | `send-keys -X back-to-indentation` |
| `M-r` | `send-keys -X middle-line` |
| `M-v` | `send-keys -X page-up` |
| `M-w` | `send-keys -X copy-pipe-and-cancel` |
| `M-x` | `send-keys -X jump-to-mark` |
| `M-{` | `send-keys -X previous-paragraph` |
| `M-}` | `send-keys -X next-paragraph` |
| `M-Up` | `send-keys -X halfpage-up` |
| `M-Down` | `send-keys -X halfpage-down` |
| `C-Space` | `send-keys -X begin-selection` |
| `C-a` | `send-keys -X start-of-line` |
| `C-b` | `send-keys -X cursor-left` |
| `C-c` | `send-keys -X cancel` |
| `C-e` | `send-keys -X end-of-line` |
| `C-f` | `send-keys -X cursor-right` |
| `C-g` | `send-keys -X clear-selection` |
| `C-k` | `send-keys -X copy-pipe-end-of-line-and-cancel` |
| `C-l` | `send-keys -X recentre-top-bottom` |
| `C-n` | `send-keys -X cursor-down` |
| `C-p` | `send-keys -X cursor-up` |
| `C-r` | `command-prompt -Pi -I "#{pane_search_string}" -T search -p "(search up)" { send-keys -X search-backward-incremental -- "%%" }` |
| `C-s` | `command-prompt -Pi -I "#{pane_search_string}" -T search -p "(search down)" { send-keys -X search-forward-incremental -- "%%" }` |
| `C-v` | `send-keys -X page-down` |
| `C-w` | `send-keys -X copy-pipe-and-cancel` |
| `C-Up` | `send-keys -X scroll-up` |
| `C-Down` | `send-keys -X scroll-down` |
| `C-M-b` | `send-keys -X previous-matching-bracket` |
| `C-M-f` | `send-keys -X next-matching-bracket` |
| `C-M-Up` | `send-keys -X previous-prompt` |
| `C-M-Down` | `send-keys -X next-prompt` |

## Differences from tmux

- **Same keyboard tables.** Both tables are tmux 3.8's, except emacs `L`
  (`line-numbers-toggle`), which is left out because lif has no line-number
  gutter, and the mouse entries of these two tables (`MouseDown1Pane`,
  `MouseDrag1Pane`, `MouseDragEnd1Pane`, `WheelUpPane`, `WheelDownPane`,
  `DoubleClick1Pane`, `TripleClick1Pane`), which are built in (the root
  table's mouse keys are bindings, as in tmux). tmux's extra `C-[` entries are the
  same key as `Escape`.
- **The drag end is an option.** What a drag end does is the
  `mouse-drag-end` option instead of the `MouseDragEnd1Pane` binding; the
  default (copy and leave) is tmux's. After a double or triple click lif copies
  at once, where tmux shows the selection for 0.3 s first.
- **Copy-mode format variables** (`copy_cursor_x`, `scroll_position`,
  `selection_present`, `pane_search_string`, ...) are not available to
  `display-message` or other formats.
- **Bindings to other commands, run with `send-keys`.** A copy-mode key bound
  to commands copy mode does not run itself (`copy-pipe` with a command,
  `kill-pane`, ...) runs them after the rest of the `send-keys` command
  list, not in its place in the list.
