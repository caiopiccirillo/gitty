//! Rendering with ratatui.

use ratatui::{
    Frame,
    layout::{Alignment, Constraint, Layout},
    style::{Color, Modifier, Style},
    text::{Line, Span},
    widgets::Paragraph,
};

use crate::app::App;
use crate::diff::{DiffLine, LineKind};

pub fn render(frame: &mut Frame, app: &mut App) {
    let [main_area, status_area] =
        Layout::vertical([Constraint::Min(1), Constraint::Length(1)]).areas(frame.area());
    app.set_viewport_height(main_area.height as usize);

    if app.diff.is_empty() {
        let message = Paragraph::new("No changes — working tree matches the index.")
            .style(Style::new().fg(Color::DarkGray))
            .alignment(Alignment::Center);
        frame.render_widget(message, main_area);
    } else {
        let lines: Vec<Line> = app
            .diff
            .lines
            .iter()
            .map(|line| {
                let prefix = match line.kind {
                    LineKind::Addition => "+",
                    LineKind::Deletion => "-",
                    LineKind::Context => " ",
                    _ => "",
                };
                // CR of CRLF files is kept in the model but must not reach
                // the terminal, where it would reset the cursor column.
                let text = format!("{prefix}{}", line.content.trim_end_matches('\r'));
                Line::styled(text, line_style(app, line))
            })
            .collect();
        let scroll = u16::try_from(app.scroll).unwrap_or(u16::MAX);
        frame.render_widget(Paragraph::new(lines).scroll((scroll, 0)), main_area);
    }

    frame.render_widget(status_bar(app), status_area);
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

/// Style of a line, highlighting it when it belongs to the selected hunk.
fn line_style(app: &App, line: &DiffLine) -> Style {
    let mut style = style_for(line.kind);
    let selected = app
        .selected_hunk
        .is_some_and(|id| id.file_idx == line.file_idx && line.hunk_idx == Some(id.hunk_idx));
    if selected {
        style = style.bg(Color::DarkGray);
        if line.kind == LineKind::Meta {
            // DarkGray on DarkGray would be unreadable.
            style = style.fg(Color::Gray);
        }
    }
    style
}

fn status_bar(app: &App) -> Paragraph<'static> {
    let position = if app.diff.is_empty() {
        "0/0".to_string()
    } else {
        format!("{}/{}", app.scroll + 1, app.diff.len())
    };
    let hunks = app.diff.hunks();
    let hunk_info = match app
        .selected_hunk
        .and_then(|selected| hunks.iter().position(|id| *id == selected))
    {
        Some(i) => format!(" · hunk {}/{}", i + 1, hunks.len()),
        None if !hunks.is_empty() => format!(" · {} hunk(s)", hunks.len()),
        None => String::new(),
    };
    let line = Line::from(vec![
        Span::styled(
            format!(
                " {} file(s){hunk_info} · line {position} ",
                app.diff.file_count
            ),
            Style::new().fg(Color::Black).bg(Color::Gray),
        ),
        Span::styled(
            " q quit · ↑/k ↓/j scroll · n/p hunk · PgUp/PgDn page · g/G top/bottom ",
            Style::new().fg(Color::DarkGray),
        ),
    ]);
    Paragraph::new(line)
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::diff::{DiffLine, DiffView};
    use ratatui::{Terminal, backend::TestBackend};

    fn render_to_string(app: &mut App, width: u16, height: u16) -> String {
        let backend = TestBackend::new(width, height);
        let mut terminal = Terminal::new(backend).unwrap();
        terminal.draw(|frame| render(frame, app)).unwrap();
        terminal
            .backend()
            .buffer()
            .content()
            .iter()
            .map(|cell| cell.symbol())
            .collect()
    }

    fn sample_app() -> App {
        let diff = DiffView {
            lines: vec![
                DiffLine {
                    kind: LineKind::FileHeader,
                    content: "diff --git a/f.txt b/f.txt".into(),
                    file_idx: 0,
                    hunk_idx: None,
                },
                DiffLine {
                    kind: LineKind::HunkHeader,
                    content: "@@ -0,0 +1 @@".into(),
                    file_idx: 0,
                    hunk_idx: Some(0),
                },
                DiffLine {
                    kind: LineKind::Addition,
                    content: "new line".into(),
                    file_idx: 0,
                    hunk_idx: Some(0),
                },
            ],
            file_count: 1,
        };
        App::new(diff)
    }

    #[test]
    fn renders_prefixed_lines_and_status_bar() {
        let mut app = sample_app();
        let screen = render_to_string(&mut app, 50, 6);
        assert!(screen.contains("diff --git a/f.txt b/f.txt"));
        assert!(screen.contains("@@ -0,0 +1 @@"));
        assert!(screen.contains("+new line"), "addition gets a + prefix");
        assert!(screen.contains("1 file(s) · 1 hunk(s) · line 1/3"));
    }

    #[test]
    fn highlights_the_selected_hunk() {
        use ratatui::crossterm::event::{KeyCode, KeyEvent, KeyModifiers};

        let mut app = sample_app();
        app.set_viewport_height(5);
        app.handle_key(KeyEvent::new(KeyCode::Char('n'), KeyModifiers::NONE));

        let backend = TestBackend::new(50, 6);
        let mut terminal = Terminal::new(backend).unwrap();
        terminal.draw(|frame| render(frame, &mut app)).unwrap();
        let buffer = terminal.backend().buffer();

        let screen: String = buffer.content().iter().map(|cell| cell.symbol()).collect();
        assert!(screen.contains("hunk 1/1"));
        // The hunk header (row 1) and the addition (row 2) are highlighted...
        for (x, y) in [(0, 1), (11, 1), (0, 2), (8, 2)] {
            assert_eq!(buffer[(x, y)].bg, Color::DarkGray, "cell ({x},{y})");
        }
        // ...but not the file header above the hunk.
        assert_ne!(buffer[(0, 0)].bg, Color::DarkGray);
    }

    #[test]
    fn renders_empty_diff_message() {
        let mut app = App::new(DiffView::default());
        let screen = render_to_string(&mut app, 50, 6);
        assert!(screen.contains("No changes"));
        assert!(screen.contains("0/0"));
    }
}
