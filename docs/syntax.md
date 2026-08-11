# Syntax highlighting

Build with the `syntax` feature to color code tokens inside diff lines
with tree-sitter:

```sh
cargo install gitty-tui --features syntax
```

Languages are inferred from the file extension; Rust, Python and JSON are
currently supported. Keywords are cyan, strings yellow, comments gray,
numbers magenta and types light blue. Lines without a known language keep
the plain diff colors.

Two caveats:

- Each line is parsed on its own, so tokens inside multi-line constructs
  (block comments, raw strings) are only colored on the line they start
  on.
- The feature is off by default because the tree-sitter grammars are
  compiled from C, which adds noticeably to the build time.
