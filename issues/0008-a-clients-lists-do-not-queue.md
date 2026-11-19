# 0008: a client's command lists do not wait for each other

Status: open · Found: 2026-11-19 · Severity: low (order of side effects of
waiting commands, e.g. `run-shell`, run from keys in quick succession)

## What happened

Writing the end-to-end test of `command-prompt -i` (0.5.0), each change of
the prompt ran `run-shell 'printf "[%%]\n" >> log'`. Typing `a`, `b`,
BSpace gave, in one run,

```
[=]
[=a]
[=a]
[=ab]
```

The four commands ran in order, but each `run-shell` parks its list and
its shell command runs on its own, so they finish in any order.

## Expected

tmux 3.8, the same keys (`command-prompt -i`, driven through a real
client in a PTY): `[=] [=a] [=ab] [=a]`, in order. In tmux each client
has one command queue: a `run-shell` that waits blocks the commands the
same client queues after it, so they run one after the other.

## Hypotheses

lif parks each list on its own (`lif-core/src/queue.rs`); nothing ties a
list to those of the same client queued before it.

## Root cause

As above (by reading the code; the tmux side by observation).

## Fix

None yet. A per-client order would park a new list of a client while an
earlier list of the same client is parked (`WaitKey` for "the previous
list of client N"), and wake it when that one finishes.

## What is not fixed

All of it. The `command-prompt` test records with `set -ga` instead.

## Measured cost

None.
