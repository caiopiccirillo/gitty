# Development

The code is split into small modules that mirror the app's structure:

| Path                | Purpose                                           |
| ------------------- | ------------------------------------------------- |
| `src/app/`          | Application state, key and mouse handling, cursor movement |
| `src/git/`          | Git operations on top of `gix`                    |
| `src/git/diff.rs`   | Producing per-file diffs from the repository      |
| `src/git/render.rs` | Turning those diffs into the display line model   |
| `src/git/staging.rs`| Index and worktree writes: staging, unstaging, discarding |
| `src/git/commit.rs` | Creating commits from the index                   |
| `src/git/splice.rs` | Rebuilds blob content from hunk material          |
| `src/diff.rs`       | The flat line model used for rendering            |
| `src/tree.rs`       | The collapsible file tree                         |
| `src/ui.rs`         | Rendering with ratatui                            |
| `src/refresh.rs`    | Background diff computation                       |
| `src/syntax.rs`     | Tree-sitter syntax highlighting (feature `syntax`) |
| `tests/`            | Integration tests that exercise real repositories |

## Building and testing

```sh
cargo build
cargo test
cargo test --features syntax
cargo clippy --all-targets -- -D warnings
cargo fmt --check
```

The `[lints.clippy]` section in `Cargo.toml` enables the pedantic lint set;
CI runs clippy with `-D warnings`, so the tree must stay warning-free.

## Design notes

gitoxide does not implement patch application (`git apply`), so hunk and
line staging does not build patch text and apply it to the index. Instead,
the new blob content is reconstructed directly from the hunk's raw
material: the hunk describes a region of the old content (index or HEAD)
plus the lines that replace it in the new content (worktree or index), and
the splice replaces one with the other, filtering by the selected lines.
Unit tests for that logic live in `src/git/splice.rs`, and the integration
tests in `tests/staging.rs` and `tests/discard.rs` cover the full flows.

Discarding is the mirror image of staging: the same splice, written to the
worktree file instead of the index, with the file type and mode restored
from the side being reverted to.
