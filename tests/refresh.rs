mod common;

use std::fs;
use std::path::Path;

use common::{BASE, commit_file};
use git2::Repository;
use ratatui::crossterm::event::{KeyCode, KeyEvent, KeyModifiers};

use gitty::app::App;
use gitty::tree::Node;

fn press(app: &mut App, code: KeyCode) {
    app.handle_key(KeyEvent::new(code, KeyModifiers::NONE));
}

fn selected_path(app: &App) -> String {
    match app.selected_node() {
        Some(Node::File { file_idx, .. }) => app.current_diff().files[*file_idx].path.clone(),
        Some(Node::Dir { path, .. }) => format!("{path}/"),
        None => "<none>".into(),
    }
}

#[test]
fn auto_refresh_picks_up_external_changes() {
    let dir = tempfile::tempdir().unwrap();
    let repo = Repository::init(dir.path()).unwrap();
    commit_file(&repo, dir.path(), "f.txt", BASE);

    let mut app = App::load(dir.path()).unwrap();
    assert!(app.unstaged.files.is_empty());

    fs::write(dir.path().join("f.txt"), BASE.replacen("l1", "L1", 1)).unwrap();
    app.auto_refresh();
    assert_eq!(app.unstaged.files.len(), 1);
}

#[test]
fn refresh_preserves_selection_and_cursor_by_path() {
    let dir = tempfile::tempdir().unwrap();
    let repo = Repository::init(dir.path()).unwrap();
    commit_file(&repo, dir.path(), "f.txt", BASE);
    commit_file(&repo, dir.path(), "g.txt", BASE);
    fs::write(dir.path().join("f.txt"), BASE.replacen("l1", "L1", 1)).unwrap();
    fs::write(dir.path().join("g.txt"), BASE.replacen("l1", "L1", 1)).unwrap();

    let mut app = App::load(dir.path()).unwrap();
    // Select g.txt and move the cursor into its diff.
    press(&mut app, KeyCode::Char('j'));
    assert_eq!(selected_path(&app), "g.txt");
    press(&mut app, KeyCode::Enter);
    press(&mut app, KeyCode::Char('j'));
    press(&mut app, KeyCode::Char('j'));
    assert_eq!(app.cursor(), 2);

    // f.txt changes again on disk; g.txt is untouched.
    fs::write(dir.path().join("f.txt"), BASE.replacen("l2", "L2", 1)).unwrap();
    app.auto_refresh();

    assert_eq!(selected_path(&app), "g.txt", "selection follows the path");
    assert_eq!(app.cursor(), 2, "cursor preserved");
}

#[test]
fn refresh_moves_selection_when_the_selected_file_disappears() {
    let dir = tempfile::tempdir().unwrap();
    let repo = Repository::init(dir.path()).unwrap();
    commit_file(&repo, dir.path(), "f.txt", BASE);
    commit_file(&repo, dir.path(), "g.txt", BASE);
    fs::write(dir.path().join("f.txt"), BASE.replacen("l1", "L1", 1)).unwrap();
    fs::write(dir.path().join("g.txt"), BASE.replacen("l1", "L1", 1)).unwrap();

    let mut app = App::load(dir.path()).unwrap();
    assert_eq!(selected_path(&app), "f.txt");

    // Stage f.txt externally: it leaves the unstaged list.
    let mut index = repo.index().unwrap();
    index.add_path(Path::new("f.txt")).unwrap();
    index.write().unwrap();
    app.auto_refresh();

    assert_eq!(app.unstaged.files.len(), 1);
    assert_eq!(
        selected_path(&app),
        "g.txt",
        "selection clamps to a live row"
    );
}

#[test]
fn auto_refresh_skips_during_visual_selection() {
    let dir = tempfile::tempdir().unwrap();
    let repo = Repository::init(dir.path()).unwrap();
    commit_file(&repo, dir.path(), "f.txt", BASE);
    fs::write(dir.path().join("f.txt"), BASE.replacen("l1", "L1", 1)).unwrap();

    let mut app = App::load(dir.path()).unwrap();
    press(&mut app, KeyCode::Enter);
    press(&mut app, KeyCode::Char('v'));
    assert!(app.visual_anchor().is_some());

    fs::write(dir.path().join("g.txt"), "new\n").unwrap();
    app.auto_refresh();
    assert!(app.visual_anchor().is_some(), "selection untouched");
    assert_eq!(app.unstaged.files.len(), 1, "no refresh mid-selection");
}

#[test]
fn background_worker_delivers_changes_off_thread() {
    let dir = tempfile::tempdir().unwrap();
    let repo = Repository::init(dir.path()).unwrap();
    commit_file(&repo, dir.path(), "f.txt", BASE);

    // App::load spawns the background worker (1s interval).
    let mut app = App::load(dir.path()).unwrap();
    assert!(app.unstaged.files.is_empty());

    fs::write(dir.path().join("f.txt"), BASE.replacen("l1", "L1", 1)).unwrap();
    let deadline = std::time::Instant::now() + std::time::Duration::from_secs(5);
    while app.unstaged.files.is_empty() && std::time::Instant::now() < deadline {
        std::thread::sleep(std::time::Duration::from_millis(50));
        app.poll_refresh();
    }
    assert_eq!(
        app.unstaged.files.len(),
        1,
        "background worker delivered the change"
    );
}

#[test]
fn refresh_is_a_noop_when_nothing_changed() {
    let dir = tempfile::tempdir().unwrap();
    let repo = Repository::init(dir.path()).unwrap();
    commit_file(&repo, dir.path(), "f.txt", BASE);
    fs::write(dir.path().join("f.txt"), BASE.replacen("l1", "L1", 1)).unwrap();

    let mut app = App::load(dir.path()).unwrap();
    press(&mut app, KeyCode::Enter);
    press(&mut app, KeyCode::Char('j'));
    let cursor = app.cursor();
    // A no-change refresh must not disturb anything (fast path).
    app.auto_refresh();
    assert_eq!(app.cursor(), cursor);
    assert_eq!(selected_path(&app), "f.txt");
}
