use std::io;

use color_eyre::Result;
use crossterm::event::{self, Event, KeyCode, KeyEventKind};
use crossterm::terminal::{
    disable_raw_mode, enable_raw_mode, EnterAlternateScreen, LeaveAlternateScreen,
};
use crossterm::ExecutableCommand;
use ratatui::prelude::*;

use metrix::data;
use metrix::ui::{self, App};

fn main() -> Result<()> {
    color_eyre::install()?;

    // Collect and parse data
    let files = data::collect_jsonl_files();
    let metrics = data::parse_all(&files);

    let mut app = App::new(metrics);

    // Terminal setup
    enable_raw_mode()?;
    io::stdout().execute(EnterAlternateScreen)?;
    let mut terminal = Terminal::new(CrosstermBackend::new(io::stdout()))?;

    // Scroll to show most recent data
    let initial_width = terminal.size()?.width;
    let visible = app.visible_bars_for_terminal(initial_width);
    app.scroll_to_end(visible);

    // Event loop
    loop {
        terminal.draw(|frame| ui::render(frame, &app))?;

        if event::poll(std::time::Duration::from_millis(100))? {
            if let Event::Key(key) = event::read()? {
                if key.kind != KeyEventKind::Press {
                    continue;
                }
                let visible = app.visible_bars_for_terminal(terminal.size()?.width);
                match key.code {
                    KeyCode::Char('q') | KeyCode::Esc => break,
                    KeyCode::Right | KeyCode::Char('l') => app.scroll_right(visible),
                    KeyCode::Left | KeyCode::Char('h') => app.scroll_left(visible),
                    KeyCode::Home => app.scroll_offset = 0,
                    KeyCode::End => app.scroll_to_end(visible),
                    _ => {}
                }
            }
        }
    }

    // Cleanup
    disable_raw_mode()?;
    io::stdout().execute(LeaveAlternateScreen)?;

    Ok(())
}
