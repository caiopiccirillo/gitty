# gitiff

A terminal user interface for reviewing and staging git changes.

gitiff shows your unstaged and staged diffs side by side in a two-pane
layout, and lets you stage or unstage whole files, individual hunks, or
single lines without leaving the terminal. It is written in Rust on top of
[gitoxide](https://github.com/gitoxideLabs/gitoxide), so it needs no `git`
binary and no C toolchain at build time.

![gitiff screenshot](docs/screenshot.png)

<!-- Add a screenshot at docs/screenshot.png and it will appear above. -->

## Features

- Two layouts, switchable with one key: the classic single diff pane, or a
  lazygit-style split with the staged and unstaged panes side by side.
- Unstaged and staged diff views, switchable with one key.
- A collapsible file tree with status badges (added, modified, deleted,
  type change, untracked) — in the split layout the tree merges both sides
  and rows carry two-letter badges like `MM` or `??`.
- Stage or unstage entire files and directories.
- Stage or unstage individual hunks, or just the lines you select with the
  visual selection mode.
- Discard unwanted changes — hunks, lines, files or directories — with a
  confirmation prompt before anything is reverted.
- Commit the staged changes from an integrated message box.
- The diff refreshes automatically when the repository changes on disk,
  computed on a background thread so the interface stays responsive.

## Installation

Requires a recent stable Rust toolchain (edition 2024).

```sh
git clone <your repository URL> gitiff
cd gitiff
cargo install --path .
```

This installs the `gitiff` binary to `~/.cargo/bin`. To try it without
installing:

```sh
cargo run --release
```

## Usage

Run gitiff from inside a repository, or pass a path to a repository or any
directory inside one:

```sh
gitiff
gitiff ~/projects/my-repo
gitiff ~/projects/my-repo/src
```

The left pane lists the changed files (directories first, collapsible). The
right pane shows the diff of the selected file, with a line cursor. A status
bar at the bottom shows which side is focused, the selected file, and the
hunk under the cursor.

Two layouts are available, toggled with `m`:

- **Classic** — one diff pane at a time; `Tab` switches between the staged
  and unstaged views.
- **Split** — the staged and unstaged panes side by side, with a shared
  file tree that merges both sides (`MM` means the file has both staged and
  unstaged changes). `Tab` moves the focus between the panes; each pane
  keeps its own cursor, and the `s`/`u`/`d` keys act on the focused one.

### Basic workflow

1. Start gitiff in your repository.
2. Use `j`/`k` to pick a file in the left pane and press `Enter` to open its
   diff.
3. Move the cursor to a hunk and press `s` to stage it, or press `v` to
   select individual lines first.
4. Press `Tab` to review the staged changes, and `u` to unstage a hunk or
   file if you change your mind.
5. Press `c` to write a commit message, then `Enter` to commit.

The working tree is never modified by gitiff: staging only touches the
index, so your files on disk stay exactly as they are.

### Key bindings

Global:

| Key            | Action                              |
| -------------- | ----------------------------------- |
| `q`, `Ctrl+C`  | Quit                                |
| `Tab`          | Switch the focused side (and the shown pane in the classic layout) |
| `c`            | Open the commit message box         |
| `m`            | Toggle between the classic and split layouts |

Commit message box:

| Key                   | Action                       |
| --------------------- | ---------------------------- |
| `Enter`               | Commit                       |
| `Esc`                 | Cancel                       |
| `←` `→`, `Home`, `End` | Move the text cursor         |
| `Backspace`, `Ctrl+U` | Delete backwards / clear all |

Files pane:

| Key                     | Action                                   |
| ----------------------- | ---------------------------------------- |
| `j` `k` / `↓` `↑`       | Move the selection                       |
| `g` `G` / `Home` `End`  | Jump to the first / last row             |
| `Enter`                 | Expand or collapse a directory           |
| `Enter` on a file       | Open the file's diff                     |
| `l` / `→`               | Expand a collapsed directory, open a file|
| `h` / `←`               | Collapse a directory, move to its parent |
| `Space`                 | Stage (unstaged tab) / unstage (staged tab) the selected file or directory |
| `d`                     | Discard the selected file or directory (asks for confirmation) |

Diff pane:

| Key                     | Action                                   |
| ----------------------- | ---------------------------------------- |
| `j` `k` / `↓` `↑`       | Move to the next / previous changed line |
| `Ctrl+D` `Ctrl+U`       | Move down / up half a page               |
| `PgDn` `PgUp`           | Move down / up a page                    |
| `g` `G`                 | Jump to the first / last changed line    |
| `n` `p`                 | Jump to the next / previous hunk         |
| `v`                     | Start or end a visual line selection     |
| `s`                     | Stage the hunk (or the selected lines)   |
| `u`                     | Unstage the hunk (or the selected lines) |
| `d`                     | Discard the hunk (or the selected lines), with confirmation |
| `h` / `←`               | Back to the files pane                   |
| `Esc`                   | Cancel the selection, then back to files |

The cursor moves between changed lines only (`+`/`-`), and when you open a
file it lands on its first change. A visual selection (`v`) cannot leave its
hunk, so it always maps to a single, well-formed patch.

Discarding reverts changes you no longer want: on the unstaged tab a hunk or
file is restored to the index version, on the staged tab to the `HEAD`
version (removing an untracked or newly added file entirely). It is
destructive, so gitiff always asks for confirmation (`y`/`n`) first.

## Development

The code is split into small modules that mirror the app's structure:

| Path                | Purpose                                           |
| ------------------- | ------------------------------------------------- |
| `src/app/`          | Application state, key handling, cursor movement  |
| `src/git/`          | Git operations on top of `gix`                    |
| `src/git/diff.rs`   | Producing per-file diffs from the repository      |
| `src/git/render.rs` | Turning those diffs into the display line model   |
| `src/git/staging.rs`| Index writes: staging and unstaging files, hunks, lines |
| `src/git/commit.rs` | Creating commits from the index                   |
| `src/git/splice.rs` | Rebuilds index blob content from hunk material    |
| `src/diff.rs`       | The flat line model used for rendering            |
| `src/tree.rs`       | The collapsible file tree                         |
| `src/ui.rs`         | Rendering with ratatui                            |
| `src/refresh.rs`    | Background diff computation                       |
| `tests/`            | Integration tests that exercise real repositories |

Run the tests with:

```sh
cargo test
```

One design note for contributors: gitoxide does not implement patch
application (`git apply`), so hunk and line staging does not build patch
text and apply it to the index. Instead, the new index blob content is
reconstructed directly from the hunk's raw material — see
`src/git/splice.rs`. Unit tests for that logic live in the same file, and
the integration tests in `tests/staging.rs` cover the full flows.
