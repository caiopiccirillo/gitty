//! Application state and input handling.

use ratatui::crossterm::event::{KeyCode, KeyEvent, KeyModifiers};

use crate::diff::DiffView;

pub struct App {
    pub diff: DiffView,
    /// First visible line (top of the viewport).
    pub scroll: usize,
    /// Height of the diff viewport in lines, updated on every render/resize.
    viewport_height: usize,
    pub should_quit: bool,
}

impl App {
    pub fn new(diff: DiffView) -> Self {
        Self {
            diff,
            scroll: 0,
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
            _ => {}
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
