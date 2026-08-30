//! `Terminal` impl over `sys::termios` (extraction map D9).

use std::time::Duration;

use platform::error::Result;
use platform::term::{JobControlTerminal, TermStream, Terminal, WinSize};

use crate::sys::termios as t;

/// The Linux terminal, over the process's standard streams. Raw-mode
/// state (the saved termios) lives here, so enter/leave pair correctly
/// and idempotently.
#[derive(Default)]
pub struct LinuxTerminal {
    saved: Option<t::SavedTermios>,
}

impl LinuxTerminal {
    pub fn new() -> Self {
        Self::default()
    }

    /// The first std stream that is a tty, if any — the fd whose
    /// controlling terminal we measure (stderr is the classic last
    /// resort: it is the stream least likely to be redirected).
    fn tty_fd(&self) -> Option<i32> {
        [TermStream::Stdin, TermStream::Stdout, TermStream::Stderr]
            .into_iter()
            .map(t::stream_fd)
            .find(|&fd| t::is_tty(fd))
    }
}

impl Terminal for LinuxTerminal {
    fn is_tty(&self, stream: TermStream) -> bool {
        t::is_tty(t::stream_fd(stream))
    }

    fn window_size(&self) -> Result<WinSize> {
        let fd = self.tty_fd().unwrap_or(t::stream_fd(TermStream::Stdout));
        let (rows, cols) = t::window_size(fd)?;
        Ok(WinSize { rows, cols })
    }

    fn enter_raw(&mut self) -> Result<()> {
        if self.saved.is_some() {
            return Ok(());
        }
        let fd = t::stream_fd(TermStream::Stdin);
        self.saved = Some(t::enter_raw(fd)?);
        Ok(())
    }

    fn leave_raw(&mut self) -> Result<()> {
        if let Some(saved) = self.saved.take() {
            t::restore(t::stream_fd(TermStream::Stdin), &saved)?;
        }
        Ok(())
    }

    fn is_raw(&self) -> bool {
        t::is_raw(t::stream_fd(TermStream::Stdin))
    }

    fn poll_readable(&self, timeout: Option<Duration>) -> Result<bool> {
        t::poll_readable(t::stream_fd(TermStream::Stdin), timeout)
    }

    fn read_chunk(&self, buf: &mut [u8]) -> Result<usize> {
        t::read_chunk(t::stream_fd(TermStream::Stdin), buf)
    }

    fn set_echo(&mut self, on: bool) -> Result<bool> {
        t::set_echo(t::stream_fd(TermStream::Stdin), on)
    }
}

impl JobControlTerminal for LinuxTerminal {
    fn give_terminal(&self, pgid: u32) -> Result<()> {
        t::give_terminal(t::stream_fd(TermStream::Stdin), pgid as i32)
    }
}
