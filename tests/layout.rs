mod common;

use std::fs;

use common::{BASE, commit_file};
use git2::Repository;
use ratatui::crossterm::event::{KeyCode, KeyEvent, KeyModifiers};

use gitiff::app::{App, Focus, Mode, Side};
use gitiff::git::{load_staged_diff, load_unstaged_diff};

/// Repo with f.txt and g.txt, both modified (one hunk each).
fn repo_with_two_files() -> tempfile::TempDir {
    let dir = tempfile::tempdir().unwrap();
    let repo = Repository::init(dir.path()).unwrap();
    commit_file(&repo, dir.path(), "f.txt", BASE);
    commit_file(&repo, dir.path(), "g.txt", BASE);
    fs::write(dir.path().join("f.txt"), BASE.replacen("l1", "L1", 1)).unwrap();
    fs::write(dir.path().join("g.txt"), BASE.replacen("l1", "G1", 1)).unwrap();
    dir
}

fn press(app: &mut App, code: KeyCode) {
    app.handle_key(KeyEvent::new(code, KeyModifiers::NONE));
}

#[test]
fn split_mode_stages_from_the_unstaged_pane_and_unstages_from_the_staged_pane() {
    let dir = repo_with_two_files();
    let mut app = App::load(dir.path()).unwrap();

    press(&mut app, KeyCode::Char('m'));
    assert_eq!(app.mode, Mode::Split);
    assert_eq!(app.entries.len(), 2, "both files merged into one tree");

    // f.txt has unstaged changes only.
    let f = app.entries.iter().position(|e| e.path == "f.txt").unwrap();
    assert!(app.entries[f].unstaged.is_some());
    assert!(app.entries[f].staged.is_none());

    // Open f.txt's diff and stage its hunk from the unstaged pane.
    press(&mut app, KeyCode::Enter);
    press(&mut app, KeyCode::Char('s'));
    assert_eq!(app.message, None);
    assert_eq!(app.staged.hunks().len(), 1, "hunk staged");
    assert_eq!(app.unstaged.files.len(), 1, "only g.txt left unstaged");

    // The merged tree now shows f.txt on both sides.
    let f = app.entries.iter().position(|e| e.path == "f.txt").unwrap();
    assert!(app.entries[f].staged.is_some());
    assert!(app.entries[f].unstaged.is_none());

    // Tab moves the focus to the staged pane: the staged pane shows the
    // hunk, and u unstages it from there.
    press(&mut app, KeyCode::Tab);
    assert_eq!(app.side, Side::Staged);
    assert!(app.display_lines_for(Side::Staged).iter().any(|l| l.content == "L1"));
    press(&mut app, KeyCode::Char('u'));
    assert_eq!(app.message, None);
    assert!(load_staged_diff(dir.path()).unwrap().files.is_empty());
    assert_eq!(load_unstaged_diff(dir.path()).unwrap().files.len(), 2);
}

#[test]
fn split_mode_keeps_the_selection_when_switching_panes() {
    let dir = repo_with_two_files();
    let mut app = App::load(dir.path()).unwrap();
    press(&mut app, KeyCode::Char('m'));

    // Stage f.txt so the staged pane has content.
    press(&mut app, KeyCode::Char(' '));
    assert_eq!(app.message, None);
    assert_eq!(app.staged.hunks().len(), 1);

    // Select g.txt (second row), open its diff in the unstaged pane.
    press(&mut app, KeyCode::Char('j'));
    press(&mut app, KeyCode::Enter);
    press(&mut app, KeyCode::Char('j')); // move within the diff
    let unstaged_cursor = app.cursor();

    press(&mut app, KeyCode::Tab);
    assert_eq!(app.side, Side::Staged);
    assert_eq!(app.selected_row, 1, "selection kept");
    assert_eq!(app.cursor(), 0, "staged pane starts at its own position");

    press(&mut app, KeyCode::Tab);
    assert_eq!(app.focus, Focus::Files, "Tab past the last pane returns to files");
    assert_eq!(app.selected_row, 1);
    assert_eq!(
        app.pane_of(Side::Unstaged).cursor,
        unstaged_cursor,
        "cursor restored"
    );
}

#[test]
fn split_mode_space_toggles_a_file_between_the_sides() {
    let dir = tempfile::tempdir().unwrap();
    let repo = Repository::init(dir.path()).unwrap();
    commit_file(&repo, dir.path(), "f.txt", BASE);
    fs::write(dir.path().join("f.txt"), BASE.replacen("l1", "L1", 1)).unwrap();

    let mut app = App::load(dir.path()).unwrap();
    press(&mut app, KeyCode::Char('m'));
    assert_eq!(app.mode, Mode::Split);

    // Space stages the unstaged file...
    press(&mut app, KeyCode::Char(' '));
    assert_eq!(app.message, None);
    assert_eq!(app.staged.hunks().len(), 1);
    assert!(app.unstaged.files.is_empty(), "file left the unstaged side");

    // ...and a second space unstages it again (the file is now staged-only).
    press(&mut app, KeyCode::Char(' '));
    assert_eq!(app.message, None);
    assert!(app.staged.files.is_empty());
    assert_eq!(app.unstaged.hunks().len(), 1);

    assert_eq!(load_staged_diff(dir.path()).unwrap().files.len(), 0);
    assert_eq!(load_unstaged_diff(dir.path()).unwrap().files.len(), 1);
}

#[test]
fn split_mode_tab_lands_in_the_staged_pane_after_staging() {
    let dir = tempfile::tempdir().unwrap();
    let repo = Repository::init(dir.path()).unwrap();
    commit_file(&repo, dir.path(), "f.txt", BASE);
    fs::write(dir.path().join("f.txt"), BASE.replacen("l1", "L1", 1)).unwrap();

    let mut app = App::load(dir.path()).unwrap();
    press(&mut app, KeyCode::Char('m'));

    // Stage the file: the unstaged pane is now hidden.
    press(&mut app, KeyCode::Char(' '));
    assert_eq!(app.message, None);
    assert!(app.unstaged.files.is_empty());

    // Tab from the files pane lands in the staged pane, which is the only
    // visible diff pane.
    press(&mut app, KeyCode::Tab);
    assert_eq!(app.focus, Focus::Diff);
    assert_eq!(app.side, Side::Staged);
    assert!(app.display_lines_for(Side::Staged).iter().any(|l| l.content == "L1"));

    // u unstages the hunk from the staged pane.
    press(&mut app, KeyCode::Char('u'));
    assert_eq!(app.message, None);
    assert!(app.staged.files.is_empty());
    assert_eq!(app.unstaged.hunks().len(), 1);
}
