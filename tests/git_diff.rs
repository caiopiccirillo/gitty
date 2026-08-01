use std::fs;
use std::path::Path;

use git2::Repository;

use gitiff::diff::LineKind;
use gitiff::git::load_workdir_diff;

/// Stage `hello.txt` into the index so that later workdir edits show up as an
/// unstaged diff. No commit needed: `diff_index_to_workdir` compares against
/// the index.
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

    let view = load_workdir_diff(dir.path()).unwrap();
    assert_eq!(view.file_count, 1);
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

    let view = load_workdir_diff(dir.path()).unwrap();
    assert!(view.is_empty());
}

#[test]
fn errors_outside_a_repository() {
    let dir = tempfile::tempdir().unwrap();
    assert!(load_workdir_diff(dir.path()).is_err());
}
