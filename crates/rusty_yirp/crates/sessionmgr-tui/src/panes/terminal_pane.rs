//! The reusable terminal-rendering building block: a `vt100` screen-state
//! machine fed live PTY bytes, rendered through `tui-term`'s
//! `PseudoTerminal` widget.
//!
//! Per ADR-0002, this is the one place in the TUI that is allowed to
//! think about escape sequences at all -- everywhere else only ever sees
//! the already-interpreted `vt100::Screen` this produces. Used directly
//! by [`super::session_pane::SessionPane`] (an agent or plain-terminal
//! session) and would be the same widget for a dependent terminal pane
//! (Phase 5), since rendering a live terminal does not differ by what is
//! running inside it.

use ratatui::layout::Rect;
use ratatui::widgets::{Block, Borders};
use ratatui::Frame;
use tui_term::widget::PseudoTerminal;

pub struct TerminalPane {
    parser: vt100::Parser,
}

impl TerminalPane {
    pub fn new(rows: u16, cols: u16) -> Self {
        // Scrollback of 200 lines: enough to scan recent output without
        // holding an unbounded history in memory per open pane -- the
        // transcript on disk is the durable record; this is just what a
        // live pane keeps warm.
        TerminalPane {
            parser: vt100::Parser::new(rows, cols, 200),
        }
    }

    /// Feeds a chunk of the session's raw output through the VT100 state
    /// machine. Never printed as-is -- this is exactly the step ADR-0002
    /// requires.
    pub fn feed(&mut self, bytes: &[u8]) {
        self.parser.process(bytes);
    }

    pub fn resize(&mut self, rows: u16, cols: u16) {
        self.parser.screen_mut().set_size(rows, cols);
    }

    /// The parser's current size, so a caller can tell whether a redraw
    /// at a new pane size needs to send `SessionResize` before rendering.
    pub fn size(&self) -> (u16, u16) {
        self.parser.screen().size()
    }

    pub fn render(&self, frame: &mut Frame, area: Rect, title: &str, focused: bool) {
        let border_style = if focused {
            ratatui::style::Style::default().fg(ratatui::style::Color::Cyan)
        } else {
            ratatui::style::Style::default()
        };
        let block = Block::default()
            .title(title.to_owned())
            .borders(Borders::ALL)
            .border_style(border_style);
        let widget = PseudoTerminal::new(self.parser.screen()).block(block);
        frame.render_widget(widget, area);
    }
}
