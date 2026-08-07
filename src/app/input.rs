//! Key handling and cursor/selection movement for [`App`].

use ratatui::crossterm::event::{KeyCode, KeyEvent, KeyModifiers};

use super::*;

impl App {
    pub fn handle_key(&mut self, key: KeyEvent) {
        self.message = None;
        if self.commit_input.is_some() {
            self.handle_commit_key(key);
            return;
        }
        match (key.modifiers, key.code) {
            (_, KeyCode::Char('q')) | (KeyModifiers::CONTROL, KeyCode::Char('c')) => {
                self.should_quit = true;
            }
            (_, KeyCode::Tab) => self.switch_tab(),
            (_, KeyCode::Char('c')) => self.open_commit(),
            _ => match self.focus {
                Focus::Files => self.handle_files_key(key),
                Focus::Diff => self.handle_diff_key(key),
            },
        }
    }

    /// Keys while the commit message box is open.
    fn handle_commit_key(&mut self, key: KeyEvent) {
        let Some(input) = self.commit_input.as_mut() else {
            return;
        };
        match (key.modifiers, key.code) {
            (_, KeyCode::Esc) => self.commit_input = None,
            (_, KeyCode::Enter) => self.commit(),
            (_, KeyCode::Left) => input.left(),
            (_, KeyCode::Right) => input.right(),
            (_, KeyCode::Home) => input.cursor = 0,
            (_, KeyCode::End) => input.cursor = input.text.len(),
            (_, KeyCode::Backspace) => input.backspace(),
            (KeyModifiers::CONTROL, KeyCode::Char('u')) => {
                input.text.clear();
                input.cursor = 0;
            }
            (m, KeyCode::Char(c)) if m.is_empty() || m == KeyModifiers::SHIFT => {
                input.insert(c);
            }
            _ => {}
        }
    }

    fn handle_files_key(&mut self, key: KeyEvent) {
        match key.code {
            KeyCode::Down | KeyCode::Char('j') => self.move_row(1),
            KeyCode::Up | KeyCode::Char('k') => self.move_row(-1),
            KeyCode::Home | KeyCode::Char('g') => self.select_row(0),
            KeyCode::End | KeyCode::Char('G') => {
                if !self.tree.is_empty() {
                    self.select_row(self.tree.len() - 1);
                }
            }
            KeyCode::Enter => self.files_activate(),
            KeyCode::Right | KeyCode::Char('l') => self.files_right(),
            KeyCode::Left | KeyCode::Char('h') => self.files_left(),
            KeyCode::Char(' ') => match (self.tab, self.selected_node().cloned()) {
                (Tab::Unstaged, Some(Node::File { .. })) => self.stage_selected_file(),
                (Tab::Unstaged, Some(Node::Dir { .. })) => self.stage_selected_dir(),
                (Tab::Staged, Some(Node::File { .. })) => self.unstage_selected_file(),
                (Tab::Staged, Some(Node::Dir { .. })) => self.unstage_selected_dir(),
                _ => {}
            },
            _ => {}
        }
    }

    /// Enter on the files pane: directories fold/unfold, files open their diff.
    fn files_activate(&mut self) {
        match self.selected_node().cloned() {
            Some(Node::Dir { path, .. }) => self.toggle_dir(&path),
            Some(Node::File { .. }) => {
                self.focus = Focus::Diff;
                self.snap_to_first_change();
            }
            None => {}
        }
    }

    /// Right/`l`: expand a collapsed directory, or open a file's diff.
    fn files_right(&mut self) {
        match self.selected_node().cloned() {
            Some(Node::Dir {
                path, collapsed, ..
            }) => {
                if collapsed {
                    self.set_dir_collapsed(&path, false);
                }
            }
            Some(Node::File { .. }) => {
                self.focus = Focus::Diff;
                self.snap_to_first_change();
            }
            None => {}
        }
    }

    /// Left/`h`: collapse an expanded directory, or move the selection to
    /// the parent directory row.
    fn files_left(&mut self) {
        match self.selected_node().cloned() {
            Some(Node::Dir {
                path,
                collapsed: false,
                ..
            }) => self.set_dir_collapsed(&path, true),
            Some(Node::Dir {
                path,
                collapsed: true,
                ..
            }) => {
                if let Some(row) = tree::parent_dir(&path).and_then(|p| self.dir_row(p)) {
                    self.select_row(row);
                }
            }
            Some(Node::File { file_idx, .. }) => {
                let path = self.current_diff().files[file_idx].path.clone();
                if let Some(row) = tree::parent_dir(&path).and_then(|p| self.dir_row(p)) {
                    self.select_row(row);
                }
            }
            None => {}
        }
    }

    fn toggle_dir(&mut self, path: &str) {
        let collapse = !self.collapsed_dirs.contains(path);
        self.set_dir_collapsed(path, collapse);
    }

    fn set_dir_collapsed(&mut self, path: &str, collapse: bool) {
        if collapse {
            self.collapsed_dirs.insert(path.to_string());
        } else {
            self.collapsed_dirs.remove(path);
        }
        self.rebuild_tree();
        // Keep the toggled directory selected.
        if let Some(row) = self.dir_row(path) {
            self.select_row(row);
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
            (_, KeyCode::Home) | (_, KeyCode::Char('g')) => self.move_to_edge(true),
            (_, KeyCode::End) | (_, KeyCode::Char('G')) => self.move_to_edge(false),
            (_, KeyCode::Char('n')) => self.jump_hunk(1),
            (_, KeyCode::Char('p')) => self.jump_hunk(-1),
            (_, KeyCode::Char('v')) => self.toggle_visual(),
            (_, KeyCode::Left) | (_, KeyCode::Char('h')) => {
                self.focus = Focus::Files;
            }
            (_, KeyCode::Esc) => {
                if self.visual_anchor.is_some() {
                    self.visual_anchor = None;
                } else {
                    self.focus = Focus::Files;
                }
            }
            (_, KeyCode::Char('s')) if self.tab == Tab::Unstaged => {
                if self.visual_anchor.is_some() {
                    self.stage_selected_lines();
                } else {
                    self.stage_selected_hunk();
                }
            }
            (_, KeyCode::Char('u')) if self.tab == Tab::Staged => {
                if self.visual_anchor.is_some() {
                    self.unstage_selected_lines();
                } else {
                    self.unstage_selected_hunk();
                }
            }
            _ => {}
        }
    }

    fn toggle_visual(&mut self) {
        self.visual_anchor = match self.visual_anchor {
            Some(_) => None,
            None if !self.display_lines().is_empty() => Some(self.cursor),
            None => None,
        };
    }

    fn switch_tab(&mut self) {
        self.tab = match self.tab {
            Tab::Unstaged => Tab::Staged,
            Tab::Staged => Tab::Unstaged,
        };
        self.focus = Focus::Files;
        self.selected_row = 0;
        self.rebuild_tree();
        self.cursor = 0;
        self.scroll = 0;
        self.visual_anchor = None;
        self.snap_to_first_change();
    }

    fn select_row(&mut self, idx: usize) {
        self.selected_row = idx;
        self.files_state.select(Some(idx));
        self.cursor = 0;
        self.scroll = 0;
        self.visual_anchor = None;
        self.snap_to_first_change();
    }

    fn move_row(&mut self, delta: isize) {
        if self.tree.is_empty() {
            return;
        }
        let next = (self.selected_row as isize + delta).clamp(0, self.tree.len() as isize - 1);
        self.select_row(next as usize);
    }

    /// Move the cursor across the changed lines (`+`/`-`) of the diff;
    /// `delta` counts changed lines, not display lines. While a visual
    /// selection is active the cursor stays inside its hunk so the selection
    /// always maps to a single, well-formed patch.
    fn move_cursor(&mut self, delta: isize) {
        let bounds = self.cursor_bounds();
        if bounds.is_empty() {
            return;
        }
        let (start, end) = (bounds.start, bounds.end);
        let lines = self.display_lines();
        let positions = changed_positions(&lines[start..end]);
        if positions.is_empty() {
            return;
        }
        let current = positions
            .binary_search(&self.cursor.saturating_sub(start))
            .unwrap_or_else(|i| i.saturating_sub(1));
        let next = (current as isize + delta).clamp(0, positions.len() as isize - 1) as usize;
        self.cursor = start + positions[next];
        self.clamp_cursor();
    }

    /// Jump to the first or last changed line of the current bounds.
    fn move_to_edge(&mut self, first: bool) {
        let bounds = self.cursor_bounds();
        if bounds.is_empty() {
            return;
        }
        let (start, end) = (bounds.start, bounds.end);
        let lines = self.display_lines();
        let positions = changed_positions(&lines[start..end]);
        let edge = if first {
            positions.first()
        } else {
            positions.last()
        };
        if let Some(&rel) = edge {
            self.cursor = start + rel;
            self.clamp_cursor();
        }
    }

    /// Lines the cursor may roam: the whole file diff normally, only the
    /// selected hunk while a visual selection is active.
    fn cursor_bounds(&self) -> Range<usize> {
        match self.visual_anchor {
            Some(anchor) => self.hunk_bounds_at(anchor),
            None => 0..self.display_lines().len(),
        }
    }

    /// Display-line range of the hunk containing display line `line_idx`:
    /// from its `@@` header to just before the next header (or the end of
    /// the file).
    pub(super) fn hunk_bounds_at(&self, line_idx: usize) -> Range<usize> {
        let len = self.display_lines().len();
        if len == 0 {
            return 0..0;
        }
        let line_idx = line_idx.min(len - 1);
        let start = self.display_lines()[..=line_idx]
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

    /// Move the cursor to the first changed line of the next/previous hunk.
    fn jump_hunk(&mut self, direction: isize) {
        let lines = self.display_lines();
        let hunks = hunk_first_changed_positions(&lines);
        let target = if direction > 0 {
            hunks.iter().copied().find(|&pos| pos > self.cursor)
        } else {
            hunks.iter().copied().rev().find(|&pos| pos < self.cursor)
        };
        if let Some(pos) = target {
            self.cursor = pos;
            self.clamp_cursor();
        }
    }

    /// Land the cursor on the first changed line of the displayed diff.
    fn snap_to_first_change(&mut self) {
        let lines = self.display_lines();
        if let Some(&pos) = changed_positions(&lines).first() {
            self.cursor = pos;
            self.clamp_cursor();
        }
    }

    /// Keep the cursor inside the displayed lines and scroll so it stays
    /// visible.
    pub(super) fn clamp_cursor(&mut self) {
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

/// Positions of the changed lines (`+`/`-`) within a slice of displayed
/// lines, in display order.
fn changed_positions(lines: &[&DiffLine]) -> Vec<usize> {
    lines
        .iter()
        .enumerate()
        .filter(|(_, line)| matches!(line.kind, LineKind::Addition | LineKind::Deletion))
        .map(|(i, _)| i)
        .collect()
}

/// Display-line position of the first changed line of each hunk, in display
/// order. Hunks are identified by `(file, hunk)` so directory aggregates
/// with repeated hunk indices stay correct.
fn hunk_first_changed_positions(lines: &[&DiffLine]) -> Vec<usize> {
    let mut positions = Vec::new();
    let mut seen: Option<(usize, usize)> = None;
    for (i, line) in lines.iter().enumerate() {
        let Some(hunk_idx) = line.hunk_idx else {
            continue;
        };
        let key = (line.file_idx, hunk_idx);
        if seen != Some(key) && matches!(line.kind, LineKind::Addition | LineKind::Deletion) {
            positions.push(i);
            seen = Some(key);
        }
    }
    positions
}
