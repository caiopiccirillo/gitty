//! Rendering with ratatui: a file list on the left, the selected file's
//! diff with a line cursor on the right, and a status bar.

use ratatui::{
    Frame,
    layout::{Constraint, Layout, Rect},
    style::{Color, Modifier, Style},
    text::{Line, Span},
    widgets::{Block, Clear, HighlightSpacing, List, ListItem, Paragraph},
};

use crate::app::{App, CommitInput, Focus, Mode, Side};
use crate::diff::{FileInfo, FileStatus, HunkId, LineKind};
use crate::tree::Node;

pub fn render(frame: &mut Frame, app: &mut App) {
    let [main_area, status_area] =
        Layout::vertical([Constraint::Min(1), Constraint::Length(1)]).areas(frame.area());
    match app.mode {
        Mode::Classic => {
            let [files_area, diff_area] =
                Layout::horizontal([Constraint::Percentage(30), Constraint::Percentage(70)])
                    .areas(main_area);
            app.set_viewport_height(diff_area.height.saturating_sub(2) as usize);
            app.files_rect = files_area;
            app.diff_rects = [None; 2];
            app.diff_rects[app.side.index()] = Some(diff_area);
            render_files(frame, app, files_area);
            render_diff(frame, app, diff_area, app.side);
        }
        Mode::Split => {
            let has_unstaged = !app.unstaged.files.is_empty();
            let has_staged = !app.staged.files.is_empty();
            app.diff_rects = [None; 2];
            match (has_unstaged, has_staged) {
                // The unstaged pane sits between the files and the staged
                // pane, mirroring the stage workflow left to right.
                (true, true) => {
                    let [files_area, unstaged_area, staged_area] = Layout::horizontal([
                        Constraint::Percentage(25),
                        Constraint::Percentage(37),
                        Constraint::Percentage(38),
                    ])
                    .areas(main_area);
                    app.set_viewport_height(unstaged_area.height.saturating_sub(2) as usize);
                    app.files_rect = files_area;
                    app.diff_rects[Side::Unstaged.index()] = Some(unstaged_area);
                    app.diff_rects[Side::Staged.index()] = Some(staged_area);
                    render_files(frame, app, files_area);
                    render_diff(frame, app, unstaged_area, Side::Unstaged);
                    render_diff(frame, app, staged_area, Side::Staged);
                }
                // Panes without content are hidden entirely.
                (true, false) => {
                    render_split_side(frame, app, main_area, Side::Unstaged);
                }
                (false, true) => {
                    render_split_side(frame, app, main_area, Side::Staged);
                }
                (false, false) => {
                    app.set_viewport_height(0);
                    app.files_rect = main_area;
                    render_files(frame, app, main_area);
                }
            }
        }
    }
    frame.render_widget(status_bar(app), status_area);
    if let Some(ref input) = app.commit_input {
        render_commit_box(frame, input);
    }
}

/// The files pane plus one diff pane, filling the whole width.
fn render_split_side(frame: &mut Frame, app: &mut App, main_area: Rect, side: Side) {
    let [files_area, diff_area] =
        Layout::horizontal([Constraint::Percentage(30), Constraint::Percentage(70)])
            .areas(main_area);
    app.set_viewport_height(diff_area.height.saturating_sub(2) as usize);
    app.files_rect = files_area;
    app.diff_rects[side.index()] = Some(diff_area);
    render_files(frame, app, files_area);
    render_diff(frame, app, diff_area, side);
}

/// The centered commit-message box, with the terminal cursor inside it.
fn render_commit_box(frame: &mut Frame, input: &CommitInput) {
    let area = frame.area();
    let width = 60.min(area.width);
    let height = 3.min(area.height);
    let rect = Rect::new(
        area.x + area.width.saturating_sub(width) / 2,
        area.y + area.height.saturating_sub(height) / 2,
        width,
        height,
    );
    frame.render_widget(Clear, rect);
    let block = Block::bordered().title(" Commit message ");
    let inner = block.inner(rect);
    frame.render_widget(block, rect);

    // Scroll the text horizontally so the cursor stays visible.
    let visible = (inner.width as usize).max(1);
    let chars: Vec<char> = input.text.chars().collect();
    let cursor = input.cursor_chars();
    let start = (cursor + 1).saturating_sub(visible);
    let text: String = chars.iter().skip(start).take(visible).collect();
    frame.render_widget(Paragraph::new(text), inner);
    frame.set_cursor_position((inner.x + (cursor - start) as u16, inner.y));
}

fn render_files(frame: &mut Frame, app: &mut App, area: Rect) {
    let focused = app.focus == Focus::Files;
    let (title, items) = {
        let title = match app.mode {
            Mode::Classic => match app.side {
                Side::Unstaged => format!(" Unstaged ({}) ", app.entries.len()),
                Side::Staged => format!(" Staged ({}) ", app.entries.len()),
            },
            Mode::Split => format!(" Files ({}) ", app.entries.len()),
        };
        let items: Vec<ListItem> = app
            .tree
            .iter()
            .map(|node| match node {
                Node::Dir {
                    name,
                    depth,
                    collapsed,
                    file_count,
                    ..
                } => {
                    let (arrow, suffix) = if *collapsed {
                        ("▸", format!(" ({file_count})"))
                    } else {
                        ("▾", String::new())
                    };
                    ListItem::new(Line::from(vec![
                        Span::raw(format!("{}{arrow} ", "  ".repeat(*depth))),
                        Span::styled(
                            format!("{name}/{suffix}"),
                            Style::new().add_modifier(Modifier::BOLD),
                        ),
                    ]))
                }
                Node::File { file_idx, depth } => {
                    let entry = &app.entries[*file_idx];
                    let name = entry.path.rsplit('/').next().unwrap_or("").to_string();
                    let mut spans = vec![Span::raw("  ".repeat(*depth))];
                    match app.mode {
                        Mode::Classic => {
                            // The badge reflects this row's file on the
                            // focused side, not the selected row.
                            let file = match app.side {
                                Side::Unstaged => {
                                    entry.unstaged.and_then(|i| app.unstaged.files.get(i))
                                }
                                Side::Staged => entry.staged.and_then(|i| app.staged.files.get(i)),
                            };
                            if let Some(file) = file {
                                let (letter, color) = status_badge(file.status);
                                spans.push(Span::styled(
                                    format!("{letter} "),
                                    Style::new().fg(color),
                                ));
                            }
                        }
                        Mode::Split => {
                            let staged = entry.staged.and_then(|i| app.staged.files.get(i));
                            let unstaged = entry.unstaged.and_then(|i| app.unstaged.files.get(i));
                            spans.extend(merge_badge(staged, unstaged));
                        }
                    }
                    spans.push(Span::raw(name));
                    ListItem::new(Line::from(spans))
                }
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

fn render_diff(frame: &mut Frame, app: &App, area: Rect, side: Side) {
    let title = match app.mode {
        Mode::Classic => match app.selected_node() {
            Some(Node::File { .. }) => app
                .selected_file_index_in(side)
                .and_then(|idx| app.diff_of(side).files.get(idx))
                .map(|file| format!(" {} ", file.path))
                .unwrap_or_else(|| " diff ".into()),
            Some(Node::Dir { path, .. }) => format!(" {path}/ "),
            None => " diff ".into(),
        },
        Mode::Split => match side {
            Side::Staged => format!(" Staged ({}) ", app.staged.files.len()),
            Side::Unstaged => format!(" Unstaged ({}) ", app.unstaged.files.len()),
        },
    };
    let focused = app.focus == Focus::Diff && app.side == side;
    let block = pane_block(title, focused);

    let pane = app.pane_of(side);
    let selection = app.selection_range_for(side);
    #[cfg(feature = "syntax")]
    let file_path = app
        .selected_file_index_in(side)
        .and_then(|i| app.diff_of(side).files.get(i))
        .map(|f| f.path.clone());
    let lines: Vec<Line> = app
        .display_lines_for(side)
        .iter()
        .enumerate()
        .map(|(i, line)| {
            let content = line.content.trim_end_matches('\r');
            let line_style = style_for(line.kind);
            let selected = selection.as_ref().is_some_and(|r| r.contains(&i)) || i == pane.cursor;
            let mut spans: Vec<Span> = Vec::new();
            let mut push = |text: &str, mut style: Style| {
                if selected {
                    style = style.bg(Color::DarkGray);
                    if line.kind == LineKind::Meta {
                        // DarkGray on DarkGray would be unreadable.
                        style = style.fg(Color::Gray);
                    }
                }
                if i == pane.cursor && pane.visual_anchor.is_some() {
                    // While selecting, the cursor end is a shade lighter.
                    style = style.bg(Color::Gray);
                }
                spans.push(Span::styled(text.to_string(), style));
            };
            let prefix = match line.kind {
                LineKind::Addition => "+",
                LineKind::Deletion => "-",
                LineKind::Context => " ",
                _ => "",
            };
            push(prefix, line_style);
            #[cfg(feature = "syntax")]
            {
                match file_path.as_deref().and_then(crate::syntax::language_of) {
                    Some(language)
                        if matches!(
                            line.kind,
                            LineKind::Context | LineKind::Addition | LineKind::Deletion
                        ) =>
                    {
                        // Color the code tokens, keeping the base style for
                        // everything between them.
                        let mut covered = 0;
                        for (start, end, color) in crate::syntax::highlight(language, content) {
                            if start > covered {
                                push(&content[covered..start], line_style);
                            }
                            push(&content[start..end], line_style.fg(color));
                            covered = end;
                        }
                        if covered < content.len() {
                            push(&content[covered..], line_style);
                        }
                    }
                    _ => push(content, line_style),
                }
            }
            #[cfg(not(feature = "syntax"))]
            push(content, line_style);
            Line::from(spans)
        })
        .collect();

    if lines.is_empty() {
        let empty = Paragraph::new("no changes")
            .style(Style::new().fg(Color::DarkGray))
            .block(block);
        frame.render_widget(empty, area);
        return;
    }
    let scroll = u16::try_from(app.pane_of(side).scroll).unwrap_or(u16::MAX);
    let diff = Paragraph::new(lines).block(block).scroll((scroll, 0));
    frame.render_widget(diff, area);
}

fn status_bar(app: &App) -> Paragraph<'static> {
    let side_name = match app.side {
        Side::Unstaged => "unstaged",
        Side::Staged => "staged",
    };
    let selection = match app.selected_node() {
        Some(Node::File { file_idx, .. }) => {
            format!("file {}/{}", file_idx + 1, app.entries.len())
        }
        Some(Node::Dir { file_count, .. }) => format!("dir ({file_count} file(s))"),
        None => "file 0/0".to_string(),
    };
    // Hunks currently visible in the diff pane.
    let mut displayed: Vec<HunkId> = Vec::new();
    for line in app.display_lines() {
        if let Some(hunk_idx) = line.hunk_idx {
            let id = HunkId {
                file_idx: line.file_idx,
                hunk_idx,
            };
            if displayed.last() != Some(&id) {
                displayed.push(id);
            }
        }
    }
    let hunk_pos = app
        .current_hunk()
        .and_then(|cur| displayed.iter().position(|id| *id == cur))
        .map(|i| format!(" · hunk {}/{}", i + 1, displayed.len()))
        .unwrap_or_default();
    let left = Span::styled(
        format!(" {side_name} · {selection}{hunk_pos} "),
        Style::new().fg(Color::Black).bg(Color::Gray),
    );

    let right = if app.commit_input.is_some() {
        Span::styled(
            " Enter commit · Esc cancel ",
            Style::new().fg(Color::Yellow),
        )
    } else if let Some(prompt) = &app.discard_confirm {
        Span::styled(
            format!(" discard {}? y confirm · n cancel ", prompt.what),
            Style::new().fg(Color::Yellow),
        )
    } else if let Some(range) = app.selection_range() {
        let verb = match app.side {
            Side::Unstaged => "s stage",
            Side::Staged => "u unstage",
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
    match (app.focus, app.side) {
        (Focus::Files, Side::Unstaged) => {
            " Tab · j/k · space stage · d discard · h/l · m layout · c commit · q quit "
        }
        (Focus::Files, Side::Staged) => {
            " Tab · j/k · space unstage · d discard · h/l · m layout · c commit · q quit "
        }
        (Focus::Diff, Side::Unstaged) => {
            " j/k change · n/p hunk · v select · s stage · d discard · m layout · c commit · q quit "
        }
        (Focus::Diff, Side::Staged) => {
            " j/k change · n/p hunk · v select · u unstage · d discard · m layout · c commit · q quit "
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

/// The lazygit-style two-letter badge of a merged file: staged status first,
/// unstaged second (`MM`, ` M`, `A?`, ...). Untracked files show `??`.
fn merge_badge(staged: Option<&FileInfo>, unstaged: Option<&FileInfo>) -> Vec<Span<'static>> {
    let staged_letter = staged.map(|f| status_badge(f.status).0);
    let unstaged_letter = unstaged.map(|f| status_badge(f.status).0);
    let (staged, unstaged) = match (staged_letter, unstaged_letter) {
        (None, Some("?")) => ("?", "?"),
        (s, u) => (s.unwrap_or(" "), u.unwrap_or(" ")),
    };
    vec![
        Span::styled(staged.to_string(), Style::new().fg(Color::Green)),
        Span::styled(format!("{unstaged} "), Style::new().fg(Color::Red)),
    ]
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
    use crate::app::{DiscardAction, DiscardPrompt};
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
                line(LineKind::HunkHeader, 1, Some(0), "@@ -1,2 +1,2 @@"),
                line(LineKind::Addition, 1, Some(0), "beta1"),
                line(LineKind::Deletion, 1, Some(0), "beta2"),
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
    fn file_rows_show_their_own_status_badge() {
        let view = DiffView {
            lines: Vec::new(),
            files: vec![
                FileInfo {
                    path: "a.txt".into(),
                    status: FileStatus::Added,
                },
                FileInfo {
                    path: "b.txt".into(),
                    status: FileStatus::Modified,
                },
            ],
        };
        let mut app = App::new(view, DiffView::default(), PathBuf::from("/unused"));
        let (screen, _) = render_app(&mut app, 80, 10);
        assert!(screen.contains("A a.txt"));
        assert!(screen.contains("M b.txt"));

        // Moving the selection must not rewrite the badges of the rows.
        press(&mut app, KeyCode::Char('j'));
        let (screen, _) = render_app(&mut app, 80, 10);
        assert!(screen.contains("A a.txt"));
        assert!(screen.contains("M b.txt"));
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
        assert_eq!(app.selected_row, 25);
        // Height 10 -> the list shows at most 7 rows, far from the top.
        let (screen, _) = render_app(&mut app, 80, 10);
        assert!(screen.contains("f25.txt"), "selected file stays visible");
        assert!(!screen.contains("f00.txt"), "top of the list scrolled away");
    }

    #[test]
    fn visual_selection_highlights_the_range() {
        let mut app = sample_app();
        // File b displays: [@@, +beta1, -beta2]; select both changes.
        press(&mut app, KeyCode::Char('j'));
        press(&mut app, KeyCode::Enter);
        press(&mut app, KeyCode::Char('v'));
        press(&mut app, KeyCode::Char('j'));
        let (screen, buffer) = render_app(&mut app, 80, 10);
        assert!(screen.contains("visual: 2 line(s)"));
        // The anchored changed line is part of the selection...
        assert_eq!(buffer[(25, 2)].bg, Color::DarkGray);
        // ...and the cursor end is lighter.
        assert_eq!(buffer[(25, 3)].bg, Color::Gray);
    }

    #[test]
    fn status_bar_asks_before_a_discard() {
        let mut app = sample_app();
        app.discard_confirm = Some(DiscardPrompt {
            what: "hunk 1 of a.txt".into(),
            action: DiscardAction::Hunk {
                hunk: crate::diff::HunkId {
                    file_idx: 0,
                    hunk_idx: 0,
                },
                side: Side::Unstaged,
            },
        });
        let (screen, _) = render_app(&mut app, 80, 10);
        assert!(screen.contains("discard hunk 1 of a.txt?"));
        assert!(screen.contains("y confirm"));
        assert!(screen.contains("n cancel"));
    }

    #[test]
    fn mouse_events_work_end_to_end() {
        use ratatui::crossterm::event::{MouseButton, MouseEvent, MouseEventKind};
        use ratatui::layout::Position;

        let mut app = sample_app();
        render_app(&mut app, 80, 10); // establishes the clickable areas

        // Click the second file row (below the border).
        app.handle_mouse(MouseEvent {
            kind: MouseEventKind::Down(MouseButton::Left),
            column: 5,
            row: 2,
            modifiers: KeyModifiers::NONE,
        });
        assert_eq!(app.selected_row, 1);
        let (screen, _) = render_app(&mut app, 80, 10);
        assert!(screen.contains("+beta1"), "b.txt diff shown");

        // Click a diff line: the cursor snaps to the changed line there.
        // b.txt displays [@@, +beta1, -beta2]; row 4 is the -beta2 line.
        app.handle_mouse(MouseEvent {
            kind: MouseEventKind::Down(MouseButton::Left),
            column: 60,
            row: 4,
            modifiers: KeyModifiers::NONE,
        });
        assert_eq!(app.focus, Focus::Diff);
        assert_eq!(app.cursor(), 2);
        assert_eq!(app.side, Side::Unstaged);
        assert!(Position::new(60, 4).y > 0);
    }

    #[cfg(feature = "syntax")]
    #[test]
    fn syntax_colors_diff_lines() {
        let line =
            |kind: LineKind, file_idx: usize, hunk_idx: Option<usize>, content: &str| DiffLine {
                kind,
                content: content.into(),
                file_idx,
                hunk_idx,
            };
        let view = DiffView {
            lines: vec![
                line(
                    LineKind::FileHeader,
                    0,
                    None,
                    "diff --git a/main.rs b/main.rs",
                ),
                line(LineKind::HunkHeader, 0, Some(0), "@@ -1 +1 @@"),
                line(LineKind::Addition, 0, Some(0), "let x = 5;"),
            ],
            files: vec![FileInfo {
                path: "main.rs".into(),
                status: FileStatus::Modified,
            }],
        };
        let mut app = App::new(view, DiffView::default(), PathBuf::from("/unused"));
        let (_, buffer) = render_app(&mut app, 80, 10);
        // The addition line is the third row (border, @@, then the line).
        let row = 2u16;
        let text: String = (0..80)
            .map(|x| buffer[(x, row)].symbol().to_string())
            .collect();
        assert!(text.contains("+let x = 5;"));
        let has_fg = |color: Color| (0..80).any(|x| buffer[(x, row)].style().fg == Some(color));
        assert!(has_fg(Color::Cyan), "the `let` keyword is cyan");
        assert!(has_fg(Color::Magenta), "the `5` number is magenta");
    }

    #[test]
    fn split_layout_renders_both_panes_side_by_side() {
        let mut app = App::new(sample_view(), sample_view(), PathBuf::from("/unused"));
        app.toggle_mode();
        app.set_viewport_height(8);
        let (screen, _) = render_app(&mut app, 120, 10);
        // The files pane is shared and merged; the diff panes are labeled.
        assert!(screen.contains("Files (2)"));
        assert!(screen.contains("Staged (2)"));
        assert!(screen.contains("Unstaged (2)"));
        // The unstaged pane sits between the files and the staged pane.
        // Column of a label in the flattened screen (border glyphs are
        // multi-byte, so positions must be counted in characters).
        let col_of = |needle: &str| {
            let byte = screen.find(needle).unwrap();
            screen[..byte].chars().count() % 120
        };
        assert!(col_of("Files (2)") < col_of("Unstaged ("));
        assert!(col_of("Unstaged (") < col_of("Staged (2)"));
        // Both panes show the selected file's diff (a.txt, one addition).
        assert!(screen.contains("+alpha"));
        // Merged rows carry the two-letter badge (staged M + unstaged M).
        assert!(screen.contains("MM"));
    }

    #[test]
    fn split_layout_hides_panes_without_content() {
        // Only unstaged changes: the staged pane is hidden.
        let mut app = App::new(sample_view(), DiffView::default(), PathBuf::from("/unused"));
        app.toggle_mode();
        app.set_viewport_height(8);
        let (screen, _) = render_app(&mut app, 120, 10);
        assert!(screen.contains("Unstaged (2)"));
        assert!(
            !screen.contains("Staged ("),
            "staged pane hidden when empty"
        );

        // Only staged changes: the unstaged pane is hidden.
        let mut app = App::new(DiffView::default(), sample_view(), PathBuf::from("/unused"));
        app.toggle_mode();
        let (screen, _) = render_app(&mut app, 120, 10);
        assert!(screen.contains("Staged (2)"));
        assert!(
            !screen.contains("Unstaged ("),
            "unstaged pane hidden when empty"
        );

        // Neither side has changes: only the files pane remains.
        let mut app = App::new(
            DiffView::default(),
            DiffView::default(),
            PathBuf::from("/unused"),
        );
        app.toggle_mode();
        let (screen, _) = render_app(&mut app, 120, 10);
        assert!(screen.contains("no changes"));
        assert!(!screen.contains("Staged ("));
        assert!(!screen.contains("Unstaged ("));
    }

    /// A staged view with one added file, distinct from the sample files.
    fn staged_x_view() -> DiffView {
        let line =
            |kind: LineKind, file_idx: usize, hunk_idx: Option<usize>, content: &str| DiffLine {
                kind,
                content: content.into(),
                file_idx,
                hunk_idx,
            };
        DiffView {
            lines: vec![
                line(LineKind::FileHeader, 0, None, "diff --git a/x.txt b/x.txt"),
                line(LineKind::HunkHeader, 0, Some(0), "@@ -1 +1 @@"),
                line(LineKind::Addition, 0, Some(0), "extra"),
            ],
            files: vec![FileInfo {
                path: "x.txt".into(),
                status: FileStatus::Added,
            }],
        }
    }

    #[test]
    fn split_layout_shows_no_changes_for_side_less_files() {
        let mut app = App::new(sample_view(), staged_x_view(), PathBuf::from("/unused"));
        app.toggle_mode();
        app.set_viewport_height(8);
        let (screen, _) = render_app(&mut app, 120, 10);
        // Both panes are visible, but the selected file (a.txt) has no
        // staged changes: the staged pane shows an empty state.
        assert!(screen.contains("Staged (1)"));
        assert!(screen.contains("Unstaged (2)"));
        assert!(screen.contains("no changes"));
        assert!(screen.contains("+alpha"));
    }

    #[test]
    fn renders_and_collapses_directory_rows() {
        let line =
            |kind: LineKind, file_idx: usize, hunk_idx: Option<usize>, content: &str| DiffLine {
                kind,
                content: content.into(),
                file_idx,
                hunk_idx,
            };
        let view = DiffView {
            lines: vec![
                line(
                    LineKind::FileHeader,
                    0,
                    None,
                    "diff --git a/src/app.rs b/src/app.rs",
                ),
                line(LineKind::HunkHeader, 0, Some(0), "@@ -1 +1 @@"),
                line(LineKind::Addition, 0, Some(0), "one"),
                line(
                    LineKind::FileHeader,
                    1,
                    None,
                    "diff --git a/src/lib.rs b/src/lib.rs",
                ),
                line(LineKind::HunkHeader, 1, Some(0), "@@ -1 +1 @@"),
                line(LineKind::Addition, 1, Some(0), "two"),
            ],
            files: ["src/app.rs", "src/lib.rs"]
                .into_iter()
                .map(|path| FileInfo {
                    path: path.into(),
                    status: FileStatus::Modified,
                })
                .collect(),
        };
        let mut app = App::new(view, DiffView::default(), PathBuf::from("/unused"));

        let (screen, _) = render_app(&mut app, 80, 10);
        assert!(screen.contains("▾ src/"));
        assert!(screen.contains("app.rs"));
        assert!(screen.contains("lib.rs"));
        // A directory selection aggregates both file diffs (headers kept).
        assert!(screen.contains("diff --git a/src/app.rs b/src/app.rs"));
        assert!(screen.contains("+one"));
        assert!(screen.contains("+two"));
        assert!(screen.contains("dir (2 file(s))"));

        press(&mut app, KeyCode::Enter);
        let (screen, _) = render_app(&mut app, 80, 10);
        assert!(screen.contains("▸ src/ (2)"));
        assert!(
            !app.tree.iter().any(|n| matches!(n, Node::File { .. })),
            "file rows hidden after collapse"
        );
    }

    #[test]
    fn renders_the_commit_box() {
        let mut app = sample_app();
        app.commit_input = Some(CommitInput {
            text: "my message".into(),
            cursor: 2,
        });
        let (screen, _) = render_app(&mut app, 80, 10);
        assert!(screen.contains("Commit message"));
        assert!(screen.contains("my message"));
        assert!(screen.contains("Enter commit"));
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
