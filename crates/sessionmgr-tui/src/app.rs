//! Application state and the main event loop.
//!
//! # Keybindings
//!
//! Every other key is forwarded straight to the focused session's
//! terminal -- this is a multiplexer, and a user who cannot send Ctrl-D
//! or Ctrl-C to their shell or agent CLI has lost real functionality.
//! So, tmux-style, exactly one combination is reserved as a **prefix**:
//! `Ctrl-B`. The key immediately following it is a command, never
//! forwarded, whether recognized or not:
//!
//! - `n` / `p` -- focus next / previous pane
//! - Left / Right / Up / Down -- grow the focused pane's column/row
//!   (Down/Up shrink and grow the row; Left/Right shrink and grow the
//!   column)
//! - `g` -- toggle the git diff pane for the focused session
//! - `q` -- quit the TUI (running sessions are left running, same as
//!   `daemon shutdown`)
//! - anything else -- cancelled, no-op

use std::path::PathBuf;
use std::time::Duration;

use crossterm::event::{Event, KeyCode, KeyEventKind, KeyModifiers};
use ratatui::layout::{Constraint, Direction, Layout};
use ratatui::style::{Color, Style};
use ratatui::text::{Line, Span};
use ratatui::widgets::Paragraph;
use ratatui::Frame;
use rusty_tokio::sync::mpsc::{unbounded_channel, UnboundedReceiver, UnboundedSender};
use sessionmgr_protocol::{SessionEvent, SessionId, SessionKind, SessionStatus};

use crate::client::{self, Attached};
use crate::error::Result;
use crate::grid::Grid;
use crate::panes::git_diff_pane::GitDiffPane;
use crate::panes::session_pane::SessionPane;
use crate::terminal::Backend;

/// How often the session list is re-polled, to pick up sessions created
/// or closed from outside this TUI (another `sessionmgr new`/`close`, or
/// a session finishing on its own).
const POLL_INTERVAL: Duration = Duration::from_secs(2);

const PREFIX: (KeyCode, KeyModifiers) = (KeyCode::Char('b'), KeyModifiers::CONTROL);

struct OpenPane {
    pane: SessionPane,
    attach: Attached,
    pump: rusty_tokio::task::JoinHandle<()>,
}

/// What woke the event loop, carried out of `select!` as a plain value
/// -- see `App::run`'s own comment for why the branches cannot `.await`
/// directly.
enum Woken {
    Input(Option<Event>),
    Session(Option<(SessionId, SessionEvent)>),
    Tick,
}

pub struct App {
    socket: PathBuf,
    panes: Vec<OpenPane>,
    grid: Grid,
    focused: usize,
    prefix_pending: bool,
    diff: Option<(SessionId, GitDiffPane)>,
    status_line: String,
    should_quit: bool,
    session_tx: UnboundedSender<(SessionId, SessionEvent)>,
}

impl App {
    /// Returns the app plus the receiving half of its session-event
    /// channel. Kept separate from `App` itself (rather than a field)
    /// because `run`'s event loop needs to hold a mutable borrow of the
    /// receiver and a mutable borrow of `self` alive at the same time
    /// inside `rusty_tokio::select!` -- two disjoint local bindings make
    /// that trivially legal; one field of `self` used from two places in
    /// the same `select!` would not be.
    pub fn new(socket: PathBuf) -> (Self, UnboundedReceiver<(SessionId, SessionEvent)>) {
        let (session_tx, session_rx) = unbounded_channel();
        (
            App {
                socket,
                panes: Vec::new(),
                grid: Grid::for_pane_count(0),
                focused: 0,
                prefix_pending: false,
                diff: None,
                status_line: String::new(),
                should_quit: false,
                session_tx,
            },
            session_rx,
        )
    }

    pub async fn run(
        &mut self,
        term: &mut ratatui::Terminal<Backend>,
        session_rx: &mut UnboundedReceiver<(SessionId, SessionEvent)>,
    ) -> Result<()> {
        self.refresh_sessions().await;
        let mut input_rx = spawn_input_thread();

        loop {
            self.sync_pane_sizes(term).await;
            term.draw(|f| self.render(f))
                .map_err(|e| crate::error::Error::io("drawing the terminal", e))?;
            if self.should_quit {
                return Ok(());
            }

            // `rusty_tokio::select!` evaluates each branch's body
            // *inside* the `poll_fn` closure that drives the race --
            // a plain synchronous closure, unlike real tokio's
            // `select!`. A body containing `.await` does not compile
            // there. So branch bodies only ever produce a plain value
            // (`Woken`); everything that needs to `.await` happens
            // below, once, after the race has already resolved.
            let tick = rusty_tokio::time::sleep(POLL_INTERVAL);
            let woken = rusty_tokio::select! {
                input = input_rx.recv() => Woken::Input(input),
                session = session_rx.recv() => Woken::Session(session),
                _ = tick => Woken::Tick,
            };
            match woken {
                Woken::Input(Some(event)) => self.handle_input(event).await,
                Woken::Input(None) => self.should_quit = true,
                Woken::Session(Some((id, event))) => self.handle_session_event(id, event),
                Woken::Session(None) => {}
                Woken::Tick => self.refresh_sessions().await,
            }
        }
    }

    // -- session list / attach lifecycle -----------------------------

    async fn refresh_sessions(&mut self) {
        let summaries = match client::session_list(&self.socket).await {
            Ok(s) => s,
            Err(e) => {
                self.status_line = format!("could not reach the daemon: {e}");
                return;
            }
        };

        let live_ids: Vec<SessionId> = summaries.iter().map(|s| s.id.clone()).collect();

        // Drop panes for sessions that no longer appear -- closed
        // elsewhere (another client, or the daemon reconciling a dead
        // worker). The pump task is aborted explicitly: it holds the
        // connection's read half itself, not reachable through
        // `Attached`, so dropping the pane alone would leak it running.
        self.panes.retain(|p| {
            let keep = live_ids.contains(&p.pane.id);
            if !keep {
                p.pump.abort();
            }
            keep
        });

        for summary in &summaries {
            if let Some(open) = self.panes.iter_mut().find(|p| p.pane.id == summary.id) {
                open.pane.set_status(summary.status);
                continue;
            }
            if !summary.status.expects_live_worker() {
                // Not attachable -- nothing streams for a session that
                // already finished before this TUI ever saw it. It still
                // won't show in the grid; `attach` from the CLI remains
                // the way to read a finished session's transcript.
                continue;
            }
            self.open_pane(summary.id.clone(), summary.kind, summary.status)
                .await;
        }

        let count = self.panes.len();
        if self.grid.rows() * self.grid.cols() != count {
            self.grid = Grid::for_pane_count(count);
        }
        if self.focused >= count.max(1) {
            self.focused = count.saturating_sub(1);
        }
    }

    async fn open_pane(&mut self, id: SessionId, kind: SessionKind, status: SessionStatus) {
        match Attached::open(&self.socket, id.clone(), self.session_tx.clone()).await {
            Ok((attach, pump)) => {
                self.panes.push(OpenPane {
                    pane: SessionPane::new(id, kind, status, 24, 80),
                    attach,
                    pump,
                });
            }
            Err(e) => {
                self.status_line = format!("could not attach to {id}: {e}");
            }
        }
    }

    fn handle_session_event(&mut self, id: SessionId, event: SessionEvent) {
        let Some(open) = self.panes.iter_mut().find(|p| p.pane.id == id) else {
            return;
        };
        match event {
            SessionEvent::Output { data } => open.pane.feed(&data),
            SessionEvent::Status { status } => open.pane.set_status(status),
            SessionEvent::Exited { code } => {
                self.status_line = format!("session {id} exited ({code:?})");
            }
            SessionEvent::RecoveryMarker => {
                open.pane
                    .feed(b"\r\n[reattached to a session that survived a manager restart]\r\n");
            }
        }
    }

    // -- input ---------------------------------------------------------

    async fn handle_input(&mut self, event: Event) {
        let Event::Key(key) = event else { return };
        if key.kind != KeyEventKind::Press {
            return;
        }

        if self.prefix_pending {
            self.prefix_pending = false;
            self.handle_command_key(key.code).await;
            return;
        }
        if (key.code, key.modifiers) == PREFIX {
            self.prefix_pending = true;
            return;
        }

        if let Some(bytes) = key_to_bytes(key.code, key.modifiers) {
            if let Some(open) = self.panes.get_mut(self.focused) {
                if let Err(e) = open.attach.send_input(bytes).await {
                    self.status_line = format!("input failed: {e}");
                }
            }
        }
    }

    async fn handle_command_key(&mut self, code: KeyCode) {
        let count = self.panes.len();
        match code {
            KeyCode::Char('q') => self.should_quit = true,
            KeyCode::Char('n') if count > 0 => self.focused = (self.focused + 1) % count,
            KeyCode::Char('p') if count > 0 => {
                self.focused = (self.focused + count - 1) % count;
            }
            KeyCode::Char('g') => self.toggle_diff().await,
            KeyCode::Left | KeyCode::Right | KeyCode::Up | KeyCode::Down if count > 0 => {
                let cols = self.grid.cols().max(1);
                let row = self.focused / cols;
                let col = self.focused % cols;
                match code {
                    KeyCode::Right => self.grid.grow_col(col),
                    KeyCode::Left => self.grid.shrink_col(col),
                    KeyCode::Down => self.grid.grow_row(row),
                    KeyCode::Up => self.grid.shrink_row(row),
                    _ => unreachable!(),
                }
            }
            _ => {}
        }
    }

    async fn toggle_diff(&mut self) {
        let Some(open) = self.panes.get(self.focused) else {
            return;
        };
        let id = open.pane.id.clone();
        if self.diff.as_ref().is_some_and(|(shown, _)| *shown == id) {
            self.diff = None;
            return;
        }
        if open.pane.kind == SessionKind::PlainTerminal {
            self.status_line = "a plain terminal session has no git workspace".to_owned();
            return;
        }
        match client::git_status(&self.socket, id.clone()).await {
            Ok(files) => {
                let mut pane = GitDiffPane::default();
                pane.set_files(files);
                if let Some(path) = pane.selected_path().map(str::to_owned) {
                    if let Ok(diff) = client::git_diff(&self.socket, id.clone(), Some(path)).await {
                        pane.set_diff(diff);
                    }
                }
                self.diff = Some((id, pane));
            }
            Err(e) => self.status_line = format!("git status failed: {e}"),
        }
    }

    // -- rendering -------------------------------------------------------

    /// Sends `SessionResize` for any pane whose computed cell size
    /// changed since the last frame -- a layout change (grid resize, the
    /// grid gaining/losing a pane, the terminal window itself resizing)
    /// all funnel through here rather than three separate call sites.
    async fn sync_pane_sizes(&mut self, term: &mut ratatui::Terminal<Backend>) {
        let area = term.get_frame().area();
        let content_area = content_area(area);
        let cells = self.grid.split(content_area);
        for (open, rect) in self.panes.iter_mut().zip(cells.iter()) {
            let (rows, cols) = cell_terminal_size(*rect);
            if open.pane.size() != (rows, cols) {
                open.pane.resize(rows, cols);
                let _ = open.attach.send_resize(rows, cols).await;
            }
        }
    }

    fn render(&mut self, frame: &mut Frame) {
        let area = frame.area();
        let [grid_area, status_area] = Layout::default()
            .direction(Direction::Vertical)
            .constraints([Constraint::Min(1), Constraint::Length(1)])
            .areas(area);

        if self.panes.is_empty() {
            let empty = Paragraph::new("no sessions -- create one with `sessionmgr new`");
            frame.render_widget(empty, grid_area);
        } else {
            let cells = self.grid.split(grid_area);
            for (i, (open, rect)) in self.panes.iter().zip(cells.iter()).enumerate() {
                let focused = i == self.focused;
                if focused {
                    if let Some((id, diff_pane)) = &mut self.diff {
                        if *id == open.pane.id {
                            diff_pane.render(frame, *rect);
                            continue;
                        }
                    }
                }
                open.pane.render(frame, *rect, focused);
            }
        }

        frame.render_widget(self.status_bar(), status_area);
    }

    fn status_bar(&self) -> Paragraph<'static> {
        let mode = if self.prefix_pending { "Ctrl-B..." } else { "" };
        let help = "Ctrl-B then: n/p focus, arrows resize, g diff, q quit";
        let text = if self.status_line.is_empty() {
            help.to_owned()
        } else {
            self.status_line.clone()
        };
        Paragraph::new(Line::from(vec![
            Span::styled(format!(" {mode} "), Style::default().fg(Color::Yellow)),
            Span::raw(text),
        ]))
    }
}

/// The grid's own area, minus the one-line status bar -- already
/// subtracted by `render`'s own layout, but `sync_pane_sizes` computes a
/// layout independently (it runs before `draw`, not inside it), so the
/// same split is repeated here rather than shared through a field that
/// would otherwise need to survive between an async step and a sync one.
fn content_area(area: ratatui::layout::Rect) -> ratatui::layout::Rect {
    let [grid_area, _status] = Layout::default()
        .direction(Direction::Vertical)
        .constraints([Constraint::Min(1), Constraint::Length(1)])
        .areas(area);
    grid_area
}

/// A cell `Rect`'s usable terminal size once its border consumes one row
/// and one column on each side.
fn cell_terminal_size(rect: ratatui::layout::Rect) -> (u16, u16) {
    (
        rect.height.saturating_sub(2).max(1),
        rect.width.saturating_sub(2).max(1),
    )
}

/// Translates a key event into the bytes a real terminal would send.
///
/// Not exhaustive -- covers ordinary text, the control keys a shell or
/// agent CLI actually needs (arrows, Enter, Backspace, Tab, Esc,
/// Ctrl-<letter>), and deliberately leaves anything else unmapped rather
/// than guessing at an escape sequence and sending the wrong one.
fn key_to_bytes(code: KeyCode, modifiers: KeyModifiers) -> Option<Vec<u8>> {
    if modifiers.contains(KeyModifiers::CONTROL) {
        if let KeyCode::Char(c) = code {
            let lower = c.to_ascii_lowercase();
            if lower.is_ascii_lowercase() {
                return Some(vec![(lower as u8) - b'a' + 1]);
            }
        }
    }
    match code {
        KeyCode::Char(c) => Some(c.to_string().into_bytes()),
        KeyCode::Enter => Some(vec![b'\r']),
        KeyCode::Backspace => Some(vec![0x7f]),
        KeyCode::Tab => Some(vec![b'\t']),
        KeyCode::Esc => Some(vec![0x1b]),
        KeyCode::Up => Some(b"\x1b[A".to_vec()),
        KeyCode::Down => Some(b"\x1b[B".to_vec()),
        KeyCode::Right => Some(b"\x1b[C".to_vec()),
        KeyCode::Left => Some(b"\x1b[D".to_vec()),
        KeyCode::Home => Some(b"\x1b[H".to_vec()),
        KeyCode::End => Some(b"\x1b[F".to_vec()),
        KeyCode::Delete => Some(b"\x1b[3~".to_vec()),
        _ => None,
    }
}

/// A dedicated OS thread reading `crossterm` events blocking, forwarding
/// them into an async channel.
///
/// `crossterm`'s own async `EventStream` is built on real `tokio`'s
/// reactor; this project's runtime is `rusty_tokio`, a different (if
/// API-compatible in spirit) implementation, so that integration is not
/// available here. A blocking read on its own thread is the standard
/// shape for exactly this situation, independent of which async runtime
/// is driving everything else.
fn spawn_input_thread() -> UnboundedReceiver<Event> {
    let (tx, rx) = unbounded_channel();
    std::thread::spawn(move || loop {
        match crossterm::event::read() {
            Ok(event) => {
                if tx.send(event).is_err() {
                    return;
                }
            }
            Err(_) => return,
        }
    });
    rx
}
