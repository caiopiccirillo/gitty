//! Structured representation of a git diff, ready for rendering.
//!
//! The diff is stored as a flat list of lines (easy to render and scroll),
//! but every line keeps track of the file and hunk it belongs to. Those
//! indices are the identity we use when staging/unstaging individual hunks.

use std::ops::Range;

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

/// Identity of a hunk: which file it belongs to and its index within the file.
/// Orders in display order (first by file, then by hunk).
#[derive(Debug, Clone, Copy, PartialEq, Eq, PartialOrd, Ord)]
pub struct HunkId {
    pub file_idx: usize,
    pub hunk_idx: usize,
}

/// What kind of change a file went through.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum FileStatus {
    Added,
    Deleted,
    Modified,
    Renamed,
    TypeChange,
}

/// Per-file metadata, parallel to the `file_idx` tags on the lines.
#[derive(Debug, Clone)]
pub struct FileInfo {
    pub path: String,
    pub status: FileStatus,
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

/// A full diff as a flat list of lines plus per-file metadata.
#[derive(Debug, Default)]
pub struct DiffView {
    pub lines: Vec<DiffLine>,
    pub files: Vec<FileInfo>,
}

impl DiffView {
    pub fn is_empty(&self) -> bool {
        self.lines.is_empty()
    }

    pub fn len(&self) -> usize {
        self.lines.len()
    }

    /// All hunks in display order.
    pub fn hunks(&self) -> Vec<HunkId> {
        let mut hunks = Vec::new();
        for line in &self.lines {
            if let Some(hunk_idx) = line.hunk_idx {
                let id = HunkId {
                    file_idx: line.file_idx,
                    hunk_idx,
                };
                if hunks.last() != Some(&id) {
                    hunks.push(id);
                }
            }
        }
        hunks
    }

    /// Range of line indices occupied by a hunk, starting at its `@@` header.
    pub fn hunk_line_range(&self, id: HunkId) -> Option<Range<usize>> {
        let belongs =
            |line: &DiffLine| line.file_idx == id.file_idx && line.hunk_idx == Some(id.hunk_idx);
        let start = self.lines.iter().position(&belongs)?;
        let end = self.lines[start..]
            .iter()
            .position(|line| !belongs(line))
            .map(|offset| start + offset)
            .unwrap_or(self.lines.len());
        Some(start..end)
    }

    /// Range of line indices occupied by a file, starting at its header.
    pub fn file_line_range(&self, file_idx: usize) -> Option<Range<usize>> {
        if file_idx >= self.files.len() {
            return None;
        }
        let start = self.lines.iter().position(|l| l.file_idx == file_idx)?;
        let end = self.lines[start..]
            .iter()
            .position(|l| l.file_idx != file_idx)
            .map(|offset| start + offset)
            .unwrap_or(self.lines.len());
        Some(start..end)
    }
}

/// Test fixture: two files, the first with one hunk, the second with two.
#[cfg(test)]
pub(crate) fn two_file_view() -> DiffView {
    let line = |kind: LineKind, file_idx: usize, hunk_idx: Option<usize>| DiffLine {
        kind,
        content: String::new(),
        file_idx,
        hunk_idx,
    };
    DiffView {
        lines: vec![
            line(LineKind::FileHeader, 0, None),
            line(LineKind::HunkHeader, 0, Some(0)),
            line(LineKind::Addition, 0, Some(0)),
            line(LineKind::FileHeader, 1, None),
            line(LineKind::HunkHeader, 1, Some(0)),
            line(LineKind::Context, 1, Some(0)),
            line(LineKind::HunkHeader, 1, Some(1)),
            line(LineKind::Deletion, 1, Some(1)),
        ],
        files: vec![
            FileInfo {
                path: "a.txt".into(),
                status: FileStatus::Modified,
            },
            FileInfo {
                path: "b.txt".into(),
                status: FileStatus::Modified,
            },
        ],
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn lists_hunks_in_display_order() {
        let view = two_file_view();
        assert_eq!(
            view.hunks(),
            vec![
                HunkId {
                    file_idx: 0,
                    hunk_idx: 0
                },
                HunkId {
                    file_idx: 1,
                    hunk_idx: 0
                },
                HunkId {
                    file_idx: 1,
                    hunk_idx: 1
                },
            ]
        );
    }

    #[test]
    fn reports_hunk_line_ranges() {
        let view = two_file_view();
        assert_eq!(
            view.hunk_line_range(HunkId {
                file_idx: 0,
                hunk_idx: 0
            }),
            Some(1..3)
        );
        assert_eq!(
            view.hunk_line_range(HunkId {
                file_idx: 1,
                hunk_idx: 0
            }),
            Some(4..6)
        );
        assert_eq!(
            view.hunk_line_range(HunkId {
                file_idx: 1,
                hunk_idx: 1
            }),
            Some(6..8)
        );
        assert_eq!(
            view.hunk_line_range(HunkId {
                file_idx: 9,
                hunk_idx: 0
            }),
            None
        );
    }

    #[test]
    fn reports_file_line_ranges() {
        let view = two_file_view();
        assert_eq!(view.file_line_range(0), Some(0..3));
        assert_eq!(view.file_line_range(1), Some(3..8));
        assert_eq!(view.file_line_range(2), None);
    }
}
