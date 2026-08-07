//! Rendering the per-file diff model into the flat, render-friendly
//! [`DiffView`] line model.

use gix::bstr::{BStr, ByteSlice};
use gix::diff::blob::unified_diff::DiffLineKind;
use gix::index::entry::Mode;

use crate::diff::{DiffLine, DiffView, FileInfo, FileStatus, LineKind};

use super::model::{FileDiff, Hunk};

/// Convert the per-file diff into our flat, render-friendly line model.
pub(super) fn diff_to_view(files: &[FileDiff]) -> DiffView {
    let mut lines = Vec::new();
    for (file_idx, file) in files.iter().enumerate() {
        for header in file_headers(file) {
            lines.push(DiffLine {
                kind: LineKind::FileHeader,
                content: header,
                file_idx,
                hunk_idx: None,
            });
        }
        if file.binary {
            lines.push(DiffLine {
                kind: LineKind::Meta,
                content: format!(
                    "Binary files a/{} and b/{} differ",
                    file.old_path, file.new_path
                ),
                file_idx,
                hunk_idx: None,
            });
            continue;
        }
        for (hunk_idx, hunk) in file.hunks.iter().enumerate() {
            lines.push(DiffLine {
                kind: LineKind::HunkHeader,
                content: hunk_header_line(hunk),
                file_idx,
                hunk_idx: Some(hunk_idx),
            });
            for (kind, content) in &hunk.lines {
                let kind = match kind {
                    DiffLineKind::Context => LineKind::Context,
                    DiffLineKind::Add => LineKind::Addition,
                    DiffLineKind::Remove => LineKind::Deletion,
                };
                lines.push(DiffLine {
                    kind,
                    content: String::from_utf8_lossy(content).into_owned(),
                    file_idx,
                    hunk_idx: Some(hunk_idx),
                });
            }
        }
    }
    DiffView {
        lines,
        files: files
            .iter()
            .map(|f| FileInfo {
                path: f.path.clone(),
                status: f.status,
            })
            .collect(),
    }
}

/// The `@@ -a,b +c,d @@` line of a hunk (positions are 1-based already),
/// with the section heading like `git`.
fn hunk_header_line(hunk: &Hunk) -> String {
    let h = &hunk.header;
    let section = hunk
        .lines
        .iter()
        .find(|(kind, _)| matches!(kind, DiffLineKind::Context))
        .map(|(_, content)| content.as_bstr())
        .and_then(section_of);
    match section {
        Some(section) => format!(
            "@@ -{},{} +{},{} @@ {}",
            h.before_hunk_start, h.before_hunk_len, h.after_hunk_start, h.after_hunk_len, section
        ),
        None => format!(
            "@@ -{},{} +{},{} @@",
            h.before_hunk_start, h.before_hunk_len, h.after_hunk_start, h.after_hunk_len
        ),
    }
}

/// The section heading of a hunk: the first context line, as `git` derives
/// it (skipping empty, whitespace-led or comment lines, capped at 80 bytes).
fn section_of(line: &BStr) -> Option<String> {
    if line.is_empty()
        || line[0].is_ascii_whitespace()
        || line[0] == b'#'
    {
        return None;
    }
    let mut section = String::from_utf8_lossy(&line[..line.len().min(80)]).into_owned();
    if line.len() > 80 {
        // Truncate at the last space so words aren't cut mid-way.
        if let Some(idx) = section.rfind(' ') {
            let before = section[..idx].trim_end();
            if !before.is_empty() {
                section = before.to_string();
            }
        }
    }
    Some(section)
}

/// The file header lines (`diff --git`, `index`, `---`, `+++`).
fn file_headers(file: &FileDiff) -> Vec<String> {
    let mut headers = vec![format!(
        "diff --git a/{} b/{}",
        file.old_path, file.new_path
    )];
    let old_mode = file.old_mode.map(mode_octal);
    let new_mode = file.new_mode.map(mode_octal);
    match file.status {
        FileStatus::Added => headers.push(format!("new file mode {}", new_mode.clone().unwrap_or_default())),
        FileStatus::Deleted => headers.push(format!("deleted file mode {}", old_mode.clone().unwrap_or_default())),
        _ if old_mode.is_some() && old_mode != new_mode => {
            headers.push(format!("old mode {}", old_mode.clone().unwrap_or_default()));
            headers.push(format!("new mode {}", new_mode.clone().unwrap_or_default()));
        }
        _ => {}
    }
    match (file.old_id, file.new_id) {
        (Some(old), Some(new)) => {
            let mode = (old_mode == new_mode)
                .then_some(old_mode)
                .flatten()
                .map(|m| format!(" {m}"))
                .unwrap_or_default();
            headers.push(format!("index {}..{}{}", short_id(old), short_id(new), mode));
        }
        (Some(old), None) => headers.push(format!("index {}..{}", short_id(old), zeros(old))),
        (None, Some(new)) => headers.push(format!("index {}..{}", zeros(new), short_id(new))),
        (None, None) => {}
    }
    let old_display = if file.old_id.is_none() {
        String::from("/dev/null")
    } else {
        format!("a/{}", file.old_path)
    };
    let new_display = if file.new_id.is_none() {
        String::from("/dev/null")
    } else {
        format!("b/{}", file.new_path)
    };
    headers.push(format!("--- {old_display}"));
    headers.push(format!("+++ {new_display}"));
    headers
}

fn mode_octal(mode: Mode) -> String {
    format!("{:06o}", mode.bits())
}

fn short_id(id: gix::ObjectId) -> String {
    id.to_string()[..7].to_string()
}

fn zeros(id: gix::ObjectId) -> String {
    "0".repeat(id.kind().len_in_hex().min(7))
}

#[cfg(test)]
mod tests {
    use super::*;
    use gix::diff::blob::unified_diff::HunkHeader;

    use crate::git::model::Hunk;

    /// A SHA-1 object id from a short hex prefix, padded with zeros.
    fn oid(prefix: &str) -> gix::ObjectId {
        let hex = format!("{prefix}{}", "0".repeat(40 - prefix.len()));
        gix::ObjectId::from_hex(hex.as_bytes()).unwrap()
    }

    fn header(before_start: u32, before_len: u32, after_start: u32, after_len: u32) -> HunkHeader {
        HunkHeader {
            before_hunk_start: before_start,
            before_hunk_len: before_len,
            after_hunk_start: after_start,
            after_hunk_len: after_len,
        }
    }

    fn hunk(header: HunkHeader, lines: Vec<(DiffLineKind, Vec<u8>)>) -> Hunk {
        Hunk { header, lines }
    }

    fn modified_file() -> FileDiff {
        FileDiff::new(
            BStr::new(b"f.txt"),
            Some(oid("a")),
            Some(Mode::FILE),
            Some(oid("b")),
            Some(Mode::FILE),
        )
    }

    #[test]
    fn derives_section_headings_like_git() {
        assert_eq!(section_of(BStr::new(b"fn main()")), Some("fn main()".into()));
        assert_eq!(section_of(BStr::new(b"")), None);
        assert_eq!(section_of(BStr::new(b"  indented")), None);
        assert_eq!(section_of(BStr::new(b"# comment")), None);
        let long = "x".repeat(100);
        assert_eq!(section_of(BStr::new(long.as_bytes())).unwrap().len(), 80);
    }

    #[test]
    fn formats_hunk_headers_with_and_without_section() {
        let with_section = hunk(
            header(1, 3, 2, 4),
            vec![
                (DiffLineKind::Context, b"fn main()".to_vec()),
                (DiffLineKind::Remove, b"old".to_vec()),
            ],
        );
        assert_eq!(hunk_header_line(&with_section), "@@ -1,3 +2,4 @@ fn main()");

        let without_section = hunk(header(1, 3, 2, 4), vec![(DiffLineKind::Remove, b"old".to_vec())]);
        assert_eq!(hunk_header_line(&without_section), "@@ -1,3 +2,4 @@");
    }

    #[test]
    fn headers_for_an_added_file() {
        let file = FileDiff::new(BStr::new(b"new.txt"), None, None, Some(oid("abcdef1")), Some(Mode::FILE));
        assert_eq!(
            file_headers(&file),
            vec![
                "diff --git a/new.txt b/new.txt",
                "new file mode 100644",
                "index 0000000..abcdef1",
                "--- /dev/null",
                "+++ b/new.txt",
            ]
        );
    }

    #[test]
    fn headers_for_a_deleted_file() {
        let file = FileDiff::new(BStr::new(b"old.txt"), Some(oid("deadbee")), Some(Mode::FILE), None, None);
        assert_eq!(
            file_headers(&file),
            vec![
                "diff --git a/old.txt b/old.txt",
                "deleted file mode 100644",
                "index deadbee..0000000",
                "--- a/old.txt",
                "+++ /dev/null",
            ]
        );
    }

    #[test]
    fn headers_for_a_modified_file_with_unchanged_mode() {
        let file = modified_file();
        assert_eq!(
            file_headers(&file),
            vec![
                "diff --git a/f.txt b/f.txt",
                "index a000000..b000000 100644",
                "--- a/f.txt",
                "+++ b/f.txt",
            ]
        );
    }

    #[test]
    fn headers_for_a_mode_change() {
        let file = FileDiff::new(
            BStr::new(b"f.txt"),
            Some(oid("a")),
            Some(Mode::FILE),
            Some(oid("b")),
            Some(Mode::FILE_EXECUTABLE),
        );
        assert_eq!(
            file_headers(&file),
            vec![
                "diff --git a/f.txt b/f.txt",
                "old mode 100644",
                "new mode 100755",
                "index a000000..b000000",
                "--- a/f.txt",
                "+++ b/f.txt",
            ]
        );
    }

    #[test]
    fn headers_for_a_type_change() {
        let file = FileDiff::new(
            BStr::new(b"f.txt"),
            Some(oid("a")),
            Some(Mode::SYMLINK),
            Some(oid("b")),
            Some(Mode::FILE),
        );
        assert_eq!(
            file_headers(&file),
            vec![
                "diff --git a/f.txt b/f.txt",
                "old mode 120000",
                "new mode 100644",
                "index a000000..b000000",
                "--- a/f.txt",
                "+++ b/f.txt",
            ]
        );
    }

    #[test]
    fn renders_lines_with_kinds_and_hunk_tags() {
        let mut file = modified_file();
        file.hunks = vec![hunk(
            header(1, 3, 1, 3),
            vec![
                (DiffLineKind::Context, b"ctx".to_vec()),
                (DiffLineKind::Remove, b"old".to_vec()),
                (DiffLineKind::Add, b"new".to_vec()),
            ],
        )];
        let view = diff_to_view(&[file]);

        let kinds: Vec<LineKind> = view.lines.iter().map(|l| l.kind).collect();
        assert_eq!(
            kinds,
            vec![
                LineKind::FileHeader, // diff --git
                LineKind::FileHeader, // index
                LineKind::FileHeader, // ---
                LineKind::FileHeader, // +++
                LineKind::HunkHeader,
                LineKind::Context,
                LineKind::Deletion,
                LineKind::Addition,
            ]
        );
        for (i, line) in view.lines.iter().enumerate() {
            assert_eq!(line.file_idx, 0);
            assert_eq!(line.hunk_idx, (i >= 4).then_some(0));
        }
        assert_eq!(view.files.len(), 1);
        assert_eq!(view.files[0].path, "f.txt");
    }

    #[test]
    fn renders_binary_files_as_a_placeholder() {
        let mut file = modified_file();
        file.binary = true;
        let view = diff_to_view(&[file]);
        let meta: Vec<&str> = view
            .lines
            .iter()
            .filter(|l| l.kind == LineKind::Meta)
            .map(|l| l.content.as_str())
            .collect();
        assert_eq!(meta, vec!["Binary files a/f.txt and b/f.txt differ"]);
        assert!(view.lines.iter().all(|l| l.hunk_idx.is_none()));
    }
}
