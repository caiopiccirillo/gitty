//! Application state and input handling.

use ratatui::crossterm::event::{KeyCode, KeyEvent, KeyModifiers};

use crate::diff::{DiffView, HunkId};

pub struct App {
    pub diff: DiffView,
    /// First visible line (top of the viewport).
    pub scroll: usize,
    /// Hunk currently selected with n/p, if any.
    pub selected_hunk: Option<HunkId>,
    /// Height of the diff viewport in lines, updated on every render/resize.
    viewport_height: usize,
    pub should_quit: bool,
}

impl App {
    pub fn new(diff: DiffView) -> Self {
        Self {
            diff,
            scroll: 0,
            selected_hunk: None,
            viewport_height: 0,
            should_quit: false,
        }
    }

    pub fn set_viewport_height(&mut self, height: usize) {
        self.viewport_height = height;
        self.clamp_scroll();
    }

    fn max_scroll(&self) -> usize {
        self.diff.len().saturating_sub(self.viewport_height)
    }

    fn clamp_scroll(&mut self) {
        self.scroll = self.scroll.min(self.max_scroll());
    }

    fn scroll_by(&mut self, delta: isize) {
        let next = self.scroll as isize + delta;
        self.scroll = next.clamp(0, self.max_scroll() as isize) as usize;
    }

    pub fn handle_key(&mut self, key: KeyEvent) {
        match (key.modifiers, key.code) {
            (_, KeyCode::Char('q'))
            | (_, KeyCode::Esc)
            | (KeyModifiers::CONTROL, KeyCode::Char('c')) => self.should_quit = true,
            (_, KeyCode::Down) | (_, KeyCode::Char('j')) => self.scroll_by(1),
            (_, KeyCode::Up) | (_, KeyCode::Char('k')) => self.scroll_by(-1),
            (_, KeyCode::PageDown) => self.scroll_by(self.viewport_height as isize),
            (_, KeyCode::PageUp) => self.scroll_by(-(self.viewport_height as isize)),
            (KeyModifiers::CONTROL, KeyCode::Char('d')) => {
                self.scroll_by(self.viewport_height as isize / 2);
            }
            (KeyModifiers::CONTROL, KeyCode::Char('u')) => {
                self.scroll_by(-(self.viewport_height as isize / 2));
            }
            (_, KeyCode::Home) | (_, KeyCode::Char('g')) => self.scroll = 0,
            (_, KeyCode::End) | (_, KeyCode::Char('G')) => self.scroll = self.max_scroll(),
            (_, KeyCode::Char('n')) => self.select_next_hunk(),
            (_, KeyCode::Char('p')) => self.select_prev_hunk(),
            _ => {}
        }
    }

    /// Select the next hunk, or the first one if nothing is selected.
    pub fn select_next_hunk(&mut self) {
        let hunks = self.diff.hunks();
        let next = match self.selected_hunk {
            None => hunks.first().copied(),
            Some(current) => hunks
                .iter()
                .position(|id| *id == current)
                .and_then(|i| hunks.get(i + 1))
                .copied()
                .or(Some(current)),
        };
        self.select_hunk(next);
    }

    /// Select the previous hunk, or the last one if nothing is selected.
    pub fn select_prev_hunk(&mut self) {
        let hunks = self.diff.hunks();
        let prev = match self.selected_hunk {
            None => hunks.last().copied(),
            Some(current) => match hunks.iter().position(|id| *id == current) {
                Some(i) if i > 0 => hunks.get(i - 1).copied(),
                _ => Some(current),
            },
        };
        self.select_hunk(prev);
    }

    fn select_hunk(&mut self, id: Option<HunkId>) {
        self.selected_hunk = id;
        if let Some(id) = id {
            self.scroll_to_hunk(id);
        }
    }

    /// Scroll so the hunk header lands near the top of the viewport, keeping a
    /// couple of lines of context (e.g. the file header) visible above it.
    fn scroll_to_hunk(&mut self, id: HunkId) {
        if let Some(range) = self.diff.hunk_line_range(id) {
            self.scroll = range.start.saturating_sub(2).min(self.max_scroll());
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::diff::{DiffLine, LineKind};

    fn app_with_lines(lines: usize, viewport: usize) -> App {
        let diff = DiffView {
            lines: (0..lines)
                .map(|i| DiffLine {
                    kind: LineKind::Context,
                    content: i.to_string(),
                    file_idx: 0,
                    hunk_idx: Some(0),
                })
                .collect(),
            file_count: 1,
        };
        let mut app = App::new(diff);
        app.set_viewport_height(viewport);
        app
    }

    fn press(app: &mut App, code: KeyCode) {
        app.handle_key(KeyEvent::new(code, KeyModifiers::NONE));
    }

    /// Two files (1 hunk + 2 hunks), 8 lines; viewport shows 3.
    fn app_with_hunks() -> App {
        let mut app = App::new(crate::diff::two_file_view());
        app.set_viewport_height(3);
        app
    }

    #[test]
    fn n_walks_hunks_forward_and_stops_at_last() {
        let mut app = app_with_hunks();
        press(&mut app, KeyCode::Char('n'));
        assert_eq!(
            app.selected_hunk,
            Some(HunkId {
                file_idx: 0,
                hunk_idx: 0
            })
        );
        press(&mut app, KeyCode::Char('n'));
        assert_eq!(
            app.selected_hunk,
            Some(HunkId {
                file_idx: 1,
                hunk_idx: 0
            })
        );
        press(&mut app, KeyCode::Char('n'));
        assert_eq!(
            app.selected_hunk,
            Some(HunkId {
                file_idx: 1,
                hunk_idx: 1
            })
        );
        press(&mut app, KeyCode::Char('n'));
        assert_eq!(
            app.selected_hunk,
            Some(HunkId {
                file_idx: 1,
                hunk_idx: 1
            }),
            "no wraparound at the last hunk"
        );
    }

    #[test]
    fn p_walks_hunks_backward_and_stops_at_first() {
        let mut app = app_with_hunks();
        press(&mut app, KeyCode::Char('p'));
        assert_eq!(
            app.selected_hunk,
            Some(HunkId {
                file_idx: 1,
                hunk_idx: 1
            }),
            "starts from the last hunk"
        );
        press(&mut app, KeyCode::Char('p'));
        assert_eq!(
            app.selected_hunk,
            Some(HunkId {
                file_idx: 1,
                hunk_idx: 0
            })
        );
        press(&mut app, KeyCode::Char('p'));
        assert_eq!(
            app.selected_hunk,
            Some(HunkId {
                file_idx: 0,
                hunk_idx: 0
            })
        );
        press(&mut app, KeyCode::Char('p'));
        assert_eq!(
            app.selected_hunk,
            Some(HunkId {
                file_idx: 0,
                hunk_idx: 0
            }),
            "no wraparound at the first hunk"
        );
    }

    #[test]
    fn selecting_a_hunk_scrolls_it_into_view() {
        let mut app = app_with_hunks();
        assert_eq!(app.scroll, 0);
        press(&mut app, KeyCode::Char('n'));
        press(&mut app, KeyCode::Char('n'));
        press(&mut app, KeyCode::Char('n'));
        // Last hunk starts at line 6; 2 lines of context above it, so 4.
        assert_eq!(app.scroll, 4);
    }

    #[test]
    fn n_on_empty_diff_does_nothing() {
        let mut app = App::new(DiffView::default());
        press(&mut app, KeyCode::Char('n'));
        assert_eq!(app.selected_hunk, None);
    }

    #[test]
    fn scrolls_down_and_clamps_at_bottom() {
        let mut app = app_with_lines(30, 10);
        press(&mut app, KeyCode::Char('j'));
        assert_eq!(app.scroll, 1);
        press(&mut app, KeyCode::End);
        assert_eq!(app.scroll, 20);
        press(&mut app, KeyCode::Down);
        assert_eq!(app.scroll, 20, "cannot scroll past the last line");
    }

    #[test]
    fn scrolls_up_and_clamps_at_top() {
        let mut app = app_with_lines(30, 10);
        press(&mut app, KeyCode::End);
        press(&mut app, KeyCode::Char('k'));
        assert_eq!(app.scroll, 19);
        press(&mut app, KeyCode::Home);
        assert_eq!(app.scroll, 0);
        press(&mut app, KeyCode::Up);
        assert_eq!(app.scroll, 0, "cannot scroll above the first line");
    }

    #[test]
    fn page_scroll_uses_viewport_height() {
        let mut app = app_with_lines(100, 10);
        press(&mut app, KeyCode::PageDown);
        assert_eq!(app.scroll, 10);
        press(&mut app, KeyCode::PageUp);
        assert_eq!(app.scroll, 0);
    }

    #[test]
    fn quits_on_q() {
        let mut app = app_with_lines(10, 10);
        press(&mut app, KeyCode::Char('q'));
        assert!(app.should_quit);
    }

    #[test]
    fn shrinking_viewport_keeps_scroll_valid() {
        let mut app = app_with_lines(30, 10);
        press(&mut app, KeyCode::End);
        app.set_viewport_height(25);
        assert_eq!(app.scroll, 5);
    }
}
