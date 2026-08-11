# Introduction

gitty is a terminal user interface for reviewing and staging git changes.

It shows your unstaged and staged changes, and lets you stage or unstage
whole files, individual hunks, or single lines without leaving the
terminal. It is written in Rust on top of
[gitoxide](https://github.com/gitoxideLabs/gitoxide), so it needs no `git`
binary and no C toolchain at build time.

## Features

- Two layouts, switchable with one key: the classic single diff pane, or a
  lazygit-style split with the staged and unstaged panes side by side.
- A collapsible file tree with status badges (added, modified, deleted,
  type change, untracked); in the split layout the tree merges both sides
  and rows carry two-letter badges like `MM` or `??`.
- Stage or unstage entire files and directories, individual hunks, or the
  lines you select with the visual selection mode.
- Discard unwanted changes (hunks, lines, files or directories) with a
  confirmation prompt before anything is reverted.
- Commit the staged changes from an integrated message box.
- Mouse support: click to select, click a diff line to jump the cursor,
  scroll with the wheel.
- Optional tree-sitter syntax highlighting for Rust, Python and JSON code
  shown in diffs.
- The diff refreshes automatically when the repository changes on disk,
  computed on a background thread so the interface stays responsive.

## The interface in one paragraph

Navigation is a three-level hierarchy: the file tree on the left, the
hunks of the selected entry, and a per-line cursor inside the diff. The
hunk under the cursor is the target of staging, unstaging and discarding.
The cursor moves between changed lines only (`+`/`-`), and when you open a
file it lands on its first change.

See [Status and comparison](./status.md) for an honest take on where gitty
stands relative to lazygit and gitui.
