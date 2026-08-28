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
///
/// # Errors
/// Returns an error if the repository cannot be opened, the diff cannot be computed, or the index cannot be written.
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
///
/// # Errors
/// Returns an error if the repository cannot be opened, the diff cannot be computed, or the index cannot be written.
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
///
/// # Errors
/// Returns an error if the repository cannot be opened, the diff cannot be computed, or the index cannot be written.
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
///
/// # Errors
/// Returns an error if the repository cannot be opened, the diff cannot be computed, or the index cannot be written.
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

/// Discard a single hunk of the unstaged diff: the worktree region reverts
/// to the index version (like `git restore` for that hunk).
///
/// # Errors
/// Returns an error if the repository cannot be opened, the diff cannot be computed, or the index cannot be written.
pub fn discard_hunk(path: &Path, file_idx: usize, hunk_idx: usize) -> Result<()> {
    let repo = open_repo(path)?;
    let fd = &workdir_diff(&repo)?[file_idx];
    let file = &reversed(fd);
    let hunk = &file.hunks[hunk_idx];
    let content = discard_content(
        file,
        hunk,
        &Selection {
            keep_removes: &|_| false,
            keep_adds: &|_| true,
        },
    )?;
    write_worktree(&repo, &fd.path, &content, fd.old_id.is_some(), fd.old_mode)
}

/// Discard only the selected changed lines of an unstaged hunk: selected
/// additions disappear from the worktree, selected deletions are restored.
///
/// # Errors
/// Returns an error if the repository cannot be opened, the diff cannot be computed, or the index cannot be written.
pub fn discard_lines(
    path: &Path,
    file_idx: usize,
    hunk_idx: usize,
    selected: &SelectedLines,
) -> Result<()> {
    let repo = open_repo(path)?;
    let fd = &workdir_diff(&repo)?[file_idx];
    let file = &reversed(fd);
    let hunk = &file.hunks[hunk_idx];
    let content = discard_content(
        file,
        hunk,
        &Selection {
            keep_removes: &|i| !selected.additions.contains(&i),
            keep_adds: &|i| selected.deletions.contains(&i),
        },
    )?;
    write_worktree(&repo, &fd.path, &content, fd.old_id.is_some(), fd.old_mode)
}

/// Discard a whole file of the unstaged diff: the worktree file reverts to
/// the index version, or is deleted if it was untracked.
///
/// # Errors
/// Returns an error if the repository cannot be opened or the index cannot be written.
pub fn discard_file(path: &Path, file: &FileInfo) -> Result<()> {
    let repo = open_repo(path)?;
    let files = workdir_diff(&repo)?;
    let fd = files
        .iter()
        .find(|f| f.path == file.path)
        .context("file not in the unstaged diff")?;
    let content = match fd.old_id {
        Some(id) => blob_content(&repo, id)?,
        None => Vec::new(),
    };
    write_worktree(&repo, &fd.path, &content, fd.old_id.is_some(), fd.old_mode)
}

/// Discard a single hunk of the staged diff: both the worktree and the index
/// region revert to HEAD (like `git checkout HEAD --` for that hunk).
///
/// # Errors
/// Returns an error if the repository cannot be opened, the diff cannot be computed, or the index cannot be written.
pub fn discard_staged_hunk(path: &Path, file_idx: usize, hunk_idx: usize) -> Result<()> {
    let repo = open_repo(path)?;
    let fd = &staged_diff(&repo)?[file_idx];
    let file = &reversed(fd);
    let hunk = &file.hunks[hunk_idx];
    let selection = Selection {
        keep_removes: &|_| false,
        keep_adds: &|_| true,
    };
    let content = discard_content(file, hunk, &selection)?;
    replace_index_blob(&repo, &fd.path, &content, fd.old_id.is_none())?;
    // Revert the same region in the worktree, preserving any other changes.
    if let Some(raw) = worktree_content(&repo, &fd.path) {
        let content = splice::hunk(
            &raw,
            raw.ends_with(b"\n"),
            file.new_ends_with_newline,
            &hunk.header,
            &hunk.lines,
            &selection,
        )?;
        write_worktree(&repo, &fd.path, &content, fd.old_id.is_some(), fd.old_mode)?;
    }
    Ok(())
}

/// Discard only the selected changed lines of a staged hunk. The reverse
/// diff swaps the roles of `+` and `-`, so the selection is swapped too.
///
/// # Errors
/// Returns an error if the repository cannot be opened, the diff cannot be computed, or the index cannot be written.
pub fn discard_staged_lines(
    path: &Path,
    file_idx: usize,
    hunk_idx: usize,
    selected: &SelectedLines,
) -> Result<()> {
    let repo = open_repo(path)?;
    let fd = &staged_diff(&repo)?[file_idx];
    let file = &reversed(fd);
    let hunk = &file.hunks[hunk_idx];
    let swapped = SelectedLines {
        additions: selected.deletions.clone(),
        deletions: selected.additions.clone(),
    };
    let selection = Selection {
        keep_removes: &|i| !swapped.deletions.contains(&i),
        keep_adds: &|i| swapped.additions.contains(&i),
    };
    let content = discard_content(file, hunk, &selection)?;
    replace_index_blob(&repo, &fd.path, &content, fd.old_id.is_none())?;
    if let Some(raw) = worktree_content(&repo, &fd.path) {
        let content = splice::hunk(
            &raw,
            raw.ends_with(b"\n"),
            file.new_ends_with_newline,
            &hunk.header,
            &hunk.lines,
            &selection,
        )?;
        write_worktree(&repo, &fd.path, &content, fd.old_id.is_some(), fd.old_mode)?;
    }
    Ok(())
}

/// Discard a whole file of the staged diff: the index entry reverts to HEAD
/// (or is dropped) and the worktree file is rewritten to HEAD's version (or
/// deleted).
///
/// # Errors
/// Returns an error if the repository cannot be opened or the index cannot be written.
pub fn discard_staged_file(path: &Path, file: &FileInfo) -> Result<()> {
    let repo = open_repo(path)?;
    let files = staged_diff(&repo)?;
    let fd = files
        .iter()
        .find(|f| f.path == file.path)
        .context("file not in the staged diff")?;
    if let Some((id, mode)) = fd.old_id.zip(fd.old_mode) {
        let content = blob_content(&repo, id)?;
        replace_index_blob(&repo, &fd.path, &content, false)?;
        write_worktree(&repo, &fd.path, &content, true, Some(mode))?;
    } else {
        // Added file: drop the entry and delete the worktree file.
        let mut index = owned_index(&repo)?;
        let rela = BString::from(fd.path.as_str());
        if let Ok(idx) = index.entry_index_by_path(rela.as_bstr()) {
            index.remove_entry_at_index(idx);
        }
        index.write(gix::index::write::Options::default())?;
        write_worktree(&repo, &fd.path, &[], false, None)?;
    }
    Ok(())
}

/// The worktree content after discarding `hunk`'s region of `file`.
/// `file` must be the reversed diff, whose new side holds what the region
/// reverts to; the selection decides which lines survive.
fn discard_content(file: &FileDiff, hunk: &Hunk, selection: &Selection<'_>) -> Result<Vec<u8>> {
    splice::hunk(
        &file.old_data,
        file.old_ends_with_newline,
        file.new_ends_with_newline,
        &hunk.header,
        &hunk.lines,
        selection,
    )
    .with_context(|| format!("cannot rebuild {}", file.path))
}

/// The raw bytes of the worktree file at `rela`, or `None` if it's missing.
fn worktree_content(repo: &gix::Repository, rela: &str) -> Option<Vec<u8>> {
    let full = repo
        .workdir()?
        .join(gix::path::from_bstr(BStr::new(rela.as_bytes())));
    std::fs::read(&full).ok()
}

/// Write `content` back to the worktree file at `rela`, restoring the file
/// type and mode of the side it reverts to.
///
/// * `has_entry`: whether the side being restored to has an index entry.
///   Without one (untracked or newly added file) the worktree file is
///   deleted instead of written.
/// * `mode`: the mode of the side being restored to, which decides between a
///   regular file, an executable and a symlink.
fn write_worktree(
    repo: &gix::Repository,
    rela: &str,
    content: &[u8],
    has_entry: bool,
    mode: Option<Mode>,
) -> Result<()> {
    let workdir = repo.workdir().context("repository has no worktree")?;
    let full = workdir.join(gix::path::from_bstr(BStr::new(rela.as_bytes())));
    if !has_entry {
        if full.is_symlink() || full.is_file() {
            std::fs::remove_file(&full)?;
        }
        return Ok(());
    }
    if full.is_symlink() || full.is_file() {
        std::fs::remove_file(&full)?;
    }
    if let Some(Mode::SYMLINK) = mode {
        #[cfg(unix)]
        std::os::unix::fs::symlink(gix::path::from_bstr(BStr::new(content)), &full)?;
        #[cfg(not(unix))]
        std::fs::write(&full, content)?;
    } else {
        std::fs::write(&full, content)?;
        set_executable(&full, mode == Some(Mode::FILE_EXECUTABLE));
    }
    Ok(())
}

/// The content of the blob with `id`.
fn blob_content(repo: &gix::Repository, id: gix::ObjectId) -> Result<Vec<u8>> {
    Ok(repo.find_object(id)?.into_blob().data.clone())
}

/// Set (or clear) the executable bit of `path`, keeping the rest of the
/// permissions untouched.
#[cfg(unix)]
fn set_executable(path: &std::path::Path, executable: bool) {
    use std::os::unix::fs::PermissionsExt;

    let Ok(mut permissions) = std::fs::metadata(path).map(|m| m.permissions()) else {
        return;
    };
    let mut mode = permissions.mode();
    if executable {
        mode |= 0o111;
    } else {
        mode &= !0o111;
    }
    permissions.set_mode(mode);
    let _ = std::fs::set_permissions(path, permissions);
}

#[cfg(not(unix))]
fn set_executable(_path: &std::path::Path, _executable: bool) {}

/// Update the index so the file at `path` holds `content`, removing the
/// entry when `content` is empty and `remove_when_empty` is set. The entry
/// is expected to exist (the unstage/discard paths).
fn replace_index_blob(
    repo: &gix::Repository,
    path: &str,
    content: &[u8],
    remove_when_empty: bool,
) -> Result<()> {
    let blob_id = repo.write_blob(content)?.detach();
    let mut index = owned_index(repo)?;
    let rela = BString::from(path);
    let idx = index
        .entry_index_by_path(rela.as_bstr())
        .ok()
        .context("file not in index")?;
    if content.is_empty() && remove_when_empty {
        index.remove_entry_at_index(idx);
    } else {
        point_entry_at(&mut index.entries_mut()[idx], blob_id);
    }
    index.write(gix::index::write::Options::default())?;
    Ok(())
}

/// A fresh, mutable copy of the index (creating an empty one if none exists).
pub(super) fn owned_index(repo: &gix::Repository) -> Result<gix::index::File> {
    Ok((**repo.index_or_empty()?).clone())
}

/// Stage a whole file (like `git add <path>`). A deleted file is staged by
/// removing it from the index.
///
/// # Errors
/// Returns an error if the repository cannot be opened or the index cannot be written.
pub fn stage_file(path: &Path, file: &FileInfo) -> Result<()> {
    let repo = open_repo(path)?;
    let workdir = repo.workdir().context("repository has no worktree")?;
    let mut index = owned_index(&repo)?;
    let rela = BString::from(file.path.as_str());
    if file.status == FileStatus::Deleted {
        if let Ok(idx) = index.entry_index_by_path(rela.as_bstr()) {
            index.remove_entry_at_index(idx);
        }
    } else {
        let full = workdir.join(gix::path::from_bstr(rela.as_bstr()));
        let (content, is_symlink) = if full.is_symlink() {
            (gix::path::into_bstr(full.read_link()?).to_vec(), true)
        } else {
            (
                std::fs::read(&full).with_context(|| format!("cannot read {}", full.display()))?,
                false,
            )
        };
        let id = repo.write_blob(&content)?.detach();
        let metadata = gix::index::fs::Metadata::from_path_no_follow(&full)?;
        let stat = gix::index::entry::Stat::from_fs(&metadata)?;
        let mode = mode_from_fs(&metadata, is_symlink);
        upsert_entry(&mut index, rela.as_bstr(), id, mode, stat);
    }
    index.write(gix::index::write::Options::default())?;
    Ok(())
}

/// Unstage a whole file (like `git reset HEAD -- <path>`). On an unborn
/// branch there is no HEAD entry to restore, so the index entry is
/// dropped and the file becomes untracked again.
///
/// # Errors
/// Returns an error if the repository cannot be opened or the index cannot be written.
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
            point_entry_at(entry, id);
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
    if let Ok(idx) = index.entry_index_by_path(rela.as_bstr()) {
        if new_content.is_empty() && remove_when_empty {
            index.remove_entry_at_index(idx);
        } else {
            point_entry_at(&mut index.entries_mut()[idx], blob_id);
        }
    } else {
        // Untracked file being staged: add a fresh entry with the
        // worktree's stat and mode.
        let workdir = repo.workdir().context("repository has no worktree")?;
        let full = workdir.join(gix::path::from_bstr(rela.as_bstr()));
        let metadata = gix::index::fs::Metadata::from_path_no_follow(&full)?;
        let stat = gix::index::entry::Stat::from_fs(&metadata)?;
        let mode = mode_from_fs(&metadata, full.is_symlink());
        upsert_entry(&mut index, rela.as_bstr(), blob_id, mode, stat);
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

/// Point an existing index entry at `id` and drop its cached stat.
///
/// The stat records the worktree file the entry was last known to match.
/// Once the entry holds a blob we built ourselves — a spliced hunk, or
/// HEAD's version on an unstage — that no longer holds, and a stat left in
/// place lets the index/worktree comparison skip reading the file and
/// report the remaining changes as absent. Zeroing it is what
/// `git apply --cached` writes, and it forces the content comparison.
fn point_entry_at(entry: &mut gix::index::Entry, id: gix::ObjectId) {
    entry.id = id;
    entry.stat = gix::index::entry::Stat::default();
}

/// Insert a fresh entry, or update an existing one in place.
fn upsert_entry(
    index: &mut gix::index::File,
    path: &BStr,
    id: gix::ObjectId,
    mode: Mode,
    stat: gix::index::entry::Stat,
) {
    if let Ok(idx) = index.entry_index_by_path(path) {
        let entry = &mut index.entries_mut()[idx];
        entry.id = id;
        entry.mode = mode;
        entry.stat = stat;
    } else {
        index.dangerously_push_entry(stat, id, gix::index::entry::Flags::empty(), mode, path);
        index.sort_entries();
    }
}
