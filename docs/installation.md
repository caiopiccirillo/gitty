# Installation

Requires a recent stable Rust toolchain (edition 2024).

```sh
cargo install gitty-tui
```

This installs the `gitty` binary to `~/.cargo/bin`. The crate is published
as `gitty-tui` because the name `gitty` is taken on crates.io.

To build from a checkout instead:

```sh
git clone <your repository URL> gitty
cd gitty
cargo install --path .
```

To try it without installing:

```sh
cargo run --release
```

## Optional features

- `syntax`: tree-sitter syntax highlighting for code shown in diffs. Off
  by default to keep the build light, since the grammars are compiled from
  C. Build with `cargo install gitty-tui --features syntax`.
