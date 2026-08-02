//! Git operations via libgit2 (`git2` crate).

mod patch;

use std::path::Path;

use anyhow::{Context, Result};
use git2::{ApplyLocation, Delta, Diff, DiffDelta, DiffFormat, DiffOptions, Repository, Tree};

use crate::diff::{DiffLine, DiffView, FileInfo, FileStatus, LineKind, SelectedLines};
use patch::{hunk_patch, partial_hunk_patch};

/// Load the unstaged diff (workdir vs. index, like `git diff`) for the
/// repository containing `path`. Untracked files are included.
pub fn load_unstaged_diff(path: &Path) -> Result<DiffView> {
    let repo = open_repo(path)?;
    Ok(diff_to_view(&workdir_diff(&repo)?))
}

/// Load the staged diff (index vs. HEAD, like `git diff --cached`).
pub fn load_staged_diff(path: &Path) -> Result<DiffView> {
    let repo = open_repo(path)?;
    let diff = repo.diff_tree_to_index(
        head_tree(&repo)?.as_ref(),
        None,
        Some(&mut DiffOptions::new()),
    )?;
    Ok(diff_to_view(&diff))
}

/// Stage a single hunk of the unstaged diff by applying it to the index.
pub fn stage_hunk(path: &Path, file_idx: usize, hunk_idx: usize) -> Result<()> {
    let repo = open_repo(path)?;
    let diff = workdir_diff(&repo)?;
    let patch = hunk_patch(&diff, file_idx, hunk_idx)?;
    apply_patch_to_index(&repo, &patch)
}

/// Unstage a single hunk of the staged diff by applying the matching hunk of
/// the *reverse* diff (index vs. HEAD) to the index. Letting libgit2 compute
/// the reversal means added and deleted files work without hand-editing
/// patch text.
pub fn unstage_hunk(path: &Path, file_idx: usize, hunk_idx: usize) -> Result<()> {
    let repo = open_repo(path)?;
    with_reverse_staged_diff(&repo, |diff| {
        let patch = hunk_patch(diff, file_idx, hunk_idx)?;
        apply_patch_to_index(&repo, &patch)
    })
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
    let diff = workdir_diff(&repo)?;
    let patch = partial_hunk_patch(&diff, file_idx, hunk_idx, selected)?;
    apply_patch_to_index(&repo, &patch)
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
    let swapped = SelectedLines {
        additions: selected.deletions.clone(),
        deletions: selected.additions.clone(),
    };
    with_reverse_staged_diff(&repo, |diff| {
        let patch = partial_hunk_patch(diff, file_idx, hunk_idx, &swapped)?;
        apply_patch_to_index(&repo, &patch)
    })
}

/// Stage a whole file (like `git add <path>`). A deleted file is staged by
/// removing it from the index.
pub fn stage_file(path: &Path, file: &FileInfo) -> Result<()> {
    let repo = open_repo(path)?;
    let mut index = repo.index()?;
    let file_path = Path::new(&file.path);
    match file.status {
        FileStatus::Deleted => index.remove_path(file_path)?,
        _ => index.add_path(file_path)?,
    }
    index.write()?;
    Ok(())
}

/// Unstage a whole file (like `git reset HEAD -- <path>`). On an unborn
/// branch there is no HEAD to reset to, so the index entry is simply
/// dropped and the file becomes untracked again.
pub fn unstage_file(path: &Path, file: &FileInfo) -> Result<()> {
    let repo = open_repo(path)?;
    match repo.head() {
        Ok(head) => {
            let commit = head.peel_to_commit()?;
            repo.reset_default(Some(commit.as_object()), [file.path.as_str()])?;
        }
        Err(e) if e.code() == git2::ErrorCode::UnbornBranch => {
            let mut index = repo.index()?;
            index.remove_path(Path::new(&file.path))?;
            index.write()?;
        }
        Err(e) => return Err(e.into()),
    }
    Ok(())
}

fn open_repo(path: &Path) -> Result<Repository> {
    Repository::discover(path)
        .with_context(|| format!("no git repository found at or above {}", path.display()))
}

/// HEAD as a tree, or `None` (i.e. the empty tree) on an unborn branch.
fn head_tree(repo: &Repository) -> Result<Option<Tree<'_>>> {
    match repo.head() {
        Ok(head) => Ok(Some(head.peel_to_tree()?)),
        Err(e) if e.code() == git2::ErrorCode::UnbornBranch => Ok(None),
        Err(e) => Err(e.into()),
    }
}

/// The workdir-vs-index diff with the options gitiff always uses (untracked
/// files included, with their content), shared so that file/hunk indices
/// are stable across calls.
fn workdir_diff(repo: &Repository) -> Result<Diff<'_>> {
    let mut opts = DiffOptions::new();
    // `show_untracked_content` implies `include_untracked`.
    opts.show_untracked_content(true)
        .recurse_untracked_dirs(true);
    Ok(repo.diff_index_to_workdir(None, Some(&mut opts))?)
}

/// Run `f` with the reverse of the staged diff (index tree vs. HEAD).
/// libgit2 has no index-to-tree diff, so the index is materialized as a
/// tree object (no commit involved) and diffed against HEAD; hunk numbering
/// matches the staged view.
fn with_reverse_staged_diff<R>(repo: &Repository, f: impl FnOnce(&Diff) -> Result<R>) -> Result<R> {
    let mut index = repo.index()?;
    let index_tree = repo.find_tree(index.write_tree()?)?;
    let diff = repo.diff_tree_to_tree(
        Some(&index_tree),
        head_tree(repo)?.as_ref(),
        Some(&mut DiffOptions::new()),
    )?;
    f(&diff)
}

/// Parse `patch` and apply it to the index (like `git apply --cached`).
fn apply_patch_to_index(repo: &Repository, patch: &str) -> Result<()> {
    let patch_diff = Diff::from_buffer(patch.as_bytes())?;
    repo.apply(&patch_diff, ApplyLocation::Index, None)?;
    Ok(())
}

/// Convert a `git2` diff into our flat, render-friendly line model.
fn diff_to_view(diff: &Diff) -> DiffView {
    let mut files = Vec::new();
    let mut lines = Vec::new();
    let mut file_idx = 0usize;
    let mut hunk_idx: Option<usize> = None;
    let mut last_paths: Option<(Option<std::path::PathBuf>, Option<std::path::PathBuf>)> = None;

    let _ = diff.print(DiffFormat::Patch, |delta, _hunk, line| {
        let paths = (
            delta.old_file().path().map(Path::to_path_buf),
            delta.new_file().path().map(Path::to_path_buf),
        );
        if last_paths.as_ref() != Some(&paths) {
            if last_paths.is_some() {
                file_idx += 1;
            }
            files.push(file_info(&delta));
            last_paths = Some(paths);
            hunk_idx = None;
        }

        let kind = match line.origin() {
            'F' => LineKind::FileHeader,
            'H' => {
                hunk_idx = Some(hunk_idx.map_or(0, |h| h + 1));
                LineKind::HunkHeader
            }
            '+' => LineKind::Addition,
            '-' => LineKind::Deletion,
            ' ' => LineKind::Context,
            // '<'/'>'/'=' ("\ No newline at end of file"), 'B' (binary).
            _ => LineKind::Meta,
        };

        // A single callback can carry several lines (file headers), so split
        // the content into individual display lines.
        for content in String::from_utf8_lossy(line.content()).split_terminator('\n') {
            lines.push(DiffLine {
                kind,
                content: content.to_string(),
                file_idx,
                hunk_idx,
            });
        }
        true
    });

    DiffView { lines, files }
}

fn file_info(delta: &DiffDelta) -> FileInfo {
    let status = match delta.status() {
        Delta::Added => FileStatus::Added,
        Delta::Deleted => FileStatus::Deleted,
        Delta::Renamed => FileStatus::Renamed,
        Delta::Typechange => FileStatus::TypeChange,
        Delta::Untracked => FileStatus::Untracked,
        _ => FileStatus::Modified,
    };
    let path = delta
        .new_file()
        .path()
        .or_else(|| delta.old_file().path())
        .map(|p| p.display().to_string())
        .unwrap_or_default();
    FileInfo { path, status }
}
