//! Rebuilding blob content from hunk material, replacing libgit2's
//! patch-parse-and-apply with direct line splicing.
//!
//! A hunk describes a region of the old content (index or HEAD) plus the
//! lines that replace it in the new content (worktree or index). Staging a
//! hunk writes the *old* content back to the index with the hunk region
//! replaced by the new side (or, for partial staging, by only the selected
//! new lines (with unselected deletions kept as context). Unstaging works on
//! the reverse diff (index vs. HEAD), where the roles of additions and
//! deletions are swapped by the caller.

use anyhow::{Result, anyhow, bail};
use gix::diff::blob::unified_diff::{DiffLineKind, HunkHeader};

/// Which old-side lines to keep in the output and which new-side lines to
/// include, as 0-based ordinals counted per kind inside the hunk.
///
/// * Staging everything: keep no removals, keep all additions.
/// * Partial staging: keep unselected removals (they become context) and
///   keep only the selected additions.
pub struct Selection<'a> {
    pub keep_removes: &'a dyn Fn(usize) -> bool,
    pub keep_adds: &'a dyn Fn(usize) -> bool,
}

/// Rebuild the new blob content: `old` with the hunk's region (given by the
/// 1-based `before_hunk_start`/`before_hunk_len` of `hunk_header`) replaced
/// by the hunk's `lines`, filtered through `selection`.
///
/// `old_ends_with_newline`/`new_ends_with_newline` decide the trailing
/// newline of the result: it matches whichever side the final emitted line
/// came from (unchanged lines after the hunk are old-side lines).
pub fn hunk(
    old: &[u8],
    old_ends_with_newline: bool,
    new_ends_with_newline: bool,
    hunk_header: &HunkHeader,
    lines: &[(DiffLineKind, Vec<u8>)],
    selection: &Selection<'_>,
) -> Result<Vec<u8>> {
    let old_lines = split_lines(old);
    let start = usize::try_from(hunk_header.before_hunk_start)
        .map_err(|_| anyhow!("hunk start too large"))?
        .saturating_sub(1);
    let end = start
        .checked_add(usize::try_from(hunk_header.before_hunk_len).map_err(|_| anyhow!("hunk too large"))?)
        .ok_or_else(|| anyhow!("hunk too large"))?;
    if end > old_lines.len() {
        bail!(
            "hunk {}..{} does not fit the {} old lines",
            start,
            end,
            old_lines.len()
        );
    }

    let mut out: Vec<&[u8]> = old_lines[..start].to_vec();
    let (mut removed_ord, mut added_ord) = (0usize, 0usize);
    // Whether the last emitted line came from the old side of the diff.
    let mut last_from_old = false;
    for (kind, content) in lines {
        let emit = match kind {
            DiffLineKind::Context => {
                out.push(content);
                false
            }
            DiffLineKind::Remove => {
                let keep = (selection.keep_removes)(removed_ord);
                removed_ord += 1;
                if keep {
                    out.push(content);
                }
                keep
            }
            DiffLineKind::Add => {
                let keep = (selection.keep_adds)(added_ord);
                added_ord += 1;
                if keep {
                    out.push(content);
                }
                keep
            }
        };
        if emit {
            last_from_old = matches!(kind, DiffLineKind::Remove);
        }
    }
    if end < old_lines.len() {
        out.extend_from_slice(&old_lines[end..]);
        last_from_old = true;
    }

    let ends_with_newline = if out.is_empty() {
        false
    } else if last_from_old {
        old_ends_with_newline
    } else {
        new_ends_with_newline
    };
    Ok(join_lines(&out, ends_with_newline))
}

/// Split `content` into lines without trailing newline separators.
fn split_lines(content: &[u8]) -> Vec<&[u8]> {
    content.split_inclusive(|&b| b == b'\n').map(strip_newline).collect()
}

/// Join `lines` (without terminators), adding a final newline only if
/// `ends_with_newline`.
fn join_lines(lines: &[&[u8]], ends_with_newline: bool) -> Vec<u8> {
    let mut out = Vec::with_capacity(lines.iter().map(|l| l.len() + 1).sum::<usize>());
    for (i, line) in lines.iter().enumerate() {
        out.extend_from_slice(line);
        if i + 1 < lines.len() || ends_with_newline {
            out.push(b'\n');
        }
    }
    out
}

fn strip_newline(line: &[u8]) -> &[u8] {
    line.strip_suffix(b"\n").unwrap_or(line)
}

#[cfg(test)]
mod tests {
    use super::*;
    use gix::diff::blob::unified_diff::HunkHeader;

    fn header(before_start: u32, before_len: u32, after_start: u32, after_len: u32) -> HunkHeader {
        HunkHeader {
            before_hunk_start: before_start,
            before_hunk_len: before_len,
            after_hunk_start: after_start,
            after_hunk_len: after_len,
        }
    }

    fn line(kind: DiffLineKind, content: &str) -> (DiffLineKind, Vec<u8>) {
        (kind, content.as_bytes().to_vec())
    }

    fn all() -> Selection<'static> {
        Selection {
            keep_removes: &|_| false,
            keep_adds: &|_| true,
        }
    }

    /// Replace one line in the middle of a file (ctx, -old, +new, ctx).
    #[test]
    fn replaces_a_middle_line() {
        let old = b"a\nb\nc\n";
        let out = hunk(
            old,
            true,
            true,
            &header(1, 3, 1, 3),
            &[
                line(DiffLineKind::Context, "a"),
                line(DiffLineKind::Remove, "b"),
                line(DiffLineKind::Add, "B"),
                line(DiffLineKind::Context, "c"),
            ],
            &all(),
        )
        .unwrap();
        assert_eq!(out, b"a\nB\nc\n");
    }

    /// A hunk covering the last line, with the old file lacking a trailing
    /// newline: the result must not gain one.
    #[test]
    fn preserves_missing_trailing_newline() {
        let old = b"a\nb";
        let out = hunk(
            old,
            false,
            false,
            &header(1, 2, 1, 2),
            &[
                line(DiffLineKind::Context, "a"),
                line(DiffLineKind::Remove, "b"),
                line(DiffLineKind::Add, "B"),
            ],
            &all(),
        )
        .unwrap();
        assert_eq!(out, b"a\nB");
    }

    /// An added file: empty old side, everything is additions.
    #[test]
    fn added_file_keeps_all_new_lines() {
        let out = hunk(
            b"",
            false,
            true,
            &header(1, 0, 1, 2),
            &[line(DiffLineKind::Add, "one"), line(DiffLineKind::Add, "two")],
            &all(),
        )
        .unwrap();
        assert_eq!(out, b"one\ntwo\n");
    }

    /// A deleted file: everything is removals, the result is empty.
    #[test]
    fn deleted_file_becomes_empty() {
        let out = hunk(
            b"one\ntwo\n",
            true,
            false,
            &header(1, 2, 1, 0),
            &[line(DiffLineKind::Remove, "one"), line(DiffLineKind::Remove, "two")],
            &all(),
        )
        .unwrap();
        assert!(out.is_empty());
    }

    /// Partial staging: unselected deletions stay as context, only the
    /// selected addition is kept. Keeping the l2 change of a "l2→L2 and
    /// l3→L3" hunk leaves l3 in place between the context and the addition.
    #[test]
    fn partial_selection_keeps_unselected_deletions() {
        let selection = Selection {
            keep_removes: &|i| i != 0,
            keep_adds: &|i| i == 0,
        };
        let old = b"l1\nl2\nl3\n";
        let out = hunk(
            old,
            true,
            true,
            &header(1, 3, 1, 3),
            &[
                line(DiffLineKind::Context, "l1"),
                line(DiffLineKind::Remove, "l2"),
                line(DiffLineKind::Add, "L2"),
                line(DiffLineKind::Remove, "l3"),
                line(DiffLineKind::Add, "L3"),
            ],
            &selection,
        )
        .unwrap();
        assert_eq!(out, b"l1\nL2\nl3\n");
    }

    /// The unstage direction: old is the index, removals are index lines and
    /// additions are the HEAD lines being restored.
    #[test]
    fn unstage_restores_head_lines() {
        let index = b"a\nB\nc\n";
        let out = hunk(
            index,
            true,
            true,
            &header(1, 3, 1, 3),
            &[
                line(DiffLineKind::Context, "a"),
                line(DiffLineKind::Remove, "B"),
                line(DiffLineKind::Add, "b"),
                line(DiffLineKind::Context, "c"),
            ],
            &all(),
        )
        .unwrap();
        assert_eq!(out, b"a\nb\nc\n");
    }

    #[test]
    fn rejects_hunks_outside_the_old_content() {
        let err = hunk(
            b"a\n",
            true,
            true,
            &header(5, 1, 5, 1),
            &[line(DiffLineKind::Context, "a"), line(DiffLineKind::Remove, "x"), line(DiffLineKind::Add, "y")],
            &all(),
        );
        assert!(err.is_err());
    }

    /// The index's trailing newline (beyond the hunk region) is preserved
    /// even when the new side of the hunk would not imply one.
    #[test]
    fn unstage_keeps_index_trailing_newline_beyond_the_hunk() {
        // index: "a\nB\nc\n", HEAD: "a\nb\nc": the trailing newline was
        // removed by a staged change past this hunk.
        let index = b"a\nB\nc\n";
        let out = hunk(
            index,
            true,
            false,
            &header(0, 2, 0, 2),
            &[
                line(DiffLineKind::Context, "a"),
                line(DiffLineKind::Remove, "B"),
                line(DiffLineKind::Add, "b"),
            ],
            &all(),
        )
        .unwrap();
        assert_eq!(out, b"a\nb\nc\n");
    }

    /// Discarding a hunk restores the old side: every removal is kept and no
    /// addition survives, so the result equals the old content.
    #[test]
    fn discard_restores_the_old_side() {
        let old = b"a\nb\nc\n";
        let selection = Selection {
            keep_removes: &|_| true,
            keep_adds: &|_| false,
        };
        let out = hunk(
            old,
            true,
            true,
            &header(0, 3, 0, 3),
            &[
                line(DiffLineKind::Context, "a"),
                line(DiffLineKind::Remove, "b"),
                line(DiffLineKind::Add, "B"),
                line(DiffLineKind::Context, "c"),
            ],
            &selection,
        )
        .unwrap();
        assert_eq!(out, b"a\nb\nc\n");
    }

    /// Discarding only the selected lines keeps the unselected worktree
    /// lines and restores the selected deletions.
    #[test]
    fn discard_only_the_selected_lines() {
        // index: l1 l2 l3; worktree: l1 L2 L3 (both changes present).
        // Discard the l2 change: restore l2 (removal 0), keep L3 (addition 1).
        let old = b"l1\nl2\nl3\n";
        let selection = Selection {
            keep_removes: &|i| i == 0,
            keep_adds: &|i| i != 0,
        };
        let out = hunk(
            old,
            true,
            true,
            &header(0, 3, 0, 3),
            &[
                line(DiffLineKind::Context, "l1"),
                line(DiffLineKind::Remove, "l2"),
                line(DiffLineKind::Add, "L2"),
                line(DiffLineKind::Remove, "l3"),
                line(DiffLineKind::Add, "L3"),
            ],
            &selection,
        )
        .unwrap();
        assert_eq!(out, b"l1\nl2\nL3\n");
    }
}
