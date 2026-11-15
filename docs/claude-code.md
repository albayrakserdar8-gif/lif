# Using lif with Claude Code

Two things are covered here: running Claude Code inside a lif pane, and
letting Claude Code's agent teams open their teammates in lif panes, the way
they do in tmux.

## Claude Code in a lif pane

Nothing needs configuring. What tmux needs settings for is on in lif:

| What | tmux needs | lif |
|---|---|---|
| Shift+Enter for a new line | `extended-keys on` and `terminal-features` | Kitty keyboard protocol end to end when the outer terminal has it; otherwise Shift+Enter reaches the pane as `CSI 27;2;13~`, when the outer terminal sends it distinctly at all |
| Flicker-free redraws | tmux 3.6 and older have no synchronized output | Synchronized output (mode 2026) per pane, and towards the outer terminal when it supports it |
| Desktop notifications | `allow-passthrough on` | OSC 9, 777 and 99 from panes reach the outer terminal as OSC 9 |
| Copying (`/copy`, selections) | `set-clipboard`, passthrough | OSC 52 from panes reaches the outer terminal (`set-clipboard external`, the default) |
| Focus reporting | `focus-events on` | Always on |
| True color | `terminal-overrides ',*:Tc'` and `CLAUDE_CODE_TMUX_TRUECOLOR=1` | `COLORTERM=truecolor` in every pane; Claude Code does not see `$TMUX`, so it does not drop to 256 colors |
| Mouse wheel in the fullscreen renderer | `mouse on` | `mouse` is on by default |

Claude Code does not know `TERM_PROGRAM=lif`, so with its default
`preferredNotifChannel` (`auto`) it does not send desktop notifications.
Pick a channel lif passes on, in `~/.claude/settings.json`:

```json
{ "preferredNotifChannel": "ghostty" }
```

`"kitty"` (OSC 99) works too. The notification reaches the outer terminal
whichever pane or window it comes from; whether it is shown is up to that
terminal.

To see the conversation's title in the outer terminal's tab, turn on
`set-titles` (Claude Code sets the pane title):

```toml
# ~/.config/lif/config.toml
set-titles = true
set-titles-string = "#S: #T"
```

## Agent teams in lif panes

Claude Code's agent teams (`CLAUDE_CODE_EXPERIMENTAL_AGENT_TEAMS=1`) can show
each teammate in its own pane. The `tmux` backend decides it runs "inside
tmux" from `$TMUX`, and runs the program called `tmux` on `PATH`. lif can
stand in for both:

1. Install the shim, a link called `tmux` to lif, and put its directory
   first on `PATH` (in `~/.zshrc` or similar):

   ```sh
   lif install-tmux-shim            # makes ~/.local/share/lif/shim/tmux
   export PATH="$HOME/.local/share/lif/shim:$PATH"
   ```

   Inside a lif pane (`$LIF` set) `tmux` is then lif; anywhere else it
   runs the real tmux, the next `tmux` on `PATH`, so tmux itself keeps
   working. To run the real tmux inside a lif pane, use its full path or
   `LIF= tmux`.

2. Have lif give panes `$TMUX` and `$TMUX_PANE`:

   ```toml
   # ~/.config/lif/config.toml
   tmux-compat = true
   ```

   (or `lif set -s tmux-compat on`; it applies to panes started after it).
   With it, tmux's passthrough wrapper (`ESC P tmux; ... ESC \`), which
   Claude Code then uses for notifications and copying, is unwrapped.

3. Tell Claude Code to use split panes and, since it now believes it runs
   in tmux and would drop to 256 colors, to keep true color:

   ```json
   {
     "teammateMode": "tmux",
     "env": {
       "CLAUDE_CODE_EXPERIMENTAL_AGENT_TEAMS": "1",
       "CLAUDE_CODE_TMUX_TRUECOLOR": "1"
     }
   }
   ```

Started in a lif pane, Claude Code then splits that window (from the second
teammate on, the leader keeps 30% on the left, `main-vertical`); each
teammate gets a pane with its name on
the border above it (`pane-border-status top`) and its text in its own color
(`window-style` per pane). `kill-pane` closes a teammate's pane.

What the backend runs, all of which lif implements: `new-session`,
`new-window`, `split-window -d -P -F`, `list-panes -F`,
`display-message -p`, `select-layout`, `resize-pane -x 30%`,
`set-option -p` (`remain-on-exit`, `window-style`, `pane-border-style`,
`pane-active-border-style`, `pane-border-format`), `set-option -w
pane-border-status`, `select-pane -T`, `respawn-pane -k`, `kill-pane`,
`has-session`, `kill-session`, `show-options -g prefix`,
`load-buffer -w -` and `tmux -V` (answered `tmux 3.8 (lif 0.2.0)`).
`crates/lif/tests/claude_compat.rs` runs the same sequences.

### Differences from tmux you may notice

- A pane's own `pane-border-style` colors its label on the border line, not
  the border lines themselves (those use the window's style).
- `select-pane -T` takes the title as given; tmux expands formats in it.
- Programs that see `$TMUX` treat lif as tmux. Most only use it to decide
  how to wrap escape sequences, which lif unwraps; a program that needs a
  tmux feature lif lacks (control mode, popups, `pipe-pane`) fails with
  `unsupported command: NAME`.
