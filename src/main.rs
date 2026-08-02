use std::path::PathBuf;
use std::time::Duration;

use anyhow::Result;
use ratatui::DefaultTerminal;
use ratatui::crossterm::event::{self, Event, KeyEventKind};

use gitiff::app::App;
use gitiff::ui;

/// How often the diffs are reloaded when the user is idle. git2 hashing the
/// workdir is cheap for typical repositories; the refresh is a no-op when
/// nothing changed, so this only costs a diff computation per tick.
const REFRESH_INTERVAL: Duration = Duration::from_millis(500);

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
        if event::poll(REFRESH_INTERVAL)? {
            if let Event::Key(key) = event::read()?
                && key.kind == KeyEventKind::Press
            {
                app.handle_key(key);
            }
        } else {
            app.auto_refresh();
        }
    }
    Ok(())
}
