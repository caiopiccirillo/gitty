//! Index writes: staging and unstaging files and hunks.
//!
//! gitoxide has no patch application, so staging a hunk rebuilds the new
//! index blob content directly from the hunk material (see [`splice`]) and
//! writes the resulting blob into the index.

use std::path::Path;

use anyhow::{Context, Result};
use gix::bstr::{BStr, BString, ByteSlice};
use gix::index::entry::Mode;

use crate::diff::{FileInfo, FileStatus, SelectedLines};

use super::diff::{staged_diff, workdir_diff};
use super::model::{FileDiff, Hunk, mode_from_fs, reversed};
use super::open_repo;
use super::splice::{self, Selection};

/// Stage a single hunk of the unstaged diff by rebuilding the hunk's region
/// of the index blob from the workdir side.
pub fn stage_hunk(path: &Path, file_idx: usize, hunk_idx: usize) -> Result<()> {
    let repo = open_repo(path)?;
    let file = &workdir_diff(&repo)?[file_idx];
    let hunk = &file.hunks[hunk_idx];
    let removed = worktree_file_missing(&repo, BString::from(file.path.as_str()).as_ref());
    apply_to_index(
        &repo,
        file,
        hunk,
        &Selection {
            keep_removes: &|_| false,
            keep_adds: &|_| true,
        },
        removed,
    )
}

/// Unstage a single hunk of the staged diff by applying the matching hunk of
/// the *reverse* diff (index vs. HEAD) to the index.
pub fn unstage_hunk(path: &Path, file_idx: usize, hunk_idx: usize) -> Result<()> {
    let repo = open_repo(path)?;
    let file = &reversed(&staged_diff(&repo)?[file_idx]);
    let hunk = &file.hunks[hunk_idx];
    apply_to_index(
        &repo,
        file,
        hunk,
        &Selection {
            keep_removes: &|_| false,
            keep_adds: &|_| true,
        },
        file.new_id.is_none(),
    )
}

/// Stage only the selected changed lines of a hunk (unselected `+` lines are
/// dropped, unselected `-` lines stay as context).
pub fn stage_lines(
    path: &Path,
    file_idx: usize,
    hunk_idx: usize,
    selected: &SelectedLines,
) -> Result<()> {
    let repo = open_repo(path)?;
    let file = &workdir_diff(&repo)?[file_idx];
    let hunk = &file.hunks[hunk_idx];
    let removed = worktree_file_missing(&repo, BString::from(file.path.as_str()).as_ref());
    apply_to_index(
        &repo,
        file,
        hunk,
        &Selection {
            keep_removes: &|i| !selected.deletions.contains(&i),
            keep_adds: &|i| selected.additions.contains(&i),
        },
        removed,
    )
}

/// Unstage only the selected changed lines of a staged hunk. The reverse
/// diff swaps the roles of `+` and `-`, so the selection is swapped too.
pub fn unstage_lines(
    path: &Path,
    file_idx: usize,
    hunk_idx: usize,
    selected: &SelectedLines,
) -> Result<()> {
    let repo = open_repo(path)?;
    let file = &reversed(&staged_diff(&repo)?[file_idx]);
    let hunk = &file.hunks[hunk_idx];
    let swapped = SelectedLines {
        additions: selected.deletions.clone(),
        deletions: selected.additions.clone(),
    };
    apply_to_index(
        &repo,
        file,
        hunk,
        &Selection {
            keep_removes: &|i| !swapped.deletions.contains(&i),
            keep_adds: &|i| swapped.additions.contains(&i),
        },
        file.new_id.is_none(),
    )
}

/// A fresh, mutable copy of the index (creating an empty one if none exists).
pub(super) fn owned_index(repo: &gix::Repository) -> Result<gix::index::File> {
    Ok((**repo.index_or_empty()?).clone())
}

/// Stage a whole file (like `git add <path>`). A deleted file is staged by
/// removing it from the index.
pub fn stage_file(path: &Path, file: &FileInfo) -> Result<()> {
    let repo = open_repo(path)?;
    let workdir = repo.workdir().context("repository has no worktree")?;
    let mut index = owned_index(&repo)?;
    let rela = BString::from(file.path.as_str());
    match file.status {
        FileStatus::Deleted => {
            if let Ok(idx) = index.entry_index_by_path(rela.as_bstr()) {
                index.remove_entry_at_index(idx);
            }
        }
        _ => {
            let full = workdir.join(gix::path::from_bstr(rela.as_bstr()));
            let (content, is_symlink) = if full.is_symlink() {
                (gix::path::into_bstr(full.read_link()?).to_vec(), true)
            } else {
                (std::fs::read(&full).with_context(|| format!("cannot read {}", full.display()))?, false)
            };
            let id = repo.write_blob(&content)?.detach();
            let metadata = gix::index::fs::Metadata::from_path_no_follow(&full)?;
            let stat = gix::index::entry::Stat::from_fs(&metadata)?;
            let mode = mode_from_fs(&metadata, is_symlink);
            upsert_entry(&mut index, rela.as_bstr(), id, mode, stat);
        }
    }
    index.write(gix::index::write::Options::default())?;
    Ok(())
}

/// Unstage a whole file (like `git reset HEAD -- <path>`). On an unborn
/// branch there is no HEAD entry to restore, so the index entry is simply
/// dropped and the file becomes untracked again.
pub fn unstage_file(path: &Path, file: &FileInfo) -> Result<()> {
    let repo = open_repo(path)?;
    let mut index = owned_index(&repo)?;
    let rela = BString::from(file.path.as_str());
    let head = staged_diff(&repo)?
        .into_iter()
        .find(|f| f.path == file.path);
    let head_entry = head.as_ref().and_then(|f| f.old_id.zip(f.old_mode));
    match head_entry {
        Some((id, mode)) => {
            let idx = index
                .entry_index_by_path(rela.as_bstr())
                .ok()
                .context("staged file not in index")?;
            let entry = &mut index.entries_mut()[idx];
            entry.id = id;
            entry.mode = mode;
        }
        None => {
            if let Ok(idx) = index.entry_index_by_path(rela.as_bstr()) {
                index.remove_entry_at_index(idx);
            }
        }
    }
    index.write(gix::index::write::Options::default())?;
    Ok(())
}

/// Rebuild the hunk's region of the index blob and write it back.
///
/// `remove_when_empty` decides what happens when the rebuilt content is
/// empty: for the unstage direction that's the case when `HEAD` had no entry
/// (added file), and for the stage direction when the worktree file is gone
/// (deleted file). An empty but *existing* worktree file still keeps its
/// index entry.
fn apply_to_index(
    repo: &gix::Repository,
    file: &FileDiff,
    hunk: &Hunk,
    selection: &Selection<'_>,
    remove_when_empty: bool,
) -> Result<()> {
    let new_content = splice::hunk(
        &file.old_data,
        file.old_ends_with_newline,
        file.new_ends_with_newline,
        &hunk.header,
        &hunk.lines,
        selection,
    )?;
    let blob_id = repo.write_blob(&new_content)?.detach();

    let mut index = owned_index(repo)?;
    let rela = BString::from(file.path.as_str());
    match index.entry_index_by_path(rela.as_bstr()) {
        Ok(idx) => {
            if new_content.is_empty() && remove_when_empty {
                index.remove_entry_at_index(idx);
            } else {
                index.entries_mut()[idx].id = blob_id;
            }
        }
        Err(_) => {
            // Untracked file being staged: add a fresh entry with the
            // worktree's stat and mode.
            let workdir = repo.workdir().context("repository has no worktree")?;
            let full = workdir.join(gix::path::from_bstr(rela.as_bstr()));
            let metadata = gix::index::fs::Metadata::from_path_no_follow(&full)?;
            let stat = gix::index::entry::Stat::from_fs(&metadata)?;
            let mode = mode_from_fs(&metadata, full.is_symlink());
            upsert_entry(&mut index, rela.as_bstr(), blob_id, mode, stat);
        }
    }
    index.write(gix::index::write::Options::default())?;
    Ok(())
}

fn worktree_file_missing(repo: &gix::Repository, rela: &BStr) -> bool {
    match repo.workdir() {
        Some(workdir) => !workdir.join(gix::path::from_bstr(rela)).exists(),
        None => true,
    }
}

/// Insert a fresh entry, or update an existing one in place.
fn upsert_entry(
    index: &mut gix::index::File,
    path: &BStr,
    id: gix::ObjectId,
    mode: Mode,
    stat: gix::index::entry::Stat,
) {
    match index.entry_index_by_path(path) {
        Ok(idx) => {
            let entry = &mut index.entries_mut()[idx];
            entry.id = id;
            entry.mode = mode;
            entry.stat = stat;
        }
        Err(_) => {
            index.dangerously_push_entry(stat, id, gix::index::entry::Flags::empty(), mode, path);
            index.sort_entries();
        }
    }
}
