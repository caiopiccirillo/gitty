mod common;

use std::fs;
use std::path::Path;

use common::{BASE, commit_file};
use git2::Repository;
use ratatui::crossterm::event::{KeyCode, KeyEvent, KeyModifiers};

use gitty::app::{App, Focus, Severity};
use gitty::diff::{FileStatus, LineKind, SelectedLines};
use gitty::git::{
    load_staged_diff, load_unstaged_diff, stage_file, stage_hunk, stage_lines, unstage_file,
    unstage_hunk, unstage_lines,
};
use gitty::tree::Node;

/// Successful operations leave a success message in the status bar.
fn assert_success(app: &App) {
    assert!(
        app.message
            .as_ref()
            .is_some_and(|m| m.severity == Severity::Success),
        "expected a success message, got {:?}",
        app.message
    );
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

fn additions(view: &gitty::diff::DiffView) -> Vec<&str> {
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
    app.wait_for_refresh();
    assert_success(&app);
    assert!(app.unstaged.files.is_empty());
    assert_eq!(app.staged.files.len(), 1);

    app.handle_key(KeyEvent::new(KeyCode::Tab, KeyModifiers::NONE));
    app.handle_key(KeyEvent::new(KeyCode::Char(' '), KeyModifiers::NONE));
    app.wait_for_refresh();
    assert_success(&app);
    assert!(app.staged.files.is_empty());
    assert_eq!(app.unstaged.files.len(), 1);
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

#[test]
fn untracked_files_appear_and_can_be_staged() {
    let dir = tempfile::tempdir().unwrap();
    let repo = Repository::init(dir.path()).unwrap();
    commit_file(&repo, dir.path(), "f.txt", BASE);
    fs::write(dir.path().join("new.txt"), "brand\nnew\nfile\n").unwrap();

    let view = load_unstaged_diff(dir.path()).unwrap();
    let idx = view
        .files
        .iter()
        .position(|f| f.path == "new.txt")
        .expect("untracked file shows up in the unstaged diff");
    assert_eq!(view.files[idx].status, FileStatus::Untracked);
    // Its content is shown as additions.
    assert!(
        view.lines
            .iter()
            .any(|l| l.file_idx == idx && l.kind == LineKind::Addition && l.content == "brand")
    );

    // Whole-file staging works.
    stage_file(dir.path(), &view.files[idx]).unwrap();
    let staged = load_staged_diff(dir.path()).unwrap();
    assert!(
        staged
            .files
            .iter()
            .any(|f| f.path == "new.txt" && f.status == FileStatus::Added)
    );
    assert!(
        !load_unstaged_diff(dir.path())
            .unwrap()
            .files
            .iter()
            .any(|f| f.path == "new.txt")
    );
}

#[test]
fn stages_a_hunk_of_an_untracked_file() {
    let dir = tempfile::tempdir().unwrap();
    let repo = Repository::init(dir.path()).unwrap();
    commit_file(&repo, dir.path(), "f.txt", BASE);
    fs::write(dir.path().join("new.txt"), "brand\nnew\nfile\n").unwrap();

    let view = load_unstaged_diff(dir.path()).unwrap();
    let idx = view.files.iter().position(|f| f.path == "new.txt").unwrap();
    // An untracked file is a single hunk; staging it stages the file.
    stage_hunk(dir.path(), idx, 0).unwrap();

    let staged = load_staged_diff(dir.path()).unwrap();
    assert!(
        staged
            .files
            .iter()
            .any(|f| f.path == "new.txt" && f.status == FileStatus::Added)
    );
}

#[test]
fn stages_only_the_selected_lines() {
    let dir = repo_with_close_changes();

    // The hunk is: ctx l1, -l2, +L2, ctx l3, -l4, +L4, ctx l5...
    // Keep only the l2 change (first addition + first deletion).
    stage_lines(dir.path(), 0, 0, &selected(&[0], &[0])).unwrap();

    let staged = load_staged_diff(dir.path()).unwrap();
    assert_eq!(additions(&staged), vec!["L2"]);
    let unstaged = load_unstaged_diff(dir.path()).unwrap();
    assert_eq!(additions(&unstaged), vec!["L4"]);
}

#[test]
fn unstages_only_the_selected_lines() {
    let dir = repo_with_close_changes();
    let file = load_unstaged_diff(dir.path()).unwrap().files[0].clone();
    stage_file(dir.path(), &file).unwrap();

    // In the staged view the l2 change is the first addition + deletion.
    unstage_lines(dir.path(), 0, 0, &selected(&[0], &[0])).unwrap();

    let staged = load_staged_diff(dir.path()).unwrap();
    assert_eq!(additions(&staged), vec!["L4"]);
    let unstaged = load_unstaged_diff(dir.path()).unwrap();
    assert_eq!(additions(&unstaged), vec!["L2"]);
}

#[test]
fn app_stages_the_visually_selected_lines() {
    let dir = repo_with_close_changes();
    let mut app = App::load(dir.path()).unwrap();

    // Display: [@@, ctx l1, -l2, +L2, ctx l3, -l4, +L4, ...]. Enter lands on
    // the first changed line (-l2); select -l2..+L2 (the whole l2 change)
    // and stage it.
    app.handle_key(KeyEvent::new(KeyCode::Enter, KeyModifiers::NONE));
    app.handle_key(KeyEvent::new(KeyCode::Char('v'), KeyModifiers::NONE));
    app.handle_key(KeyEvent::new(KeyCode::Char('j'), KeyModifiers::NONE));
    app.handle_key(KeyEvent::new(KeyCode::Char('s'), KeyModifiers::NONE));
    app.wait_for_refresh();

    assert_success(&app);
    assert_eq!(additions(&app.staged), vec!["L2"]);
    assert_eq!(additions(&app.unstaged), vec!["L4"]);
    assert_eq!(app.visual_anchor(), None, "selection cleared after staging");
}

#[test]
fn app_stages_a_whole_directory() {
    let dir = tempfile::tempdir().unwrap();
    let repo = Repository::init(dir.path()).unwrap();
    fs::create_dir(dir.path().join("d")).unwrap();
    commit_file(&repo, dir.path(), "d/a.txt", BASE);
    commit_file(&repo, dir.path(), "d/b.txt", BASE);
    fs::write(dir.path().join("d/a.txt"), BASE.replacen("l1", "L1", 1)).unwrap();
    fs::write(dir.path().join("d/b.txt"), BASE.replacen("l1", "L1", 1)).unwrap();

    let mut app = App::load(dir.path()).unwrap();
    // The first row is the "d" directory.
    assert!(matches!(app.selected_node(), Some(Node::Dir { .. })));

    app.handle_key(KeyEvent::new(KeyCode::Char(' '), KeyModifiers::NONE));
    app.wait_for_refresh();
    assert_success(&app);
    assert!(app.unstaged.files.is_empty());
    assert_eq!(app.staged.files.len(), 2);

    app.handle_key(KeyEvent::new(KeyCode::Tab, KeyModifiers::NONE));
    assert!(matches!(app.selected_node(), Some(Node::Dir { .. })));
    app.handle_key(KeyEvent::new(KeyCode::Char(' '), KeyModifiers::NONE));
    app.wait_for_refresh();
    assert_success(&app);
    assert!(app.staged.files.is_empty());
    assert_eq!(app.unstaged.files.len(), 2);
}

#[test]
fn app_stages_and_unstages_the_hunk_under_the_cursor() {
    let (dir, _) = repo_with_two_hunks();
    let mut app = App::load(dir.path()).unwrap();
    assert_eq!(app.focus, Focus::Files);

    // Stage hunk 0 (cursor starts on the first hunk header).
    app.handle_key(KeyEvent::new(KeyCode::Enter, KeyModifiers::NONE));
    app.handle_key(KeyEvent::new(KeyCode::Char('s'), KeyModifiers::NONE));
    app.wait_for_refresh();
    assert_success(&app);
    assert_eq!(app.unstaged.hunks().len(), 1);
    assert_eq!(app.staged.hunks().len(), 1);

    // Stage the remaining hunk: the file leaves the unstaged list and the
    // focus falls back to the files pane.
    app.handle_key(KeyEvent::new(KeyCode::Char('s'), KeyModifiers::NONE));
    app.wait_for_refresh();
    assert_success(&app);
    assert!(app.unstaged.files.is_empty());
    assert_eq!(app.staged.hunks().len(), 2);
    assert_eq!(app.focus, Focus::Files);

    // Switch to the staged tab and unstage one hunk back.
    app.handle_key(KeyEvent::new(KeyCode::Tab, KeyModifiers::NONE));
    app.handle_key(KeyEvent::new(KeyCode::Enter, KeyModifiers::NONE));
    app.handle_key(KeyEvent::new(KeyCode::Char('u'), KeyModifiers::NONE));
    app.wait_for_refresh();
    assert_success(&app);
    assert_eq!(app.staged.hunks().len(), 1);
    assert_eq!(app.unstaged.hunks().len(), 1);
}
