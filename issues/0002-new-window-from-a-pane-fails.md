# 0002: `lif new-window` typed in a pane fails with "index N in use"

Status: fixed in the working tree (not committed) · Found: 2026-11-15 · Severity: medium (a basic command fails from inside lif; scripts run in panes cannot open windows without `-t`)

## What happened

Typing `lif new-window` at the shell of a pane failed:

```
$ lif new-window -d; echo rc=$?
create window failed: index 0 in use
rc=1
```

The same command from the prefix key (`C-b c`), from the command prompt and
from a shell outside lif (`lif -L x new-window`) worked. Found while writing
hooks: a hook's commands run with the pane the event is about as their
current pane, the way a client inside a pane does, and
`set-hook -g after-new-window 'new-window -d'` logged the same error.

## Expected

tmux 3.8: `tmux new-window` from a pane's shell creates a window at the
next free index of that pane's session.

## Root cause

`Model::find_target` (`crates/lif-core/src/target.rs`) returns the current
state when a command has no `-t`. For a client that is not attached but
runs in a pane (`$LIF_PANE`), the current state is that pane, built by
`found_pane`, which fills in `Found::idx` with the index of the pane's
window. `idx` means "the window index the target named explicitly":
`new-window` refuses an explicit index that is taken, so it refused its own
window's. An attached client's current state comes from `found_session`,
which leaves `idx` empty, so keys and the prompt were not affected.

## Fix

With no target, `find_target` clears `idx`: nothing was named. Explicit
targets are unchanged (`new-window -t %0` and `-t :0` still report the
index in use, as tmux does).

Tests: `new_window_from_inside_a_pane_takes_a_free_index`
(`crates/lif-core/src/tests.rs`) and `new_window_from_a_shell_in_a_pane`
(`crates/lif/tests/scripting.rs`, typed into a real pane).

## Not fixed

Nothing known. `break-pane` and `move-window` read `idx` only from targets
given with `-t`, so they were not affected.

## Cost

None: one field cleared on a path that runs once per command.
