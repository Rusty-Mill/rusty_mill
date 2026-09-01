//! The git diff panel: a changed-files list beside the selected file's
//! unified diff, both scoped to one session's workspace.
//!
//! No syntax highlighting -- real, deliberate v1 scope per PLAN.md's own
//! TUI design section, not an oversight. Framed there as a place this
//! project can beat Xirp's own full-screen-only diff view rather than
//! just match it: a split pane beside the session it belongs to, not a
//! separate full-screen mode.

use ratatui::layout::{Constraint, Direction, Layout, Rect};
use ratatui::style::{Color, Style};
use ratatui::text::{Line, Span};
use ratatui::widgets::{Block, Borders, List, ListItem, ListState, Paragraph, Wrap};
use ratatui::Frame;
use sessionmgr_protocol::ChangedFile;

#[derive(Default)]
pub struct GitDiffPane {
    pub files: Vec<ChangedFile>,
    pub diff: String,
    selected: usize,
    list_state: ListState,
}

impl GitDiffPane {
    pub fn set_files(&mut self, files: Vec<ChangedFile>) {
        self.selected = self.selected.min(files.len().saturating_sub(1));
        self.files = files;
        self.list_state.select(if self.files.is_empty() {
            None
        } else {
            Some(self.selected)
        });
    }

    pub fn set_diff(&mut self, diff: String) {
        self.diff = diff;
    }

    pub fn selected_path(&self) -> Option<&str> {
        self.files.get(self.selected).map(|f| f.path.as_str())
    }

    /// Moves the selection and reports whether it actually changed --
    /// the caller uses that to know whether to re-request the diff for
    /// the newly selected file rather than doing it on every keypress.
    pub fn select_next(&mut self) -> bool {
        self.move_selection(1)
    }

    pub fn select_prev(&mut self) -> bool {
        self.move_selection(-1)
    }

    fn move_selection(&mut self, delta: i32) -> bool {
        if self.files.is_empty() {
            return false;
        }
        let len = self.files.len() as i32;
        let next = (self.selected as i32 + delta).rem_euclid(len) as usize;
        if next == self.selected {
            return false;
        }
        self.selected = next;
        self.list_state.select(Some(self.selected));
        true
    }

    pub fn render(&mut self, frame: &mut Frame, area: Rect) {
        let [list_area, diff_area] = Layout::default()
            .direction(Direction::Horizontal)
            .constraints([Constraint::Percentage(30), Constraint::Percentage(70)])
            .areas(area);

        let items: Vec<ListItem> = self
            .files
            .iter()
            .map(|f| {
                let color = status_color(&f.status);
                ListItem::new(Line::from(vec![
                    Span::styled(format!("{} ", f.status), Style::default().fg(color)),
                    Span::raw(f.path.clone()),
                ]))
            })
            .collect();
        let list = List::new(items)
            .block(
                Block::default()
                    .title(" Changed files ")
                    .borders(Borders::ALL),
            )
            .highlight_style(Style::default().bg(Color::DarkGray));
        frame.render_stateful_widget(list, list_area, &mut self.list_state);

        let title = match self.selected_path() {
            Some(path) => format!(" Diff: {path} "),
            None => " Diff ".to_owned(),
        };
        let diff = Paragraph::new(self.diff.as_str())
            .block(Block::default().title(title).borders(Borders::ALL))
            .wrap(Wrap { trim: false });
        frame.render_widget(diff, diff_area);
    }
}

fn status_color(status: &str) -> Color {
    match status.trim() {
        s if s.contains('A') || s == "??" => Color::Green,
        s if s.contains('D') => Color::Red,
        s if s.contains('R') => Color::Magenta,
        _ => Color::Yellow,
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn file(status: &str, path: &str) -> ChangedFile {
        ChangedFile {
            status: status.to_owned(),
            path: path.to_owned(),
        }
    }

    #[test]
    fn selection_wraps_forward_and_backward() {
        let mut pane = GitDiffPane::default();
        pane.set_files(vec![file(" M", "a"), file("??", "b"), file(" D", "c")]);
        assert_eq!(pane.selected_path(), Some("a"));
        assert!(pane.select_next());
        assert_eq!(pane.selected_path(), Some("b"));
        pane.select_next();
        assert_eq!(pane.selected_path(), Some("c"));
        assert!(pane.select_next());
        assert_eq!(pane.selected_path(), Some("a"), "wraps forward");
        assert!(pane.select_prev());
        assert_eq!(pane.selected_path(), Some("c"), "wraps backward");
    }

    #[test]
    fn empty_file_list_has_no_selection_and_no_op_navigation() {
        let mut pane = GitDiffPane::default();
        assert_eq!(pane.selected_path(), None);
        assert!(!pane.select_next());
        assert!(!pane.select_prev());
    }

    #[test]
    fn selection_clamps_when_the_file_list_shrinks() {
        let mut pane = GitDiffPane::default();
        pane.set_files(vec![file(" M", "a"), file("??", "b"), file(" D", "c")]);
        pane.select_next();
        pane.select_next();
        assert_eq!(pane.selected_path(), Some("c"));
        pane.set_files(vec![file(" M", "a")]);
        assert_eq!(pane.selected_path(), Some("a"));
    }
}
