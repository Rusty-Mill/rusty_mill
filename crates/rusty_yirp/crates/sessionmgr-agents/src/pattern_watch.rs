//! The shared engine behind every adapter's tier-3 `needs_input`: a real
//! terminal-state machine, not text-stripping.
//!
//! Measured, not assumed (`docs/phase-3-report.md` has the full capture):
//! feeding raw PTY bytes through an ANSI-escape-stripping regex and then
//! substring-matching the result silently fails, because both Claude
//! Code and Codex lay out large parts of their screen with
//! cursor-positioning escape sequences rather than literal space
//! characters. Stripping the escapes without *interpreting* them leaves
//! words run together with no separating whitespace at all
//! (`"WelcomebackNano!"` for what a real terminal renders as
//! `"Welcome back Nano!"`), which breaks every substring pattern that
//! spans a word boundary. `vt100` is already a workspace dependency
//! (`sessionmgr-tui` uses it for the same reason, per ADR-0002); reusing
//! it here is the same "interpret, don't print" rule applied to the
//! daemon's own pattern matching, not a new dependency.

/// A sane ceiling on either terminal dimension. `vt100::Screen` is a
/// dense 2D grid of cells, so an unclamped, client-supplied `u16`
/// (`SessionResize` carries the value straight from the wire) would
/// drive an allocation of up to 65535 x 65535 cells -- gigabytes for one
/// resize. No real terminal is anywhere near this large; it is
/// generous headroom, not a realistic size.
const MAX_TERMINAL_DIM: u16 = 500;

/// Feeds a session's raw output through a `vt100` screen and renders the
/// current screen as plain text on demand.
pub struct ScreenWatcher {
    parser: vt100::Parser,
}

impl ScreenWatcher {
    pub fn new(rows: u16, cols: u16) -> Self {
        // No scrollback: this only ever reads the *current* screen to
        // answer "is it waiting on me right now", never history.
        ScreenWatcher {
            parser: vt100::Parser::new(rows, cols, 0),
        }
    }

    pub fn feed(&mut self, bytes: &[u8]) {
        self.parser.process(bytes);
    }

    /// Clamped to [`MAX_TERMINAL_DIM`] in each dimension: a `vt100`
    /// screen is a dense 2D cell grid, so an unclamped client-supplied
    /// size drives an uncapped allocation.
    pub fn resize(&mut self, rows: u16, cols: u16) {
        self.parser
            .screen_mut()
            .set_size(rows.min(MAX_TERMINAL_DIM), cols.min(MAX_TERMINAL_DIM));
    }

    /// The current screen, rendered as plain text -- ANSI already
    /// interpreted, one line per row. What every adapter's `needs_input`
    /// actually matches against.
    pub fn text(&self) -> String {
        self.parser.screen().contents()
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn cursor_positioned_text_renders_with_real_spaces() {
        // The exact defect this module exists to avoid: text laid out via
        // absolute cursor moves, not literal spaces.
        let mut watcher = ScreenWatcher::new(5, 40);
        // Move to column 5, print "back", move to column 10, print "Nano!"
        watcher.feed(b"Welcome\x1b[1;5Hback\x1b[1;10HNano!");
        // A naive ANSI-strip-then-concatenate would read this back
        // wrong; vt100 places each write at its real screen position.
        assert!(watcher.text().contains("back"));
        assert!(watcher.text().contains("Nano"));
    }

    #[test]
    fn resize_clamps_an_out_of_range_size_instead_of_allocating_it() {
        // A `SessionResize` request carries client-supplied `u16`s
        // straight off the wire; feeding 65535x65535 through unclamped
        // would drive a multi-gigabyte `vt100` grid allocation.
        let mut watcher = ScreenWatcher::new(5, 40);
        watcher.resize(65535, 65535);
        let (rows, cols) = watcher.parser.screen().size();
        assert_eq!(rows, MAX_TERMINAL_DIM);
        assert_eq!(cols, MAX_TERMINAL_DIM);
    }
}
