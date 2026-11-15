# 0005: `split-window -f -b` gave the new pane the smaller half

Status: fixed (0.4.0).

## What happened

Found while comparing `join-pane -f` with tmux 3.8. A full-size split
that puts the new pane first, with no size given, divided the window the
other way round from tmux:

```
$ lif new-session -d -s a -x 80 -y 24; lif split-window -h -t a; lif split-window -f -b -t a
$ lif list-panes -t a -F '#{pane_index} #{pane_left},#{pane_top} #{pane_width}x#{pane_height}'
0 0,0 80x11          (tmux 3.8: 0 0,0 80x12)
1 0,12 40x12         (tmux 3.8: 1 0,13 40x11)
2 41,12 39x12        (tmux 3.8: 2 41,13 39x11)
```

The same for `-f -h -b` (39 columns instead of 40). Present since
`split-window -f` was added in 0.3.0.

## Expected

tmux's halves: of an odd space to divide, the first pane gets the larger
half, whichever of the two is new.

## Root cause

`Layout::split_full` computed the new pane's size as tmux's `size2`
(the second pane's, the smaller half) without looking at `before`.
`Layout::split` already swapped the two for `before`.

## Fix

`split_full` gives the new pane the first pane's half when `before` is
set. Sizes given with `-l` were right and are unchanged.
`full_size_splits_before_and_join_pane_f` in `lif-core` holds tmux's
numbers for `split-window -f -b`, `-f -h -b` and `join-pane -f`
(new in 0.4.0); a third comparison with mixed sizes on a 91x33 window
matched tmux by hand.

## What is not fixed

Nothing known.

## Measured cost

None: one comparison when a pane is split.
