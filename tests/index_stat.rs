//! The index entry's cached stat must never claim to match a worktree file
//! whose content differs from the entry's blob.
//!
//! gitty writes blobs it builds itself: a spliced hunk when staging part of
//! a file, HEAD's version when unstaging. The entry's stat still describes
//! the worktree file it was last known to match, and the index/worktree
//! comparison uses that stat to decide whether it needs to read the file at
//! all. Leaving it in place makes the remaining changes disappear from the
//! unstaged view.
//!
//! The window is small — the stat has to still look current, which happens
//! when the edit lands in the same second as the last stat with the file
//! size unchanged — and it cannot be forced from userspace, because a test
//! can set a file's mtime but not its ctime. So these are stress tests:
//! before the fix they failed on every run, and the failure is a wrong
//! answer, never a flaky pass.

mod common;

use std::fs;

use common::{BASE, commit_file};
use git2::Repository;

use gitty::diff::{DiffView, LineKind, SelectedLines};
use gitty::git::{load_staged_diff, load_unstaged_diff, stage_file, stage_lines, unstage_file};

/// Enough concurrency to land an edit and a stat in the same tick.
const THREADS: usize = 16;
const ITERATIONS: usize = 60;

fn additions(view: &DiffView) -> Vec<String> {
    view.lines
        .iter()
        .filter(|l| l.kind == LineKind::Addition)
        .map(|l| l.content.clone())
        .collect()
}

fn selected(adds: &[usize], dels: &[usize]) -> SelectedLines {
    SelectedLines {
        additions: adds.iter().copied().collect(),
        deletions: dels.iter().copied().collect(),
    }
}

/// A repository whose single file has two same-length edits, so the file
/// size never changes and only the timestamp or the content can tell the
/// two versions apart.
fn repo_with_close_changes() -> tempfile::TempDir {
    let dir = tempfile::tempdir().unwrap();
    let repo = Repository::init(dir.path()).unwrap();
    commit_file(&repo, dir.path(), "f.txt", BASE);
    let changed = BASE.replacen("l2", "L2", 1).replacen("l4", "L4", 1);
    fs::write(dir.path().join("f.txt"), changed).unwrap();
    dir
}

fn stress(scenario: fn(usize)) {
    let workers: Vec<_> = (0..THREADS)
        .map(|t| std::thread::spawn(move || (0..ITERATIONS).for_each(|i| scenario(t * 1000 + i))))
        .collect();
    for worker in workers {
        worker.join().expect("a worker saw the wrong diff");
    }
}

/// Staging one line of a hunk must leave the other one unstaged.
fn partial_stage(i: usize) {
    let dir = repo_with_close_changes();
    stage_lines(dir.path(), 0, 0, &selected(&[0], &[0])).unwrap();

    let staged = additions(&load_staged_diff(dir.path()).unwrap());
    let unstaged = additions(&load_unstaged_diff(dir.path()).unwrap());
    assert_eq!(staged, ["L2"], "iteration {i}: staged half");
    assert_eq!(unstaged, ["L4"], "iteration {i}: unstaged half");
}

#[test]
fn a_partial_stage_keeps_the_rest_unstaged() {
    stress(partial_stage);
}

/// Unstaging a whole file must bring all of its changes back.
fn whole_file_round_trip(i: usize) {
    let dir = repo_with_close_changes();
    let file = load_unstaged_diff(dir.path()).unwrap().files[0].clone();
    stage_file(dir.path(), &file).unwrap();
    let staged = load_staged_diff(dir.path()).unwrap().files[0].clone();
    unstage_file(dir.path(), &staged).unwrap();

    let unstaged = additions(&load_unstaged_diff(dir.path()).unwrap());
    assert_eq!(unstaged, ["L2", "L4"], "iteration {i}: after unstaging");
}

#[test]
fn unstaging_a_whole_file_restores_its_changes() {
    stress(whole_file_round_trip);
}

/// An untracked file has no index entry to reuse, so it takes the other
/// branch of `apply_to_index`; guard it too.
#[test]
fn a_partial_stage_of_an_untracked_file_keeps_the_rest_unstaged() {
    let dir = tempfile::tempdir().unwrap();
    let repo = Repository::init(dir.path()).unwrap();
    commit_file(&repo, dir.path(), "f.txt", BASE);
    fs::write(dir.path().join("new.txt"), "a\nb\nc\nd\n").unwrap();

    let view = load_unstaged_diff(dir.path()).unwrap();
    let idx = view.files.iter().position(|f| f.path == "new.txt").unwrap();
    stage_lines(dir.path(), idx, 0, &selected(&[0], &[])).unwrap();

    assert_eq!(additions(&load_staged_diff(dir.path()).unwrap()), ["a"]);
    assert_eq!(
        additions(&load_unstaged_diff(dir.path()).unwrap()),
        ["b", "c", "d"]
    );
}
