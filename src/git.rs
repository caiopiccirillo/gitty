//! Git operations via libgit2 (`git2` crate).

use std::path::{Path, PathBuf};

use anyhow::{Context, Result};
use git2::{Diff, DiffFormat, DiffOptions, Repository};

use crate::diff::{DiffLine, DiffView, LineKind};

/// Load the unstaged diff (workdir vs. index, like `git diff`) for the
/// repository containing `path`.
pub fn load_workdir_diff(path: &Path) -> Result<DiffView> {
    let repo = Repository::discover(path)
        .with_context(|| format!("no git repository found at or above {}", path.display()))?;
    let diff = repo.diff_index_to_workdir(None, Some(&mut DiffOptions::new()))?;
    Ok(diff_to_view(&diff))
}

/// Convert a `git2` diff into our flat, render-friendly line model.
fn diff_to_view(diff: &Diff) -> DiffView {
    let file_count = diff.deltas().len();
    let mut lines = Vec::new();
    let mut file_idx = 0usize;
    let mut hunk_idx: Option<usize> = None;
    // A file header spans several lines (`diff --git`, `index`, `---`, `+++`),
    // so we detect file boundaries by the delta's paths changing.
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

    DiffView { lines, file_count }
}
