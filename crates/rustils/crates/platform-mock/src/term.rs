//! Scripted `Terminal`: tests configure tty-ness, size, and pending
//! input by hand and observe raw-mode transitions, under the same
//! contract the native backends implement (idempotent enter/leave,
//! errors when not a tty).

use std::cell::RefCell;
use std::collections::VecDeque;
use std::time::Duration;

use platform::error::{ErrorKind, OsCode, PlatformError, Result};
use platform::term::{TermStream, Terminal, WinSize};

/// A mock terminal. Defaults to "not a tty at all" — the shape a CI
/// pipe presents — so tests opt *in* to tty behavior.
///
/// `pending` is a `RefCell`: [`Terminal::read_chunk`] takes `&self` (it
/// mirrors the native backends, which read a live fd/handle through a
/// shared reference — the kernel owns the mutable state, not the Rust
/// struct), so draining the mock's scripted input queue needs interior
/// mutability to match that same shape.
#[derive(Debug, Default)]
pub struct MockTerminal {
    stdin_tty: bool,
    stdout_tty: bool,
    stderr_tty: bool,
    size: Option<WinSize>,
    raw: bool,
    raw_transitions: u32,
    echo: bool,
    pending: RefCell<VecDeque<u8>>,
}

impl MockTerminal {
    pub fn new() -> Self {
        Self {
            echo: true,
            ..Self::default()
        }
    }

    /// A fully-interactive terminal of the given size.
    pub fn interactive(rows: u16, cols: u16) -> Self {
        Self {
            stdin_tty: true,
            stdout_tty: true,
            stderr_tty: true,
            size: Some(WinSize { rows, cols }),
            echo: true,
            ..Self::default()
        }
    }

    /// Mark one stream as a tty.
    pub fn set_tty(&mut self, stream: TermStream, tty: bool) {
        match stream {
            TermStream::Stdin => self.stdin_tty = tty,
            TermStream::Stdout => self.stdout_tty = tty,
            TermStream::Stderr => self.stderr_tty = tty,
        }
    }

    /// Whether the terminal is currently in raw mode.
    pub fn in_raw(&self) -> bool {
        self.raw
    }

    /// How many times raw mode was actually entered (idempotent
    /// re-enters do not count — that is the contract under test).
    pub fn raw_transitions(&self) -> u32 {
        self.raw_transitions
    }

    /// Whether echo is currently on (defaults to on, like a real tty).
    pub fn echo_on(&self) -> bool {
        self.echo
    }

    /// Queue bytes for the next [`Terminal::poll_readable`]/
    /// [`Terminal::read_chunk`] calls to see, simulating input arriving
    /// (a scripted keystroke or paste).
    pub fn push_input(&mut self, bytes: &[u8]) {
        self.pending.get_mut().extend(bytes);
    }

    /// Simulate drift: force the raw-mode flag [`Terminal::is_raw`]
    /// reports, without going through `enter_raw`/`leave_raw` — the
    /// mock's analog of an external `stty` call, for exercising a
    /// consumer's self-healing re-assert logic.
    pub fn force_raw(&mut self, raw: bool) {
        self.raw = raw;
    }
}

fn not_a_tty(op: &'static str) -> PlatformError {
    PlatformError::new(ErrorKind::Other, OsCode::None, op)
}

impl Terminal for MockTerminal {
    fn is_tty(&self, stream: TermStream) -> bool {
        match stream {
            TermStream::Stdin => self.stdin_tty,
            TermStream::Stdout => self.stdout_tty,
            TermStream::Stderr => self.stderr_tty,
        }
    }

    fn window_size(&self) -> Result<WinSize> {
        self.size.ok_or_else(|| not_a_tty("window_size"))
    }

    fn enter_raw(&mut self) -> Result<()> {
        if !self.stdin_tty {
            return Err(not_a_tty("enter_raw"));
        }
        if !self.raw {
            self.raw = true;
            self.raw_transitions += 1;
        }
        Ok(())
    }

    fn leave_raw(&mut self) -> Result<()> {
        self.raw = false;
        Ok(())
    }

    fn is_raw(&self) -> bool {
        self.raw
    }

    /// The mock never actually blocks: it reports readiness from the
    /// queue [`push_input`](MockTerminal::push_input) filled, instantly
    /// — there is no real clock to wait on a timeout against. A test
    /// that wants to observe a timeout path queues nothing and reads
    /// `Ok(false)` back immediately rather than after `timeout` elapses.
    fn poll_readable(&self, _timeout: Option<Duration>) -> Result<bool> {
        Ok(!self.pending.borrow().is_empty())
    }

    /// Drains from the queue [`push_input`](MockTerminal::push_input)
    /// filled; `Ok(0)` when it's empty (the mock's stand-in for "would
    /// block", since it has no real EOF/blocking distinction to model).
    fn read_chunk(&self, buf: &mut [u8]) -> Result<usize> {
        let mut pending = self.pending.borrow_mut();
        let n = pending.len().min(buf.len());
        for (slot, byte) in buf.iter_mut().zip(pending.drain(..n)) {
            *slot = byte;
        }
        Ok(n)
    }

    fn set_echo(&mut self, on: bool) -> Result<bool> {
        if !self.stdin_tty {
            return Err(not_a_tty("set_echo"));
        }
        let was_on = self.echo;
        self.echo = on;
        Ok(was_on)
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn defaults_to_no_tty_and_size_errors() {
        let t = MockTerminal::new();
        assert!(!t.is_tty(TermStream::Stdin));
        assert!(t.window_size().is_err());
    }

    #[test]
    fn raw_mode_is_idempotent_and_gated_on_stdin_tty() {
        let mut t = MockTerminal::new();
        assert!(t.enter_raw().is_err(), "raw mode without a tty must fail");

        let mut t = MockTerminal::interactive(24, 80);
        t.enter_raw().unwrap();
        t.enter_raw().unwrap();
        assert!(t.in_raw());
        assert_eq!(t.raw_transitions(), 1, "re-enter must be a no-op");
        t.leave_raw().unwrap();
        t.leave_raw().unwrap();
        assert!(!t.in_raw());
        assert_eq!(t.window_size().unwrap(), WinSize { rows: 24, cols: 80 });
    }

    #[test]
    fn read_chunk_drains_pushed_input_and_reports_empty_as_zero() {
        let mut t = MockTerminal::interactive(24, 80);
        assert!(!t.poll_readable(None).unwrap(), "nothing queued yet");
        t.push_input(b"hi");
        assert!(t.poll_readable(None).unwrap());
        let mut buf = [0u8; 1];
        assert_eq!(t.read_chunk(&mut buf).unwrap(), 1, "one byte at a time");
        assert_eq!(buf[0], b'h');
        assert!(t.poll_readable(None).unwrap(), "'i' still queued");
        let mut buf = [0u8; 8];
        assert_eq!(t.read_chunk(&mut buf).unwrap(), 1);
        assert_eq!(&buf[..1], b"i");
        assert_eq!(
            t.read_chunk(&mut buf).unwrap(),
            0,
            "drained: Ok(0), not an error"
        );
    }

    #[test]
    fn set_echo_reports_previous_state_and_is_gated_on_tty() {
        let mut t = MockTerminal::new();
        assert!(t.set_echo(false).is_err(), "no tty: set_echo must refuse");

        let mut t = MockTerminal::interactive(24, 80);
        assert!(t.echo_on(), "starts on, like a real tty");
        assert!(
            t.set_echo(false).unwrap(),
            "returns the PREVIOUS state (true)"
        );
        assert!(!t.echo_on());
        assert!(!t.set_echo(true).unwrap());
        assert!(t.echo_on());
    }

    #[test]
    fn is_raw_reflects_forced_drift_not_just_our_own_enter_raw() {
        let mut t = MockTerminal::interactive(24, 80);
        assert!(!t.is_raw());
        t.enter_raw().unwrap();
        assert!(t.is_raw());
        // Simulate an external `stty sane` resetting the terminal
        // without this handle's enter_raw/leave_raw being involved.
        t.force_raw(false);
        assert!(!t.is_raw(), "is_raw is a live probe, not a cached flag");
    }
}
