//! LEL tab inbox list and top-strip chip (card #1416, slice 3).
//!
//! Pure view model over the slice 1 [`Row`]s, no UI and no I/O: the chip text
//! (D22 pattern, D53, L3c) and the list rows with a keyboard selection that
//! survives a refresh by id (D60). Acting on a row is slice 4.

use super::lel::Row;

/// The strip chip's text.
pub(crate) fn chip(waiting: usize) -> String {
    match waiting {
        0 => "No decisions waiting".to_owned(),
        1 => "1 decision waiting".to_owned(),
        n => format!("{n} decisions waiting"),
    }
}

/// One list row: what the decision is, in the host's words.
#[derive(Debug, Clone, PartialEq, Eq)]
pub(crate) struct Item {
    /// The escalation id an action names (slice 4).
    pub id: String,
    /// The claim text; a task-level stop shows its card title, else its task.
    pub title: String,
    /// The line under the title: candidate count, or the task-level stop.
    pub detail: Option<String>,
    /// The host's reason label ("Taste call or deadlock").
    pub reason: String,
    /// A claim that cannot be undone: drawn with a glyph and a word.
    pub irreversible: bool,
}

impl Item {
    fn new(row: &Row) -> Self {
        let task = row.id.split('/').next().unwrap_or(&row.id);
        let title = match (&row.claim, &row.card) {
            (Some(c), _) => c.text.clone(),
            (None, Some(card)) => card.title.clone(),
            (None, None) => format!("Task {task}"),
        };
        let detail = match row.candidates.len() {
            0 if row.task_level => Some("The whole task waits".to_owned()),
            0 => None,
            n => Some(format!("{n} candidates")),
        };
        Self {
            id: row.id.clone(),
            title,
            detail,
            reason: row.reason.clone(),
            irreversible: row.reversible == Some(false),
        }
    }
}

/// The open inbox and its keyboard selection.
#[derive(Debug, Clone, PartialEq, Eq, Default)]
pub(crate) struct List {
    items: Vec<Item>,
    selected: usize,
}

impl List {
    /// Builds the list from a fresh read, keeping the selection on the same
    /// escalation when `prev` shows it still waits; else on the same position
    /// (clamped), so the cursor never jumps to the top.
    pub(crate) fn new(rows: &[Row], prev: Option<&Self>) -> Self {
        let items: Vec<Item> = rows.iter().map(Item::new).collect();
        let selected = prev
            .and_then(|p| {
                let was = p.selected()?;
                items
                    .iter()
                    .position(|i| i.id == was.id)
                    .or(Some(p.selected))
            })
            .map_or(0, |i| i.min(items.len().saturating_sub(1)));
        Self { items, selected }
    }

    /// Rows waiting; also the chip's count.
    pub(crate) fn items(&self) -> &[Item] {
        &self.items
    }

    /// The chip text for this list.
    pub(crate) fn chip(&self) -> String {
        chip(self.items.len())
    }

    /// The selected row; `None` for an empty inbox.
    pub(crate) fn selected(&self) -> Option<&Item> {
        self.items.get(self.selected)
    }

    /// Moves the selection `delta` rows, wrapping (D60). No-op when empty.
    pub(crate) fn step(&mut self, delta: isize) {
        let n = self.items.len() as isize;
        if n > 0 {
            self.selected = (self.selected as isize + delta).rem_euclid(n) as usize;
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::gui::lel::parse;

    const FIXTURE: &str = include_str!("testdata/lel_inbox.json");

    fn list() -> List {
        List::new(&parse(FIXTURE).unwrap(), None)
    }

    #[test]
    fn chip_counts_in_words() {
        assert_eq!(chip(0), "No decisions waiting");
        assert_eq!(chip(1), "1 decision waiting");
        assert_eq!(chip(3), "3 decisions waiting");
    }

    #[test]
    fn rows_say_what_waits() {
        let l = list();
        let [a, b, c] = l.items() else { panic!() };
        assert_eq!(a.title, "Drop the legacy export table");
        assert!(a.irreversible && a.detail.is_none());
        assert_eq!(b.detail.as_deref(), Some("2 candidates"));
        assert!(!b.irreversible);
        assert_eq!(c.title, "Task T3");
        assert_eq!(c.detail.as_deref(), Some("The whole task waits"));
        assert_eq!(l.chip(), "3 decisions waiting");
    }

    #[test]
    fn selection_wraps_both_ways() {
        let mut l = list();
        l.step(-1);
        assert_eq!(l.selected().unwrap().id, "T3/task/7");
        l.step(1);
        assert_eq!(l.selected().unwrap().id, "T1/msg:12/3");
    }

    #[test]
    fn refresh_keeps_the_selection_by_id() {
        let mut l = list();
        l.step(1);
        let rows = parse(FIXTURE).unwrap();
        // A decision above it was settled: the selection follows its id.
        let after = List::new(&rows[1..], Some(&l));
        assert_eq!(after.selected().unwrap().id, "T2/msg:20/5");
        // Its own row left: the position holds, clamped to the new end.
        let gone = List::new(&rows[2..], Some(&l));
        assert_eq!(gone.selected().unwrap().id, "T3/task/7");
    }

    #[test]
    fn empty_inbox_has_no_selection_and_ignores_keys() {
        let mut l = List::new(&[], None);
        l.step(1);
        assert!(l.selected().is_none());
        assert_eq!(l.chip(), "No decisions waiting");
    }
}
