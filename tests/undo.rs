mod common;

use std::fs;

use common::{BASE, commit_file};
use git2::Repository;
use ratatui::crossterm::event::{KeyCode, KeyEvent, KeyModifiers};

use gitty::app::{App, Severity};
use gitty::git::{load_staged_diff, load_unstaged_diff};

fn press(app: &mut App, code: KeyCode) {
    app.handle_key(KeyEvent::new(code, KeyModifiers::NONE));
}

/// Repo with `f.txt` committed, then changed at lines 1 and 10 (two hunks).
fn repo_with_two_hunks() -> tempfile::TempDir {
    let dir = tempfile::tempdir().unwrap();
    let repo = Repository::init(dir.path()).unwrap();
    commit_file(&repo, dir.path(), "f.txt", BASE);
    let changed = BASE.replacen("l1", "L1", 1).replacen("l10", "L10", 1);
    fs::write(dir.path().join("f.txt"), changed).unwrap();
    dir
}

#[test]
fn undo_restores_a_staged_file() {
    let dir = repo_with_two_hunks();
    let mut app = App::load(dir.path()).unwrap();

    // Stage the whole file from the files pane.
    press(&mut app, KeyCode::Char(' '));
    app.wait_for_refresh();
    assert!(app.unstaged.files.is_empty());
    assert_eq!(app.staged.files.len(), 1);

    // Undo: the file is unstaged again and the worktree is untouched.
    press(&mut app, KeyCode::Char('z'));
    app.wait_for_refresh();
    assert_eq!(app.staged.files.len(), 0);
    assert_eq!(app.unstaged.files.len(), 1);
    assert!(load_staged_diff(dir.path()).unwrap().files.is_empty());
    assert_eq!(load_unstaged_diff(dir.path()).unwrap().files.len(), 1);
    let content = fs::read_to_string(dir.path().join("f.txt")).unwrap();
    assert!(
        content.starts_with("L1\n"),
        "worktree unchanged by the undo"
    );

    // The message reports the undo.
    assert!(
        app.message
            .as_ref()
            .is_some_and(|m| m.severity == Severity::Success && m.text.starts_with("undid"))
    );
}

#[test]
fn undo_restores_discarded_changes() {
    let dir = repo_with_two_hunks();
    let mut app = App::load(dir.path()).unwrap();

    // Discard the first hunk from the diff pane.
    press(&mut app, KeyCode::Enter);
    press(&mut app, KeyCode::Char('d'));
    press(&mut app, KeyCode::Char('y'));
    app.wait_for_refresh();
    assert_eq!(app.unstaged.hunks().len(), 1, "one hunk discarded");
    let content = fs::read_to_string(dir.path().join("f.txt")).unwrap();
    assert!(!content.starts_with("L1\n"), "the first change is gone");

    // Undo: both hunks are back in the worktree.
    press(&mut app, KeyCode::Char('z'));
    app.wait_for_refresh();
    let content = fs::read_to_string(dir.path().join("f.txt")).unwrap();
    assert!(
        content.starts_with("L1\n"),
        "the discarded change is restored"
    );
    assert!(content.contains("L10"), "the other change survives");
    assert_eq!(app.unstaged.hunks().len(), 2);
}

#[test]
fn undo_unstages_a_file() {
    let dir = repo_with_two_hunks();
    let mut app = App::load(dir.path()).unwrap();

    // Stage, then unstage, then undo: the file is staged again.
    press(&mut app, KeyCode::Char(' '));
    app.wait_for_refresh();
    press(&mut app, KeyCode::Tab);
    press(&mut app, KeyCode::Char(' '));
    app.wait_for_refresh();
    assert!(app.staged.files.is_empty());

    press(&mut app, KeyCode::Char('z'));
    app.wait_for_refresh();
    assert_eq!(app.staged.files.len(), 1, "undo restores the staging");
}

#[test]
fn undo_with_an_empty_stack_reports_it() {
    let dir = repo_with_two_hunks();
    let mut app = App::load(dir.path()).unwrap();

    press(&mut app, KeyCode::Char('z'));
    assert!(
        app.message
            .as_ref()
            .is_some_and(|m| m.severity == Severity::Info && m.text == "nothing to undo")
    );
}

#[test]
fn undo_stacks_multiple_operations() {
    let dir = repo_with_two_hunks();
    let mut app = App::load(dir.path()).unwrap();

    // Stage hunk 0, then hunk 1, then undo twice: both back unstaged.
    press(&mut app, KeyCode::Enter);
    press(&mut app, KeyCode::Char('s'));
    app.wait_for_refresh();
    press(&mut app, KeyCode::Char('s'));
    app.wait_for_refresh();
    assert!(app.unstaged.files.is_empty());
    assert_eq!(app.staged.hunks().len(), 2);

    press(&mut app, KeyCode::Char('z'));
    app.wait_for_refresh();
    assert_eq!(app.staged.hunks().len(), 1);
    press(&mut app, KeyCode::Char('z'));
    app.wait_for_refresh();
    assert!(app.staged.files.is_empty());
    assert_eq!(app.unstaged.hunks().len(), 2);

    press(&mut app, KeyCode::Char('z'));
    assert!(
        app.message
            .as_ref()
            .is_some_and(|m| m.text == "nothing to undo")
    );
}
