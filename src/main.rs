use std::path::PathBuf;
use std::time::Duration;

use anyhow::Result;
use ratatui::DefaultTerminal;
use ratatui::crossterm::event::{self, Event, KeyEventKind};

use gitiff::app::App;
use gitiff::ui;

/// Idle-tick interval: how often the loop wakes to apply finished
/// background refreshes. Key events wake it immediately; the actual diff
/// computation happens off-thread (see `refresh.rs`).
const TICK: Duration = Duration::from_millis(250);

fn main() -> Result<()> {
    let path = PathBuf::from(std::env::args().nth(1).unwrap_or_else(|| ".".into()));
    let mut app = App::load(&path)?;

    let mut terminal = ratatui::init();
    let result = run(&mut terminal, &mut app);
    ratatui::restore();
    result
}

fn run(terminal: &mut DefaultTerminal, app: &mut App) -> Result<()> {
    while !app.should_quit {
        terminal.draw(|frame| ui::render(frame, app))?;
        if event::poll(TICK)?
            && let Event::Key(key) = event::read()?
            && key.kind == KeyEventKind::Press
        {
            app.handle_key(key);
        }
        app.poll_refresh();
    }
    Ok(())
}
