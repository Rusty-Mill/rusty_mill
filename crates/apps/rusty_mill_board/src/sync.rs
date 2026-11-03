//! Forge sync: reconcile the board with a forge's issues.
//!
//! Stateless between runs and idempotent: every run lists both sides and fixes
//! the difference, so a missed run or a crash heals on the next one. Policy:
//! - the **board owns state**: the column is mirrored as one `board:<column>`
//!   label, and a Done card closes its issue. Other labels are left alone.
//! - **title and body are merged three ways** against the text both sides last
//!   agreed on (a hash stored on the card). Only one side changed it: that
//!   side wins. Both changed it: the board wins and the conflict is reported.
//!   Timestamps are not used: a comment, a claim or a label change moves
//!   them without touching the text.
//! - an issue created from a card carries a hidden marker with the card's id
//!   and the hash of the text it was created with, so a run interrupted between
//!   creating the issue and linking it **adopts** that issue next time instead
//!   of creating a duplicate, with the creation text as the common baseline (an
//!   edit made on the forge in between is then pulled, not overwritten). With no
//!   proven baseline, a difference is reported and neither side is touched.
//! - unlinked **open** issues become Backlog cards; closed ones are not imported.
//! - an issue closed on the forge while its card is not Done is reported as
//!   drift, never reopened and never moved (moving to Done needs review).
//!
//! The sync actor holds no capabilities: it can create, edit and link cards
//! but cannot move them.

use std::collections::{BTreeSet, HashMap, HashSet};
use std::fmt;

use uuid::Uuid;

use crate::api::Shared;
use crate::domain::{Actor, BoardError, Card, Column, import_id, reserved_tag, text_hash};

/// Prefix of the label that mirrors a card's column.
pub const BOARD_PREFIX: &str = "board:";

/// An issue as the forge reports it (pull requests excluded by the adapter).
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct Issue {
    /// Issue number.
    pub number: u64,
    /// Title.
    pub title: String,
    /// Body, including the hidden card marker if the issue has one.
    pub body: String,
    /// All labels.
    pub labels: Vec<String>,
    /// Closed?
    pub closed: bool,
}

/// What to write to an issue.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct Draft {
    /// Title.
    pub title: String,
    /// Body, with the hidden card marker if the issue is to carry one.
    pub body: String,
    /// Full label set.
    pub labels: Vec<String>,
    /// Closed?
    pub closed: bool,
}

/// The forge port.
pub trait Forge {
    /// Every issue, open and closed.
    fn list_issues(&self) -> Result<Vec<Issue>, SyncError>;
    /// Create an issue.
    fn create_issue(&self, draft: &Draft) -> Result<Issue, SyncError>;
    /// Overwrite an issue.
    fn update_issue(&self, number: u64, draft: &Draft) -> Result<Issue, SyncError>;
}

/// Why a sync run failed.
#[derive(Debug)]
pub enum SyncError {
    /// The forge refused or was unreachable.
    Forge(String),
    /// The board failed.
    Board(BoardError),
}

impl fmt::Display for SyncError {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        match self {
            Self::Forge(why) => write!(f, "forge: {why}"),
            Self::Board(e) => write!(f, "board: {e}"),
        }
    }
}

impl std::error::Error for SyncError {}

impl From<BoardError> for SyncError {
    fn from(e: BoardError) -> Self {
        Self::Board(e)
    }
}

/// What one run did.
#[derive(Debug, Default, PartialEq, Eq)]
pub struct Report {
    /// Issues created for unlinked cards.
    pub issues_created: usize,
    /// Existing issues recognised (by their marker) as made for an unlinked card, and linked.
    pub issues_adopted: usize,
    /// Issues updated to match their cards.
    pub issues_updated: usize,
    /// Cards created from unlinked open issues.
    pub cards_imported: usize,
    /// Cards whose title/body were taken from an issue that changed them.
    pub cards_pulled: usize,
    /// Issues whose text and card text both changed since the last sync (the board's won).
    pub conflicts: Vec<u64>,
    /// Issues whose text differs from the card's with no proven common baseline: neither side was touched.
    pub unresolved: Vec<u64>,
    /// Cards left for the next run because they changed after this run read them.
    pub deferred: usize,
    /// Issues closed on the forge whose card is not Done.
    pub drift: Vec<u64>,
    /// Linked issue numbers the forge no longer lists.
    pub missing: Vec<u64>,
}

impl Report {
    /// Nothing changed on either side and nothing waits for a person to reconcile.
    pub fn is_quiet(&self) -> bool {
        self.unresolved.is_empty()
            && self.issues_created
                + self.issues_adopted
                + self.issues_updated
                + self.cards_imported
                + self.cards_pulled
                == 0
    }
}

const MARKER_START: &str = "<!-- mill-board:card=";
const MARKER_END: &str = " -->";

/// The hidden marker an issue made from a card carries.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct Marker {
    /// The card the issue was made from.
    pub card: Uuid,
    /// [`text_hash`] of the title and body it was created with: the common
    /// baseline if the run is interrupted before the link is saved. Absent on
    /// markers written without one.
    pub text: Option<u64>,
}

/// `body` with the hidden marker appended.
fn with_marker(body: &str, marker: Marker) -> String {
    let text = marker
        .text
        .map_or(String::new(), |t| format!(" text={t:016x}"));
    format!(
        "{}\n\n{MARKER_START}{}{text}{MARKER_END}",
        body.trim_end(),
        marker.card
    )
}

fn parse_marker(inner: &str) -> Option<Marker> {
    let mut parts = inner.split(' ');
    let card = Uuid::parse_str(parts.next()?).ok()?;
    let text = match parts.next() {
        None => None,
        Some(rest) => Some(u64::from_str_radix(rest.strip_prefix("text=")?, 16).ok()?),
    };
    parts.next().is_none().then_some(Marker { card, text })
}

/// The body without its marker, and the marker, if any. Only the marker is
/// removed: text before and after it is kept (a person may type below it).
/// An unparseable lookalike is just text and is left alone.
fn split_marker(body: &str) -> (String, Option<Marker>) {
    let mut end = body.len();
    while let Some(start) = body[..end].rfind(MARKER_START) {
        let after = &body[start + MARKER_START.len()..];
        let parsed = after
            .find(MARKER_END)
            .and_then(|e| Some((e, parse_marker(&after[..e])?)));
        if let Some((e, marker)) = parsed {
            let before = body[..start].trim_end();
            // Drop only the blank-line separator: indentation of the first line is content
            // (an indented Markdown code block) and must survive a pull.
            let rest = body[start + MARKER_START.len() + e + MARKER_END.len()..]
                .trim_start_matches(['\n', '\r']);
            let clean = match (before.is_empty(), rest.is_empty()) {
                (_, true) => before.to_owned(),
                (true, false) => rest.to_owned(),
                (false, false) => format!("{before}\n\n{rest}"),
            };
            return (clean, Some(marker));
        }
        end = start;
    }
    (body.to_owned(), None)
}

fn column_label(column: &Column) -> String {
    format!("{BOARD_PREFIX}{column}")
}

/// Text hash of an issue, ignoring the marker.
fn issue_text(issue: &Issue) -> u64 {
    text_hash(&issue.title, &split_marker(&issue.body).0)
}

/// The issue a card should be, given what the forge already has (if any):
/// foreign labels are kept, the column label is replaced, missing tags added.
/// `text` is the title and body to write; `marker` the card id to embed.
fn draft(
    card: &Card,
    existing: Option<&Issue>,
    text: (&str, &str),
    marker: Option<Marker>,
) -> Draft {
    let mut labels: Vec<String> = existing
        .map(|i| {
            i.labels
                .iter()
                .filter(|l| !l.starts_with(BOARD_PREFIX))
                .cloned()
                .collect()
        })
        .unwrap_or_default();
    labels.push(column_label(&card.column));
    for tag in &card.tags {
        if !labels.contains(tag) {
            labels.push(tag.clone());
        }
    }
    Draft {
        title: text.0.to_owned(),
        body: marker.map_or_else(|| text.1.to_owned(), |m| with_marker(text.1, m)),
        labels,
        closed: card.column == Column::DONE || existing.is_some_and(|i| i.closed),
    }
}

fn same(draft: &Draft, issue: &Issue) -> bool {
    let set = |v: &[String]| v.iter().cloned().collect::<BTreeSet<_>>();
    text_hash(&draft.title, &split_marker(&draft.body).0) == issue_text(issue)
        && draft.closed == issue.closed
        && set(&draft.labels) == set(&issue.labels)
}

/// Reconcile `board` with `forge`.
///
/// # Errors
/// On the first forge or board failure; earlier fixes stay applied and the
/// next run continues from there.
pub fn reconcile(board: &Shared, forge: &dyn Forge) -> Result<Report, SyncError> {
    let actor = Actor {
        name: "github-sync".into(),
        caps: BTreeSet::new(),
        projects: None,
    };
    let issues = forge.list_issues()?;
    let by_number: HashMap<u64, &Issue> = issues.iter().map(|i| (i.number, i)).collect();
    let origins = Origins {
        by_marker: issues
            .iter()
            .filter_map(|i| split_marker(&i.body).1.map(|m| (m.card, (i, m))))
            .collect(),
        by_import: issues.iter().map(|i| (import_id(i.number), i)).collect(),
    };
    let cards = board.list(None)?;
    let card_ids: HashSet<Uuid> = cards.iter().map(|c| c.id).collect();
    let mut linked: HashSet<u64> = cards.iter().filter_map(|c| c.issue).collect();
    let mut report = Report::default();

    for card in cards {
        let Some(number) = card.issue else {
            link_or_create(
                board,
                forge,
                &actor,
                &card,
                &origins,
                &mut linked,
                &mut report,
            )?;
            continue;
        };
        match by_number.get(&number) {
            Some(issue) => sync_pair(board, forge, &actor, card, issue, &mut report)?,
            None => report.missing.push(number),
        }
    }

    for issue in issues
        .iter()
        .filter(|i| !i.closed && !linked.contains(&i.number))
    {
        let (body, marker) = split_marker(&issue.body);
        // An issue made from a card (even one that has since been linked elsewhere) is not new work.
        if marker.is_some_and(|m| card_ids.contains(&m.card)) {
            continue;
        }
        let tags = issue
            .labels
            .iter()
            .filter(|l| !reserved_tag(l))
            .cloned()
            .collect();
        // An import's card id is derived from the issue, so a conflict means it was imported
        // before: its card was since relinked, or (rusty_tick) taken off the board by the owner.
        match board.import(&actor, &issue.title, &body, tags, issue.number) {
            Ok(_) => report.cards_imported += 1,
            Err(BoardError::Conflict) => {}
            Err(e) => return Err(e.into()),
        }
    }
    Ok(report)
}

/// Where an unlinked card's issue may already be: one made for it (its marker
/// names the card), or the one it was imported from (its id names the issue).
struct Origins<'a> {
    by_marker: HashMap<Uuid, (&'a Issue, Marker)>,
    by_import: HashMap<Uuid, &'a Issue>,
}

/// An unlinked card: adopt the issue an interrupted run made for it, or the one it
/// was imported from, else create one.
fn link_or_create(
    board: &Shared,
    forge: &dyn Forge,
    actor: &Actor,
    card: &Card,
    origins: &Origins<'_>,
    linked: &mut HashSet<u64>,
    report: &mut Report,
) -> Result<(), SyncError> {
    // A card imported from an issue whose link was not committed with it (a store that
    // writes the card and its link separately): its id names the issue. No baseline is
    // known, so none is recorded and a difference stays unresolved.
    if let Some(issue) = origins
        .by_import
        .get(&card.id)
        .filter(|i| !linked.contains(&i.number))
    {
        board.link_issue(card.id, issue.number, None, actor)?;
        linked.insert(issue.number);
        report.issues_adopted += 1;
        return Ok(());
    }
    if let Some((issue, marker)) = origins
        .by_marker
        .get(&card.id)
        .filter(|(i, _)| !linked.contains(&i.number))
    {
        // The baseline is the text the issue was *created* with (recorded in its marker),
        // not what it says now: an edit made on the forge since then is a change to merge,
        // not the starting point. A marker without that hash proves no baseline, so none
        // is recorded and a difference stays unresolved rather than being overwritten.
        board.link_issue(card.id, issue.number, marker.text, actor)?;
        linked.insert(issue.number);
        report.issues_adopted += 1;
        return Ok(());
    }
    let created = text_hash(&card.title, &card.body);
    let marker = Marker {
        card: card.id,
        text: Some(created),
    };
    let issue = forge.create_issue(&draft(card, None, (&card.title, &card.body), Some(marker)))?;
    board.link_issue(card.id, issue.number, Some(created), actor)?;
    linked.insert(issue.number);
    report.issues_created += 1;
    Ok(())
}

/// A linked card and its issue: merge the text, then bring labels and state in line.
fn sync_pair(
    board: &Shared,
    forge: &dyn Forge,
    actor: &Actor,
    mut card: Card,
    issue: &Issue,
    report: &mut Report,
) -> Result<(), SyncError> {
    let (issue_body, marker) = split_marker(&issue.body);
    let (card_hash, issue_hash) = (text_hash(&card.title, &card.body), issue_text(issue));
    let mut push_text = false;
    if card_hash == issue_hash {
        if card.synced_text != Some(card_hash) {
            card = board.mark_synced(card.id, card_hash, actor)?;
        }
    } else {
        match card.synced_text {
            // Only the board changed the text since the last sync.
            Some(base) if base == issue_hash => push_text = true,
            // Only the forge did: take it, but only if the card is still as this run read it.
            // A board edit that landed since must not be overwritten by a decision made
            // from the older snapshot; the next run sees both changes and merges them.
            Some(base) if base == card_hash => {
                match board.edit_at(
                    card.id,
                    actor,
                    Some(card.rev),
                    Some(&issue.title),
                    Some(&issue_body),
                    None,
                ) {
                    Ok(edited) => card = edited,
                    Err(BoardError::Conflict) => {
                        report.deferred += 1;
                        return Ok(());
                    }
                    Err(e) => return Err(e.into()),
                }
                card = board.mark_synced(card.id, issue_hash, actor)?;
                report.cards_pulled += 1;
            }
            // Both changed it since the agreed text: the board owns state.
            Some(_) => {
                push_text = true;
                report.conflicts.push(issue.number);
            }
            // No proven common text (a marker without a baseline): either side could be
            // the newer one, so change neither and say so.
            None => report.unresolved.push(issue.number),
        }
    }
    if issue.closed && card.column != Column::DONE {
        report.drift.push(issue.number);
    }
    let text = if push_text {
        (card.title.as_str(), card.body.as_str())
    } else {
        (issue.title.as_str(), issue_body.as_str())
    };
    let want = draft(&card, Some(issue), text, marker);
    if !same(&want, issue) {
        forge.update_issue(issue.number, &want)?;
        report.issues_updated += 1;
    }
    if push_text {
        board.mark_synced(card.id, text_hash(&card.title, &card.body), actor)?;
    }
    Ok(())
}

#[cfg(test)]
mod tests {
    use std::cell::{Cell, RefCell};
    use std::sync::Arc;
    use std::sync::atomic::{AtomicBool, AtomicUsize, Ordering};

    use super::*;
    use crate::board::{Board, Roles};
    use crate::store::{MemStore, Store};

    /// An in-memory forge. Issues are numbered from 1.
    #[derive(Default)]
    struct Fake {
        issues: RefCell<Vec<Issue>>,
        /// Create the issue, then report failure (a lost response).
        lose_create_response: Cell<bool>,
    }

    impl Fake {
        fn put(&self, number: u64, title: &str, body: &str, labels: &[&str], closed: bool) {
            self.issues.borrow_mut().push(Issue {
                number,
                title: title.into(),
                body: body.into(),
                labels: labels.iter().map(|s| (*s).into()).collect(),
                closed,
            });
        }
        fn get(&self, number: u64) -> Issue {
            self.issues
                .borrow()
                .iter()
                .find(|i| i.number == number)
                .cloned()
                .expect("issue")
        }
        fn edit(&self, number: u64, f: impl FnOnce(&mut Issue)) {
            f(self
                .issues
                .borrow_mut()
                .iter_mut()
                .find(|i| i.number == number)
                .expect("issue"));
        }
        fn count(&self) -> usize {
            self.issues.borrow().len()
        }
    }

    impl Forge for Fake {
        fn list_issues(&self) -> Result<Vec<Issue>, SyncError> {
            Ok(self.issues.borrow().clone())
        }
        fn create_issue(&self, d: &Draft) -> Result<Issue, SyncError> {
            let number = self.issues.borrow().len() as u64 + 1;
            self.put(number, "", "", &[], false);
            let issue = self.update_issue(number, d)?;
            if self.lose_create_response.get() {
                return Err(SyncError::Forge(
                    "connection reset after the issue was created".into(),
                ));
            }
            Ok(issue)
        }
        fn update_issue(&self, number: u64, d: &Draft) -> Result<Issue, SyncError> {
            let mut all = self.issues.borrow_mut();
            let i = all.iter_mut().find(|i| i.number == number).expect("issue");
            (i.title, i.body, i.labels, i.closed) =
                (d.title.clone(), d.body.clone(), d.labels.clone(), d.closed);
            Ok(i.clone())
        }
    }

    fn board() -> Shared {
        Board::new(Box::new(MemStore::default()), Roles::default())
    }

    fn human(b: &Shared) -> Actor {
        b.roles().actor("nano", "human").expect("role")
    }

    /// A marker with no creation baseline.
    fn plain(card: Uuid) -> Marker {
        Marker { card, text: None }
    }

    fn marker(card: Uuid, text: u64) -> Marker {
        Marker {
            card,
            text: Some(text),
        }
    }

    /// A store that fails its Nth commit, standing in for a crash between two steps.
    struct FailNth {
        inner: MemStore,
        at: usize,
        count: AtomicUsize,
    }

    impl FailNth {
        fn tick(&self) -> Result<(), BoardError> {
            if self.count.fetch_add(1, Ordering::SeqCst) + 1 == self.at {
                return Err(BoardError::Storage("injected failure".into()));
            }
            Ok(())
        }
    }

    impl Store for FailNth {
        fn get(&self, id: Uuid) -> Result<Option<Card>, BoardError> {
            self.inner.get(id)
        }
        fn list(&self) -> Result<Vec<Card>, BoardError> {
            self.inner.list()
        }
        fn insert(&self, card: &Card, event: crate::domain::Event) -> Result<u64, BoardError> {
            self.tick()?;
            self.inner.insert(card, event)
        }
        fn replace(&self, card: &Card, event: crate::domain::Event) -> Result<u64, BoardError> {
            self.tick()?;
            self.inner.replace(card, event)
        }
        fn events_since(
            &self,
            a: Option<Uuid>,
            l: usize,
        ) -> Result<Vec<crate::domain::Event>, BoardError> {
            self.inner.events_since(a, l)
        }
        fn last_event_id(&self) -> Result<Option<Uuid>, BoardError> {
            self.inner.last_event_id()
        }
        fn reserve_number(&self, card: Uuid, floor: u64) -> Result<u64, BoardError> {
            self.inner.reserve_number(card, floor)
        }
    }

    #[test]
    fn card_becomes_issue_then_run_is_quiet() {
        let b = board();
        let card = b
            .add(&human(&b), "Ship", "body", vec!["bbp".into()])
            .expect("add");
        let forge = Fake::default();

        let r = reconcile(&b, &forge).expect("sync");
        assert_eq!((r.issues_created, r.issues_updated), (1, 0));
        let issue = forge.get(1);
        assert_eq!(issue.labels, ["board:backlog", "bbp"]);
        assert_eq!(
            split_marker(&issue.body),
            (
                "body".to_owned(),
                Some(marker(card.id, text_hash("Ship", "body")))
            ),
            "the marker names the card and the text it was created with"
        );
        let linked = b.get(card.id).expect("card");
        assert_eq!(
            (linked.issue, linked.synced_text),
            (Some(1), Some(text_hash("Ship", "body")))
        );

        assert!(
            reconcile(&b, &forge).expect("again").is_quiet(),
            "idempotent"
        );
    }

    #[test]
    fn column_label_follows_card_and_foreign_labels_survive() {
        let b = board();
        let h = human(&b);
        let card = b.add(&h, "Ship", "", vec![]).expect("add");
        let forge = Fake::default();
        reconcile(&b, &forge).expect("sync");
        forge.edit(1, |i| i.labels.push("bug".into()));
        b.move_card(card.id, &Column::READY, &h).expect("move");

        let r = reconcile(&b, &forge).expect("sync");
        assert_eq!(r.issues_updated, 1);
        assert_eq!(forge.get(1).labels, ["bug", "board:ready"]);
    }

    #[test]
    fn done_closes_the_issue() {
        let b = board();
        let h = human(&b);
        let card = b.add(&h, "Ship", "", vec![]).expect("add");
        let forge = Fake::default();
        reconcile(&b, &forge).expect("sync");
        b.move_card(card.id, &Column::READY, &h).expect("move");
        b.move_card(card.id, &Column::BUILDING, &h).expect("move");
        for to in [Column::QA, Column::REVIEW, Column::DONE] {
            b.claim(card.id, &h).expect("claim");
            b.move_card(card.id, &to, &h).expect("move");
        }
        reconcile(&b, &forge).expect("sync");
        assert!(forge.get(1).closed);
        assert_eq!(forge.get(1).labels, ["board:done"]);
    }

    #[test]
    fn open_issues_are_imported_closed_are_not() {
        let b = board();
        let forge = Fake::default();
        forge.put(5, "Open one", "details", &["bug", "board:ready"], false);
        forge.put(6, "Old", "", &[], true);

        let r = reconcile(&b, &forge).expect("sync");
        assert_eq!(r.cards_imported, 1);
        let cards = b.list(None).expect("list");
        assert_eq!(cards.len(), 1);
        assert_eq!(
            (
                cards[0].title.as_str(),
                cards[0].issue,
                cards[0].tags.clone()
            ),
            ("Open one", Some(5), vec!["bug".to_owned()])
        );
        assert_eq!(cards[0].synced_text, Some(text_hash("Open one", "details")));
        // The import lands in Backlog, so its label is corrected rather than trusted.
        let r = reconcile(&b, &forge).expect("sync");
        assert_eq!(r.issues_updated, 1);
        assert_eq!(forge.get(5).labels, ["bug", "board:backlog"]);
        assert!(reconcile(&b, &forge).expect("again").is_quiet());
    }

    #[test]
    fn an_import_whose_link_was_lost_is_relinked_not_exported() {
        let store = MemStore::default();
        let mut card = Card::new("Open one", "details", vec![], 1).expect("card");
        card.id = import_id(5);
        store
            .insert(
                &card,
                crate::domain::Event::of(&card, crate::domain::EventKind::Created, "x"),
            )
            .expect("insert");
        let b: Shared = Board::new(Box::new(store), Roles::default());
        let forge = Fake::default();
        forge.put(5, "Open one", "details", &[], false);

        let r = reconcile(&b, &forge).expect("sync");
        assert_eq!(
            (r.issues_created, r.issues_adopted, r.cards_imported),
            (0, 1, 0)
        );
        assert_eq!(b.get(card.id).expect("card").issue, Some(5));
        assert_eq!(b.list(None).expect("list").len(), 1);
        assert_eq!(forge.issues.borrow().len(), 1, "no issue was created");
    }

    #[test]
    fn an_issue_imported_before_is_not_imported_again() {
        let b = board();
        let forge = Fake::default();
        forge.put(5, "Open one", "", &[], false);
        reconcile(&b, &forge).expect("sync");
        let card = b.list(None).expect("list")[0].id;
        // Its card now tracks another issue, so issue 5 looks unimported; its derived card id says otherwise.
        forge.put(7, "Other", "", &[], false);
        b.link_issue(card, 7, None, &human(&b)).expect("relink");

        let r = reconcile(&b, &forge).expect("sync");
        assert_eq!(r.cards_imported, 0);
        assert_eq!(b.list(None).expect("list").len(), 1);
    }

    // ---- the text merge (review: timestamps are moved by unrelated activity)

    #[test]
    fn a_forge_title_edit_survives_later_board_activity() {
        let b = board();
        let h = human(&b);
        let card = b.add(&h, "Old", "body", vec![]).expect("add");
        let forge = Fake::default();
        reconcile(&b, &forge).expect("sync");

        forge.edit(1, |i| i.title = "Edited on GitHub".into()); // t2
        b.comment(card.id, &h, "unrelated board activity")
            .expect("comment"); // t3, newer than t2
        b.move_card(card.id, &Column::READY, &h).expect("move");

        let r = reconcile(&b, &forge).expect("sync");
        assert_eq!((r.cards_pulled, r.conflicts.len()), (1, 0));
        assert_eq!(
            b.get(card.id).expect("card").title,
            "Edited on GitHub",
            "the newer GitHub title wins"
        );
        assert_eq!(forge.get(1).title, "Edited on GitHub");
        assert!(reconcile(&b, &forge).expect("again").is_quiet());
    }

    #[test]
    fn a_board_edit_survives_an_unrelated_forge_change() {
        let b = board();
        let h = human(&b);
        let card = b.add(&h, "Old", "body", vec![]).expect("add");
        let forge = Fake::default();
        reconcile(&b, &forge).expect("sync");

        b.edit(card.id, &h, Some("Edited on the board"), None, None)
            .expect("edit");
        forge.edit(1, |i| i.labels.push("triaged".into())); // makes the issue "newer", text unchanged

        let r = reconcile(&b, &forge).expect("sync");
        assert_eq!((r.cards_pulled, r.conflicts.len()), (0, 0));
        assert_eq!(
            forge.get(1).title,
            "Edited on the board",
            "not overwritten by the older GitHub title"
        );
        assert!(forge.get(1).labels.contains(&"triaged".to_owned()));
        assert!(reconcile(&b, &forge).expect("again").is_quiet());
    }

    #[test]
    fn when_both_sides_changed_the_text_the_board_wins_and_it_is_reported() {
        let b = board();
        let h = human(&b);
        let card = b.add(&h, "Old", "body", vec![]).expect("add");
        let forge = Fake::default();
        reconcile(&b, &forge).expect("sync");
        b.edit(card.id, &h, Some("Board version"), None, None)
            .expect("edit");
        forge.edit(1, |i| i.title = "GitHub version".into());

        let r = reconcile(&b, &forge).expect("sync");
        assert_eq!(r.conflicts, [1]);
        assert_eq!(forge.get(1).title, "Board version");
        assert_eq!(b.get(card.id).expect("card").title, "Board version");
        assert!(
            reconcile(&b, &forge).expect("again").is_quiet(),
            "settled after the conflict"
        );
    }

    #[test]
    fn forge_line_ending_and_whitespace_changes_are_not_edits() {
        let b = board();
        let h = human(&b);
        b.add(&h, "T", "line one\nline two", vec![]).expect("add");
        let forge = Fake::default();
        reconcile(&b, &forge).expect("sync");
        forge.edit(1, |i| i.body = i.body.replace('\n', "\r\n") + "\r\n");
        assert!(
            reconcile(&b, &forge).expect("sync").is_quiet(),
            "GitHub's CRLF and trailing newline are not changes"
        );
    }

    #[test]
    fn a_pulled_body_does_not_carry_the_marker() {
        let b = board();
        let h = human(&b);
        let card = b.add(&h, "T", "old", vec![]).expect("add");
        let forge = Fake::default();
        reconcile(&b, &forge).expect("sync");
        forge.edit(1, |i| i.body = with_marker("new body", plain(card.id)));
        reconcile(&b, &forge).expect("sync");
        assert_eq!(b.get(card.id).expect("card").body, "new body");
    }

    // ---- idempotent creation (review: a crash between create and link duplicated issues)

    #[test]
    fn a_crash_between_creating_the_issue_and_linking_it_does_not_duplicate() {
        let store = FailNth {
            inner: MemStore::default(),
            at: 2,
            count: 0.into(),
        }; // 1 = add, 2 = the link
        let b: Shared = Board::new(Box::new(store), Roles::default());
        let card = b.add(&human(&b), "Ship", "body", vec![]).expect("add");
        let forge = Fake::default();

        assert!(
            reconcile(&b, &forge).is_err(),
            "the link failed after the issue was created"
        );
        assert_eq!(
            (forge.count(), b.get(card.id).expect("card").issue),
            (1, None)
        );

        let r = reconcile(&b, &forge).expect("retry");
        assert_eq!(
            (r.issues_created, r.issues_adopted),
            (0, 1),
            "adopted, not created again"
        );
        assert_eq!(forge.count(), 1, "still one issue");
        assert_eq!(
            b.list(None).expect("list").len(),
            1,
            "and no duplicate card imported from it"
        );
        assert_eq!(b.get(card.id).expect("card").issue, Some(1));
        assert!(reconcile(&b, &forge).expect("again").is_quiet());
    }

    #[test]
    fn a_lost_create_response_does_not_duplicate() {
        let b = board();
        let card = b.add(&human(&b), "Ship", "", vec![]).expect("add");
        let forge = Fake::default();
        forge.lose_create_response.set(true);
        assert!(
            reconcile(&b, &forge).is_err(),
            "the forge made the issue but the reply was lost"
        );
        forge.lose_create_response.set(false);

        reconcile(&b, &forge).expect("retry");
        assert_eq!(forge.count(), 1);
        assert_eq!(b.get(card.id).expect("card").issue, Some(1));
        assert_eq!(b.list(None).expect("list").len(), 1);
    }

    #[test]
    fn text_edited_after_an_adopted_create_still_syncs() {
        let b = board();
        let h = human(&b);
        let card = b.add(&h, "Ship", "v1", vec![]).expect("add");
        let forge = Fake::default();
        // The issue exists from an interrupted run; meanwhile the board edited the text.
        forge.put(
            1,
            "Ship",
            &with_marker("v1", marker(card.id, text_hash("Ship", "v1"))),
            &["board:backlog"],
            false,
        );
        b.edit(card.id, &h, None, Some("v2"), None).expect("edit");

        reconcile(&b, &forge).expect("sync"); // adopts (baseline = the issue's v1)
        reconcile(&b, &forge).expect("sync"); // pushes v2
        assert_eq!(split_marker(&forge.get(1).body).0, "v2");
    }

    #[test]
    fn issues_made_from_a_card_are_never_imported_as_new_work() {
        let b = board();
        let card = b.add(&human(&b), "Ship", "", vec![]).expect("add");
        let forge = Fake::default();
        forge.put(9, "Ship", &with_marker("", plain(card.id)), &[], false);
        forge.put(4, "elsewhere", &with_marker("", plain(card.id)), &[], false); // a second one claiming the same card
        reconcile(&b, &forge).expect("sync");
        assert_eq!(
            b.list(None).expect("list").len(),
            1,
            "no card was imported from either issue"
        );
    }

    #[test]
    fn markers_round_trip_and_keep_the_text_around_them() {
        let id = Uuid::from_u128(7);
        let m = marker(id, 0xdead_beef);
        assert_eq!(
            split_marker(&with_marker("hello  \n", m)),
            ("hello".to_owned(), Some(m))
        );
        assert_eq!(
            split_marker(&with_marker("x", plain(id))),
            ("x".to_owned(), Some(plain(id)))
        );
        assert_eq!(split_marker("no marker"), ("no marker".to_owned(), None));

        // Text typed above, below or around the marker is kept; only the marker goes.
        let line = format!("{MARKER_START}{id} text=00000000deadbeef{MARKER_END}");
        for (body, clean) in [
            (format!("before\n\n{line}\n\nafter"), "before\n\nafter"),
            (format!("{line}\n\nonly after"), "only after"),
            (format!("only before\n\n{line}"), "only before"),
            (format!("a\n{line}\nb\nc"), "a\n\nb\nc"),
            // Leading indentation after the marker is content, not separator.
            (format!("{line}\n\n    code()\n"), "    code()\n"),
            (
                format!("intro\n\n{line}\n\n    code()"),
                "intro\n\n    code()",
            ),
        ] {
            assert_eq!(split_marker(&body), (clean.to_owned(), Some(m)), "{body}");
        }
    }

    #[test]
    fn malformed_markers_are_just_text_and_are_never_deleted() {
        let id = Uuid::from_u128(7);
        for bad in [
            "text <!-- mill-board:card=not-a-uuid -->".to_owned(),
            format!("text {MARKER_START}{id}"), // no closing
            format!("text {MARKER_START}{id} text=zz{MARKER_END}"), // bad hex
            format!("text {MARKER_START}{id} extra=1{MARKER_END}"), // unknown field
            format!("text {MARKER_START}{id} text=1 text=2{MARKER_END}"), // too many parts
            format!("text {MARKER_START}{MARKER_END}"), // empty
        ] {
            assert_eq!(split_marker(&bad), (bad.clone(), None), "{bad}");
        }
        // A broken marker after a good one does not hide it, and neither is lost.
        let good = with_marker("body", plain(id));
        let both = format!("{good}\n\nlater <!-- mill-board:card=oops -->");
        let (clean, found) = split_marker(&both);
        assert_eq!(found, Some(plain(id)));
        assert_eq!(clean, "body\n\nlater <!-- mill-board:card=oops -->");
    }

    #[test]
    fn text_written_below_the_marker_is_pulled_and_survives_later_pushes() {
        let b = board();
        let h = human(&b);
        let card = b.add(&h, "Ship", "body", vec![]).expect("add");
        let forge = Fake::default();
        reconcile(&b, &forge).expect("sync");

        // Someone types under the hidden marker in the issue.
        forge.edit(1, |i| i.body.push_str("\n\nadded below the marker"));
        let r = reconcile(&b, &forge).expect("sync");
        assert_eq!(r.cards_pulled, 1);
        assert_eq!(
            b.get(card.id).expect("card").body,
            "body\n\nadded below the marker",
            "the text after the marker is part of the issue's text"
        );

        // A later outbound sync must not delete it.
        b.edit(card.id, &h, Some("Ship it"), None, None)
            .expect("edit");
        reconcile(&b, &forge).expect("sync");
        let issue = forge.get(1);
        assert_eq!(issue.title, "Ship it");
        assert!(
            issue.body.contains("added below the marker"),
            "{}",
            issue.body
        );
        assert!(issue.body.contains(MARKER_START), "the marker is kept");
        assert!(reconcile(&b, &forge).expect("again").is_quiet());
    }

    #[test]
    fn an_indented_code_block_after_the_marker_keeps_its_indentation() {
        let b = board();
        let h = human(&b);
        let card = b.add(&h, "Ship", "body", vec![]).expect("add");
        let forge = Fake::default();
        reconcile(&b, &forge).expect("sync");

        forge.edit(1, |i| {
            i.body = i.body.replace("-->", "-->\n\n    let x = 1;\n    x");
        });
        assert_eq!(reconcile(&b, &forge).expect("sync").cards_pulled, 1);
        let pulled = b.get(card.id).expect("card").body;
        assert!(pulled.ends_with("\n\n    let x = 1;\n    x"), "{pulled:?}");

        b.edit(card.id, &h, Some("Ship it"), None, None)
            .expect("edit");
        reconcile(&b, &forge).expect("sync");
        assert!(
            forge.get(1).body.contains("\n\n    let x = 1;\n    x"),
            "a later push keeps the code block: {:?}",
            forge.get(1).body
        );
    }

    // ---- lost-create recovery keeps the creation baseline

    #[test]
    fn an_edit_made_on_the_forge_between_a_lost_create_and_its_recovery_is_pulled_not_overwritten()
    {
        let b = board();
        let card = b.add(&human(&b), "Ship", "v1", vec![]).expect("add");
        let forge = Fake::default();
        forge.lose_create_response.set(true);
        assert!(reconcile(&b, &forge).is_err(), "created, response lost");
        forge.lose_create_response.set(false);

        // Before the run recovers, someone edits the issue on GitHub (the marker stays in the body).
        forge.edit(1, |i| i.body = i.body.replace("v1", "v2"));
        assert_eq!(split_marker(&forge.get(1).body).0, "v2");

        let r = reconcile(&b, &forge).expect("recovery");
        assert_eq!(r.issues_adopted, 1);
        reconcile(&b, &forge).expect("next tick");

        assert_eq!(
            b.get(card.id).expect("card").body,
            "v2",
            "the forge edit wins: it is a change since creation"
        );
        assert_eq!(
            split_marker(&forge.get(1).body).0,
            "v2",
            "and GitHub still says v2"
        );
        assert_eq!(forge.count(), 1);
        assert!(reconcile(&b, &forge).expect("again").is_quiet());
    }

    #[test]
    fn if_both_sides_edited_during_a_lost_create_the_board_wins_and_it_is_reported() {
        let b = board();
        let h = human(&b);
        let card = b.add(&h, "Ship", "v1", vec![]).expect("add");
        let forge = Fake::default();
        forge.lose_create_response.set(true);
        assert!(reconcile(&b, &forge).is_err());
        forge.lose_create_response.set(false);
        forge.edit(1, |i| i.body = i.body.replace("v1", "forge v2"));
        b.edit(card.id, &h, None, Some("board v2"), None)
            .expect("edit");

        reconcile(&b, &forge).expect("recovery");
        let r = reconcile(&b, &forge).expect("next tick");
        assert_eq!(r.conflicts, [1]);
        assert_eq!(split_marker(&forge.get(1).body).0, "board v2");
    }

    #[test]
    fn recovery_without_a_proven_baseline_touches_neither_side() {
        let b = board();
        let card = b
            .add(&human(&b), "Ship", "board text", vec![])
            .expect("add");
        let forge = Fake::default();
        // An issue that names the card but records no creation text (an older or hand-made
        // marker), whose text has since diverged from the card's.
        forge.put(
            1,
            "Ship",
            &with_marker("forge text", plain(card.id)),
            &["board:backlog"],
            false,
        );

        let r = reconcile(&b, &forge).expect("sync");
        assert_eq!(r.issues_adopted, 1);
        let r = reconcile(&b, &forge).expect("sync");
        assert_eq!(r.unresolved, [1], "reported, not guessed at");
        assert!(
            !r.is_quiet(),
            "a pair that needs a person is never a quiet run"
        );
        assert_eq!(b.get(card.id).expect("card").body, "board text");
        assert_eq!(
            split_marker(&forge.get(1).body).0,
            "forge text",
            "neither side overwritten"
        );
        assert_eq!(r.issues_updated + r.cards_pulled, 0);

        // A person making the two agree resolves it for good.
        b.edit(card.id, &human(&b), None, Some("forge text"), None)
            .expect("edit");
        assert!(reconcile(&b, &forge).expect("sync").unresolved.is_empty());
        let settled = reconcile(&b, &forge).expect("sync");
        assert!(settled.is_quiet() && settled.unresolved.is_empty());
    }

    // ---- a pull must not clobber a board edit that landed after the run read the card

    /// A store that, right after the next `list` once armed, lands a board edit on the
    /// first card: the race between a run's snapshot and its pull.
    struct RaceAfterList {
        inner: MemStore,
        armed: Arc<AtomicBool>,
    }

    impl Store for RaceAfterList {
        fn get(&self, id: Uuid) -> Result<Option<Card>, BoardError> {
            self.inner.get(id)
        }
        fn list(&self) -> Result<Vec<Card>, BoardError> {
            let snapshot = self.inner.list()?;
            if self.armed.swap(false, Ordering::SeqCst) {
                let edited = snapshot[0]
                    .edit(Some("Edited on the board meanwhile"), None, None, 1)
                    .expect("edit");
                let event =
                    crate::domain::Event::of(&edited, crate::domain::EventKind::Edited, "nano");
                self.inner.replace(&edited, event).expect("concurrent edit");
            }
            Ok(snapshot)
        }
        fn insert(&self, card: &Card, event: crate::domain::Event) -> Result<u64, BoardError> {
            self.inner.insert(card, event)
        }
        fn replace(&self, card: &Card, event: crate::domain::Event) -> Result<u64, BoardError> {
            self.inner.replace(card, event)
        }
        fn events_since(
            &self,
            after: Option<Uuid>,
            limit: usize,
        ) -> Result<Vec<crate::domain::Event>, BoardError> {
            self.inner.events_since(after, limit)
        }
        fn last_event_id(&self) -> Result<Option<Uuid>, BoardError> {
            self.inner.last_event_id()
        }
        fn reserve_number(&self, card: Uuid, floor: u64) -> Result<u64, BoardError> {
            self.inner.reserve_number(card, floor)
        }
    }

    #[test]
    fn a_pull_decided_from_an_older_snapshot_does_not_overwrite_a_board_edit() {
        let armed = Arc::new(AtomicBool::new(false));
        let store = RaceAfterList {
            inner: MemStore::default(),
            armed: Arc::clone(&armed),
        };
        let b: Shared = Board::new(Box::new(store), Roles::default());
        let card = b.add(&human(&b), "Old", "body", vec![]).expect("add");
        let forge = Fake::default();
        reconcile(&b, &forge).expect("sync");

        forge.edit(1, |i| i.title = "From GitHub".into());
        armed.store(true, Ordering::SeqCst); // a board edit lands right after the next run reads the card

        let r = reconcile(&b, &forge).expect("sync");
        assert_eq!(
            (r.deferred, r.cards_pulled),
            (1, 0),
            "the stale pull is deferred"
        );
        assert_eq!(
            b.get(card.id).expect("card").title,
            "Edited on the board meanwhile",
            "the concurrent board edit is not overwritten"
        );

        // Next run sees both changes since the agreed text: the board wins, reported.
        let r = reconcile(&b, &forge).expect("sync");
        assert_eq!(r.conflicts, [1]);
        assert_eq!(forge.get(1).title, "Edited on the board meanwhile");
        assert!(reconcile(&b, &forge).expect("again").is_quiet());
    }

    #[test]
    fn forge_closed_issue_is_drift_not_reopened() {
        let b = board();
        let card = b.add(&human(&b), "Ship", "", vec![]).expect("add");
        let forge = Fake::default();
        reconcile(&b, &forge).expect("sync");
        forge.edit(1, |i| i.closed = true);

        let r = reconcile(&b, &forge).expect("sync");
        assert_eq!((r.drift.clone(), r.issues_updated), (vec![1], 0));
        assert!(forge.get(1).closed);
        assert_eq!(b.get(card.id).expect("card").column, Column::BACKLOG);
    }

    #[test]
    fn deleted_issue_is_reported() {
        let b = board();
        let card = b.add(&human(&b), "Ship", "", vec![]).expect("add");
        b.link_issue(card.id, 42, None, &human(&b)).expect("link");
        let r = reconcile(&b, &Fake::default()).expect("sync");
        assert_eq!(r.missing, [42]);
    }
}
