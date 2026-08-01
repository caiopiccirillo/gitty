//! Rendering with ratatui: a file list on the left, the selected file's
//! diff with a line cursor on the right, and a status bar.

use ratatui::{
    Frame,
    layout::{Constraint, Layout, Rect},
    style::{Color, Modifier, Style},
    text::{Line, Span},
    widgets::{Block, HighlightSpacing, List, ListItem, Paragraph},
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

fn render_files(frame: &mut Frame, app: &mut App, area: Rect) {
    let focused = app.focus == Focus::Files;
    let (title, items) = {
        let diff = app.current_diff();
        let title = match app.tab {
            Tab::Unstaged => format!(" Unstaged ({}) ", diff.files.len()),
            Tab::Staged => format!(" Staged ({}) ", diff.files.len()),
        };
        let items: Vec<ListItem> = diff
            .files
            .iter()
            .map(|file| {
                let (letter, color) = status_badge(file.status);
                ListItem::new(Line::from(vec![
                    Span::styled(format!("{letter} "), Style::new().fg(color)),
                    Span::raw(file.path.clone()),
                ]))
            })
            .collect();
        (title, items)
    };
    let block = pane_block(title, focused);

    if items.is_empty() {
        let empty = Paragraph::new("no changes")
            .style(Style::new().fg(Color::DarkGray))
            .block(block);
        frame.render_widget(empty, area);
        return;
    }

    // Stateful rendering keeps the selection visible by scrolling the list.
    let list = List::new(items)
        .block(block)
        .highlight_style(Style::new().bg(Color::DarkGray))
        .highlight_symbol("")
        .highlight_spacing(HighlightSpacing::Never);
    frame.render_stateful_widget(list, area, &mut app.files_state);
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
            let in_selection = app.selection_range().is_some_and(|r| r.contains(&i));
            if in_selection || i == app.cursor {
                style = style.bg(Color::DarkGray);
                if line.kind == LineKind::Meta {
                    // DarkGray on DarkGray would be unreadable.
                    style = style.fg(Color::Gray);
                }
            }
            if i == app.cursor && app.visual_anchor.is_some() {
                // While selecting, the cursor end is a shade lighter.
                style = style.bg(Color::Gray);
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

    let right = if let Some(range) = app.selection_range() {
        let verb = match app.tab {
            Tab::Unstaged => "s stage",
            Tab::Staged => "u unstage",
        };
        Span::styled(
            format!(
                " visual: {} line(s) · {verb} lines · v/Esc cancel ",
                range.len()
            ),
            Style::new().fg(Color::Yellow),
        )
    } else {
        match app.message {
            Some(ref message) => Span::styled(format!(" {message} "), Style::new().fg(Color::Red)),
            None => Span::styled(hints(app), Style::new().fg(Color::DarkGray)),
        }
    };
    Paragraph::new(Line::from(vec![left, right]))
}

fn hints(app: &App) -> &'static str {
    match (app.focus, app.tab) {
        (Focus::Files, Tab::Unstaged) => " Tab · j/k file · space stage · Enter diff · q quit ",
        (Focus::Files, Tab::Staged) => " Tab · j/k file · space unstage · Enter diff · q quit ",
        (Focus::Diff, Tab::Unstaged) => {
            " j/k line · n/p hunk · v select · s stage · h back · q quit "
        }
        (Focus::Diff, Tab::Staged) => {
            " j/k line · n/p hunk · v select · u unstage · h back · q quit "
        }
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
        FileStatus::Untracked => ("?", Color::Green),
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
    fn file_list_scrolls_to_keep_the_selection_visible() {
        let view = DiffView {
            lines: (0..30)
                .map(|i| DiffLine {
                    kind: LineKind::FileHeader,
                    content: format!("diff --git a/f{i:02} b/f{i:02}"),
                    file_idx: i,
                    hunk_idx: None,
                })
                .collect(),
            files: (0..30)
                .map(|i| FileInfo {
                    path: format!("f{i:02}.txt"),
                    status: FileStatus::Modified,
                })
                .collect(),
        };
        let mut app = App::new(view, DiffView::default(), PathBuf::from("/unused"));
        for _ in 0..25 {
            press(&mut app, KeyCode::Char('j'));
        }
        assert_eq!(app.selected_file, 25);
        // Height 10 -> the list shows at most 7 rows, far from the top.
        let (screen, _) = render_app(&mut app, 80, 10);
        assert!(screen.contains("f25.txt"), "selected file stays visible");
        assert!(!screen.contains("f00.txt"), "top of the list scrolled away");
    }

    #[test]
    fn visual_selection_highlights_the_range() {
        let mut app = sample_app();
        press(&mut app, KeyCode::Enter);
        press(&mut app, KeyCode::Char('v'));
        press(&mut app, KeyCode::Char('j'));
        let (screen, buffer) = render_app(&mut app, 80, 10);
        assert!(screen.contains("visual: 2 line(s)"));
        // The anchored @@ line is part of the selection...
        assert_eq!(buffer[(25, 1)].bg, Color::DarkGray);
        // ...and the cursor end is lighter.
        assert_eq!(buffer[(25, 2)].bg, Color::Gray);
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
