//! A grid cell showing one session: its live terminal plus a title bar
//! carrying id, status, and kind.

use ratatui::layout::Rect;
use ratatui::Frame;
use sessionmgr_protocol::{SessionId, SessionKind, SessionStatus};

use super::terminal_pane::TerminalPane;

pub struct SessionPane {
    pub id: SessionId,
    pub kind: SessionKind,
    pub status: SessionStatus,
    /// The user-chosen display label, set by the command palette's
    /// `rename` action; `None` until renamed.
    pub name: Option<String>,
    terminal: TerminalPane,
}

impl SessionPane {
    pub fn new(
        id: SessionId,
        kind: SessionKind,
        status: SessionStatus,
        rows: u16,
        cols: u16,
    ) -> Self {
        SessionPane {
            id,
            kind,
            status,
            name: None,
            terminal: TerminalPane::new(rows, cols),
        }
    }

    pub fn feed(&mut self, bytes: &[u8]) {
        self.terminal.feed(bytes);
    }

    pub fn resize(&mut self, rows: u16, cols: u16) {
        self.terminal.resize(rows, cols);
    }

    pub fn size(&self) -> (u16, u16) {
        self.terminal.size()
    }

    pub fn set_status(&mut self, status: SessionStatus) {
        self.status = status;
    }

    pub fn set_name(&mut self, name: Option<String>) {
        self.name = name;
    }

    /// What the command palette's "Focus: ..." entries and the pane
    /// title both call this session -- the display name if it was
    /// renamed, else its id.
    pub fn display_label(&self) -> String {
        self.name.clone().unwrap_or_else(|| self.id.to_string())
    }

    /// The title bar: the display label, a bracketed status word, and
    /// the kind -- low-confidence-vs-high-confidence status badging
    /// (PLAN.md's per-CLI confidence tiers) is Phase 3 adapter work, out
    /// of scope here; this just shows the state the daemon actually
    /// reports.
    fn title(&self) -> String {
        format!(
            " {} [{:?}] {:?} ",
            self.display_label(),
            self.status,
            self.kind
        )
    }

    pub fn render(&self, frame: &mut Frame, area: Rect, focused: bool) {
        let title = self.title();
        self.terminal.render(frame, area, &title, focused);
    }
}
