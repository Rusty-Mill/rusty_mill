//! Raw-mode/alternate-screen setup and teardown.
//!
//! A panic hook is installed alongside enabling raw mode, not after:
//! a panic while the terminal is in raw mode with no hook leaves the
//! user's shell broken (no echo, no line discipline) with the panic
//! message itself invisible until they blindly type `reset` -- worse
//! than losing the message.

use std::io::Stdout;

use crossterm::execute;
use crossterm::terminal::{EnterAlternateScreen, LeaveAlternateScreen};
use ratatui::backend::CrosstermBackend;
use ratatui::Terminal;

use crate::error::{Error, Result};

pub type Backend = CrosstermBackend<Stdout>;

pub fn enter() -> Result<Terminal<Backend>> {
    crossterm::terminal::enable_raw_mode().map_err(|e| Error::io("enabling raw mode", e))?;
    execute!(std::io::stdout(), EnterAlternateScreen)
        .map_err(|e| Error::io("entering the alternate screen", e))?;

    // Restore the terminal before the default panic hook prints anything,
    // so a panic's message and backtrace land on a normal, readable
    // screen instead of scrolling through raw-mode's line discipline (or
    // being overwritten by whatever redraws next).
    let default_hook = std::panic::take_hook();
    std::panic::set_hook(Box::new(move |info| {
        let _ = crossterm::terminal::disable_raw_mode();
        let _ = execute!(std::io::stdout(), LeaveAlternateScreen);
        default_hook(info);
    }));

    Terminal::new(CrosstermBackend::new(std::io::stdout()))
        .map_err(|e| Error::io("initializing the terminal backend", e))
}

pub fn leave(mut term: Terminal<Backend>) -> Result<()> {
    crossterm::terminal::disable_raw_mode().map_err(|e| Error::io("disabling raw mode", e))?;
    execute!(term.backend_mut(), LeaveAlternateScreen)
        .map_err(|e| Error::io("leaving the alternate screen", e))?;
    term.show_cursor()
        .map_err(|e| Error::io("restoring the cursor", e))
}
