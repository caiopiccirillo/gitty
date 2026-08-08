# gitty

A terminal user interface for reviewing and staging git changes.

gitty shows your unstaged and staged diffs side by side in a two-pane
layout, and lets you stage or unstage whole files, individual hunks, or
single lines without leaving the terminal. It is written in Rust on top of
[gitoxide](https://github.com/gitoxideLabs/gitoxide), so it needs no `git`
binary and no C toolchain at build time.

## Features

- Two layouts, switchable with one key: the classic single diff pane, or a
  lazygit-style split with the staged and unstaged panes side by side.
- Unstaged and staged diff views, switchable with one key.
- A collapsible file tree with status badges (added, modified, deleted,
  type change, untracked); in the split layout the tree merges both sides
  and rows carry two-letter badges like `MM` or `??`.
- Stage or unstage entire files and directories.
- Stage or unstage individual hunks, or just the lines you select with the
  visual selection mode.
- Discard unwanted changes (hunks, lines, files or directories) with a
  confirmation prompt before anything is reverted.
- Optional tree-sitter syntax highlighting for Rust, Python and JSON code
  shown in diffs (keywords, strings, comments, numbers and types are
  colored). Off by default; build with `--features syntax` to enable it.
- Mouse support: click a file to select it, click a diff line to jump the
  cursor to the nearest change, and scroll with the wheel.
- Commit the staged changes from an integrated message box.
- The diff refreshes automatically when the repository changes on disk,
  computed on a background thread so the interface stays responsive.

## Experimental status and comparison

gitty is experimental. It started as a tool for exploring a
staging-focused workflow, and it shows: layouts, keys and internals change
as ideas get worked out, and it has none of the maturity of the
established clients.

For most people the honest advice is to use
[lazygit](https://github.com/jesseduffield/lazygit) or
[gitui](https://github.com/gitui-org/gitui). They are mature, widely used,
and cover far more than staging: history, branches, stashes, rebases and
remotes. gitty does none of that.

What gitty offers instead is one workflow done to the exclusion of
everything else:

- **The whole screen is the staging area.** In the split layout the files,
  the unstaged changes and the staged changes are visible side by side, and
  panes with nothing to show collapse away.
- **Staging matches `git add -p` muscle memory.** Hunks stage with a key,
  lines stage through a visual selection (`v`), and discards are confirmed
  before anything is reverted.
- **Runs anywhere git runs.** Everything is pure Rust on top of gitoxide,
  so gitty needs no `git` binary and compiles no C code (lazygit shells out
  to git, gitui builds libgit2).

If you are comfortable with experimental software and curious about this
workflow, gitty is worth a try. If you need something dependable, or
anything beyond staging, use lazygit or gitui.

## Installation

Requires a recent stable Rust toolchain (edition 2024).

```sh
git clone <your repository URL> gitty
cd gitty
cargo install --path .
```

This installs the `gitty` binary to `~/.cargo/bin`. To try it without
installing:

```sh
cargo run --release
```

## Usage

Run gitty from inside a repository, or pass a path to a repository or any
directory inside one:

```sh
gitty
gitty ~/projects/my-repo
gitty ~/projects/my-repo/src
```

The left pane lists the changed files (directories first, collapsible). The
right pane shows the diff of the selected file, with a line cursor. A status
bar at the bottom shows which side is focused, the selected file, and the
hunk under the cursor.

Two layouts are available, toggled with `m`:

- **Classic**: one diff pane at a time; `Tab` switches between the staged
  and unstaged views.
- **Split**: the unstaged pane in the middle with the staged pane to its
  right, sharing one file tree that merges both sides (`MM` means the file
  has both staged and unstaged changes). Panes with nothing to show are
  hidden, so the split collapses to `Files | Unstaged` while you stage and
  back to `Files | Staged` once everything is staged. `Tab` cycles the
  focus through the visible panes (files first, then left to right),
  skipping hidden ones, so after staging `Tab` lands in the staged pane
  where `u` unstages. Each pane keeps its own cursor. In the files pane,
  `Space` acts like lazygit: stage the selected file or directory, and
  press it again to unstage. The `s`/`u`/`d` diff keys act on the focused
  pane.

### Basic workflow

1. Start gitty in your repository.
2. Use `j`/`k` to pick a file in the left pane and press `Enter` to open its
   diff.
3. Move the cursor to a hunk and press `s` to stage it, or press `v` to
   select individual lines first.
4. Press `Tab` to review the staged changes, and `u` to unstage a hunk or
   file if you change your mind.
5. Press `c` to write a commit message, then `Enter` to commit.

The working tree is never modified by gitty: staging only touches the
index, so your files on disk stay exactly as they are.

### Key bindings

Global:

| Key            | Action                              |
| -------------- | ----------------------------------- |
| `q`, `Ctrl+C`  | Quit                                |
| `Tab`          | Classic: switch the shown side. Split: cycle the focus through the visible panes |
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
| `Space`                 | Classic: stage/unstage the selected file or directory. Split: toggle it (stage if it has unstaged changes, otherwise unstage) |
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

### Mouse

- Click a file row to select it.
- Click inside a diff pane to focus it and jump the cursor to the nearest
  changed line.
- Scroll the wheel over the files pane to move the selection, or over a
  diff pane to scroll it (the pane under the wheel gains focus).

Discarding reverts changes you no longer want: on the unstaged side a hunk or
file is restored to the index version, on the staged side to the `HEAD`
version (removing an untracked or newly added file entirely). It is
destructive, so gitty always asks for confirmation (`y`/`n`) first.

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
reconstructed directly from the hunk's raw material (see
`src/git/splice.rs`. Unit tests for that logic live in the same file, and
the integration tests in `tests/staging.rs` cover the full flows.
