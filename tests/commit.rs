mod common;

use std::fs;
use std::path::Path;

use common::{BASE, commit_file};
use git2::Repository;
use ratatui::crossterm::event::{KeyCode, KeyEvent, KeyModifiers};

use gitty::app::App;
use gitty::git::{commit, load_staged_diff, load_unstaged_diff, stage_file};

fn init_with_identity() -> (tempfile::TempDir, Repository) {
    let dir = tempfile::tempdir().unwrap();
    let repo = Repository::init(dir.path()).unwrap();
    let mut config = repo.config().unwrap();
    config.set_str("user.name", "t").unwrap();
    config.set_str("user.email", "t@t").unwrap();
    (dir, repo)
}

fn press(app: &mut App, code: KeyCode) {
    app.handle_key(KeyEvent::new(code, KeyModifiers::NONE));
}

#[test]
fn commits_the_staged_changes() {
    let (dir, repo) = init_with_identity();
    commit_file(&repo, dir.path(), "f.txt", BASE);
    fs::write(dir.path().join("f.txt"), BASE.replacen("l1", "L1", 1)).unwrap();
    let file = load_unstaged_diff(dir.path()).unwrap().files[0].clone();
    stage_file(dir.path(), &file).unwrap();

    let short = commit(dir.path(), "change l1").unwrap();
    assert_eq!(short.len(), 7);
    assert!(load_staged_diff(dir.path()).unwrap().files.is_empty());
    assert!(load_unstaged_diff(dir.path()).unwrap().files.is_empty());
    let head = repo.head().unwrap().peel_to_commit().unwrap();
    assert_eq!(head.message().unwrap().trim(), "change l1");
    assert_eq!(head.parent_count(), 1);
}

#[test]
fn commits_on_an_unborn_branch() {
    let (dir, repo) = init_with_identity();
    fs::write(dir.path().join("f.txt"), BASE).unwrap();
    let mut index = repo.index().unwrap();
    index.add_path(Path::new("f.txt")).unwrap();
    index.write().unwrap();

    commit(dir.path(), "initial").unwrap();
    let head = repo.head().unwrap().peel_to_commit().unwrap();
    assert_eq!(head.parent_count(), 0);
    assert_eq!(head.message().unwrap().trim(), "initial");
}

#[test]
fn commits_nested_directories_in_git_order() {
    let (dir, repo) = init_with_identity();
    commit_file(&repo, dir.path(), "a.txt", BASE);
    fs::create_dir_all(dir.path().join("d/e")).unwrap();
    commit_file(&repo, dir.path(), "d/e/b.txt", BASE);
    commit_file(&repo, dir.path(), "d/f.txt", BASE);
    commit_file(&repo, dir.path(), "x.txt", BASE);
    fs::write(dir.path().join("d/e/b.txt"), BASE.replacen("l1", "L1", 1)).unwrap();
    let file = load_unstaged_diff(dir.path()).unwrap().files[0].clone();
    assert_eq!(file.path, "d/e/b.txt");
    stage_file(dir.path(), &file).unwrap();

    commit(dir.path(), "change b").unwrap();

    let tree = repo
        .head()
        .unwrap()
        .peel_to_commit()
        .unwrap()
        .tree()
        .unwrap();
    let names: Vec<String> = tree.iter().map(|e| e.name().unwrap().to_string()).collect();
    assert_eq!(names, ["a.txt", "d", "x.txt"], "entries are sorted like git");
    let d = tree
        .get_name("d")
        .unwrap()
        .to_object(&repo)
        .unwrap()
        .peel_to_tree()
        .unwrap();
    let names: Vec<String> = d.iter().map(|e| e.name().unwrap().to_string()).collect();
    assert_eq!(names, ["e", "f.txt"]);
    let b = d
        .get_name("e")
        .unwrap()
        .to_object(&repo)
        .unwrap()
        .into_tree()
        .unwrap()
        .get_name("b.txt")
        .unwrap()
        .to_object(&repo)
        .unwrap()
        .into_blob()
        .unwrap();
    assert_eq!(b.content(), BASE.replacen("l1", "L1", 1).as_bytes());
    assert!(load_unstaged_diff(dir.path()).unwrap().files.is_empty());
}

#[test]
fn app_commit_flow() {
    let (dir, repo) = init_with_identity();
    commit_file(&repo, dir.path(), "f.txt", BASE);
    fs::write(dir.path().join("f.txt"), BASE.replacen("l1", "L1", 1)).unwrap();
    let mut app = App::load(dir.path()).unwrap();

    // Nothing staged yet: the box must not open.
    press(&mut app, KeyCode::Char('c'));
    assert!(app.commit_input.is_none());
    assert_eq!(app.message.as_deref(), Some("nothing staged"));

    // Stage the file, then commit through the box.
    press(&mut app, KeyCode::Char(' '));
    assert_eq!(app.staged.files.len(), 1);
    press(&mut app, KeyCode::Char('c'));
    assert!(app.commit_input.is_some());
    for c in "my first commit".chars() {
        press(&mut app, KeyCode::Char(c));
    }
    press(&mut app, KeyCode::Enter);
    assert!(
        app.message
            .as_deref()
            .is_some_and(|m| m.starts_with("committed "))
    );
    assert!(app.staged.files.is_empty());
    assert!(app.unstaged.files.is_empty());
    let head = repo.head().unwrap().peel_to_commit().unwrap();
    assert_eq!(head.message().unwrap().trim(), "my first commit");
}

#[test]
fn esc_cancels_the_commit() {
    let (dir, repo) = init_with_identity();
    commit_file(&repo, dir.path(), "f.txt", BASE);
    fs::write(dir.path().join("f.txt"), BASE.replacen("l1", "L1", 1)).unwrap();
    let mut app = App::load(dir.path()).unwrap();

    press(&mut app, KeyCode::Char(' '));
    press(&mut app, KeyCode::Char('c'));
    press(&mut app, KeyCode::Char('x'));
    press(&mut app, KeyCode::Esc);
    assert!(app.commit_input.is_none());
    assert_eq!(app.staged.files.len(), 1, "still staged, nothing committed");
    let head = repo.head().unwrap().peel_to_commit().unwrap();
    assert_eq!(head.message().unwrap().trim(), "commit");
}

#[test]
fn empty_message_is_rejected() {
    let (dir, _repo) = init_with_identity();
    commit_file(&_repo, dir.path(), "f.txt", BASE);
    fs::write(dir.path().join("f.txt"), BASE.replacen("l1", "L1", 1)).unwrap();
    let mut app = App::load(dir.path()).unwrap();

    press(&mut app, KeyCode::Char(' '));
    press(&mut app, KeyCode::Char('c'));
    press(&mut app, KeyCode::Enter);
    assert_eq!(app.message.as_deref(), Some("empty commit message"));
    assert_eq!(app.staged.files.len(), 1);
}
