//! Structured representation of a git diff, ready for rendering.
//!
//! The diff is stored as a flat list of lines (easy to render and scroll),
//! but every line keeps track of the file and hunk it belongs to. Those
//! indices will become the identity we use when stage/unstage of individual
//! hunks is implemented.

/// The kind of a single line in a diff.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum LineKind {
    /// `diff --git ...`, `index ...`, `---/+++` file header lines.
    FileHeader,
    /// `@@ -a,b +c,d @@` hunk header.
    HunkHeader,
    /// `+` added line.
    Addition,
    /// `-` removed line.
    Deletion,
    /// Context line (unchanged).
    Context,
    /// Meta lines such as "\ No newline at end of file" or binary markers.
    Meta,
}

/// One displayable line of the diff.
#[derive(Debug, Clone)]
pub struct DiffLine {
    pub kind: LineKind,
    /// Raw line content without the origin prefix (`+`/`-`/` `) and without
    /// the trailing newline. The UI adds the prefix back for display; the
    /// combination of `kind` + `content` is enough to rebuild the patch text.
    pub content: String,
    /// Index of the file (delta) this line belongs to.
    pub file_idx: usize,
    /// Index of the hunk within the file, if the line is inside a hunk.
    pub hunk_idx: Option<usize>,
}

/// A full diff as a flat list of lines.
#[derive(Debug, Default)]
pub struct DiffView {
    pub lines: Vec<DiffLine>,
    /// Number of files touched by the diff.
    pub file_count: usize,
}

impl DiffView {
    pub fn is_empty(&self) -> bool {
        self.lines.is_empty()
    }

    pub fn len(&self) -> usize {
        self.lines.len()
    }
}
