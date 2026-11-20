# A pane's title could run commands through the prefix T prompt

Status: fixed in the working tree (not committed) · Found: 2026-11-20 (security review of 19af493) · Severity: medium (needs the user to press prefix `T`, then Enter)

## What happened

A program sets its pane's title with OSC 0/2, and `#T` gives it back
unfiltered. Prefix `T` was bound to

    command-prompt -I "#T" { select-pane -T "%%" }

`-I` expands `#T` into the prompt, and `%%` puts the answer into the
template as it is. A title that closes the quotes becomes more commands:

    $ printf '\033]2;a"; set -g @owned "yes\007'
    (prefix T, Enter)
    $ lif display-message -p '#{@owned}|#{pane_title}'
    yes|a

Reproduced with the test `pane_title_in_the_title_prompt_is_not_a_command`
(`crates/lif/tests/basic.rs`) before the fix. With `run-shell` in place of
`set` this is a shell command run as the user, from the output of whatever
runs in the pane (an ssh session, a build log, `cat` of a file). A long title
can keep the tail out of sight in the prompt.

## Expected

Whatever the prompt holds becomes the title, as one argument.

## Root cause

`expand_prompt_template` (`crates/lif-core/src/exec.rs`) escapes the answer
only for `%%%`, as tmux does, and the default templates used `%%`. The same
form was in the prompts for the session name (prefix `$`), the window name
(prefix `,`) and Rename in the window and session menus; those names are not
set by a pane's output, but they are offered for editing in the same way.

## Fix

Those templates use `%%%` (`crates/lif-core/src/bindings.rs`): prefix `$`,
`,`, `T`, and Rename in the `<` menu, the status line's window menu and
its session menu. `list-keys` shows them so; the migration guide counts them
as different from tmux 3.8.

## What is not fixed

- The other default prompts keep `%%` (prefix `'`, `.`, `/`, `f` and the
  copy-mode tables, which are compared with tmux's `list-keys`). They hold
  only what the user types (`#{pane_search_string}` is the user's last
  search), so there is nothing to inject through, but an answer with `"`
  in it is a syntax error or more commands, as in tmux.
- A binding of the user's own that passes `#T` through `command-prompt -I`
  into `%%` behaves as before; the changelog says so.
- Whether tmux 3.8 runs the same title as commands was not checked.

## Cost

None measured: seven characters in the default bindings.
