//! Computing the per-file diffs (`FileDiff`s) from a repository.

use std::path::Path;

use anyhow::{Context, Result};
use gix::bstr::{BStr, BString, ByteSlice};
use gix::diff::blob::pipeline::{Mode as PipelineMode, WorktreeRoots};
use gix::diff::blob::unified_diff::{ConsumeHunk, ContextSize, DiffLineKind};
use gix::diff::blob::{ResourceKind, platform};
use gix::index::entry::Mode;
use gix::object::tree::EntryKind;
use gix::status::tree_index::TrackRenames;
use gix::status::{UntrackedFiles, index_worktree};
use gix::status::plumbing::index_as_worktree::{Change, EntryStatus};

use crate::diff::{DiffView, FileStatus};

use super::model::{FileDiff, Hunk, kind_of, mode_from_fs};
use super::open_repo;
use super::render::diff_to_view;

/// Load the unstaged diff (workdir vs. index, like `git diff`) for the
/// repository containing `path`. Untracked files are included.
pub fn load_unstaged_diff(path: &Path) -> Result<DiffView> {
    let repo = open_repo(path)?;
    Ok(diff_to_view(&workdir_diff(&repo)?))
}

/// Load the staged diff (index vs. HEAD, like `git diff --cached`).
pub fn load_staged_diff(path: &Path) -> Result<DiffView> {
    let repo = open_repo(path)?;
    Ok(diff_to_view(&staged_diff(&repo)?))
}

/// The workdir-vs-index diff with the options gitiff always uses (untracked
/// files included, with their content), shared so that file/hunk indices
/// are stable across calls.
pub(super) fn workdir_diff(repo: &gix::Repository) -> Result<Vec<FileDiff>> {
    let workdir = repo.workdir().context("repository has no worktree")?;
    let mut cache = repo.diff_resource_cache(
        PipelineMode::ToGit,
        WorktreeRoots {
            old_root: None,
            new_root: Some(workdir.to_path_buf()),
        },
    )?;
    let iter = repo
        .status(gix::progress::Discard)?
        .untracked_files(UntrackedFiles::Files)
        .index_worktree_rewrites(None)
        // Submodules are skipped: their entries hold commit ids which the
        // blob-diff pipeline cannot handle.
        .index_worktree_submodules(None)
        .into_index_worktree_iter(Vec::<BString>::new())?;

    let mut files = Vec::new();
    for item in iter {
        let item = item?;
        match item {
            index_worktree::Item::Modification { entry, rela_path, status, .. } => {
                let path = rela_path.to_str_lossy().into_owned();
                match status {
                    EntryStatus::Change(Change::Removed) => {
                        files.push(diff_worktree(repo, &mut cache, &path, Some(entry.id), Some(entry.mode), None, FileStatus::Deleted)?);
                    }
                    EntryStatus::Change(Change::Type { worktree_mode }) => {
                        files.push(diff_worktree(repo, &mut cache, &path, Some(entry.id), Some(entry.mode), Some(worktree_mode), FileStatus::TypeChange)?);
                    }
                    EntryStatus::Change(
                        Change::Modification { .. } | Change::SubmoduleModification(_),
                    ) => {
                        files.push(diff_worktree(repo, &mut cache, &path, Some(entry.id), Some(entry.mode), None, FileStatus::Modified)?);
                    }
                    _ => {} // NeedsUpdate, IntentToAdd, Conflict.
                }
            }
            index_worktree::Item::DirectoryContents { entry, .. }
                if entry.status == gix::dir::entry::Status::Untracked =>
            {
                if entry.disk_kind == Some(gix::dir::entry::Kind::Directory) {
                    continue;
                }
                let path = entry.rela_path.to_str_lossy().into_owned();
                files.push(diff_worktree(repo, &mut cache, &path, None, None, None, FileStatus::Untracked)?);
            }
            _ => {}
        }
    }
    files.sort_by(|a, b| a.path.cmp(&b.path));
    Ok(files)
}

/// The staged diff (index vs. HEAD). Each change's old side is the HEAD blob
/// and its new side the index blob.
pub(super) fn staged_diff(repo: &gix::Repository) -> Result<Vec<FileDiff>> {
    let index = repo.index_or_empty()?;
    let tree_id = repo.head_tree_id_or_empty()?.detach();
    let mut cache = repo.diff_resource_cache_for_tree_diff()?;
    let mut files = Vec::new();
    repo.tree_index_status(
        &tree_id,
        &index,
        None,
        TrackRenames::Disabled,
        |change, _tree_index, _index| {
            use gix::diff::index::ChangeRef;
            let file = match change {
                ChangeRef::Addition { location, entry_mode, id, .. } => {
                    if entry_mode.is_submodule() {
                        return Ok(std::ops::ControlFlow::Continue(()));
                    }
                    Some(diff_blobs(
                        repo, &mut cache, location.as_ref(), None, None, Some(id.into_owned()), Some(entry_mode),
                    )?)
                }
                ChangeRef::Deletion { location, entry_mode, id, .. } => {
                    if entry_mode.is_submodule() {
                        return Ok(std::ops::ControlFlow::Continue(()));
                    }
                    Some(diff_blobs(
                        repo, &mut cache, location.as_ref(), Some(id.into_owned()), Some(entry_mode), None, None,
                    )?)
                }
                ChangeRef::Modification {
                    location,
                    previous_entry_mode,
                    previous_id,
                    entry_mode,
                    id,
                    ..
                } => {
                    if entry_mode.is_submodule() || previous_entry_mode.is_submodule() {
                        return Ok(std::ops::ControlFlow::Continue(()));
                    }
                    Some(diff_blobs(
                        repo,
                        &mut cache,
                        location.as_ref(),
                        Some(previous_id.into_owned()),
                        Some(previous_entry_mode),
                        Some(id.into_owned()),
                        Some(entry_mode),
                    )?)
                }
                ChangeRef::Rewrite { .. } => unreachable!("rewrite tracking is disabled"),
            };
            if let Some(file) = file {
                files.push(file);
            }
            Ok::<_, anyhow::Error>(std::ops::ControlFlow::Continue(()))
        },
    )?;
    files.sort_by(|a, b| a.path.cmp(&b.path));
    Ok(files)
}

/// Compute the blob diff of one changed file of the staged diff.
fn diff_blobs(
    repo: &gix::Repository,
    cache: &mut gix::diff::blob::Platform,
    path: &BStr,
    old_id: Option<gix::ObjectId>,
    old_mode: Option<Mode>,
    new_id: Option<gix::ObjectId>,
    new_mode: Option<Mode>,
) -> Result<FileDiff> {
    set_resource(repo, cache, path, old_id, old_mode, ResourceKind::OldOrSource)?;
    set_resource(repo, cache, path, new_id, new_mode, ResourceKind::NewOrDestination)?;
    let mut file = FileDiff::new(path, old_id, old_mode, new_id, new_mode);
    collect_diff(cache, &mut file)?;
    Ok(file)
}

/// Compute the blob diff of one changed file of the unstaged diff, where the
/// new side is read from the worktree. The status is known from the status
/// walk (the worktree side has no blob id to classify from).
fn diff_worktree(
    repo: &gix::Repository,
    cache: &mut gix::diff::blob::Platform,
    path: &str,
    old_id: Option<gix::ObjectId>,
    old_mode: Option<Mode>,
    new_mode: Option<Mode>,
    status: FileStatus,
) -> Result<FileDiff> {
    let path = BString::from(path);
    set_resource(repo, cache, path.as_bstr(), old_id, old_mode, ResourceKind::OldOrSource)?;
    // The new side is read from the worktree; its kind is only used to tell
    // symlinks apart from regular files.
    let fs_mode = worktree_mode_of(repo, path.as_bstr());
    let new_kind_mode = new_mode.or(fs_mode).unwrap_or(Mode::FILE);
    set_resource(repo, cache, path.as_bstr(), None, Some(new_kind_mode), ResourceKind::NewOrDestination)?;
    let mut file = FileDiff::new(path.as_bstr(), old_id, old_mode, None, new_mode.or(fs_mode));
    file.status = status;
    collect_diff(cache, &mut file)?;
    Ok(file)
}

/// The index mode the worktree file at `rela` would get, or `None` if it
/// doesn't exist (which is also the case for the mode of a type change,
/// where the caller provides it explicitly).
fn worktree_mode_of(repo: &gix::Repository, rela: &BStr) -> Option<Mode> {
    let full = repo.workdir()?.join(gix::path::from_bstr(rela));
    let metadata = gix::index::fs::Metadata::from_path_no_follow(&full).ok()?;
    Some(mode_from_fs(&metadata, metadata.is_symlink()))
}

fn set_resource(
    repo: &gix::Repository,
    cache: &mut gix::diff::blob::Platform,
    path: &BStr,
    id: Option<gix::ObjectId>,
    mode: Option<Mode>,
    kind: ResourceKind,
) -> Result<()> {
    let id = id.unwrap_or_else(|| gix::ObjectId::null(repo.object_hash()));
    let entry_kind = mode.map(kind_of).unwrap_or(EntryKind::Blob);
    cache.set_resource(id, entry_kind, path, kind, &repo.objects)?;
    Ok(())
}

/// Run the diff for `file` and store the hunks and both sides' content.
fn collect_diff(cache: &mut gix::diff::blob::Platform, file: &mut FileDiff) -> Result<()> {
    let outcome = cache.prepare_diff()?;
    file.old_data = outcome.old.data.as_slice().unwrap_or_default().to_vec();
    file.new_data = outcome.new.data.as_slice().unwrap_or_default().to_vec();
    file.old_ends_with_newline = file.old_data.ends_with(b"\n");
    file.new_ends_with_newline = file.new_data.ends_with(b"\n");
    match outcome.operation {
        platform::prepare_diff::Operation::InternalDiff { algorithm } => {
            let input = outcome.interned_input();
            let diff = gix::diff::blob::diff_with_slider_heuristics(algorithm, &input);
            let collector = gix::diff::blob::UnifiedDiff::new(
                &diff,
                &input,
                HunkCollector::default(),
                ContextSize::default(),
            )
            .consume()?;
            file.hunks = collector.hunks;
        }
        // Binary content or an external diff driver: no hunks, the view
        // shows a placeholder line.
        platform::prepare_diff::Operation::SourceOrDestinationIsBinary
        | platform::prepare_diff::Operation::ExternalCommand { .. } => {
            file.binary = true;
        }
    }
    Ok(())
}

/// Collects hunk material from a `UnifiedDiff` render pass.
#[derive(Default)]
struct HunkCollector {
    hunks: Vec<Hunk>,
}

impl ConsumeHunk for HunkCollector {
    type Out = Self;

    fn consume_hunk(&mut self, header: gix::diff::blob::unified_diff::HunkHeader, lines: &[(DiffLineKind, &[u8])]) -> std::io::Result<()> {
        self.hunks.push(Hunk {
            header,
            lines: lines.iter().map(|(kind, content)| (*kind, content.to_vec())).collect(),
        });
        Ok(())
    }

    fn finish(self) -> Self::Out {
        self
    }
}
