mod common;

use std::fs;
use std::path::Path;

use common::{BASE, commit_file};
use git2::Repository;
use ratatui::crossterm::event::{KeyCode, KeyEvent, KeyModifiers};

use gitty::app::App;
use gitty::diff::{LineKind, SelectedLines};
use gitty::git::{
    discard_file, discard_hunk, discard_lines, discard_staged_file, discard_staged_hunk,
    load_staged_diff, load_unstaged_diff, stage_file, stage_hunk,
};

/// Repo with `f.txt` committed, then changed at lines 1 and 10 (two hunks).
fn repo_with_two_hunks() -> (tempfile::TempDir, String) {
    let dir = tempfile::tempdir().unwrap();
    let repo = Repository::init(dir.path()).unwrap();
    commit_file(&repo, dir.path(), "f.txt", BASE);
    let changed = BASE.replacen("l1", "L1", 1).replacen("l10", "L10", 1);
    fs::write(dir.path().join("f.txt"), &changed).unwrap();
    (dir, changed)
}

/// Repo with f.txt where l2 and l4 changed, close enough to share one hunk.
fn repo_with_close_changes() -> tempfile::TempDir {
    let dir = tempfile::tempdir().unwrap();
    let repo = Repository::init(dir.path()).unwrap();
    commit_file(&repo, dir.path(), "f.txt", BASE);
    let changed = BASE.replacen("l2", "L2", 1).replacen("l4", "L4", 1);
    fs::write(dir.path().join("f.txt"), changed).unwrap();
    dir
}

fn selected(adds: &[usize], dels: &[usize]) -> SelectedLines {
    SelectedLines {
        additions: adds.iter().copied().collect(),
        deletions: dels.iter().copied().collect(),
    }
}

fn additions(view: &gitty::diff::DiffView) -> Vec<&str> {
    view.lines
        .iter()
        .filter(|l| l.kind == LineKind::Addition)
        .map(|l| l.content.as_str())
        .collect()
}

#[test]
fn discards_a_hunk_of_the_unstaged_diff() {
    let (dir, _) = repo_with_two_hunks();
    assert_eq!(load_unstaged_diff(dir.path()).unwrap().hunks().len(), 2);

    discard_hunk(dir.path(), 0, 0).unwrap();

    // Only the L1 change is gone from the worktree; the L10 change stays.
    assert_eq!(
        fs::read_to_string(dir.path().join("f.txt")).unwrap(),
        BASE.replacen("l10", "L10", 1)
    );
    // The index is untouched.
    assert!(load_staged_diff(dir.path()).unwrap().files.is_empty());
    let unstaged = load_unstaged_diff(dir.path()).unwrap();
    assert_eq!(unstaged.hunks().len(), 1);
    assert_eq!(additions(&unstaged), vec!["L10"]);
}

#[test]
fn discards_only_the_selected_lines() {
    let dir = repo_with_close_changes();

    // The hunk is: ctx l1, -l2, +L2, ctx l3, -l4, +L4, ctx l5...
    // Discard only the l2 change (first addition + first deletion).
    discard_lines(dir.path(), 0, 0, &selected(&[0], &[0])).unwrap();

    assert_eq!(
        fs::read_to_string(dir.path().join("f.txt")).unwrap(),
        BASE.replacen("l4", "L4", 1)
    );
}

#[test]
fn discard_restores_a_deleted_file() {
    let dir = tempfile::tempdir().unwrap();
    let repo = Repository::init(dir.path()).unwrap();
    commit_file(&repo, dir.path(), "f.txt", BASE);
    fs::remove_file(dir.path().join("f.txt")).unwrap();

    let view = load_unstaged_diff(dir.path()).unwrap();
    assert_eq!(view.files[0].status, gitty::diff::FileStatus::Deleted);
    discard_file(dir.path(), &view.files[0]).unwrap();

    assert_eq!(fs::read_to_string(dir.path().join("f.txt")).unwrap(), BASE);
    assert!(load_unstaged_diff(dir.path()).unwrap().files.is_empty());
}

#[test]
fn discard_deletes_an_untracked_file() {
    let dir = tempfile::tempdir().unwrap();
    let repo = Repository::init(dir.path()).unwrap();
    commit_file(&repo, dir.path(), "f.txt", BASE);
    fs::write(dir.path().join("new.txt"), "brand\nnew\n").unwrap();

    let view = load_unstaged_diff(dir.path()).unwrap();
    let file = view.files.iter().find(|f| f.path == "new.txt").unwrap().clone();
    discard_file(dir.path(), &file).unwrap();

    assert!(!dir.path().join("new.txt").exists());
    assert!(load_unstaged_diff(dir.path()).unwrap().files.is_empty());
}

#[test]
fn discard_restores_a_whole_modified_file() {
    let (dir, _) = repo_with_two_hunks();
    let file = load_unstaged_diff(dir.path()).unwrap().files[0].clone();

    discard_file(dir.path(), &file).unwrap();

    assert_eq!(fs::read_to_string(dir.path().join("f.txt")).unwrap(), BASE);
    assert!(load_unstaged_diff(dir.path()).unwrap().files.is_empty());
}

#[test]
fn discards_a_staged_hunk() {
    let (dir, _) = repo_with_two_hunks();
    stage_hunk(dir.path(), 0, 0).unwrap();
    assert_eq!(load_staged_diff(dir.path()).unwrap().hunks().len(), 1);

    discard_staged_hunk(dir.path(), 0, 0).unwrap();

    // The index entry reverted to HEAD for the hunk region...
    assert!(load_staged_diff(dir.path()).unwrap().files.is_empty());
    // ...and so did the worktree, leaving only the unstaged L10 change.
    assert_eq!(
        fs::read_to_string(dir.path().join("f.txt")).unwrap(),
        BASE.replacen("l10", "L10", 1)
    );
    let unstaged = load_unstaged_diff(dir.path()).unwrap();
    assert_eq!(unstaged.hunks().len(), 1);
    assert_eq!(additions(&unstaged), vec!["L10"]);
}

#[test]
fn discards_a_staged_file() {
    let (dir, _) = repo_with_two_hunks();
    let file = load_unstaged_diff(dir.path()).unwrap().files[0].clone();
    stage_file(dir.path(), &file).unwrap();
    let staged_file = load_staged_diff(dir.path()).unwrap().files[0].clone();

    discard_staged_file(dir.path(), &staged_file).unwrap();

    assert!(load_staged_diff(dir.path()).unwrap().files.is_empty());
    assert!(load_unstaged_diff(dir.path()).unwrap().files.is_empty());
    assert_eq!(fs::read_to_string(dir.path().join("f.txt")).unwrap(), BASE);
}

#[test]
fn discards_a_staged_added_file() {
    let dir = tempfile::tempdir().unwrap();
    let repo = Repository::init(dir.path()).unwrap();
    fs::write(dir.path().join("new.txt"), "hello\n").unwrap();
    let mut index = repo.index().unwrap();
    index.add_path(Path::new("new.txt")).unwrap();
    index.write().unwrap();

    let staged = load_staged_diff(dir.path()).unwrap();
    assert_eq!(staged.files[0].status, gitty::diff::FileStatus::Added);
    discard_staged_file(dir.path(), &staged.files[0]).unwrap();

    assert!(load_staged_diff(dir.path()).unwrap().files.is_empty());
    assert!(!dir.path().join("new.txt").exists());
    assert!(load_unstaged_diff(dir.path()).unwrap().files.is_empty());
}

#[test]
fn app_discards_a_hunk_after_confirmation() {
    let (dir, _) = repo_with_two_hunks();
    let mut app = App::load(dir.path()).unwrap();
    assert_eq!(app.unstaged.hunks().len(), 2);

    // Enter focuses the diff on the first changed line (hunk 0).
    app.handle_key(KeyEvent::new(KeyCode::Enter, KeyModifiers::NONE));
    app.handle_key(KeyEvent::new(KeyCode::Char('d'), KeyModifiers::NONE));
    assert!(app.discard_confirm.is_some());

    // n cancels: nothing changes.
    app.handle_key(KeyEvent::new(KeyCode::Char('n'), KeyModifiers::NONE));
    assert!(app.discard_confirm.is_none());
    assert_eq!(load_unstaged_diff(dir.path()).unwrap().hunks().len(), 2);

    // d again, then y: the hunk is discarded.
    app.handle_key(KeyEvent::new(KeyCode::Char('d'), KeyModifiers::NONE));
    app.handle_key(KeyEvent::new(KeyCode::Char('y'), KeyModifiers::NONE));
    assert_eq!(app.message, None);
    assert_eq!(app.unstaged.hunks().len(), 1);
    assert!(app.discard_confirm.is_none());
}

#[test]
fn app_discards_a_staged_hunk_with_confirmation() {
    let (dir, _) = repo_with_two_hunks();
    stage_hunk(dir.path(), 0, 0).unwrap();
    let mut app = App::load(dir.path()).unwrap();

    // Switch to the staged tab, open the file, discard the hunk.
    app.handle_key(KeyEvent::new(KeyCode::Tab, KeyModifiers::NONE));
    app.handle_key(KeyEvent::new(KeyCode::Enter, KeyModifiers::NONE));
    app.handle_key(KeyEvent::new(KeyCode::Char('d'), KeyModifiers::NONE));
    assert!(app.discard_confirm.is_some());
    app.handle_key(KeyEvent::new(KeyCode::Char('y'), KeyModifiers::NONE));

    assert_eq!(app.message, None);
    assert!(app.staged.files.is_empty());
    assert_eq!(app.unstaged.hunks().len(), 1);
    assert_eq!(
        fs::read_to_string(dir.path().join("f.txt")).unwrap(),
        BASE.replacen("l10", "L10", 1)
    );
}
