//! Key and mouse handling and cursor/selection movement for [`App`].
//!
//! Cursor math moves bounded counts (viewport height, list lengths) through
//! `isize` so deltas can be negative; none can overflow in practice.
#![allow(clippy::cast_possible_wrap, clippy::cast_sign_loss)]

use ratatui::crossterm::event::{
    KeyCode, KeyEvent, KeyModifiers, MouseButton, MouseEvent, MouseEventKind,
};
use ratatui::layout::Position;

use super::{App, DiffLine, Focus, LineKind, Mode, Node, Range, Side, tree};

impl App {
    pub fn handle_key(&mut self, key: KeyEvent) {
        if self.help_open {
            // The help overlay is modal: only a close key leaves it.
            match key.code {
                KeyCode::Char('?' | 'q' | 'h') | KeyCode::Esc => self.help_open = false,
                _ => {}
            }
            return;
        }
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
            (_, KeyCode::Char('?')) => self.help_open = true,
            (_, KeyCode::Tab) => self.cycle_focus(),
            (_, KeyCode::Char('c')) => self.open_commit(),
            (_, KeyCode::Char('m')) => self.toggle_mode(),
            (_, KeyCode::Char('[')) => self.shrink_files_pane(),
            (_, KeyCode::Char(']')) => self.grow_files_pane(),
            _ => match self.focus {
                Focus::Files => self.handle_files_key(key),
                Focus::Diff => self.handle_diff_key(key),
            },
        }
    }

    /// Handle a mouse event against the last rendered layout. Clicks are
    /// ignored while the help overlay is open.
    pub fn handle_mouse(&mut self, event: MouseEvent) {
        if self.help_open {
            return;
        }
        let position = Position::new(event.column, event.row);
        match event.kind {
            MouseEventKind::Down(MouseButton::Left) => self.mouse_click(position),
            // Dragging over a scrollbar (button held) scrolls like a click.
            MouseEventKind::Drag(MouseButton::Left) => self.mouse_drag(position),
            MouseEventKind::ScrollUp => self.mouse_scroll(position, 1),
            MouseEventKind::ScrollDown => self.mouse_scroll(position, -1),
            _ => {}
        }
    }

    /// The tree row under `position`, if the click hit the files list.
    fn row_at(&self, position: Position) -> Option<usize> {
        if !self.files_rect.contains(position) {
            return None;
        }
        // The first inner row is below the block border; the list may be
        // scrolled, so the visual row maps through the list's offset.
        let visual = usize::from(position.y).checked_sub(usize::from(self.files_rect.y) + 1)?;
        let row = visual + self.files_state.offset();
        (row < self.tree.len()).then_some(row)
    }

    /// The diff pane under `position`, if any.
    fn side_at(&self, position: Position) -> Option<Side> {
        [Side::Unstaged, Side::Staged]
            .into_iter()
            .find(|&side| self.diff_rects[side.index()].is_some_and(|r| r.contains(position)))
    }

    /// The overflowing pane whose scrollbar column is under `position`.
    fn scrollbar_at(&self, position: Position) -> Option<Side> {
        [Side::Unstaged, Side::Staged].into_iter().find(|&side| {
            let Some(rect) = self.diff_rects[side.index()] else {
                return false;
            };
            // The scrollbar sits in the inner last column: one in from
            // the right border, and one in from the top and bottom.
            let column = rect.x + rect.width.saturating_sub(2);
            let track_len = usize::from(rect.height.saturating_sub(2));
            position.x == column
                && position.y > rect.y
                && position.y < rect.y + rect.height.saturating_sub(1)
                && self.display_lines_for(side).len() > track_len
        })
    }

    fn mouse_click(&mut self, position: Position) {
        if let Some(row) = self.row_at(position) {
            self.focus = Focus::Files;
            self.select_row(row);
            return;
        }
        if let Some(side) = self.scrollbar_at(position) {
            self.side = side;
            self.focus = Focus::Diff;
            self.scroll_pane_to(side, position);
            return;
        }
        let Some(side) = self.side_at(position) else {
            return;
        };
        self.side = side;
        self.focus = Focus::Diff;
        // Jump the cursor to the changed line nearest to the click.
        let rect = self.diff_rects[side.index()].expect("found above");
        let Some(y) = usize::from(position.y).checked_sub(usize::from(rect.y) + 1) else {
            return;
        };
        self.snap_cursor_to_nearest(side, y + self.pane_of(side).scroll);
    }

    /// Drag events: only a scrollbar column reacts while the button is
    /// held; everywhere else a drag is a no-op.
    fn mouse_drag(&mut self, position: Position) {
        if let Some(side) = self.scrollbar_at(position) {
            self.scroll_pane_to(side, position);
        }
    }

    /// Scroll the pane so the clicked scrollbar row lands at that
    /// proportion of the content, and move the cursor to the changed line
    /// nearest to the click.
    fn scroll_pane_to(&mut self, side: Side, position: Position) {
        let rect = self.diff_rects[side.index()].expect("found above");
        let track_len = usize::from(rect.height.saturating_sub(2));
        let Some(offset) = usize::from(position.y)
            .checked_sub(usize::from(rect.y) + 1)
            .filter(|&offset| offset < track_len)
        else {
            return;
        };
        let total = self.display_lines_for(side).len();
        let max_scroll = total.saturating_sub(track_len);
        let steps = track_len.saturating_sub(1).max(1);
        let scroll = offset.saturating_mul(max_scroll) / steps;
        self.pane_of_mut(side).scroll = scroll;
        self.snap_cursor_to_nearest(side, scroll + offset);
        self.clamp_cursor_for(side);
    }

    fn mouse_scroll(&mut self, position: Position, wheel_steps: isize) {
        if self.row_at(position).is_some() {
            self.focus = Focus::Files;
            self.move_row(-wheel_steps);
            return;
        }
        if let Some(side) = self.side_at(position) {
            // Wheeling a diff pane scrolls it and brings it into focus.
            self.side = side;
            self.focus = Focus::Diff;
            self.move_cursor_in(side, -wheel_steps);
        }
    }

    /// Move the pane's cursor to the changed line nearest to `index`.
    fn snap_cursor_to_nearest(&mut self, side: Side, index: usize) {
        let positions = changed_positions(&self.display_lines_for(side));
        if positions.is_empty() {
            return;
        }
        let nearest = match positions.binary_search(&index) {
            Ok(i) => positions[i],
            Err(0) => positions[0],
            Err(i) => {
                let before = positions[i - 1];
                match positions.get(i) {
                    Some(&after) if index - before > after - index => after,
                    _ => before,
                }
            }
        };
        self.pane_of_mut(side).cursor = nearest;
        self.clamp_cursor_for(side);
    }

    /// Keys while the discard confirmation is open.
    fn handle_discard_key(&mut self, key: KeyEvent) {
        match (key.modifiers, key.code) {
            (_, KeyCode::Char('y' | 'Y')) => self.confirm_discard(),
            (_, KeyCode::Char('n' | 'N') | KeyCode::Esc) => {
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
            (_, KeyCode::Home) | (KeyModifiers::CONTROL, KeyCode::Char('a')) => {
                input.cursor = 0;
            }
            (_, KeyCode::End) | (KeyModifiers::CONTROL, KeyCode::Char('e')) => {
                input.cursor = input.text.len();
            }
            (KeyModifiers::CONTROL, KeyCode::Char('w'))
            | (KeyModifiers::ALT, KeyCode::Backspace) => input.backspace_word(),
            (_, KeyCode::Backspace) => input.backspace(),
            (_, KeyCode::Delete) => input.delete(),
            (KeyModifiers::CONTROL, KeyCode::Char('k')) => input.kill_to_end(),
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
                Mode::Classic => match (self.side, self.selected_node().cloned()) {
                    (Side::Unstaged, Some(Node::File { .. })) => self.stage_selected_file(),
                    (Side::Unstaged, Some(Node::Dir { .. })) => self.stage_selected_dir(),
                    (Side::Staged, Some(Node::File { .. })) => self.unstage_selected_file(),
                    (Side::Staged, Some(Node::Dir { .. })) => self.unstage_selected_dir(),
                    _ => {}
                },
                // Lazygit-style toggle: stage the unstaged part of the
                // selection, otherwise unstage it.
                Mode::Split => match self.selected_node().cloned() {
                    Some(Node::File { .. }) => {
                        if self.selected_file_index_in(Side::Unstaged).is_some() {
                            self.stage_file_in(Side::Unstaged);
                        } else if self.selected_file_index_in(Side::Staged).is_some() {
                            self.unstage_file_in(Side::Staged);
                        }
                    }
                    Some(Node::Dir { path, .. }) => {
                        if !self.dir_file_indices(Side::Unstaged, &path).is_empty() {
                            self.stage_dir_in(Side::Unstaged);
                        } else if !self.dir_file_indices(Side::Staged, &path).is_empty() {
                            self.unstage_dir_in(Side::Staged);
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
            (_, KeyCode::Down | KeyCode::Char('j')) => self.move_cursor(1),
            (_, KeyCode::Up | KeyCode::Char('k')) => self.move_cursor(-1),
            (_, KeyCode::PageDown) => self.move_cursor(self.viewport_height as isize),
            (_, KeyCode::PageUp) => self.move_cursor(-(self.viewport_height as isize)),
            (KeyModifiers::CONTROL, KeyCode::Char('d')) => {
                self.move_cursor(self.viewport_height as isize / 2);
            }
            (KeyModifiers::CONTROL, KeyCode::Char('u')) => {
                self.move_cursor(-(self.viewport_height as isize / 2));
            }
            (_, KeyCode::Home | KeyCode::Char('g')) => self.move_to_edge(true),
            (_, KeyCode::End | KeyCode::Char('G')) => self.move_to_edge(false),
            (_, KeyCode::Char('n')) => self.jump_hunk(1),
            (_, KeyCode::Char('p')) => self.jump_hunk(-1),
            (_, KeyCode::Char('v')) => self.toggle_visual(),
            (_, KeyCode::Left | KeyCode::Char('h')) => {
                self.focus = Focus::Files;
            }
            (_, KeyCode::Esc) => {
                if self.pane().visual_anchor.is_some() {
                    self.pane_mut().visual_anchor = None;
                } else {
                    self.focus = Focus::Files;
                }
            }
            (_, KeyCode::Char('s' | ' ')) if self.side == Side::Unstaged => {
                if self.pane().visual_anchor.is_some() {
                    self.stage_selected_lines();
                } else {
                    self.stage_selected_hunk();
                }
            }
            (_, KeyCode::Char('u' | ' ')) if self.side == Side::Staged => {
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

    fn cycle_focus(&mut self) {
        match self.mode {
            // The classic layout swaps which side is shown. Each side
            // remembers its files selection and its pane cursor, so Tab
            // restores the user's place instead of resetting it.
            Mode::Classic => {
                self.saved_rows[self.side.index()] = self.selected_row;
                self.side = self.side.other();
                self.focus = Focus::Files;
                self.rebuild_tree();
                self.selected_row = if self.tree.is_empty() {
                    0
                } else {
                    self.saved_rows[self.side.index()].min(self.tree.len() - 1)
                };
                self.files_state.select(Some(self.selected_row));
                if self.pane().cursor == 0 {
                    // First visit to this side: land on the first change.
                    self.snap_to_first_change();
                } else {
                    self.clamp_cursor();
                }
            }
            // The split layout cycles the focus through the visible panes
            // (files, then the diff panes left to right), skipping sides
            // whose pane is hidden, so after staging Tab lands in the
            // staged pane where `u` unstages.
            Mode::Split => {
                let visible: Vec<Side> = [Side::Unstaged, Side::Staged]
                    .into_iter()
                    .filter(|&side| !self.diff_of(side).files.is_empty())
                    .collect();
                match self.focus {
                    Focus::Files => {
                        if let Some(&side) = visible.first() {
                            self.side = side;
                            self.focus = Focus::Diff;
                        }
                    }
                    Focus::Diff => match visible.iter().position(|&side| side == self.side) {
                        // The focused side is hidden: move to the first
                        // visible pane instead of dropping back to files.
                        None => {
                            if let Some(&side) = visible.first() {
                                self.side = side;
                            } else {
                                self.focus = Focus::Files;
                            }
                        }
                        Some(i) if i + 1 < visible.len() => {
                            self.side = visible[i + 1];
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
        self.reset_panes();
        for side in [Side::Unstaged, Side::Staged] {
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
        self.move_cursor_in(self.side, delta);
    }

    fn move_cursor_in(&mut self, side: Side, delta: isize) {
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
        self.move_to_edge_in(self.side, first);
    }

    fn move_to_edge_in(&mut self, side: Side, first: bool) {
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
    fn cursor_bounds_for(&self, side: Side) -> Range<usize> {
        match self.pane_of(side).visual_anchor {
            Some(anchor) => self.hunk_bounds_for(side, anchor),
            None => 0..self.display_lines_for(side).len(),
        }
    }

    /// Display-line range of the hunk containing display line `line_idx` of
    /// the focused pane: from its `@@` header to just before the next
    /// header (or the end of the file).
    pub(super) fn hunk_bounds_at(&self, line_idx: usize) -> Range<usize> {
        self.hunk_bounds_for(self.side, line_idx)
    }

    fn hunk_bounds_for(&self, side: Side, line_idx: usize) -> Range<usize> {
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
            .map_or(len, |(i, _)| i);
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
        self.snap_to_first_change_for(self.side);
    }

    fn snap_to_first_change_for(&mut self, side: Side) {
        let first = {
            let lines = self.display_lines_for(side);
            changed_positions(&lines).first().copied()
        };
        if let Some(pos) = first {
            self.pane_of_mut(side).cursor = pos;
            self.clamp_cursor_for(side);
        }
    }

    /// Clamp the focused pane's cursor to the displayed lines, scrolling
    /// as needed to keep it visible.
    pub(super) fn clamp_cursor(&mut self) {
        self.clamp_cursor_for(self.side);
    }

    /// Clamp the cursor of `side` to the displayed lines, scrolling as
    /// needed to keep it visible.
    pub(super) fn clamp_cursor_for(&mut self, side: Side) {
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
