use std::fs;
use std::path::Path;

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
