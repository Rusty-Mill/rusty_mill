//! The workspace explorer panel's view state (gui-design D99, card #1573).
//!
//! The source owns the tree and its calls; the panel holds what it was told
//! to show ([`Snapshot`]), the scroll that keeps the cursor in view, and
//! turns keys into [`Intent`]s for the source. Pure: no pixels, no I/O.

/// One visible row of the tree, as the source flattened it.
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct Row {
    /// Nesting depth; the root's children are 0.
    pub depth: u16,
    /// The name to show (unescaped); the cut summary's text for a summary.
    pub name: String,
    pub dir: bool,
    /// Whether a directory shows its children.
    pub open: bool,
    /// The change letter, or `*` for a directory with a change below it.
    pub mark: Option<char>,
    /// A cut-listing summary, such as "3 not shown": drawn quiet, never selected.
    pub summary: bool,
}

/// What the panel shows besides rows.
#[derive(Clone, Debug, Default, PartialEq, Eq)]
pub enum State {
    /// Reading the folder at this path.
    Loading(String),
    /// The tree.
    #[default]
    Tree,
    /// The workspace has no entries.
    Empty,
    /// The server refused, in one line.
    Refused(String),
    /// An owner-only block this client does not hold: take over first.
    TakeOver,
}

/// Everything the panel draws, pushed whole by the source on each change.
#[derive(Clone, Debug, Default, PartialEq, Eq)]
pub struct Snapshot {
    pub state: State,
    pub rows: Vec<Row>,
    /// Index into `rows` of the selected row.
    pub cursor: usize,
    /// Size and age of the selected row, when the server tells them.
    pub footer: Option<String>,
}

/// What the user asked of the source.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum Intent {
    /// Move the cursor by this many rows.
    Step(isize),
    /// Open the selected directory.
    Enter,
    /// Close it, or go to its parent.
    Leave,
    /// Put the selected path on the clipboard.
    Copy,
    /// Select row `n` of the snapshot (a click).
    Select(usize),
}

/// A key the panel understands, already mapped from the window's.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum Key {
    Up,
    Down,
    Left,
    Right,
    Enter,
    PageUp,
    PageDown,
    Copy,
    Close,
}

/// What a key or click did.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum Out {
    Ask(Intent),
    Close,
}

/// The open panel.
#[derive(Debug, Default)]
pub struct Panel {
    /// The block the tree belongs to; `None` while closed.
    key: Option<u64>,
    snap: Snapshot,
    /// Index of the first row drawn.
    top: usize,
    /// How many rows fit; set by the view each paint.
    page: usize,
}

impl Panel {
    /// Opens the panel for block `key`, showing nothing until a snapshot comes.
    pub fn open(&mut self, key: u64) {
        *self = Self {
            key: Some(key),
            page: 1,
            ..Self::default()
        };
    }

    pub fn close(&mut self) {
        *self = Self::default();
    }

    #[must_use]
    pub fn is_open(&self) -> bool {
        self.key.is_some()
    }

    /// The block the open panel belongs to.
    #[must_use]
    pub fn block(&self) -> Option<u64> {
        self.key
    }

    #[must_use]
    pub fn snapshot(&self) -> &Snapshot {
        &self.snap
    }

    /// Index of the first row to draw.
    #[must_use]
    pub fn top(&self) -> usize {
        self.top
    }

    /// Takes the source's newest snapshot for block `key`; one for a block
    /// the panel is not open on is dropped.
    pub fn set(&mut self, key: u64, mut snap: Snapshot) -> bool {
        if self.key != Some(key) {
            return false;
        }
        snap.cursor = snap.cursor.min(snap.rows.len().saturating_sub(1));
        self.snap = snap;
        self.scroll();
        true
    }

    /// Tells the panel how many rows fit, from the last paint.
    pub fn fit(&mut self, page: usize) {
        self.page = page.max(1);
        self.scroll();
    }

    fn scroll(&mut self) {
        let (cursor, page) = (self.snap.cursor, self.page);
        let last = self.snap.rows.len().saturating_sub(page);
        self.top = self.top.min(last);
        if cursor < self.top {
            self.top = cursor;
        } else if cursor >= self.top + page {
            self.top = cursor + 1 - page;
        }
    }

    /// A key while the panel is open.
    #[must_use]
    pub fn key(&self, key: Key) -> Out {
        let page = isize::try_from(self.page).unwrap_or(isize::MAX);
        Out::Ask(match key {
            Key::Close => return Out::Close,
            Key::Up => Intent::Step(-1),
            Key::Down => Intent::Step(1),
            Key::PageUp => Intent::Step(-page),
            Key::PageDown => Intent::Step(page),
            Key::Left => Intent::Leave,
            Key::Right | Key::Enter => Intent::Enter,
            Key::Copy => Intent::Copy,
        })
    }

    /// A click on visible row `n` (counted from the first drawn); one past
    /// the rows, or on a summary, asks nothing.
    #[must_use]
    pub fn click(&self, n: usize) -> Option<Out> {
        let at = self.top + n;
        let row = self.snap.rows.get(at)?;
        (!row.summary).then_some(Out::Ask(Intent::Select(at)))
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn row(name: &str) -> Row {
        Row {
            depth: 0,
            name: name.into(),
            dir: false,
            open: false,
            mark: None,
            summary: false,
        }
    }

    fn snap(n: usize, cursor: usize) -> Snapshot {
        Snapshot {
            rows: (0..n).map(|i| row(&i.to_string())).collect(),
            cursor,
            ..Snapshot::default()
        }
    }

    fn open(page: usize) -> Panel {
        let mut p = Panel::default();
        p.open(7);
        p.fit(page);
        p
    }

    #[test]
    fn closed_panel_drops_snapshots() {
        let mut p = Panel::default();
        assert!(!p.set(7, snap(3, 0)));
        p.open(7);
        assert!(!p.set(8, snap(3, 0)));
        assert!(p.set(7, snap(3, 0)));
        p.close();
        assert!(!p.is_open());
    }

    #[test]
    fn cursor_is_kept_in_view() {
        let mut p = open(4);
        p.set(7, snap(10, 0));
        assert_eq!(p.top(), 0);
        p.set(7, snap(10, 6));
        assert_eq!(p.top(), 3, "the cursor sits on the last visible row");
        p.set(7, snap(10, 2));
        assert_eq!(p.top(), 2, "scrolls back up to the cursor");
    }

    #[test]
    fn cursor_past_the_rows_is_clamped() {
        let mut p = open(4);
        p.set(7, snap(3, 9));
        assert_eq!(p.snapshot().cursor, 2);
        p.set(7, snap(0, 9));
        assert_eq!(p.snapshot().cursor, 0);
    }

    #[test]
    fn keys_become_intents() {
        let p = open(5);
        assert_eq!(p.key(Key::Down), Out::Ask(Intent::Step(1)));
        assert_eq!(p.key(Key::PageUp), Out::Ask(Intent::Step(-5)));
        assert_eq!(p.key(Key::Left), Out::Ask(Intent::Leave));
        assert_eq!(p.key(Key::Right), Out::Ask(Intent::Enter));
        assert_eq!(p.key(Key::Enter), Out::Ask(Intent::Enter));
        assert_eq!(p.key(Key::Copy), Out::Ask(Intent::Copy));
        assert_eq!(p.key(Key::Close), Out::Close);
    }

    #[test]
    fn click_selects_a_row_of_the_scrolled_view() {
        let mut p = open(3);
        p.set(7, snap(10, 5));
        assert_eq!(p.top(), 3);
        assert_eq!(p.click(1), Some(Out::Ask(Intent::Select(4))));
        assert_eq!(p.click(9), None, "past the last row");
    }

    #[test]
    fn click_on_a_summary_asks_nothing() {
        let mut p = open(3);
        let mut s = snap(2, 0);
        s.rows[1].summary = true;
        p.set(7, s);
        assert_eq!(p.click(1), None);
        assert_eq!(p.click(0), Some(Out::Ask(Intent::Select(0))));
    }
}
