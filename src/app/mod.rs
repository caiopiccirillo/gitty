//! Application state: the two diffs, the files pane tree, the diff cursor
//! and the staging actions.
//!
//! Navigation is a three-level hierarchy: the file tree (left pane), the
//! hunks of the selected entry, and a per-line cursor inside the diff
//! (right pane). The hunk under the cursor is the target of stage/unstage.
//! Key handling and cursor movement live in [`input`].

mod input;

use std::collections::HashSet;
use std::ops::Range;
use std::path::{Path, PathBuf};
use std::sync::Arc;
use std::sync::atomic::{AtomicU64, Ordering};
use std::sync::mpsc;

use anyhow::Result;
use ratatui::widgets::ListState;

use crate::diff::{DiffLine, DiffView, FileInfo, HunkId, LineKind, SelectedLines};
use crate::git;
use crate::refresh::{self, RefreshOutcome};
use crate::tree::{self, Node};

/// Which side of the staging area is shown.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum Tab {
    Unstaged,
    Staged,
}

/// Which pane has keyboard focus.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum Focus {
    Files,
    Diff,
}

pub struct App {
    pub unstaged: DiffView,
    pub staged: DiffView,
    pub tab: Tab,
    pub focus: Focus,
    /// Visible rows of the files pane tree (directories + files).
    pub tree: Vec<Node>,
    /// Directory paths the user has collapsed.
    pub collapsed_dirs: HashSet<String>,
    /// Selected row in the files pane tree.
    pub selected_row: usize,
    /// Scroll state of the files pane; ratatui manages the offset so the
    /// selection stays visible in long lists.
    pub files_state: ListState,
    /// Cursor line within the selected file's displayed diff.
    pub cursor: usize,
    /// Where a visual line selection was started with `v`, if any.
    pub visual_anchor: Option<usize>,
    /// Scroll offset of the diff pane, relative to the displayed lines.
    pub scroll: usize,
    /// The commit message being typed (`c`), if the commit box is open.
    pub commit_input: Option<CommitInput>,
    /// A destructive discard awaiting confirmation (`d` then `y`).
    pub discard_confirm: Option<DiscardPrompt>,
    /// One-off feedback shown in the status bar (e.g. staging errors).
    pub message: Option<String>,
    viewport_height: usize,
    repo_path: PathBuf,
    pub should_quit: bool,
    /// Bumped on every mutation; background snapshots stamped with an older
    /// epoch are discarded as stale.
    epoch: Arc<AtomicU64>,
    /// Channel of the background refresh worker (`None` in tests).
    refresh_rx: Option<mpsc::Receiver<RefreshOutcome>>,
}

impl App {
    /// Load both diffs of the repository containing `repo_path` and spawn
    /// the background refresh worker.
    pub fn load(repo_path: &Path) -> Result<Self> {
        let mut app = Self::new(
            git::load_unstaged_diff(repo_path)?,
            git::load_staged_diff(repo_path)?,
            repo_path.to_path_buf(),
        );
        app.refresh_rx = Some(refresh::spawn(
            repo_path.to_path_buf(),
            Arc::clone(&app.epoch),
        ));
        Ok(app)
    }

    pub fn new(unstaged: DiffView, staged: DiffView, repo_path: PathBuf) -> Self {
        let mut app = Self {
            unstaged,
            staged,
            tab: Tab::Unstaged,
            focus: Focus::Files,
            tree: Vec::new(),
            collapsed_dirs: HashSet::new(),
            selected_row: 0,
            files_state: ListState::default().with_selected(Some(0)),
            cursor: 0,
            visual_anchor: None,
            scroll: 0,
            commit_input: None,
            discard_confirm: None,
            message: None,
            viewport_height: 0,
            repo_path,
            should_quit: false,
            epoch: Arc::new(AtomicU64::new(0)),
            refresh_rx: None,
        };
        app.rebuild_tree();
        app
    }

    pub fn current_diff(&self) -> &DiffView {
        match self.tab {
            Tab::Unstaged => &self.unstaged,
            Tab::Staged => &self.staged,
        }
    }

    /// Lines shown in the diff pane. For a file: its diff without the
    /// `diff --git`/`index`/`---`/`+++` header lines (the pane title shows
    /// the path instead). For a directory: the diffs of all files beneath
    /// it, concatenated, keeping each file's header lines as separators.
    pub fn display_lines(&self) -> Vec<&DiffLine> {
        let diff = self.current_diff();
        match self.selected_node() {
            Some(&Node::File { file_idx, .. }) => file_display_lines(diff, file_idx),
            Some(Node::Dir { path, .. }) => {
                let mut lines = Vec::new();
                for idx in self.dir_file_indices(path) {
                    if let Some(range) = diff.file_line_range(idx) {
                        lines.extend(diff.lines[range].iter());
                    }
                }
                lines
            }
            None => Vec::new(),
        }
    }

    /// The currently selected row of the files pane tree.
    pub fn selected_node(&self) -> Option<&Node> {
        self.tree.get(self.selected_row)
    }

    /// File index of the selection, when it is a file row.
    fn selected_file_idx(&self) -> Option<usize> {
        match self.selected_node() {
            Some(Node::File { file_idx, .. }) => Some(*file_idx),
            _ => None,
        }
    }

    /// Indices of all files beneath a directory, recursively.
    fn dir_file_indices(&self, dir: &str) -> Vec<usize> {
        let prefix = format!("{dir}/");
        self.current_diff()
            .files
            .iter()
            .enumerate()
            .filter(|(_, f)| f.path.starts_with(&prefix))
            .map(|(i, _)| i)
            .collect()
    }

    /// All files beneath a directory, recursively.
    fn dir_files(&self, dir: &str) -> Vec<FileInfo> {
        self.dir_file_indices(dir)
            .into_iter()
            .filter_map(|i| self.current_diff().files.get(i).cloned())
            .collect()
    }

    /// Row index of a directory in the tree, if visible.
    fn dir_row(&self, path: &str) -> Option<usize> {
        self.tree
            .iter()
            .position(|n| matches!(n, Node::Dir { path: p, .. } if p == path))
    }

    /// Rebuild the files pane tree and keep the selection valid.
    fn rebuild_tree(&mut self) {
        self.tree = tree::visible_rows(&self.current_diff().files, &self.collapsed_dirs);
        self.selected_row = if self.tree.is_empty() {
            0
        } else {
            self.selected_row.min(self.tree.len() - 1)
        };
        self.files_state.select(Some(self.selected_row));
    }

    /// Hunk under the cursor — the target of stage/unstage.
    pub fn current_hunk(&self) -> Option<HunkId> {
        let lines = self.display_lines();
        let line = lines.get(self.cursor)?;
        line.hunk_idx.map(|hunk_idx| HunkId {
            file_idx: line.file_idx,
            hunk_idx,
        })
    }

    pub fn set_viewport_height(&mut self, height: usize) {
        self.viewport_height = height;
        self.clamp_cursor();
    }

    /// Stage the hunk under the cursor (unstaged tab).
    pub fn stage_selected_hunk(&mut self) {
        self.with_hunk("stage", |repo_path, hunk| {
            git::stage_hunk(repo_path, hunk.file_idx, hunk.hunk_idx)
        });
    }

    /// Unstage the hunk under the cursor (staged tab).
    pub fn unstage_selected_hunk(&mut self) {
        self.with_hunk("unstage", |repo_path, hunk| {
            git::unstage_hunk(repo_path, hunk.file_idx, hunk.hunk_idx)
        });
    }

    /// Stage the whole selected file (files pane, unstaged tab).
    pub fn stage_selected_file(&mut self) {
        self.with_file("stage", git::stage_file);
    }

    /// Unstage the whole selected file (files pane, staged tab).
    pub fn unstage_selected_file(&mut self) {
        self.with_file("unstage", git::unstage_file);
    }

    fn with_file(&mut self, verb: &str, op: impl FnOnce(&Path, &FileInfo) -> Result<()>) {
        let Some(file) = self
            .selected_file_idx()
            .and_then(|i| self.current_diff().files.get(i))
            .cloned()
        else {
            return;
        };
        self.message = match op(&self.repo_path, &file).and_then(|()| self.refresh()) {
            Ok(()) => None,
            Err(e) => Some(format!("{verb} failed: {e}")),
        };
    }

    /// Stage all files beneath the selected directory (files pane, unstaged tab).
    pub fn stage_selected_dir(&mut self) {
        if let Some(Node::Dir { path, .. }) = self.selected_node().cloned() {
            self.with_dir("stage", &path, git::stage_file);
        }
    }

    /// Unstage all files beneath the selected directory (files pane, staged tab).
    pub fn unstage_selected_dir(&mut self) {
        if let Some(Node::Dir { path, .. }) = self.selected_node().cloned() {
            self.with_dir("unstage", &path, git::unstage_file);
        }
    }

    fn with_dir(&mut self, verb: &str, dir: &str, op: impl Fn(&Path, &FileInfo) -> Result<()>) {
        let files = self.dir_files(dir);
        self.message = match files
            .iter()
            .try_for_each(|f| op(&self.repo_path, f))
            .and_then(|()| self.refresh())
        {
            Ok(()) => None,
            Err(e) => Some(format!("{verb} failed: {e}")),
        };
    }

    /// Range of display lines covered by the visual selection, if active.
    pub fn selection_range(&self) -> Option<Range<usize>> {
        let anchor = self.visual_anchor?;
        Some(anchor.min(self.cursor)..anchor.max(self.cursor) + 1)
    }

    /// The changed (`+`/`-`) lines covered by the visual selection, as
    /// per-hunk ordinals consumed by git::stage_lines/unstage_lines.
    fn selected_lines(&self) -> Option<(HunkId, SelectedLines)> {
        let range = self.selection_range()?;
        let anchor = self.visual_anchor?;
        let lines = self.display_lines();
        let hunk = HunkId {
            file_idx: lines.get(anchor)?.file_idx,
            hunk_idx: lines.get(anchor)?.hunk_idx?,
        };
        let mut selected = SelectedLines::default();
        let (mut adds, mut dels) = (0, 0);
        for i in self.hunk_bounds_at(anchor) {
            match lines[i].kind {
                LineKind::Addition => {
                    if range.contains(&i) {
                        selected.additions.insert(adds);
                    }
                    adds += 1;
                }
                LineKind::Deletion => {
                    if range.contains(&i) {
                        selected.deletions.insert(dels);
                    }
                    dels += 1;
                }
                _ => {}
            }
        }
        (!selected.is_empty()).then_some((hunk, selected))
    }

    /// Stage only the visually selected lines (unstaged tab).
    pub fn stage_selected_lines(&mut self) {
        let Some((hunk, selected)) = self.selected_lines() else {
            self.message = Some("no changed lines selected".into());
            return;
        };
        self.message =
            match git::stage_lines(&self.repo_path, hunk.file_idx, hunk.hunk_idx, &selected)
                .and_then(|()| self.refresh())
            {
                Ok(()) => None,
                Err(e) => Some(format!("stage failed: {e}")),
            };
    }

    /// Unstage only the visually selected lines (staged tab).
    pub fn unstage_selected_lines(&mut self) {
        let Some((hunk, selected)) = self.selected_lines() else {
            self.message = Some("no changed lines selected".into());
            return;
        };
        self.message =
            match git::unstage_lines(&self.repo_path, hunk.file_idx, hunk.hunk_idx, &selected)
                .and_then(|()| self.refresh())
            {
                Ok(()) => None,
                Err(e) => Some(format!("unstage failed: {e}")),
            };
    }

    /// Open the discard prompt for what's under the cursor (`d`).
    pub fn prompt_discard(&mut self) {
        let staged = self.tab == Tab::Staged;
        let action = match self.focus {
            Focus::Files => match self.selected_node().cloned() {
                Some(Node::File { file_idx, .. }) => {
                    let file = self.current_diff().files[file_idx].clone();
                    DiscardAction::File(file, staged)
                }
                Some(Node::Dir { path, .. }) => DiscardAction::Dir(path, staged),
                None => return,
            },
            Focus::Diff => {
                let Some(hunk) = self.current_hunk() else {
                    self.message = Some("no hunk under the cursor".into());
                    return;
                };
                match self.selected_lines() {
                    Some((hunk, selected)) => DiscardAction::Lines {
                        file_idx: hunk.file_idx,
                        hunk_idx: hunk.hunk_idx,
                        selected,
                        staged,
                    },
                    None if self.visual_anchor.is_some() => {
                        self.message = Some("no changed lines selected".into());
                        return;
                    }
                    None => DiscardAction::Hunk {
                        file_idx: hunk.file_idx,
                        hunk_idx: hunk.hunk_idx,
                        staged,
                    },
                }
            }
        };
        let what = match &action {
            DiscardAction::Hunk { file_idx, hunk_idx, .. } => {
                let path = &self.current_diff().files[*file_idx].path;
                format!("hunk {} of {path}", hunk_idx + 1)
            }
            DiscardAction::Lines {
                file_idx,
                hunk_idx,
                selected,
                ..
            } => {
                let path = &self.current_diff().files[*file_idx].path;
                format!(
                    "{} line(s) of hunk {} in {path}",
                    selected.additions.len() + selected.deletions.len(),
                    hunk_idx + 1
                )
            }
            DiscardAction::File(file, _) => format!("file {}", file.path),
            DiscardAction::Dir(path, _) => format!("directory {path}/"),
        };
        self.discard_confirm = Some(DiscardPrompt { what, action });
    }

    /// Run the confirmed discard action.
    fn confirm_discard(&mut self) {
        let Some(prompt) = self.discard_confirm.take() else {
            return;
        };
        self.message = match self.execute_discard(prompt.action) {
            Ok(()) => None,
            Err(e) => Some(format!("discard failed: {e}")),
        };
    }

    fn execute_discard(&mut self, action: DiscardAction) -> Result<()> {
        let repo_path = self.repo_path.clone();
        match action {
            DiscardAction::Hunk {
                file_idx,
                hunk_idx,
                staged,
            } => {
                if staged {
                    git::discard_staged_hunk(&repo_path, file_idx, hunk_idx)?;
                } else {
                    git::discard_hunk(&repo_path, file_idx, hunk_idx)?;
                }
            }
            DiscardAction::Lines {
                file_idx,
                hunk_idx,
                selected,
                staged,
            } => {
                if staged {
                    git::discard_staged_lines(&repo_path, file_idx, hunk_idx, &selected)?;
                } else {
                    git::discard_lines(&repo_path, file_idx, hunk_idx, &selected)?;
                }
            }
            DiscardAction::File(file, staged) => {
                if staged {
                    git::discard_staged_file(&repo_path, &file)?;
                } else {
                    git::discard_file(&repo_path, &file)?;
                }
            }
            DiscardAction::Dir(path, staged) => {
                for file in self.dir_files(&path) {
                    if staged {
                        git::discard_staged_file(&repo_path, &file)?;
                    } else {
                        git::discard_file(&repo_path, &file)?;
                    }
                }
            }
        }
        self.refresh()
    }

    fn with_hunk(&mut self, verb: &str, op: impl FnOnce(&Path, HunkId) -> Result<()>) {
        let Some(hunk) = self.current_hunk() else {
            self.message = Some("no hunk under the cursor".into());
            return;
        };
        self.message = match op(&self.repo_path, hunk).and_then(|()| self.refresh()) {
            Ok(()) => None,
            Err(e) => Some(format!("{verb} failed: {e}")),
        };
    }

    /// Reload both diffs from disk (synchronously, after a mutation) and
    /// invalidate any background snapshot still in flight.
    pub fn refresh(&mut self) -> Result<()> {
        self.epoch.fetch_add(1, Ordering::SeqCst);
        let unstaged = git::load_unstaged_diff(&self.repo_path)?;
        let staged = git::load_staged_diff(&self.repo_path)?;
        self.apply_refreshed(unstaged, staged);
        Ok(())
    }

    /// Apply any finished background snapshot (called on idle ticks; cheap
    /// no-op when the channel is empty).
    pub fn poll_refresh(&mut self) {
        if self.visual_anchor.is_some() {
            return; // apply later, when the selection is done
        }
        let Some(rx) = &self.refresh_rx else {
            return;
        };
        let mut latest = None;
        while let Ok(outcome) = rx.try_recv() {
            latest = Some(outcome);
        }
        if let Some(outcome) = latest
            && outcome.epoch >= self.epoch.load(Ordering::SeqCst)
        {
            self.apply_refreshed(outcome.unstaged, outcome.staged);
        }
    }

    /// Synchronous refresh used by tests and explicit calls.
    pub fn auto_refresh(&mut self) {
        if self.visual_anchor.is_none() {
            let _ = self.refresh();
        }
    }

    /// Open the commit message box (`c`), if there is anything staged.
    pub fn open_commit(&mut self) {
        if self.staged.files.is_empty() {
            self.message = Some("nothing staged".into());
        } else {
            self.commit_input = Some(CommitInput::default());
        }
    }

    /// Commit the staged changes with the typed message and close the box.
    pub fn commit(&mut self) {
        let Some(input) = self.commit_input.take() else {
            return;
        };
        let message = input.text.trim().to_string();
        if message.is_empty() {
            self.message = Some("empty commit message".into());
            return;
        }
        self.message = match git::commit(&self.repo_path, &message)
            .and_then(|short| self.refresh().map(|()| format!("committed {short}")))
        {
            Ok(msg) => Some(msg),
            Err(e) => Some(format!("commit failed: {e}")),
        };
    }

    /// Swap in new diffs, preserving the selection and cursor by path.
    /// No-op when nothing changed.
    fn apply_refreshed(&mut self, unstaged: DiffView, staged: DiffView) {
        if unstaged == self.unstaged && staged == self.staged {
            return;
        }
        let identity = self.selected_node().map(|n| self.identity_of(n));
        let (cursor, scroll) = (self.cursor, self.scroll);
        self.unstaged = unstaged;
        self.staged = staged;
        self.rebuild_tree();
        if self.tree.is_empty() {
            self.focus = Focus::Files;
        }
        self.visual_anchor = None;
        if let Some(row) = identity.and_then(|id| self.find_row(&id)) {
            self.selected_row = row;
            self.files_state.select(Some(row));
            self.cursor = cursor;
            self.scroll = scroll;
        } else {
            self.cursor = 0;
            self.scroll = 0;
        }
        self.clamp_cursor();
    }

    /// Path-based identity of a tree row, so it can be re-found after a
    /// refresh even if file indices shifted.
    fn identity_of(&self, node: &Node) -> NodeIdentity {
        match node {
            Node::Dir { path, .. } => NodeIdentity::Dir(path.clone()),
            Node::File { file_idx, .. } => {
                NodeIdentity::File(self.current_diff().files[*file_idx].path.clone())
            }
        }
    }

    fn find_row(&self, identity: &NodeIdentity) -> Option<usize> {
        self.tree.iter().position(|node| match (node, identity) {
            (Node::Dir { path, .. }, NodeIdentity::Dir(want)) => path == want,
            (Node::File { file_idx, .. }, NodeIdentity::File(want)) => {
                self.current_diff().files[*file_idx].path == *want
            }
            _ => false,
        })
    }
}

/// Path-based identity of a tree row (see [`App::refresh`]).
enum NodeIdentity {
    Dir(String),
    File(String),
}

/// Text of the commit message box with a byte-offset cursor that always
/// sits on a char boundary.
#[derive(Debug, Default)]
pub struct CommitInput {
    pub text: String,
    pub cursor: usize,
}

/// A destructive discard awaiting confirmation.
#[derive(Debug)]
pub struct DiscardPrompt {
    /// What the status bar should ask about, e.g. "hunk 2 of f.txt".
    pub what: String,
    pub action: DiscardAction,
}

/// What a confirmed discard should revert. `staged` selects the staged-tab
/// operations, which revert both the worktree and the index to HEAD.
#[derive(Debug)]
pub enum DiscardAction {
    Hunk {
        file_idx: usize,
        hunk_idx: usize,
        staged: bool,
    },
    Lines {
        file_idx: usize,
        hunk_idx: usize,
        selected: SelectedLines,
        staged: bool,
    },
    File(FileInfo, bool),
    Dir(String, bool),
}

impl CommitInput {
    fn insert(&mut self, c: char) {
        self.text.insert(self.cursor, c);
        self.cursor += c.len_utf8();
    }

    fn backspace(&mut self) {
        if let Some(prev) = self.prev_boundary() {
            self.text.replace_range(prev..self.cursor, "");
            self.cursor = prev;
        }
    }

    fn left(&mut self) {
        if let Some(prev) = self.prev_boundary() {
            self.cursor = prev;
        }
    }

    fn right(&mut self) {
        self.cursor = self.text[self.cursor..]
            .chars()
            .next()
            .map_or(self.text.len(), |c| self.cursor + c.len_utf8());
    }

    fn prev_boundary(&self) -> Option<usize> {
        self.text[..self.cursor]
            .char_indices()
            .last()
            .map(|(i, _)| i)
    }

    /// Cursor position in characters (for rendering).
    pub fn cursor_chars(&self) -> usize {
        self.text[..self.cursor].chars().count()
    }
}

/// A file's diff lines without its file-header lines.
fn file_display_lines(diff: &DiffView, file_idx: usize) -> Vec<&DiffLine> {
    let Some(range) = diff.file_line_range(file_idx) else {
        return Vec::new();
    };
    let lines = &diff.lines[range];
    let headers = lines
        .iter()
        .take_while(|l| l.kind == LineKind::FileHeader)
        .count();
    lines[headers..].iter().collect()
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::diff::{FileStatus, two_file_view};
    use ratatui::crossterm::event::{KeyCode, KeyEvent, KeyModifiers};

    /// Two files (1 hunk + 2 hunks), staged side empty.
    fn test_app() -> App {
        let mut app = App::new(
            two_file_view(),
            DiffView::default(),
            PathBuf::from("/unused"),
        );
        app.set_viewport_height(10);
        app
    }

    fn press(app: &mut App, code: KeyCode) {
        app.handle_key(KeyEvent::new(code, KeyModifiers::NONE));
    }

    fn hunk(file_idx: usize, hunk_idx: usize) -> Option<HunkId> {
        Some(HunkId { file_idx, hunk_idx })
    }

    #[test]
    fn row_selection_moves_and_clamps() {
        let mut app = test_app();
        press(&mut app, KeyCode::Char('j'));
        assert_eq!(app.selected_row, 1);
        press(&mut app, KeyCode::Char('j'));
        assert_eq!(app.selected_row, 1, "clamped at last row");
        press(&mut app, KeyCode::Char('k'));
        assert_eq!(app.selected_row, 0);
    }

    #[test]
    fn enter_focuses_diff_and_cursor_moves_within_file() {
        let mut app = test_app();
        assert_eq!(app.focus, Focus::Files);
        press(&mut app, KeyCode::Enter);
        assert_eq!(app.focus, Focus::Diff);
        // File 0 displays 2 lines: the hunk header and one addition.
        press(&mut app, KeyCode::Char('j'));
        assert_eq!(app.cursor, 1);
        press(&mut app, KeyCode::Char('j'));
        assert_eq!(app.cursor, 1, "clamped at last line");
        press(&mut app, KeyCode::Esc);
        assert_eq!(app.focus, Focus::Files);
    }

    #[test]
    fn n_and_p_jump_between_hunks_of_the_file() {
        let mut app = test_app();
        // File 1 displays: [hunk0 header, deletion, hunk1 header, deletion].
        press(&mut app, KeyCode::Char('j'));
        press(&mut app, KeyCode::Enter);
        assert_eq!(app.cursor, 1, "snaps to the first changed line");
        press(&mut app, KeyCode::Char('n'));
        assert_eq!(app.cursor, 3, "lands on the next hunk's changed line");
        press(&mut app, KeyCode::Char('n'));
        assert_eq!(app.cursor, 3, "no next hunk");
        press(&mut app, KeyCode::Char('p'));
        assert_eq!(app.cursor, 1);
        press(&mut app, KeyCode::Char('p'));
        assert_eq!(app.cursor, 1, "no previous hunk");
    }

    #[test]
    fn cursor_defines_the_current_hunk() {
        let mut app = test_app();
        press(&mut app, KeyCode::Char('j'));
        press(&mut app, KeyCode::Enter);
        assert_eq!(app.current_hunk(), hunk(1, 0));
        press(&mut app, KeyCode::Char('n'));
        assert_eq!(app.current_hunk(), hunk(1, 1));
    }

    #[test]
    fn tab_switches_and_resets() {
        let mut app = test_app();
        press(&mut app, KeyCode::Char('j'));
        press(&mut app, KeyCode::Tab);
        assert_eq!(app.tab, Tab::Staged);
        assert_eq!(app.selected_row, 0);
        assert_eq!(app.focus, Focus::Files);
        // Staged side is empty: Enter must not focus the diff.
        press(&mut app, KeyCode::Enter);
        assert_eq!(app.focus, Focus::Files);
        assert_eq!(app.current_hunk(), None);
    }

    #[test]
    fn cursor_traverses_changed_lines_freely() {
        let mut app = test_app();
        // File 1 displays: [hunk0 header, deletion, hunk1 header, deletion].
        press(&mut app, KeyCode::Char('j'));
        press(&mut app, KeyCode::Enter);
        assert_eq!(app.cursor, 1, "snaps to the first changed line");
        press(&mut app, KeyCode::Char('j'));
        assert_eq!(app.cursor, 3, "j jumps to the next changed line");
        press(&mut app, KeyCode::Char('j'));
        assert_eq!(app.cursor, 3, "clamped at the last changed line");
        press(&mut app, KeyCode::Char('k'));
        assert_eq!(app.cursor, 1);
    }

    #[test]
    fn scroll_follows_the_cursor() {
        let mut app = test_app();
        app.set_viewport_height(1);
        press(&mut app, KeyCode::Char('j'));
        press(&mut app, KeyCode::Enter);
        press(&mut app, KeyCode::Char('G'));
        assert_eq!(app.cursor, 3);
        assert_eq!(app.scroll, 3);
        press(&mut app, KeyCode::Char('g'));
        assert_eq!(app.cursor, 1);
        assert_eq!(app.scroll, 1);
    }

    #[test]
    fn visual_selection_extends_and_stays_in_the_hunk() {
        let mut app = test_app();
        press(&mut app, KeyCode::Char('j'));
        press(&mut app, KeyCode::Enter);
        press(&mut app, KeyCode::Char('v'));
        assert_eq!(app.selection_range(), Some(1..2));
        press(&mut app, KeyCode::Char('j'));
        assert_eq!(
            app.selection_range(),
            Some(1..2),
            "the hunk has only one changed line"
        );
        press(&mut app, KeyCode::Esc);
        assert_eq!(app.visual_anchor, None);
        assert_eq!(app.focus, Focus::Diff, "Esc only cancels the selection");
        press(&mut app, KeyCode::Esc);
        assert_eq!(app.focus, Focus::Files);
    }

    #[test]
    fn discard_requires_confirmation() {
        let mut app = test_app();
        press(&mut app, KeyCode::Enter);
        press(&mut app, KeyCode::Char('d'));
        assert!(app.discard_confirm.is_some());

        // n cancels without touching anything.
        press(&mut app, KeyCode::Char('n'));
        assert!(app.discard_confirm.is_none());

        // y runs the operation (which fails here: /unused is not a repo).
        press(&mut app, KeyCode::Char('d'));
        assert!(app.discard_confirm.is_some());
        press(&mut app, KeyCode::Char('y'));
        assert!(app.discard_confirm.is_none());
        assert!(
            app.message
                .as_deref()
                .is_some_and(|m| m.starts_with("discard failed"))
        );
    }

    #[test]
    fn files_pane_discard_targets_the_selected_file() {
        let mut app = test_app();
        press(&mut app, KeyCode::Char('d'));
        let prompt = app.discard_confirm.take().unwrap();
        assert!(matches!(prompt.action, DiscardAction::File(ref f, false) if f.path == "a.txt"));
        assert_eq!(prompt.what, "file a.txt");
    }

    #[test]
    fn visual_selection_discards_selected_lines() {
        let mut app = test_app();
        // File 1: [@@, deletion, @@, deletion]; select the first deletion.
        press(&mut app, KeyCode::Char('j'));
        press(&mut app, KeyCode::Enter);
        press(&mut app, KeyCode::Char('v'));
        press(&mut app, KeyCode::Char('d'));
        let prompt = app.discard_confirm.take().unwrap();
        assert!(matches!(
            prompt.action,
            DiscardAction::Lines {
                file_idx: 1,
                hunk_idx: 0,
                staged: false,
                ..
            }
        ));
    }

    /// src/app.rs, src/git/ops.rs, top.rs — one hunk each.
    fn nested_view() -> DiffView {
        let line = |kind: LineKind, file_idx: usize, hunk_idx: Option<usize>| DiffLine {
            kind,
            content: String::new(),
            file_idx,
            hunk_idx,
        };
        let mut lines = Vec::new();
        for idx in 0..3 {
            lines.push(line(LineKind::FileHeader, idx, None));
            lines.push(line(LineKind::HunkHeader, idx, Some(0)));
            lines.push(line(LineKind::Addition, idx, Some(0)));
        }
        DiffView {
            lines,
            files: ["src/app.rs", "src/git/ops.rs", "top.rs"]
                .into_iter()
                .map(|path| FileInfo {
                    path: path.into(),
                    status: FileStatus::Modified,
                })
                .collect(),
        }
    }

    fn nested_app() -> App {
        // Rows (dirs first): [Dir src, Dir src/git, File ops.rs, File app.rs, File top.rs]
        let mut app = App::new(nested_view(), DiffView::default(), PathBuf::from("/unused"));
        app.set_viewport_height(10);
        app
    }

    #[test]
    fn enter_on_a_dir_collapses_and_expands_it() {
        let mut app = nested_app();
        assert_eq!(app.tree.len(), 5);
        press(&mut app, KeyCode::Enter);
        assert_eq!(app.tree.len(), 2, "collapsed to [Dir src, File top.rs]");
        assert_eq!(app.selected_row, 0, "the dir stays selected");
        assert!(app.collapsed_dirs.contains("src"));
        press(&mut app, KeyCode::Enter);
        assert_eq!(app.tree.len(), 5);
    }

    #[test]
    fn h_on_a_file_moves_to_its_parent_dir() {
        let mut app = nested_app();
        press(&mut app, KeyCode::Char('j'));
        press(&mut app, KeyCode::Char('j'));
        assert!(matches!(app.selected_node(), Some(Node::File { .. })));
        press(&mut app, KeyCode::Char('h'));
        assert_eq!(app.selected_row, 1, "parent dir src/git");
        press(&mut app, KeyCode::Char('h'));
        assert_eq!(app.tree.len(), 4, "h on the expanded dir collapses it");
    }

    #[test]
    fn h_collapses_an_expanded_dir_and_l_expands_it_back() {
        let mut app = nested_app();
        press(&mut app, KeyCode::Char('h'));
        assert_eq!(app.tree.len(), 2);
        // h on a collapsed root dir has no parent to jump to.
        press(&mut app, KeyCode::Char('h'));
        assert_eq!(app.selected_row, 0);
        press(&mut app, KeyCode::Char('l'));
        assert_eq!(app.tree.len(), 5);
    }

    #[test]
    fn commit_input_edits_text_at_char_boundaries() {
        let mut input = CommitInput::default();
        for c in "héllo".chars() {
            input.insert(c);
        }
        input.left();
        input.left();
        input.left();
        input.backspace();
        assert_eq!(input.text, "hllo", "backspace removed the multibyte é");
        assert_eq!(input.cursor, 1);
        input.insert('e');
        assert_eq!(input.text, "hello");
        for _ in 0..4 {
            input.right();
        }
        input.insert('!');
        assert_eq!(input.text, "hello!");
        input.left();
        input.backspace();
        assert_eq!(input.text, "hell!", "removes the char before the cursor");
    }

    #[test]
    fn dir_selection_shows_the_aggregate_diff() {
        let mut app = nested_app();
        // Dir src aggregates app.rs + git/ops.rs (3 lines each, headers kept).
        assert_eq!(app.display_lines().len(), 6);
        press(&mut app, KeyCode::Char('j'));
        // Dir src/git aggregates ops.rs only.
        assert_eq!(app.display_lines().len(), 3);
        press(&mut app, KeyCode::Char('j'));
        // File row: header lines stripped (hunk header + addition).
        assert_eq!(app.display_lines().len(), 2);
    }
}
