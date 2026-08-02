//! Rebuilding unified-diff patch text for single hunks or selected lines.
//!
//! Pure string manipulation over the raw material extracted from a `git2`
//! diff; the results are fed back to libgit2 via `Diff::from_buffer`.

use std::path::{Path, PathBuf};

use anyhow::{Context, Result, bail};
use git2::{Diff, DiffFormat};

use crate::diff::SelectedLines;

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
pub(super) fn hunk_patch(diff: &Diff, file_idx: usize, hunk_idx: usize) -> Result<String> {
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
pub(super) fn partial_hunk_patch(
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

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn parses_full_hunk_header_with_section() {
        let (old, new, section) = parse_hunk_header("@@ -1,3 +1,4 @@ fn main()\n").unwrap();
        assert_eq!(old, 1);
        assert_eq!(new, 1);
        assert_eq!(section, " fn main()\n");
    }

    #[test]
    fn parses_header_without_counts_and_section() {
        let (old, new, section) = parse_hunk_header("@@ -10 +12 @@\n").unwrap();
        assert_eq!(old, 10);
        assert_eq!(new, 12);
        assert_eq!(section, "\n");
    }

    #[test]
    fn rejects_malformed_headers() {
        assert!(parse_hunk_header("not a header").is_err());
        assert!(parse_hunk_header("@@ -x +y @@\n").is_err());
    }
}
