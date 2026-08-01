//! Git operations via libgit2 (`git2` crate).

use std::path::{Path, PathBuf};

use anyhow::{Context, Result, bail};
use git2::{ApplyLocation, Delta, Diff, DiffDelta, DiffFormat, DiffOptions, Repository, Tree};

use crate::diff::{DiffLine, DiffView, FileInfo, FileStatus, LineKind, SelectedLines};

/// Load the unstaged diff (workdir vs. index, like `git diff`) for the
/// repository containing `path`. Untracked files are included.
pub fn load_unstaged_diff(path: &Path) -> Result<DiffView> {
    let repo = open_repo(path)?;
    Ok(diff_to_view(&workdir_diff(&repo)?))
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
    apply_hunk_to_index(&repo, &diff, file_idx, hunk_idx)
}

/// Unstage a single hunk of the staged diff by applying the matching hunk of
/// the *reverse* diff (index vs. HEAD) to the index. Letting libgit2 compute
/// the reversal means added and deleted files work without hand-editing
/// patch text.
pub fn unstage_hunk(path: &Path, file_idx: usize, hunk_idx: usize) -> Result<()> {
    let repo = open_repo(path)?;
    with_reverse_staged_diff(&repo, |diff| {
        apply_hunk_to_index(&repo, diff, file_idx, hunk_idx)
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

/// Apply the single hunk `hunk_idx` of file `file_idx` from `diff` to the index.
fn apply_hunk_to_index(
    repo: &Repository,
    diff: &Diff,
    file_idx: usize,
    hunk_idx: usize,
) -> Result<()> {
    let patch = hunk_patch(diff, file_idx, hunk_idx)?;
    apply_patch_to_index(repo, &patch)
}

/// Parse `patch` and apply it to the index (like `git apply --cached`).
fn apply_patch_to_index(repo: &Repository, patch: &str) -> Result<()> {
    let patch_diff = Diff::from_buffer(patch.as_bytes())?;
    repo.apply(&patch_diff, ApplyLocation::Index, None)?;
    Ok(())
}

/// A hunk's raw material plus everything needed to rebuild its patch text.
struct RawHunk {
    /// The file's header lines (`diff --git`, `index`, `---`, `+++`), verbatim.
    file_header: String,
    /// The `@@ ... @@` line, verbatim.
    hunk_header: String,
    /// (origin, content with trailing newline) for each line inside the hunk.
    lines: Vec<(char, String)>,
}

/// Extract the file header and one hunk's raw lines from a diff.
fn find_hunk(diff: &Diff, target_file: usize, target_hunk: usize) -> Result<RawHunk> {
    let mut raw = RawHunk {
        file_header: String::new(),
        hunk_header: String::new(),
        lines: Vec::new(),
    };
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

        let content = String::from_utf8_lossy(line.content()).into_owned();
        match line.origin() {
            'F' => raw.file_header.push_str(&content),
            'H' => {
                hunk_idx = Some(hunk_idx.map_or(0, |h| h + 1));
                if hunk_idx == Some(target_hunk) {
                    found = true;
                    raw.hunk_header.push_str(&content);
                }
            }
            origin if hunk_idx == Some(target_hunk) => raw.lines.push((origin, content)),
            _ => {}
        }
        true
    })?;

    if !found {
        bail!("hunk {target_hunk} of file {target_file} not found");
    }
    Ok(raw)
}

/// Rebuild the unified-diff patch text of a single hunk: the file's header
/// lines plus exactly one hunk, with origin prefixes (`+`/`-`/` `) restored.
fn hunk_patch(diff: &Diff, file_idx: usize, hunk_idx: usize) -> Result<String> {
    let raw = find_hunk(diff, file_idx, hunk_idx)?;
    let mut patch = raw.file_header + &raw.hunk_header;
    for (origin, content) in &raw.lines {
        // `+`/`-`/` ` need their origin prefix restored; "\ No newline at
        // end of file" markers already contain the full line text.
        if matches!(origin, '+' | '-' | ' ') {
            patch.push(*origin);
        }
        patch.push_str(content);
    }
    Ok(patch)
}

/// Rebuild the patch of one hunk keeping only the selected changed lines:
/// unselected additions are dropped, unselected deletions become context,
/// and the `@@` counts are recomputed.
fn partial_hunk_patch(
    diff: &Diff,
    file_idx: usize,
    hunk_idx: usize,
    selected: &SelectedLines,
) -> Result<String> {
    let raw = find_hunk(diff, file_idx, hunk_idx)?;
    let (mut old_start, mut new_start, section) = parse_hunk_header(&raw.hunk_header)?;

    let mut body = String::new();
    let (mut old_count, mut new_count) = (0usize, 0usize);
    let (mut add_ord, mut del_ord) = (0usize, 0usize);
    for (origin, content) in &raw.lines {
        match origin {
            ' ' => {
                body.push(' ');
                body.push_str(content);
                old_count += 1;
                new_count += 1;
            }
            '-' => {
                if selected.deletions.contains(&del_ord) {
                    body.push('-');
                } else {
                    // Not (un)staged: the line stays as it is on this side.
                    body.push(' ');
                    new_count += 1;
                }
                body.push_str(content);
                old_count += 1;
                del_ord += 1;
            }
            '+' => {
                if selected.additions.contains(&add_ord) {
                    body.push('+');
                    body.push_str(content);
                    new_count += 1;
                }
                add_ord += 1;
            }
            _ => body.push_str(content), // "\ No newline at end of file"
        }
    }
    // For an empty side, the start is the line before the position.
    if old_count == 0 {
        old_start = old_start.saturating_sub(1);
    }
    if new_count == 0 {
        new_start = new_start.saturating_sub(1);
    }
    let header = format!("@@ -{old_start},{old_count} +{new_start},{new_count} @@{section}");
    Ok(raw.file_header + &header + &body)
}

/// Parse `@@ -a,b +c,d @@ section` into `(a, c, " section\n")`.
fn parse_hunk_header(header: &str) -> Result<(usize, usize, &str)> {
    let (ranges, section) = header
        .split_once("@@ ")
        .and_then(|(_, rest)| rest.split_once("@@"))
        .context("malformed hunk header")?;
    let mut sides = ranges.trim().split(' ');
    let start_of = |side: Option<&str>| -> Result<usize> {
        side.and_then(|s| s.get(1..))
            .and_then(|s| s.split(',').next())
            .and_then(|s| s.parse().ok())
            .context("malformed hunk header")
    };
    let old_start = start_of(sides.next())?;
    let new_start = start_of(sides.next())?;
    Ok((old_start, new_start, section))
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
