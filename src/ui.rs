//! Rendering with ratatui: a file list on the left, the selected file's
//! diff with a line cursor on the right, and a status bar.

use ratatui::{
    Frame,
    layout::{Constraint, Layout, Rect},
    style::{Color, Modifier, Style},
    text::{Line, Span},
    widgets::{Block, List, ListItem, Paragraph},
};

use crate::app::{App, Focus, Tab};
use crate::diff::{FileStatus, LineKind};

pub fn render(frame: &mut Frame, app: &mut App) {
    let [main_area, status_area] =
        Layout::vertical([Constraint::Min(1), Constraint::Length(1)]).areas(frame.area());
    let [files_area, diff_area] =
        Layout::horizontal([Constraint::Percentage(30), Constraint::Percentage(70)])
            .areas(main_area);
    app.set_viewport_height(diff_area.height.saturating_sub(2) as usize);

    render_files(frame, app, files_area);
    render_diff(frame, app, diff_area);
    frame.render_widget(status_bar(app), status_area);
}

fn render_files(frame: &mut Frame, app: &App, area: Rect) {
    let diff = app.current_diff();
    let title = match app.tab {
        Tab::Unstaged => format!(" Unstaged ({}) ", diff.files.len()),
        Tab::Staged => format!(" Staged ({}) ", diff.files.len()),
    };
    let block = pane_block(title, app.focus == Focus::Files);

    if diff.files.is_empty() {
        let empty = Paragraph::new("no changes")
            .style(Style::new().fg(Color::DarkGray))
            .block(block);
        frame.render_widget(empty, area);
        return;
    }

    let items: Vec<ListItem> = diff
        .files
        .iter()
        .enumerate()
        .map(|(i, file)| {
            let (letter, color) = status_badge(file.status);
            let row = Line::from(vec![
                Span::styled(format!("{letter} "), Style::new().fg(color)),
                Span::raw(file.path.clone()),
            ]);
            let style = if i == app.selected_file {
                Style::new().bg(Color::DarkGray)
            } else {
                Style::new()
            };
            ListItem::new(row).style(style)
        })
        .collect();
    frame.render_widget(List::new(items).block(block), area);
}

fn render_diff(frame: &mut Frame, app: &App, area: Rect) {
    let title = app
        .current_diff()
        .files
        .get(app.selected_file)
        .map(|file| format!(" {} ", file.path))
        .unwrap_or_else(|| " diff ".into());
    let block = pane_block(title, app.focus == Focus::Diff);

    let lines: Vec<Line> = app
        .display_lines()
        .iter()
        .enumerate()
        .map(|(i, line)| {
            let prefix = match line.kind {
                LineKind::Addition => "+",
                LineKind::Deletion => "-",
                LineKind::Context => " ",
                _ => "",
            };
            // CR of CRLF files is kept in the model but must not reach the
            // terminal, where it would reset the cursor column.
            let text = format!("{prefix}{}", line.content.trim_end_matches('\r'));
            let mut style = style_for(line.kind);
            if i == app.cursor {
                style = style.bg(Color::DarkGray);
                if line.kind == LineKind::Meta {
                    // DarkGray on DarkGray would be unreadable.
                    style = style.fg(Color::Gray);
                }
            }
            Line::styled(text, style)
        })
        .collect();
    let scroll = u16::try_from(app.scroll).unwrap_or(u16::MAX);
    let diff = Paragraph::new(lines).block(block).scroll((scroll, 0));
    frame.render_widget(diff, area);
}

fn status_bar(app: &App) -> Paragraph<'static> {
    let diff = app.current_diff();
    let tab_name = match app.tab {
        Tab::Unstaged => "unstaged",
        Tab::Staged => "staged",
    };
    let file_pos = if diff.files.is_empty() {
        "0/0".to_string()
    } else {
        format!("{}/{}", app.selected_file + 1, diff.files.len())
    };
    let file_hunks: Vec<_> = diff
        .hunks()
        .into_iter()
        .filter(|id| id.file_idx == app.selected_file)
        .collect();
    let hunk_pos = app
        .current_hunk()
        .and_then(|cur| file_hunks.iter().position(|id| *id == cur))
        .map(|i| format!(" · hunk {}/{}", i + 1, file_hunks.len()))
        .unwrap_or_default();
    let left = Span::styled(
        format!(" {tab_name} · file {file_pos}{hunk_pos} "),
        Style::new().fg(Color::Black).bg(Color::Gray),
    );

    let right = match app.message {
        Some(ref message) => Span::styled(format!(" {message} "), Style::new().fg(Color::Red)),
        None => Span::styled(hints(app), Style::new().fg(Color::DarkGray)),
    };
    Paragraph::new(Line::from(vec![left, right]))
}

fn hints(app: &App) -> &'static str {
    match (app.focus, app.tab) {
        (Focus::Files, _) => " Tab switch · j/k file · Enter diff · q quit ",
        (Focus::Diff, Tab::Unstaged) => " j/k line · n/p hunk · s stage · h back · q quit ",
        (Focus::Diff, Tab::Staged) => " j/k line · n/p hunk · u unstage · h back · q quit ",
    }
}

fn pane_block(title: String, focused: bool) -> Block<'static> {
    let border = if focused {
        Style::new().fg(Color::White)
    } else {
        Style::new().fg(Color::DarkGray)
    };
    Block::bordered().title(title).border_style(border)
}

fn status_badge(status: FileStatus) -> (&'static str, Color) {
    match status {
        FileStatus::Added => ("A", Color::Green),
        FileStatus::Deleted => ("D", Color::Red),
        FileStatus::Modified => ("M", Color::Yellow),
        FileStatus::Renamed => ("R", Color::Blue),
        FileStatus::TypeChange => ("T", Color::Magenta),
    }
}

fn style_for(kind: LineKind) -> Style {
    match kind {
        LineKind::FileHeader => Style::new().fg(Color::Yellow).add_modifier(Modifier::BOLD),
        LineKind::HunkHeader => Style::new().fg(Color::Cyan),
        LineKind::Addition => Style::new().fg(Color::Green),
        LineKind::Deletion => Style::new().fg(Color::Red),
        LineKind::Context => Style::new(),
        LineKind::Meta => Style::new().fg(Color::DarkGray),
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::diff::{DiffLine, DiffView, FileInfo};
    use ratatui::crossterm::event::{KeyCode, KeyEvent, KeyModifiers};
    use ratatui::{Terminal, backend::TestBackend};
    use std::path::PathBuf;

    fn sample_view() -> DiffView {
        let line =
            |kind: LineKind, file_idx: usize, hunk_idx: Option<usize>, content: &str| DiffLine {
                kind,
                content: content.into(),
                file_idx,
                hunk_idx,
            };
        DiffView {
            lines: vec![
                line(LineKind::FileHeader, 0, None, "diff --git a/a.txt b/a.txt"),
                line(LineKind::HunkHeader, 0, Some(0), "@@ -1 +1 @@"),
                line(LineKind::Addition, 0, Some(0), "alpha"),
                line(LineKind::FileHeader, 1, None, "diff --git a/b.txt b/b.txt"),
                line(LineKind::HunkHeader, 1, Some(0), "@@ -1 +1 @@"),
                line(LineKind::Addition, 1, Some(0), "beta1"),
                line(LineKind::HunkHeader, 1, Some(1), "@@ -5 +5 @@"),
                line(LineKind::Deletion, 1, Some(1), "beta2"),
            ],
            files: vec![
                FileInfo {
                    path: "a.txt".into(),
                    status: FileStatus::Modified,
                },
                FileInfo {
                    path: "b.txt".into(),
                    status: FileStatus::Modified,
                },
            ],
        }
    }

    fn sample_app() -> App {
        App::new(sample_view(), DiffView::default(), PathBuf::from("/unused"))
    }

    fn render_app(app: &mut App, width: u16, height: u16) -> (String, ratatui::buffer::Buffer) {
        let backend = TestBackend::new(width, height);
        let mut terminal = Terminal::new(backend).unwrap();
        terminal.draw(|frame| render(frame, app)).unwrap();
        let buffer = terminal.backend().buffer().clone();
        let screen = buffer.content().iter().map(|cell| cell.symbol()).collect();
        (screen, buffer)
    }

    fn press(app: &mut App, code: KeyCode) {
        app.handle_key(KeyEvent::new(code, KeyModifiers::NONE));
    }

    #[test]
    fn renders_file_list_and_selected_files_diff() {
        let mut app = sample_app();
        let (screen, _) = render_app(&mut app, 80, 10);
        assert!(screen.contains("Unstaged (2)"));
        assert!(screen.contains("M a.txt"));
        assert!(screen.contains("b.txt"));
        // Only the selected file's hunks appear in the diff pane.
        assert!(screen.contains("+alpha"));
        assert!(!screen.contains("beta1"));
        assert!(!screen.contains("beta2"));
    }

    #[test]
    fn moving_file_selection_switches_the_diff() {
        let mut app = sample_app();
        press(&mut app, KeyCode::Char('j'));
        let (screen, _) = render_app(&mut app, 80, 10);
        assert!(screen.contains("+beta1"));
        assert!(screen.contains("-beta2"));
        assert!(!screen.contains("+alpha"));
        assert!(screen.contains("file 2/2"));
    }

    #[test]
    fn cursor_line_is_highlighted_in_diff_focus() {
        let mut app = sample_app();
        press(&mut app, KeyCode::Enter);
        press(&mut app, KeyCode::Char('j'));
        let (screen, buffer) = render_app(&mut app, 80, 10);
        assert!(screen.contains("hunk 1/1"));
        // 30% of 80 = 24 columns for the files pane, +1 for the border: the
        // diff content starts at column 25. Row 1 is the @@ header, row 2
        // the addition the cursor is on.
        assert_eq!(buffer[(25, 2)].bg, Color::DarkGray);
        assert_ne!(buffer[(25, 1)].bg, Color::DarkGray);
    }

    #[test]
    fn tab_shows_the_staged_side() {
        let mut app = sample_app();
        press(&mut app, KeyCode::Tab);
        let (screen, _) = render_app(&mut app, 80, 10);
        assert!(screen.contains("Staged (0)"));
        assert!(screen.contains("no changes"));
        assert!(screen.contains("staged · file 0/0"));
    }
}
