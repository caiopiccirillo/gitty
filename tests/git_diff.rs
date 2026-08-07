mod common;

use std::fs;
use std::path::Path;

use common::{BASE, commit_file};
use git2::Repository;

use gitiff::diff::{FileStatus, LineKind};
use gitiff::git::{load_staged_diff, load_unstaged_diff};

/// Add `name` to the index. No commit needed: `diff_index_to_workdir`
/// compares the workdir against the index, `diff_tree_to_index` against the
/// empty tree when there is no HEAD.
fn stage_file(repo: &Repository, name: &str) {
    let mut index = repo.index().unwrap();
    index.add_path(Path::new(name)).unwrap();
    index.write().unwrap();
}

#[test]
fn loads_unstaged_diff() {
    let dir = tempfile::tempdir().unwrap();
    let repo = Repository::init(dir.path()).unwrap();

    fs::write(dir.path().join("hello.txt"), "hello\n").unwrap();
    stage_file(&repo, "hello.txt");
    fs::write(dir.path().join("hello.txt"), "hello\nworld\n").unwrap();

    let view = load_unstaged_diff(dir.path()).unwrap();
    assert_eq!(view.files.len(), 1);
    assert_eq!(view.files[0].path, "hello.txt");
    assert_eq!(view.files[0].status, FileStatus::Modified);
    assert!(
        view.lines
            .iter()
            .any(|l| l.kind == LineKind::Addition && l.content == "world")
    );
    assert!(
        view.lines
            .iter()
            .any(|l| l.kind == LineKind::Context && l.content == "hello")
    );
    assert!(view.lines.iter().any(|l| l.kind == LineKind::HunkHeader));
    // The hunk header shows 1-based positions plus the section heading.
    assert!(
        view.lines
            .iter()
            .any(|l| l.kind == LineKind::HunkHeader && l.content == "@@ -1,1 +1,2 @@ hello")
    );
    // The file header arrives as one chunk and must be split into lines.
    assert!(view.lines.iter().any(
        |l| l.kind == LineKind::FileHeader && l.content == "diff --git a/hello.txt b/hello.txt"
    ));
    // Every hunk line is tagged with its file and hunk index.
    assert!(
        view.lines
            .iter()
            .filter(|l| l.kind == LineKind::Addition)
            .all(|l| l.file_idx == 0 && l.hunk_idx == Some(0))
    );
}

#[test]
fn clean_repo_has_empty_diff() {
    let dir = tempfile::tempdir().unwrap();
    let repo = Repository::init(dir.path()).unwrap();

    fs::write(dir.path().join("hello.txt"), "hello\n").unwrap();
    stage_file(&repo, "hello.txt");

    let view = load_unstaged_diff(dir.path()).unwrap();
    assert!(view.is_empty());
}

#[test]
fn staged_diff_shows_index_changes() {
    let dir = tempfile::tempdir().unwrap();
    let repo = Repository::init(dir.path()).unwrap();

    fs::write(dir.path().join("new.txt"), "hello\n").unwrap();
    stage_file(&repo, "new.txt");

    // No HEAD commit: the whole index is staged against the empty tree.
    let view = load_staged_diff(dir.path()).unwrap();
    assert_eq!(view.files.len(), 1);
    assert_eq!(view.files[0].status, FileStatus::Added);
    assert!(
        view.lines
            .iter()
            .any(|l| l.kind == LineKind::Addition && l.content == "hello")
    );
}

#[test]
fn errors_outside_a_repository() {
    let dir = tempfile::tempdir().unwrap();
    assert!(load_unstaged_diff(dir.path()).is_err());
    assert!(load_staged_diff(dir.path()).is_err());
}

#[test]
fn binary_files_show_a_placeholder_line() {
    let dir = tempfile::tempdir().unwrap();
    let repo = Repository::init(dir.path()).unwrap();

    fs::write(dir.path().join("bin.dat"), b"\x00\x01 binary\n").unwrap();
    stage_file(&repo, "bin.dat");
    fs::write(dir.path().join("bin.dat"), b"\x00\x01 binary changed\n").unwrap();

    let view = load_unstaged_diff(dir.path()).unwrap();
    assert_eq!(view.files.len(), 1);
    assert!(
        view.lines.iter().any(|l| {
            l.kind == LineKind::Meta
                && l.content == "Binary files a/bin.dat and b/bin.dat differ"
        })
    );
    assert!(view.hunks().is_empty(), "binary files have no hunks");
}

#[cfg(unix)]
#[test]
fn mode_only_change_shows_mode_lines() {
    use std::os::unix::fs::PermissionsExt;

    let dir = tempfile::tempdir().unwrap();
    let repo = Repository::init(dir.path()).unwrap();
    commit_file(&repo, dir.path(), "f.txt", BASE);

    let mut perms = fs::metadata(dir.path().join("f.txt")).unwrap().permissions();
    perms.set_mode(0o755);
    fs::set_permissions(dir.path().join("f.txt"), perms).unwrap();

    let view = load_unstaged_diff(dir.path()).unwrap();
    assert_eq!(view.files[0].status, FileStatus::Modified);
    let headers: Vec<&str> = view
        .lines
        .iter()
        .filter(|l| l.kind == LineKind::FileHeader)
        .map(|l| l.content.as_str())
        .collect();
    assert!(headers.contains(&"old mode 100644"));
    assert!(headers.contains(&"new mode 100755"));
    assert!(view.hunks().is_empty(), "no content change, no hunks");
}

#[cfg(unix)]
#[test]
fn a_replaced_by_a_symlink_is_a_type_change() {
    let dir = tempfile::tempdir().unwrap();
    let repo = Repository::init(dir.path()).unwrap();
    commit_file(&repo, dir.path(), "f.txt", BASE);

    std::fs::remove_file(dir.path().join("f.txt")).unwrap();
    std::os::unix::fs::symlink("target", dir.path().join("f.txt")).unwrap();

    let view = load_unstaged_diff(dir.path()).unwrap();
    assert_eq!(view.files[0].status, FileStatus::TypeChange);
    let headers: Vec<&str> = view
        .lines
        .iter()
        .filter(|l| l.kind == LineKind::FileHeader)
        .map(|l| l.content.as_str())
        .collect();
    assert!(headers.contains(&"old mode 100644"));
    assert!(headers.contains(&"new mode 120000"));
}
