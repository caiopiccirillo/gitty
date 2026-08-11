# Usage

Run gitty from inside a repository, or pass a path to a repository or any
directory inside one:

```sh
gitty
gitty ~/projects/my-repo
gitty ~/projects/my-repo/src
```

The left pane lists the changed files (directories first, collapsible).
The right pane shows the diff of the selected file, with a line cursor. A
status bar at the bottom shows which side is focused, the selected file,
and the hunk under the cursor.

## Basic workflow

1. Start gitty in your repository.
2. Use `j`/`k` to pick a file in the left pane and press `Enter` to open
   its diff.
3. Move the cursor to a hunk and press `s` to stage it, or press `v` to
   select individual lines first.
4. Press `Tab` to review the staged changes, and `u` to unstage a hunk or
   file if you change your mind.
5. Press `c` to write a commit message, then `Enter` to commit.

The working tree is never modified by staging: gitty only touches the
index, so your files on disk stay exactly as they are. Discarding is the
exception and always asks for confirmation first (see
[Staging and discarding](./staging.md)).

The diff refreshes automatically whenever the repository changes on disk.
The computation happens on a background thread, so the interface stays
responsive even in large repositories.
