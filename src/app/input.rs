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
        if self.discard_confirm.is_some() {
            self.handle_discard_key(key);
            return;
        }
        match (key.modifiers, key.code) {
            (_, KeyCode::Char('q')) | (KeyModifiers::CONTROL, KeyCode::Char('c')) => {
                self.should_quit = true;
            }
            (_, KeyCode::Tab) => self.switch_tab(),
            (_, KeyCode::Char('c')) => self.open_commit(),
            (_, KeyCode::Char('m')) => self.toggle_mode(),
            _ => match self.focus {
                Focus::Files => self.handle_files_key(key),
                Focus::Diff => self.handle_diff_key(key),
            },
        }
    }

    /// Keys while the discard confirmation is open.
    fn handle_discard_key(&mut self, key: KeyEvent) {
        match (key.modifiers, key.code) {
            (_, KeyCode::Char('y') | KeyCode::Char('Y')) => self.confirm_discard(),
            (_, KeyCode::Char('n') | KeyCode::Char('N')) | (_, KeyCode::Esc) => {
                self.discard_confirm = None;
            }
            _ => {}
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
            KeyCode::Char(' ') => match self.mode {
                Mode::Classic => match (self.tab, self.selected_node().cloned()) {
                    (Tab::Unstaged, Some(Node::File { .. })) => self.stage_selected_file(),
                    (Tab::Unstaged, Some(Node::Dir { .. })) => self.stage_selected_dir(),
                    (Tab::Staged, Some(Node::File { .. })) => self.unstage_selected_file(),
                    (Tab::Staged, Some(Node::Dir { .. })) => self.unstage_selected_dir(),
                    _ => {}
                },
                // Lazygit-style toggle: stage the unstaged part of the
                // selection, otherwise unstage it.
                Mode::Split => match self.selected_node().cloned() {
                    Some(Node::File { .. }) => {
                        if self.selected_file_index_in(Tab::Unstaged).is_some() {
                            self.stage_file_in(Tab::Unstaged);
                        } else if self.selected_file_index_in(Tab::Staged).is_some() {
                            self.unstage_file_in(Tab::Staged);
                        }
                    }
                    Some(Node::Dir { path, .. }) => {
                        if !self.dir_file_indices(Tab::Unstaged, &path).is_empty() {
                            self.stage_dir_in(Tab::Unstaged);
                        } else if !self.dir_file_indices(Tab::Staged, &path).is_empty() {
                            self.unstage_dir_in(Tab::Staged);
                        }
                    }
                    None => {}
                },
            },
            KeyCode::Char('d') => self.prompt_discard(),
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
                let path = self.file_path_at(file_idx);
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
                if self.pane().visual_anchor.is_some() {
                    self.pane_mut().visual_anchor = None;
                } else {
                    self.focus = Focus::Files;
                }
            }
            (_, KeyCode::Char('s')) if self.tab == Tab::Unstaged => {
                if self.pane().visual_anchor.is_some() {
                    self.stage_selected_lines();
                } else {
                    self.stage_selected_hunk();
                }
            }
            (_, KeyCode::Char('u')) if self.tab == Tab::Staged => {
                if self.pane().visual_anchor.is_some() {
                    self.unstage_selected_lines();
                } else {
                    self.unstage_selected_hunk();
                }
            }
            (_, KeyCode::Char('d')) => self.prompt_discard(),
            _ => {}
        }
    }

    fn toggle_visual(&mut self) {
        if self.display_lines().is_empty() {
            return;
        }
        let pane = self.pane_mut();
        pane.visual_anchor = match pane.visual_anchor {
            Some(_) => None,
            None => Some(pane.cursor),
        };
    }

    fn switch_tab(&mut self) {
        match self.mode {
            // The classic layout swaps which side is shown and resets the
            // files selection, as before.
            Mode::Classic => {
                self.tab = self.tab.other();
                self.focus = Focus::Files;
                self.selected_row = 0;
                self.rebuild_tree();
                let pane = self.pane_mut();
                pane.cursor = 0;
                pane.scroll = 0;
                pane.visual_anchor = None;
                self.snap_to_first_change();
            }
            // The split layout cycles the focus through the visible panes
            // (files, then the diff panes left to right), skipping sides
            // whose pane is hidden. This way, Tab after staging lands in
            // the staged pane where `u` unstages.
            Mode::Split => {
                let visible: Vec<Tab> = [Tab::Unstaged, Tab::Staged]
                    .into_iter()
                    .filter(|&side| !self.diff_of(side).files.is_empty())
                    .collect();
                match self.focus {
                    Focus::Files => {
                        if let Some(&side) = visible.first() {
                            self.tab = side;
                            self.focus = Focus::Diff;
                        }
                    }
                    Focus::Diff => match visible.iter().position(|&side| side == self.tab) {
                        // The focused side is hidden: move to the first
                        // visible pane instead of dropping back to files.
                        None => {
                            if let Some(&side) = visible.first() {
                                self.tab = side;
                            } else {
                                self.focus = Focus::Files;
                            }
                        }
                        Some(i) if i + 1 < visible.len() => {
                            self.tab = visible[i + 1];
                        }
                        Some(_) => self.focus = Focus::Files,
                    },
                }
            }
        }
    }

    fn select_row(&mut self, idx: usize) {
        self.selected_row = idx;
        self.files_state.select(Some(idx));
        self.panes = [PaneState::default(); 2];
        for side in [Tab::Unstaged, Tab::Staged] {
            self.snap_to_first_change_for(side);
        }
    }

    fn move_row(&mut self, delta: isize) {
        if self.tree.is_empty() {
            return;
        }
        let next = (self.selected_row as isize + delta).clamp(0, self.tree.len() as isize - 1);
        self.select_row(next as usize);
    }

    /// Move the cursor across the changed lines (`+`/`-`) of the focused
    /// pane; `delta` counts changed lines, not display lines. While a visual
    /// selection is active the cursor stays inside its hunk so the selection
    /// always maps to a single, well-formed patch.
    fn move_cursor(&mut self, delta: isize) {
        self.move_cursor_in(self.tab, delta);
    }

    fn move_cursor_in(&mut self, side: Tab, delta: isize) {
        let (start, positions, current) = {
            let bounds = self.cursor_bounds_for(side);
            if bounds.is_empty() {
                return;
            }
            let (start, end) = (bounds.start, bounds.end);
            let lines = self.display_lines_for(side);
            let positions = changed_positions(&lines[start..end]);
            if positions.is_empty() {
                return;
            }
            let current = self.pane_of(side).cursor;
            (start, positions, current)
        };
        let ord = positions
            .binary_search(&current.saturating_sub(start))
            .unwrap_or_else(|i| i.saturating_sub(1));
        let next = (ord as isize + delta).clamp(0, positions.len() as isize - 1) as usize;
        self.pane_of_mut(side).cursor = start + positions[next];
        self.clamp_cursor_for(side);
    }

    /// Jump to the first or last changed line of the focused pane.
    fn move_to_edge(&mut self, first: bool) {
        self.move_to_edge_in(self.tab, first);
    }

    fn move_to_edge_in(&mut self, side: Tab, first: bool) {
        let (start, edge) = {
            let bounds = self.cursor_bounds_for(side);
            if bounds.is_empty() {
                return;
            }
            let (start, end) = (bounds.start, bounds.end);
            let lines = self.display_lines_for(side);
            let positions = changed_positions(&lines[start..end]);
            let edge = if first {
                positions.first().copied()
            } else {
                positions.last().copied()
            };
            (start, edge)
        };
        if let Some(rel) = edge {
            self.pane_of_mut(side).cursor = start + rel;
            self.clamp_cursor_for(side);
        }
    }

    /// Lines the cursor of `side` may roam: the whole file diff normally,
    /// only the selected hunk while a visual selection is active.
    fn cursor_bounds_for(&self, side: Tab) -> Range<usize> {
        match self.pane_of(side).visual_anchor {
            Some(anchor) => self.hunk_bounds_for(side, anchor),
            None => 0..self.display_lines_for(side).len(),
        }
    }

    /// Display-line range of the hunk containing display line `line_idx` of
    /// the focused pane: from its `@@` header to just before the next
    /// header (or the end of the file).
    pub(super) fn hunk_bounds_at(&self, line_idx: usize) -> Range<usize> {
        self.hunk_bounds_for(self.tab, line_idx)
    }

    fn hunk_bounds_for(&self, side: Tab, line_idx: usize) -> Range<usize> {
        let lines = self.display_lines_for(side);
        let len = lines.len();
        if len == 0 {
            return 0..0;
        }
        let line_idx = line_idx.min(len - 1);
        let start = lines[..=line_idx]
            .iter()
            .rposition(|l| l.kind == LineKind::HunkHeader)
            .unwrap_or(0);
        let end = lines
            .iter()
            .enumerate()
            .skip(start + 1)
            .find(|(_, l)| l.kind == LineKind::HunkHeader)
            .map(|(i, _)| i)
            .unwrap_or(len);
        start..end
    }

    /// Move the cursor of the focused pane to the first changed line of the
    /// next/previous hunk.
    fn jump_hunk(&mut self, direction: isize) {
        let (hunks, cursor) = {
            let lines = self.display_lines();
            (hunk_first_changed_positions(&lines), self.pane().cursor)
        };
        let target = if direction > 0 {
            hunks.iter().copied().find(|&pos| pos > cursor)
        } else {
            hunks.iter().copied().rev().find(|&pos| pos < cursor)
        };
        if let Some(pos) = target {
            self.pane_mut().cursor = pos;
            self.clamp_cursor();
        }
    }

    /// Land the cursor of the focused pane on the first changed line.
    pub(super) fn snap_to_first_change(&mut self) {
        self.snap_to_first_change_for(self.tab);
    }

    fn snap_to_first_change_for(&mut self, side: Tab) {
        let first = {
            let lines = self.display_lines_for(side);
            changed_positions(&lines).first().copied()
        };
        if let Some(pos) = first {
            self.pane_of_mut(side).cursor = pos;
            self.clamp_cursor_for(side);
        }
    }

    /// Keep the cursor of the focused pane inside the displayed lines and
    /// scroll so it stays visible.
    pub(super) fn clamp_cursor(&mut self) {
        self.clamp_cursor_for(self.tab);
    }

    /// Keep the cursor of `side` inside the displayed lines and scroll so
    /// it stays visible.
    pub(super) fn clamp_cursor_for(&mut self, side: Tab) {
        let (len, viewport) = {
            let lines = self.display_lines_for(side);
            (lines.len(), self.viewport_height)
        };
        let pane = self.pane_of_mut(side);
        if len == 0 {
            pane.cursor = 0;
            pane.scroll = 0;
            return;
        }
        pane.cursor = pane.cursor.min(len - 1);
        if viewport == 0 {
            // Before the first render there is no viewport yet; keep the
            // cursor position and leave scroll alone.
            return;
        }
        if pane.cursor < pane.scroll {
            pane.scroll = pane.cursor;
        } else if pane.cursor >= pane.scroll + viewport {
            pane.scroll = pane.cursor + 1 - viewport;
        }
        pane.scroll = pane.scroll.min(len.saturating_sub(viewport));
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
