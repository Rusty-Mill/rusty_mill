//! `Terminal` impl over `sys::console` (extraction map D9), plus
//! `ConsoleAcquisition` (the `rusty_naner` facet).

use std::time::Duration;

use platform::error::Result;
use platform::term::{ConsoleAcquisition, ConsoleState, TermStream, Terminal, WinSize};

use crate::sys::console;

/// The Windows terminal, over the process's std handles. Raw-mode state
/// (the saved console modes) lives here for correct, idempotent
/// enter/leave pairing. `console_state` tracks which console-acquisition
/// personality this handle last put the process into — instance state,
/// not a live OS probe, the same discipline `saved` already uses for
/// raw mode.
pub struct WindowsTerminal {
    saved: Option<console::SavedModes>,
    console_state: ConsoleState,
}

impl Default for WindowsTerminal {
    fn default() -> Self {
        Self {
            saved: None,
            console_state: console::initial_state(),
        }
    }
}

impl WindowsTerminal {
    pub fn new() -> Self {
        Self::default()
    }
}

impl Terminal for WindowsTerminal {
    fn is_tty(&self, stream: TermStream) -> bool {
        console::is_tty(stream)
    }

    fn window_size(&self) -> Result<WinSize> {
        let (rows, cols) = console::window_size()?;
        Ok(WinSize { rows, cols })
    }

    fn enter_raw(&mut self) -> Result<()> {
        if self.saved.is_some() {
            return Ok(());
        }
        self.saved = Some(console::enter_raw()?);
        Ok(())
    }

    fn leave_raw(&mut self) -> Result<()> {
        if let Some(saved) = self.saved.take() {
            console::restore(&saved)?;
        }
        Ok(())
    }

    fn is_raw(&self) -> bool {
        console::is_raw()
    }

    fn poll_readable(&self, timeout: Option<Duration>) -> Result<bool> {
        console::poll_readable(timeout)
    }

    fn read_chunk(&self, buf: &mut [u8]) -> Result<usize> {
        console::read_chunk(buf)
    }

    fn set_echo(&mut self, on: bool) -> Result<bool> {
        console::set_echo(on)
    }
}

impl ConsoleAcquisition for WindowsTerminal {
    fn console_state(&self) -> ConsoleState {
        self.console_state
    }

    fn alloc_console(&mut self) -> Result<()> {
        console::alloc()?;
        console::reopen_std_handles()?;
        self.console_state = ConsoleState::Allocated;
        Ok(())
    }

    fn attach_console(&mut self, pid: Option<u32>) -> Result<()> {
        console::attach(pid)?;
        console::reopen_std_handles()?;
        self.console_state = ConsoleState::Attached;
        Ok(())
    }

    fn free_console(&mut self) -> Result<()> {
        if self.console_state == ConsoleState::None {
            return Ok(());
        }
        console::free()?;
        self.console_state = ConsoleState::None;
        Ok(())
    }
}
