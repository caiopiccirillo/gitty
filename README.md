# gitty

A terminal user interface for reviewing and staging git changes.

gitty shows your unstaged and staged changes and lets you stage or unstage
whole files, individual hunks, or single lines without leaving the
terminal. It is written in Rust on top of
[gitoxide](https://github.com/gitoxideLabs/gitoxide), so it needs no `git`
binary and no C toolchain at build time.

## Features

- Classic and split layouts, switchable with one key
- Hunk, line, file and directory staging
- Discard changes with confirmation
- Commit from the TUI
- Mouse support
- Optional tree-sitter syntax highlighting
- Automatic refresh as the repository changes on disk

## Installation

Requires a recent stable Rust toolchain (edition 2024).

```sh
cargo install gitty-tui
```

This installs the `gitty` binary to `~/.cargo/bin` (the crate is published
as `gitty-tui` because `gitty` is taken on crates.io).

## Quick start

Run gitty inside a repository, or pass a path:

```sh
gitty
gitty ~/projects/my-repo/src
```

Pick a file with `j`/`k`, press `Enter` to open its diff, and `s` to stage
the hunk under the cursor. `m` switches between the classic and split
layouts, `c` opens the commit box, and `d` discards after confirmation.

## Documentation

The full documentation (key bindings, layouts, staging and discarding,
mouse, development) is an [mdBook](https://rust-lang.github.io/mdBook/)
in `docs/`. Serve it locally with:

```sh
mdbook serve
```

## Status

gitty is experimental. It is a staging tool, not a full git client; for
most purposes lazygit or gitui is the better choice. See the
[status chapter](docs/status.md) for the honest comparison.

## License

MIT
