use std::fs;
use std::path::Path;

use git2::{Repository, Signature};
use ratatui::crossterm::event::{KeyCode, KeyEvent, KeyModifiers};

use gitiff::app::{App, Focus};
use gitiff::diff::{FileStatus, LineKind};
use gitiff::git::{
    load_staged_diff, load_unstaged_diff, stage_file, stage_hunk, unstage_file, unstage_hunk,
};

const BASE: &str = "l1\nl2\nl3\nl4\nl5\nl6\nl7\nl8\nl9\nl10\n";

fn commit_file(repo: &Repository, dir: &Path, name: &str, content: &str) {
    fs::write(dir.join(name), content).unwrap();
    let mut index = repo.index().unwrap();
    index.add_path(Path::new(name)).unwrap();
    index.write().unwrap();
    let tree = repo.find_tree(index.write_tree().unwrap()).unwrap();
    let sig = Signature::now("t", "t@t").unwrap();
    let parent = repo
        .head()
        .ok()
        .and_then(|head| head.target())
        .map(|oid| repo.find_commit(oid).unwrap());
    let parents: Vec<&git2::Commit> = parent.iter().collect();
    repo.commit(Some("HEAD"), &sig, &sig, "commit", &tree, &parents)
        .unwrap();
}

/// Repo with `f.txt` committed, then changed at lines 1 and 10 (two hunks).
fn repo_with_two_hunks() -> (tempfile::TempDir, String) {
    let dir = tempfile::tempdir().unwrap();
    let repo = Repository::init(dir.path()).unwrap();
    commit_file(&repo, dir.path(), "f.txt", BASE);
    let changed = BASE.replacen("l1", "L1", 1).replacen("l10", "L10", 1);
    fs::write(dir.path().join("f.txt"), &changed).unwrap();
    (dir, changed)
}

fn additions(view: &gitiff::diff::DiffView) -> Vec<&str> {
    view.lines
        .iter()
        .filter(|l| l.kind == LineKind::Addition)
        .map(|l| l.content.as_str())
        .collect()
}

#[test]
fn stages_one_of_two_hunks() {
    let (dir, changed) = repo_with_two_hunks();
    assert_eq!(load_unstaged_diff(dir.path()).unwrap().hunks().len(), 2);

    stage_hunk(dir.path(), 0, 0).unwrap();

    let unstaged = load_unstaged_diff(dir.path()).unwrap();
    assert_eq!(unstaged.hunks().len(), 1);
    assert_eq!(additions(&unstaged), vec!["L10"]);

    let staged = load_staged_diff(dir.path()).unwrap();
    assert_eq!(staged.hunks().len(), 1);
    assert_eq!(additions(&staged), vec!["L1"]);

    // The workdir file is untouched by staging.
    assert_eq!(
        fs::read_to_string(dir.path().join("f.txt")).unwrap(),
        changed
    );
}

#[test]
fn unstages_a_hunk() {
    let (dir, _) = repo_with_two_hunks();
    stage_hunk(dir.path(), 0, 0).unwrap();
    assert_eq!(load_staged_diff(dir.path()).unwrap().hunks().len(), 1);

    unstage_hunk(dir.path(), 0, 0).unwrap();

    assert!(load_staged_diff(dir.path()).unwrap().files.is_empty());
    let unstaged = load_unstaged_diff(dir.path()).unwrap();
    assert_eq!(unstaged.hunks().len(), 2);
    assert_eq!(additions(&unstaged), vec!["L1", "L10"]);
}

#[test]
fn unstages_a_newly_added_file_on_an_unborn_branch() {
    let dir = tempfile::tempdir().unwrap();
    let repo = Repository::init(dir.path()).unwrap();
    fs::write(dir.path().join("new.txt"), "hello\n").unwrap();
    let mut index = repo.index().unwrap();
    index.add_path(Path::new("new.txt")).unwrap();
    index.write().unwrap();

    let staged = load_staged_diff(dir.path()).unwrap();
    assert_eq!(staged.files.len(), 1);
    assert_eq!(staged.files[0].status, FileStatus::Added);

    unstage_hunk(dir.path(), 0, 0).unwrap();

    assert!(load_staged_diff(dir.path()).unwrap().files.is_empty());
    // Back to untracked, still on disk.
    assert!(dir.path().join("new.txt").exists());
}

#[test]
fn stages_and_unstages_a_whole_file() {
    let (dir, _) = repo_with_two_hunks();
    let file = load_unstaged_diff(dir.path()).unwrap().files[0].clone();

    stage_file(dir.path(), &file).unwrap();
    assert!(load_unstaged_diff(dir.path()).unwrap().files.is_empty());
    assert_eq!(load_staged_diff(dir.path()).unwrap().hunks().len(), 2);

    let staged_file = load_staged_diff(dir.path()).unwrap().files[0].clone();
    unstage_file(dir.path(), &staged_file).unwrap();
    assert!(load_staged_diff(dir.path()).unwrap().files.is_empty());
    assert_eq!(load_unstaged_diff(dir.path()).unwrap().hunks().len(), 2);
}

#[test]
fn stages_a_deleted_file() {
    let dir = tempfile::tempdir().unwrap();
    let repo = Repository::init(dir.path()).unwrap();
    commit_file(&repo, dir.path(), "f.txt", BASE);
    fs::remove_file(dir.path().join("f.txt")).unwrap();

    let view = load_unstaged_diff(dir.path()).unwrap();
    assert_eq!(view.files[0].status, FileStatus::Deleted);
    stage_file(dir.path(), &view.files[0]).unwrap();

    let staged = load_staged_diff(dir.path()).unwrap();
    assert_eq!(staged.files.len(), 1);
    assert_eq!(staged.files[0].status, FileStatus::Deleted);
    assert!(load_unstaged_diff(dir.path()).unwrap().files.is_empty());
}

#[test]
fn unstages_a_file_on_an_unborn_branch() {
    let dir = tempfile::tempdir().unwrap();
    let repo = Repository::init(dir.path()).unwrap();
    fs::write(dir.path().join("new.txt"), "hello\n").unwrap();
    let mut index = repo.index().unwrap();
    index.add_path(Path::new("new.txt")).unwrap();
    index.write().unwrap();

    let staged = load_staged_diff(dir.path()).unwrap();
    assert_eq!(staged.files[0].status, FileStatus::Added);
    unstage_file(dir.path(), &staged.files[0]).unwrap();
    assert!(load_staged_diff(dir.path()).unwrap().files.is_empty());
    assert!(dir.path().join("new.txt").exists());
}

#[test]
fn app_stages_a_file_from_the_files_pane() {
    let (dir, _) = repo_with_two_hunks();
    let mut app = App::load(dir.path()).unwrap();

    app.handle_key(KeyEvent::new(KeyCode::Char(' '), KeyModifiers::NONE));
    assert_eq!(app.message, None);
    assert!(app.unstaged.files.is_empty());
    assert_eq!(app.staged.files.len(), 1);

    app.handle_key(KeyEvent::new(KeyCode::Tab, KeyModifiers::NONE));
    app.handle_key(KeyEvent::new(KeyCode::Char(' '), KeyModifiers::NONE));
    assert_eq!(app.message, None);
    assert!(app.staged.files.is_empty());
    assert_eq!(app.unstaged.files.len(), 1);
}

#[test]
fn app_stages_and_unstages_the_hunk_under_the_cursor() {
    let (dir, _) = repo_with_two_hunks();
    let mut app = App::load(dir.path()).unwrap();
    assert_eq!(app.focus, Focus::Files);

    // Stage hunk 0 (cursor starts on the first hunk header).
    app.handle_key(KeyEvent::new(KeyCode::Enter, KeyModifiers::NONE));
    app.handle_key(KeyEvent::new(KeyCode::Char('s'), KeyModifiers::NONE));
    assert_eq!(app.message, None);
    assert_eq!(app.unstaged.hunks().len(), 1);
    assert_eq!(app.staged.hunks().len(), 1);

    // Stage the remaining hunk: the file leaves the unstaged list and the
    // focus falls back to the files pane.
    app.handle_key(KeyEvent::new(KeyCode::Char('s'), KeyModifiers::NONE));
    assert_eq!(app.message, None);
    assert!(app.unstaged.files.is_empty());
    assert_eq!(app.staged.hunks().len(), 2);
    assert_eq!(app.focus, Focus::Files);

    // Switch to the staged tab and unstage one hunk back.
    app.handle_key(KeyEvent::new(KeyCode::Tab, KeyModifiers::NONE));
    app.handle_key(KeyEvent::new(KeyCode::Enter, KeyModifiers::NONE));
    app.handle_key(KeyEvent::new(KeyCode::Char('u'), KeyModifiers::NONE));
    assert_eq!(app.message, None);
    assert_eq!(app.staged.hunks().len(), 1);
    assert_eq!(app.unstaged.hunks().len(), 1);
}
