//! The data model shared by diff production, rendering and staging:
//! one changed file with both sides' content and its hunks.

use gix::bstr::{BStr, ByteSlice};
use gix::diff::blob::unified_diff::{DiffLineKind, HunkHeader};
use gix::index::entry::Mode;
use gix::object::tree::EntryKind;

use crate::diff::FileStatus;

/// One changed file: its metadata, both sides' content and its hunks.
pub(super) struct FileDiff {
    /// Display path (the new path, or the old path for deletions).
    pub(super) path: String,
    pub(super) old_path: String,
    pub(super) new_path: String,
    pub(super) status: FileStatus,
    pub(super) old_id: Option<gix::ObjectId>,
    pub(super) new_id: Option<gix::ObjectId>,
    pub(super) old_mode: Option<Mode>,
    pub(super) new_mode: Option<Mode>,
    /// Old-side content in git format (index or HEAD blob).
    pub(super) old_data: Vec<u8>,
    pub(super) old_ends_with_newline: bool,
    /// New-side content in git format (worktree or index blob).
    pub(super) new_data: Vec<u8>,
    pub(super) new_ends_with_newline: bool,
    pub(super) binary: bool,
    pub(super) hunks: Vec<Hunk>,
}

impl FileDiff {
    pub(super) fn new(
        path: &BStr,
        old_id: Option<gix::ObjectId>,
        old_mode: Option<Mode>,
        new_id: Option<gix::ObjectId>,
        new_mode: Option<Mode>,
    ) -> Self {
        let status = classify(
            old_id.zip(old_mode).map(|(_, m)| m),
            new_id.zip(new_mode).map(|(_, m)| m),
        );
        let display = path.to_str_lossy().into_owned();
        FileDiff {
            path: display,
            old_path: path.to_str_lossy().into_owned(),
            new_path: path.to_str_lossy().into_owned(),
            status,
            old_id,
            new_id,
            old_mode,
            new_mode,
            old_data: Vec::new(),
            old_ends_with_newline: false,
            new_data: Vec::new(),
            new_ends_with_newline: false,
            binary: false,
            hunks: Vec::new(),
        }
    }
}

/// One hunk's raw material, with 1-based start positions in its header.
pub(super) struct Hunk {
    pub(super) header: HunkHeader,
    pub(super) lines: Vec<(DiffLineKind, Vec<u8>)>,
}

/// Swap a staged diff around so that the index is the old side, making the
/// hunk material directly applicable to the index.
pub(super) fn reversed(file: &FileDiff) -> FileDiff {
    let mut reversed = FileDiff {
        path: file.path.clone(),
        old_path: file.new_path.clone(),
        new_path: file.old_path.clone(),
        status: file.status,
        old_id: file.new_id,
        new_id: file.old_id,
        old_mode: file.new_mode,
        new_mode: file.old_mode,
        old_data: file.new_data.clone(),
        new_data: file.old_data.clone(),
        old_ends_with_newline: file.new_ends_with_newline,
        new_ends_with_newline: file.old_ends_with_newline,
        binary: file.binary,
        hunks: Vec::new(),
    };
    reversed.hunks = file
        .hunks
        .iter()
        .map(|h| Hunk {
            header: HunkHeader {
                before_hunk_start: h.header.after_hunk_start,
                before_hunk_len: h.header.after_hunk_len,
                after_hunk_start: h.header.before_hunk_start,
                after_hunk_len: h.header.before_hunk_len,
            },
            lines: h
                .lines
                .iter()
                .map(|(kind, content)| (flip(kind), content.clone()))
                .collect(),
        })
        .collect();
    reversed
}

fn flip(kind: &DiffLineKind) -> DiffLineKind {
    match kind {
        DiffLineKind::Context => DiffLineKind::Context,
        DiffLineKind::Add => DiffLineKind::Remove,
        DiffLineKind::Remove => DiffLineKind::Add,
    }
}

/// Classify the status of a file of the staged diff from its modes.
fn classify(prev_mode: Option<Mode>, new_mode: Option<Mode>) -> FileStatus {
    match (prev_mode, new_mode) {
        (None, Some(_)) => FileStatus::Added,
        (Some(_), None) => FileStatus::Deleted,
        (Some(prev), Some(new)) => {
            if (prev == Mode::SYMLINK) != (new == Mode::SYMLINK)
                || prev.is_submodule() != new.is_submodule()
            {
                FileStatus::TypeChange
            } else {
                FileStatus::Modified
            }
        }
        (None, None) => FileStatus::Untracked,
    }
}

/// Convert an index entry mode into the entry kind used by the diff pipeline.
pub(super) fn kind_of(mode: Mode) -> EntryKind {
    if mode == Mode::SYMLINK {
        EntryKind::Link
    } else {
        EntryKind::Blob
    }
}

pub(super) fn mode_from_fs(metadata: &gix::index::fs::Metadata, is_symlink: bool) -> Mode {
    if is_symlink {
        Mode::SYMLINK
    } else if metadata.is_executable() {
        Mode::FILE_EXECUTABLE
    } else {
        Mode::FILE
    }
}
