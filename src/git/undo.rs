//! Undo support: snapshots of the index and worktree state of files,
//! taken before a mutation so the app can restore them (`z`).

use std::path::Path;

use anyhow::Result;
use gix::bstr::{BString, ByteSlice};
use gix::index::entry::Mode;

use super::model::mode_from_fs;
use super::open_repo;
use super::staging::{blob_content, owned_index, upsert_entry, worktree_content, write_worktree};

/// The index and worktree state of one file, taken before a mutation.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct FileSnapshot {
    pub path: String,
    /// The staged (index) blob content, or `None` when the file had no
    /// index entry.
    pub staged: Option<Vec<u8>>,
    /// The mode of the index entry, if any.
    pub staged_mode: Option<Mode>,
    /// The worktree file content, or `None` when the file did not exist.
    pub worktree: Option<Vec<u8>>,
    /// The worktree file mode, if the file existed.
    pub worktree_mode: Option<Mode>,
}

/// Capture the index and worktree state of the file at `path`.
///
/// # Errors
/// Returns an error if the repository cannot be opened.
pub fn snapshot_file(repo_path: &Path, path: &str) -> Result<FileSnapshot> {
    let repo = open_repo(repo_path)?;
    let rela = BString::from(path);
    let index = repo.index_or_empty()?;
    let (staged, staged_mode) = match index.entry_index_by_path(rela.as_bstr()) {
        Ok(idx) => {
            let entry = &index.entries()[idx];
            (Some(blob_content(&repo, entry.id)?), Some(entry.mode))
        }
        Err(_) => (None, None),
    };
    let worktree = worktree_content(&repo, path);
    let worktree_mode = worktree.as_ref().and_then(|_| {
        let workdir = repo.workdir()?;
        let full = workdir.join(gix::path::from_bstr(rela.as_bstr()));
        let metadata = gix::index::fs::Metadata::from_path_no_follow(&full).ok()?;
        Some(mode_from_fs(&metadata, full.is_symlink()))
    });
    Ok(FileSnapshot {
        path: path.to_string(),
        staged,
        staged_mode,
        worktree,
        worktree_mode,
    })
}

/// Restore the index and worktree state captured in `snapshots`.
///
/// # Errors
/// Returns an error if the repository cannot be opened or a state cannot
/// be written back.
pub fn restore_files(repo_path: &Path, snapshots: &[FileSnapshot]) -> Result<()> {
    let repo = open_repo(repo_path)?;
    let mut index = owned_index(&repo)?;
    for snap in snapshots {
        let rela = BString::from(snap.path.as_str());
        // Worktree: write the old content back, or delete the file when
        // it did not exist before the mutation.
        match &snap.worktree {
            Some(content) => {
                write_worktree(&repo, &snap.path, content, true, snap.worktree_mode)?;
            }
            None => write_worktree(&repo, &snap.path, b"", false, snap.worktree_mode)?,
        }
        // Index: update, insert or remove the entry.
        match &snap.staged {
            Some(content) => {
                let blob_id = repo.write_blob(content)?.detach();
                let stat = gix::index::entry::Stat::default();
                let mode = snap
                    .staged_mode
                    .or(snap.worktree_mode)
                    .unwrap_or(Mode::FILE);
                upsert_entry(&mut index, rela.as_bstr(), blob_id, mode, stat);
            }
            None => {
                if let Ok(idx) = index.entry_index_by_path(rela.as_bstr()) {
                    index.remove_entry_at_index(idx);
                }
            }
        }
    }
    index.write(gix::index::write::Options::default())?;
    Ok(())
}
