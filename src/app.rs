//! Application state and input handling.
//!
//! Navigation is a three-level hierarchy: the file list (left pane), the
//! hunks of the selected file, and a per-line cursor inside the diff (right
//! pane). The hunk under the cursor is the target of stage/unstage.

use std::ops::Range;
use std::path::{Path, PathBuf};

use anyhow::Result;
use ratatui::crossterm::event::{KeyCode, KeyEvent, KeyModifiers};
use ratatui::widgets::ListState;

use crate::diff::{DiffLine, DiffView, FileInfo, HunkId, LineKind};
use crate::git;

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
    /// Selected file index within the current tab's file list.
    pub selected_file: usize,
    /// Scroll state of the files pane; ratatui manages the offset so the
    /// selection stays visible in long lists.
    pub files_state: ListState,
    /// Cursor line within the selected file's displayed diff.
    pub cursor: usize,
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
        Self {
            unstaged,
            staged,
            tab: Tab::Unstaged,
            focus: Focus::Files,
            selected_file: 0,
            files_state: ListState::default().with_selected(Some(0)),
            cursor: 0,
            scroll: 0,
            message: None,
            viewport_height: 0,
            repo_path,
            should_quit: false,
        }
    }

    pub fn current_diff(&self) -> &DiffView {
        match self.tab {
            Tab::Unstaged => &self.unstaged,
            Tab::Staged => &self.staged,
        }
    }

    /// Lines shown in the diff pane: the selected file's diff without its
    /// `diff --git`/`index`/`---`/`+++` header lines (the pane title shows
    /// the path instead).
    pub fn display_lines(&self) -> &[DiffLine] {
        let Some(range) = self.current_diff().file_line_range(self.selected_file) else {
            return &[];
        };
        let lines = &self.current_diff().lines[range];
        let headers = lines
            .iter()
            .take_while(|l| l.kind == LineKind::FileHeader)
            .count();
        &lines[headers..]
    }

    /// Hunk under the cursor — the target of stage/unstage.
    pub fn current_hunk(&self) -> Option<HunkId> {
        let line = self.display_lines().get(self.cursor)?;
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
        let Some(file) = self.current_diff().files.get(self.selected_file).cloned() else {
            return;
        };
        self.message = match op(&self.repo_path, &file).and_then(|()| self.reload()) {
            Ok(()) => None,
            Err(e) => Some(format!("{verb} failed: {e}")),
        };
    }

    fn with_hunk(&mut self, verb: &str, op: impl FnOnce(&Path, HunkId) -> Result<()>) {
        let Some(hunk) = self.current_hunk() else {
            self.message = Some("no hunk under the cursor".into());
            return;
        };
        self.message = match op(&self.repo_path, hunk).and_then(|()| self.reload()) {
            Ok(()) => None,
            Err(e) => Some(format!("{verb} failed: {e}")),
        };
    }

    /// Reload both diffs after a mutation and clamp the selection into the
    /// (possibly shrunk) file lists.
    fn reload(&mut self) -> Result<()> {
        self.unstaged = git::load_unstaged_diff(&self.repo_path)?;
        self.staged = git::load_staged_diff(&self.repo_path)?;
        let len = self.current_diff().files.len();
        self.selected_file = len.saturating_sub(1).min(self.selected_file);
        self.files_state.select(Some(self.selected_file));
        if len == 0 {
            self.focus = Focus::Files;
        }
        self.clamp_cursor();
        Ok(())
    }

    pub fn handle_key(&mut self, key: KeyEvent) {
        self.message = None;
        match (key.modifiers, key.code) {
            (_, KeyCode::Char('q')) | (KeyModifiers::CONTROL, KeyCode::Char('c')) => {
                self.should_quit = true;
            }
            (_, KeyCode::Tab) => self.switch_tab(),
            _ => match self.focus {
                Focus::Files => self.handle_files_key(key),
                Focus::Diff => self.handle_diff_key(key),
            },
        }
    }

    fn handle_files_key(&mut self, key: KeyEvent) {
        match key.code {
            KeyCode::Down | KeyCode::Char('j') => self.move_file(1),
            KeyCode::Up | KeyCode::Char('k') => self.move_file(-1),
            KeyCode::Home | KeyCode::Char('g') => self.select_file(0),
            KeyCode::End | KeyCode::Char('G') => {
                let len = self.current_diff().files.len();
                if len > 0 {
                    self.select_file(len - 1);
                }
            }
            KeyCode::Enter | KeyCode::Right | KeyCode::Char('l')
                if !self.current_diff().files.is_empty() =>
            {
                self.focus = Focus::Diff;
            }
            KeyCode::Char(' ') => match self.tab {
                Tab::Unstaged => self.stage_selected_file(),
                Tab::Staged => self.unstage_selected_file(),
            },
            _ => {}
        }
    }

    fn handle_diff_key(&mut self, key: KeyEvent) {
        match (key.modifiers, key.code) {
            (_, KeyCode::Down) | (_, KeyCode::Char('j')) => self.move_cursor(1),
            (_, KeyCode::Up) | (_, KeyCode::Char('k')) => self.move_cursor(-1),
            (_, KeyCode::PageDown) => self.move_cursor(self.viewport_height as isize),
            (_, KeyCode::PageUp) => self.move_cursor(-(self.viewport_height as isize)),
            (KeyModifiers::CONTROL, KeyCode::Char('d')) => {
                self.move_cursor(self.viewport_height as isize / 2);
            }
            (KeyModifiers::CONTROL, KeyCode::Char('u')) => {
                self.move_cursor(-(self.viewport_height as isize / 2));
            }
            (_, KeyCode::Home) | (_, KeyCode::Char('g')) => {
                self.cursor = self.current_hunk_bounds().start;
                self.clamp_cursor();
            }
            (_, KeyCode::End) | (_, KeyCode::Char('G')) => {
                self.cursor = self.current_hunk_bounds().end.saturating_sub(1);
                self.clamp_cursor();
            }
            (_, KeyCode::Char('n')) => self.jump_hunk(1),
            (_, KeyCode::Char('p')) => self.jump_hunk(-1),
            (_, KeyCode::Left) | (_, KeyCode::Char('h')) | (_, KeyCode::Esc) => {
                self.focus = Focus::Files;
            }
            (_, KeyCode::Char('s')) if self.tab == Tab::Unstaged => self.stage_selected_hunk(),
            (_, KeyCode::Char('u')) if self.tab == Tab::Staged => self.unstage_selected_hunk(),
            _ => {}
        }
    }

    fn switch_tab(&mut self) {
        self.tab = match self.tab {
            Tab::Unstaged => Tab::Staged,
            Tab::Staged => Tab::Unstaged,
        };
        self.focus = Focus::Files;
        self.select_file(0);
    }

    fn select_file(&mut self, idx: usize) {
        self.selected_file = idx;
        self.files_state.select(Some(idx));
        self.cursor = 0;
        self.scroll = 0;
    }

    fn move_file(&mut self, delta: isize) {
        let len = self.current_diff().files.len();
        if len == 0 {
            return;
        }
        let next = (self.selected_file as isize + delta).clamp(0, len as isize - 1);
        self.select_file(next as usize);
    }

    /// Move the cursor within its hunk; line-wise movement never crosses
    /// hunk boundaries (n/p are for moving between hunks).
    fn move_cursor(&mut self, delta: isize) {
        let bounds = self.current_hunk_bounds();
        if bounds.is_empty() {
            return;
        }
        self.cursor = (self.cursor as isize + delta)
            .clamp(bounds.start as isize, bounds.end as isize - 1) as usize;
        self.clamp_cursor();
    }

    /// Display-line range of the hunk the cursor is in: from its `@@` header
    /// to just before the next header (or the end of the file).
    fn current_hunk_bounds(&self) -> Range<usize> {
        let len = self.display_lines().len();
        if len == 0 {
            return 0..0;
        }
        let cursor = self.cursor.min(len - 1);
        let start = self.display_lines()[..=cursor]
            .iter()
            .rposition(|l| l.kind == LineKind::HunkHeader)
            .unwrap_or(0);
        let end = self
            .display_lines()
            .iter()
            .enumerate()
            .skip(start + 1)
            .find(|(_, l)| l.kind == LineKind::HunkHeader)
            .map(|(i, _)| i)
            .unwrap_or(len);
        start..end
    }

    /// Move the cursor to the next/previous hunk header within the file.
    fn jump_hunk(&mut self, direction: isize) {
        let lines = self.display_lines();
        let is_header = |(_, line): &(usize, &DiffLine)| line.kind == LineKind::HunkHeader;
        let target = if direction > 0 {
            lines
                .iter()
                .enumerate()
                .skip(self.cursor + 1)
                .find(is_header)
                .map(|(i, _)| i)
        } else {
            lines
                .iter()
                .enumerate()
                .take(self.cursor)
                .rfind(is_header)
                .map(|(i, _)| i)
        };
        if let Some(i) = target {
            self.cursor = i;
            self.clamp_cursor();
        }
    }

    /// Keep the cursor inside the displayed lines and scroll so it stays
    /// visible.
    fn clamp_cursor(&mut self) {
        let len = self.display_lines().len();
        if len == 0 {
            self.cursor = 0;
            self.scroll = 0;
            return;
        }
        self.cursor = self.cursor.min(len - 1);
        if self.viewport_height == 0 {
            // Before the first render there is no viewport yet; keep the
            // cursor position and leave scroll alone.
            return;
        }
        if self.cursor < self.scroll {
            self.scroll = self.cursor;
        } else if self.cursor >= self.scroll + self.viewport_height {
            self.scroll = self.cursor + 1 - self.viewport_height;
        }
        self.scroll = self.scroll.min(len.saturating_sub(self.viewport_height));
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::diff::two_file_view;

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
    fn file_selection_moves_and_clamps() {
        let mut app = test_app();
        press(&mut app, KeyCode::Char('j'));
        assert_eq!(app.selected_file, 1);
        press(&mut app, KeyCode::Char('j'));
        assert_eq!(app.selected_file, 1, "clamped at last file");
        press(&mut app, KeyCode::Char('k'));
        assert_eq!(app.selected_file, 0);
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
        assert_eq!(app.selected_file, 0);
        assert_eq!(app.focus, Focus::Files);
        // Staged side is empty: Enter must not focus the diff.
        press(&mut app, KeyCode::Enter);
        assert_eq!(app.focus, Focus::Files);
        assert_eq!(app.current_hunk(), None);
    }

    #[test]
    fn cursor_is_confined_to_the_hunk() {
        let mut app = test_app();
        // File 1 displays: [hunk0 header, context | hunk1 header, deletion].
        press(&mut app, KeyCode::Char('j'));
        press(&mut app, KeyCode::Enter);
        press(&mut app, KeyCode::Char('j'));
        assert_eq!(app.cursor, 1);
        press(&mut app, KeyCode::Char('j'));
        assert_eq!(app.cursor, 1, "j stops at the end of the hunk");
        press(&mut app, KeyCode::Char('n'));
        assert_eq!(app.cursor, 2);
        press(&mut app, KeyCode::Char('k'));
        assert_eq!(app.cursor, 2, "k stops at the start of the hunk");
    }

    #[test]
    fn scroll_follows_the_cursor() {
        let mut app = test_app();
        app.set_viewport_height(1);
        press(&mut app, KeyCode::Char('j'));
        press(&mut app, KeyCode::Enter);
        press(&mut app, KeyCode::Char('n'));
        press(&mut app, KeyCode::Char('G'));
        assert_eq!(app.cursor, 3, "G goes to the end of the current hunk");
        assert_eq!(app.scroll, 3);
        press(&mut app, KeyCode::Char('g'));
        assert_eq!(app.cursor, 2, "g goes to the start of the current hunk");
        assert_eq!(app.scroll, 2);
    }
}
