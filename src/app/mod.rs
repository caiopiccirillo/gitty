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

use anyhow::Result;
use ratatui::widgets::ListState;

use crate::diff::{DiffLine, DiffView, FileInfo, HunkId, LineKind, SelectedLines};
use crate::git;
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
    /// One-off feedback shown in the status bar (e.g. staging errors).
    pub message: Option<String>,
    viewport_height: usize,
    repo_path: PathBuf,
    pub should_quit: bool,
}

impl App {
    /// Load both diffs of the repository containing `repo_path`.
    pub fn load(repo_path: &Path) -> Result<Self> {
        Ok(Self::new(
            git::load_unstaged_diff(repo_path)?,
            git::load_staged_diff(repo_path)?,
            repo_path.to_path_buf(),
        ))
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
            message: None,
            viewport_height: 0,
            repo_path,
            should_quit: false,
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

    /// Reload both diffs from disk, preserving the selection and cursor by
    /// path. No-op when nothing changed.
    pub fn refresh(&mut self) -> Result<()> {
        let unstaged = git::load_unstaged_diff(&self.repo_path)?;
        let staged = git::load_staged_diff(&self.repo_path)?;
        if unstaged == self.unstaged && staged == self.staged {
            return Ok(());
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
        Ok(())
    }

    /// Refresh from disk changes, unless the user is mid-selection
    /// (auto-refresh on the event loop's idle tick).
    pub fn auto_refresh(&mut self) {
        if self.visual_anchor.is_none() {
            let _ = self.refresh();
        }
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
        // File 1 displays: [hunk0 header, context, hunk1 header, deletion].
        press(&mut app, KeyCode::Char('j'));
        press(&mut app, KeyCode::Enter);
        assert_eq!(app.cursor, 0);
        press(&mut app, KeyCode::Char('n'));
        assert_eq!(app.cursor, 2);
        press(&mut app, KeyCode::Char('n'));
        assert_eq!(app.cursor, 2, "no next hunk");
        press(&mut app, KeyCode::Char('p'));
        assert_eq!(app.cursor, 0);
        press(&mut app, KeyCode::Char('p'));
        assert_eq!(app.cursor, 0, "no previous hunk");
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
    fn cursor_traverses_hunks_freely() {
        let mut app = test_app();
        // File 1 displays: [hunk0 header, context, hunk1 header, deletion].
        press(&mut app, KeyCode::Char('j'));
        press(&mut app, KeyCode::Enter);
        press(&mut app, KeyCode::Char('j'));
        press(&mut app, KeyCode::Char('j'));
        assert_eq!(app.cursor, 2, "j crosses into the next hunk");
        press(&mut app, KeyCode::Char('j'));
        assert_eq!(app.cursor, 3);
        press(&mut app, KeyCode::Char('j'));
        assert_eq!(app.cursor, 3, "clamped at the last line");
        press(&mut app, KeyCode::Char('k'));
        assert_eq!(app.cursor, 2);
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
        assert_eq!(app.cursor, 0);
        assert_eq!(app.scroll, 0);
    }

    #[test]
    fn visual_selection_extends_and_stays_in_the_hunk() {
        let mut app = test_app();
        press(&mut app, KeyCode::Char('j'));
        press(&mut app, KeyCode::Enter);
        press(&mut app, KeyCode::Char('v'));
        assert_eq!(app.selection_range(), Some(0..1));
        press(&mut app, KeyCode::Char('j'));
        assert_eq!(app.selection_range(), Some(0..2));
        press(&mut app, KeyCode::Char('j'));
        assert_eq!(app.cursor, 1, "selection cannot leave its hunk");
        press(&mut app, KeyCode::Esc);
        assert_eq!(app.visual_anchor, None);
        assert_eq!(app.focus, Focus::Diff, "Esc only cancels the selection");
        press(&mut app, KeyCode::Esc);
        assert_eq!(app.focus, Focus::Files);
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
