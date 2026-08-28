use std::path::{Path, PathBuf};
use std::time::Duration;

use anyhow::Result;
use ratatui::DefaultTerminal;
use ratatui::crossterm::event::{
    self, DisableMouseCapture, EnableMouseCapture, Event, KeyEventKind,
};

use gitty::app::App;
use gitty::ui;

/// Idle-tick interval: how often the loop wakes to apply finished
/// background refreshes and expire status messages. Key events wake it
/// immediately; the actual diff computation happens off-thread (see
/// `refresh.rs`).
const TICK: Duration = Duration::from_millis(250);

fn main() -> Result<()> {
    let path = PathBuf::from(std::env::args().nth(1).unwrap_or_else(|| ".".into()));

    let mut terminal = ratatui::init();
    // Every exit path has to pass through the teardown below. A `?` inside
    // `session` (a bad path, an unreadable repository) would otherwise
    // leave the shell in raw mode, on the alternate screen, with mouse
    // capture still on: ratatui restores nothing on drop.
    let result = session(&mut terminal, &path);
    let _ = ratatui::crossterm::execute!(std::io::stdout(), DisableMouseCapture);
    ratatui::restore();
    result
}

/// Everything that runs with the terminal in raw mode.
fn session(terminal: &mut DefaultTerminal, path: &Path) -> Result<()> {
    ratatui::crossterm::execute!(std::io::stdout(), EnableMouseCapture)?;
    // The initial diff load can take a while on large repositories; show
    // a frame immediately so the user gets feedback instead of a blank
    // terminal.
    terminal.draw(ui::render_loading)?;
    let mut app = App::load(path)?;
    run(terminal, &mut app)
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
        app.expire_message();
        app.poll_refresh();
    }
    Ok(())
}
