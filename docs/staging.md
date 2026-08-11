# Staging and discarding

Everything gitty does can be understood as moving a change between three
places: the worktree, the index and `HEAD`. The unstaged view compares the
worktree against the index; the staged view compares the index against
`HEAD`.

## Staging and unstaging

- **Files and directories**: select a row in the files pane and press
  `Space` (or `d` to discard).
- **Hunks**: with the diff pane focused, `s` stages the hunk under the
  cursor, `u` unstages it.
- **Lines**: start a visual selection with `v`, move the cursor, and press
  `s`/`u` to stage or unstage only the selected changed lines. Unselected
  additions are dropped and unselected deletions stay as context, the same
  way `git add -p` behaves.

Staging only touches the index. The working tree is never modified by
staging or unstaging.

## Discarding

`d` reverts changes you no longer want, and always asks for confirmation
(`y` to confirm, `n` or `Esc` to cancel) before anything is reverted:

- On the unstaged side, a hunk or file is restored to the index version.
- On the staged side, both the worktree and the index revert to `HEAD`,
  like `git checkout HEAD -- <path>` for that hunk or file.
- An untracked (or newly added) file is removed entirely.

Discarding is destructive: confirmed discards cannot be undone by gitty.

## Committing

With something staged, press `c`, type a message and press `Enter`. The
commit is created from the index on top of `HEAD` (or as the root commit
on an unborn branch), using the configured `user.name` and `user.email`.
