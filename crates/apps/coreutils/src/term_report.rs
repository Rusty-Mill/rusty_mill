//! The logic behind `rterm` — the Terminal surface's reference consumer
//! (extraction map D9; RFC v2 §3). Written against `dyn Terminal` only;
//! tested against `platform-mock`.

use std::fmt::Write as _;

use platform::term::{TermStream, Terminal, WinSize};

/// Human-readable report of what the terminal looks like from inside
/// this process — the diagnostic a redirected CI run and an interactive
/// session answer differently.
pub fn report(term: &dyn Terminal) -> String {
    let mut out = String::new();
    for (name, stream) in [
        ("stdin", TermStream::Stdin),
        ("stdout", TermStream::Stdout),
        ("stderr", TermStream::Stderr),
    ] {
        let what = if term.is_tty(stream) {
            "tty"
        } else {
            "not a tty"
        };
        let _ = writeln!(out, "{name}: {what}");
    }
    match term.window_size() {
        Ok(WinSize { rows, cols }) => {
            let _ = writeln!(out, "size: {rows} rows x {cols} cols");
        }
        Err(_) => {
            let _ = writeln!(out, "size: unavailable (no terminal attached)");
        }
    }
    out
}

/// Enter raw mode, hand the terminal back to `body` (so it can use
/// [`Terminal::poll_readable`]/[`Terminal::read_chunk`] while raw), and
/// restore — restoring even when `body` errors. The pairing discipline
/// the trait asks consumers to own, in its simplest correct form.
pub fn with_raw<T>(
    term: &mut dyn Terminal,
    body: impl FnOnce(&mut dyn Terminal) -> platform::error::Result<T>,
) -> platform::error::Result<T> {
    term.enter_raw()?;
    let out = body(&mut *term);
    // Restore before surfacing body's result; a restore failure only
    // wins when the body itself succeeded.
    let restored = term.leave_raw();
    let val = out?;
    restored?;
    Ok(val)
}

#[cfg(test)]
mod tests {
    use super::*;
    use platform::error::{ErrorKind, OsCode, PlatformError};
    use platform_mock::MockTerminal;

    #[test]
    fn report_names_each_stream_and_the_size() {
        let t = MockTerminal::interactive(24, 80);
        let r = report(&t);
        assert!(r.contains("stdin: tty"));
        assert!(r.contains("size: 24 rows x 80 cols"));

        let none = MockTerminal::new();
        let r = report(&none);
        assert!(r.contains("stdout: not a tty"));
        assert!(r.contains("size: unavailable"));
    }

    #[test]
    fn with_raw_restores_on_success_and_on_error() {
        let mut t = MockTerminal::interactive(24, 80);
        with_raw(&mut t, |_| Ok(())).unwrap();
        assert!(!t.in_raw(), "raw mode must be restored after success");

        let boom: platform::error::Result<()> = with_raw(&mut t, |_| {
            Err(PlatformError::new(ErrorKind::Other, OsCode::None, "body"))
        });
        assert!(boom.is_err());
        assert!(!t.in_raw(), "raw mode must be restored after an error");
        assert_eq!(t.raw_transitions(), 2);
    }

    #[test]
    fn with_raw_fails_cleanly_without_a_tty() {
        let mut t = MockTerminal::new();
        let r: platform::error::Result<()> = with_raw(&mut t, |_| Ok(()));
        assert!(r.is_err(), "no tty: enter_raw must refuse");
    }

    #[test]
    fn with_raw_hands_back_a_live_terminal_for_poll_and_read() {
        let mut t = MockTerminal::interactive(24, 80);
        t.push_input(b"X");
        let byte = with_raw(&mut t, |raw| {
            assert!(raw.is_raw(), "body runs while raw mode is active");
            assert!(raw.poll_readable(None).unwrap(), "input was queued");
            let mut buf = [0u8; 1];
            raw.read_chunk(&mut buf)?;
            Ok(buf[0])
        })
        .unwrap();
        assert_eq!(byte, b'X');
        assert!(!t.in_raw(), "restored after body returns");
    }
}
