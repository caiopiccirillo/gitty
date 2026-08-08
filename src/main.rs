use std::path::PathBuf;
use std::time::Duration;

use anyhow::Result;
use ratatui::DefaultTerminal;
use ratatui::crossterm::event::{self, Event, KeyEventKind};

use gitty::app::App;
use gitty::ui;

/// Idle-tick interval: how often the loop wakes to apply finished
/// background refreshes. Key events wake it immediately; the actual diff
/// computation happens off-thread (see `refresh.rs`).
const TICK: Duration = Duration::from_millis(250);

fn main() -> Result<()> {
    let path = PathBuf::from(std::env::args().nth(1).unwrap_or_else(|| ".".into()));
    let mut app = App::load(&path)?;

    let mut terminal = ratatui::init();
    ratatui::crossterm::execute!(
        std::io::stdout(),
        ratatui::crossterm::event::EnableMouseCapture
    )?;
    let result = run(&mut terminal, &mut app);
    let _ = ratatui::crossterm::execute!(
        std::io::stdout(),
        ratatui::crossterm::event::DisableMouseCapture
    );
    ratatui::restore();
    result
}

fn run(terminal: &mut DefaultTerminal, app: &mut App) -> Result<()> {
    while !app.should_quit {
        terminal.draw(|frame| ui::render(frame, app))?;
        if event::poll(TICK)? {
            match event::read()? {
                Event::Key(key) if key.kind == KeyEventKind::Press => app.handle_key(key),
                Event::Mouse(mouse) => app.handle_mouse(mouse),
                _ => {}
            }
        }
        app.poll_refresh();
    }
    Ok(())
}
