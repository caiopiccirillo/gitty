//! Git operations via libgit2 (`git2` crate).

use std::path::{Path, PathBuf};

use anyhow::{Context, Result, bail};
use git2::{ApplyLocation, Delta, Diff, DiffDelta, DiffFormat, DiffOptions, Repository, Tree};

use crate::diff::{DiffLine, DiffView, FileInfo, FileStatus, LineKind};

/// Load the unstaged diff (workdir vs. index, like `git diff`) for the
/// repository containing `path`.
pub fn load_unstaged_diff(path: &Path) -> Result<DiffView> {
    let repo = open_repo(path)?;
    let diff = repo.diff_index_to_workdir(None, Some(&mut DiffOptions::new()))?;
    Ok(diff_to_view(&diff))
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
    let diff = repo.diff_index_to_workdir(None, Some(&mut DiffOptions::new()))?;
    apply_hunk_to_index(&repo, &diff, file_idx, hunk_idx)
}

/// Unstage a single hunk of the staged diff by applying the matching hunk of
/// the *reverse* diff (index vs. HEAD) to the index. Letting libgit2 compute
/// the reversal means added and deleted files work without hand-editing
/// patch text.
pub fn unstage_hunk(path: &Path, file_idx: usize, hunk_idx: usize) -> Result<()> {
    let repo = open_repo(path)?;
    // libgit2 has no index-to-tree diff, so materialize the index as a tree
    // object (no commit involved) and diff it against HEAD: that is exactly
    // the reverse of the staged diff, with matching hunk numbering.
    let mut index = repo.index()?;
    let index_tree = repo.find_tree(index.write_tree()?)?;
    let diff = repo.diff_tree_to_tree(
        Some(&index_tree),
        head_tree(&repo)?.as_ref(),
        Some(&mut DiffOptions::new()),
    )?;
    apply_hunk_to_index(&repo, &diff, file_idx, hunk_idx)
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

/// Apply the single hunk `hunk_idx` of file `file_idx` from `diff` to the index.
fn apply_hunk_to_index(
    repo: &Repository,
    diff: &Diff,
    file_idx: usize,
    hunk_idx: usize,
) -> Result<()> {
    let patch = hunk_patch(diff, file_idx, hunk_idx)?;
    let patch_diff = Diff::from_buffer(patch.as_bytes())?;
    repo.apply(&patch_diff, ApplyLocation::Index, None)?;
    Ok(())
}

/// Rebuild the unified-diff patch text of a single hunk: the file's header
/// lines plus exactly one hunk, with origin prefixes (`+`/`-`/` `) restored.
fn hunk_patch(diff: &Diff, target_file: usize, target_hunk: usize) -> Result<String> {
    let mut patch = String::new();
    let mut file_idx = 0usize;
    let mut hunk_idx: Option<usize> = None;
    // A file header spans several lines, so we detect file boundaries by the
    // delta's paths changing.
    let mut last_paths: Option<(Option<PathBuf>, Option<PathBuf>)> = None;
    let mut found = false;

    diff.print(DiffFormat::Patch, |delta, _hunk, line| {
        let paths = (
            delta.old_file().path().map(Path::to_path_buf),
            delta.new_file().path().map(Path::to_path_buf),
        );
        if last_paths.as_ref() != Some(&paths) {
            if last_paths.is_some() {
                file_idx += 1;
            }
            last_paths = Some(paths);
            hunk_idx = None;
        }
        if file_idx != target_file {
            return true;
        }

        let content = String::from_utf8_lossy(line.content());
        match line.origin() {
            'F' => patch.push_str(&content),
            'H' => {
                hunk_idx = Some(hunk_idx.map_or(0, |h| h + 1));
                if hunk_idx == Some(target_hunk) {
                    found = true;
                    patch.push_str(&content);
                }
            }
            origin if hunk_idx == Some(target_hunk) => {
                // `+`/`-`/` ` need their origin prefix restored; "\ No newline
                // at end of file" markers already contain the full line text.
                if matches!(origin, '+' | '-' | ' ') {
                    patch.push(origin);
                }
                patch.push_str(&content);
            }
            _ => {}
        }
        true
    })?;

    if !found {
        bail!("hunk {target_hunk} of file {target_file} not found");
    }
    Ok(patch)
}

/// Convert a `git2` diff into our flat, render-friendly line model.
fn diff_to_view(diff: &Diff) -> DiffView {
    let mut files = Vec::new();
    let mut lines = Vec::new();
    let mut file_idx = 0usize;
    let mut hunk_idx: Option<usize> = None;
    let mut last_paths: Option<(Option<PathBuf>, Option<PathBuf>)> = None;

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
