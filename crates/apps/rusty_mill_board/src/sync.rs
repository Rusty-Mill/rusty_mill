//! Forge sync: reconcile the board with a forge's issues.
//!
//! Stateless and idempotent: every run lists both sides and fixes the
//! difference, so a missed run or a crash heals on the next one. Policy:
//! - the **board owns state**: the column is mirrored as one `board:<column>`
//!   label, and a Done card closes its issue. Other labels are left alone.
//! - **title and body are last-write-wins** by modification time.
//! - unlinked **open** issues become Backlog cards; closed ones are not imported.
//! - an issue closed on the forge while its card is not Done is reported as
//!   drift, never reopened and never moved (moving to Done needs review).
//!
//! The sync actor holds no capabilities: it can create, edit and link cards
//! but cannot move them.

use std::collections::{BTreeSet, HashMap, HashSet};
use std::fmt;

use crate::api::Shared;
use crate::domain::{Actor, BoardError, Card, Column};

/// Prefix of the label that mirrors a card's column.
pub const BOARD_PREFIX: &str = "board:";

/// An issue as the forge reports it (pull requests excluded by the adapter).
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct Issue {
    /// Issue number.
    pub number: u64,
    /// Title.
    pub title: String,
    /// Body.
    pub body: String,
    /// All labels.
    pub labels: Vec<String>,
    /// Closed?
    pub closed: bool,
    /// Last modification, Unix ms.
    pub updated_ms: i64,
}

/// What to write to an issue.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct Draft {
    /// Title.
    pub title: String,
    /// Body.
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
    /// Issues updated to match their cards.
    pub issues_updated: usize,
    /// Cards created from unlinked open issues.
    pub cards_imported: usize,
    /// Cards whose title/body were taken from a newer issue.
    pub cards_pulled: usize,
    /// Issues closed on the forge whose card is not Done.
    pub drift: Vec<u64>,
    /// Linked issue numbers the forge no longer lists.
    pub missing: Vec<u64>,
}

impl Report {
    /// Nothing changed on either side.
    pub fn is_quiet(&self) -> bool {
        self.issues_created + self.issues_updated + self.cards_imported + self.cards_pulled == 0
    }
}

fn column_label(column: Column) -> String {
    let name = serde_json::to_value(column)
        .ok()
        .and_then(|v| v.as_str().map(str::to_owned))
        .unwrap_or_default();
    format!("{BOARD_PREFIX}{name}")
}

/// The issue a card should be, given what the forge already has (if any):
/// foreign labels are kept, the column label is replaced, missing tags added.
fn draft(card: &Card, existing: Option<&Issue>) -> Draft {
    let mut labels: Vec<String> = existing
        .map(|i| {
            i.labels
                .iter()
                .filter(|l| !l.starts_with(BOARD_PREFIX))
                .cloned()
                .collect()
        })
        .unwrap_or_default();
    labels.push(column_label(card.column));
    for tag in &card.tags {
        if !labels.contains(tag) {
            labels.push(tag.clone());
        }
    }
    Draft {
        title: card.title.clone(),
        body: card.body.clone(),
        labels,
        closed: card.column == Column::Done || existing.is_some_and(|i| i.closed),
    }
}

fn same(draft: &Draft, issue: &Issue) -> bool {
    let set = |v: &[String]| v.iter().cloned().collect::<BTreeSet<_>>();
    draft.title == issue.title
        && draft.body == issue.body
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
    };
    let issues = forge.list_issues()?;
    let by_number: HashMap<u64, &Issue> = issues.iter().map(|i| (i.number, i)).collect();
    let cards = board.list(None)?;
    let linked: HashSet<u64> = cards.iter().filter_map(|c| c.issue).collect();
    let mut report = Report::default();

    for card in cards {
        let Some(number) = card.issue else {
            let issue = forge.create_issue(&draft(&card, None))?;
            board.link_issue(card.id, issue.number, &actor)?;
            report.issues_created += 1;
            continue;
        };
        let Some(issue) = by_number.get(&number) else {
            report.missing.push(number);
            continue;
        };
        let mut card = card;
        if (card.title != issue.title || card.body != issue.body)
            && issue.updated_ms > card.updated_ms
        {
            card = board.edit(card.id, &actor, Some(&issue.title), Some(&issue.body), None)?;
            report.cards_pulled += 1;
        }
        if issue.closed && card.column != Column::Done {
            report.drift.push(number);
        }
        let want = draft(&card, Some(issue));
        if !same(&want, issue) {
            forge.update_issue(number, &want)?;
            report.issues_updated += 1;
        }
    }

    for issue in issues
        .iter()
        .filter(|i| !i.closed && !linked.contains(&i.number))
    {
        let tags = issue
            .labels
            .iter()
            .filter(|l| !l.starts_with(BOARD_PREFIX))
            .cloned()
            .collect();
        board.import(&actor, &issue.title, &issue.body, tags, issue.number)?;
        report.cards_imported += 1;
    }
    Ok(report)
}

/// Parse a GitHub timestamp (`2026-10-13T14:48:34Z`) to Unix milliseconds.
pub fn parse_iso_ms(s: &str) -> Option<i64> {
    let (date, time) = s.strip_suffix('Z')?.split_once('T')?;
    let mut d = date.splitn(3, '-').map(|p| p.parse::<i64>().ok());
    let (y, m, day) = (d.next()??, d.next()??, d.next()??);
    let mut t = time.splitn(3, ':').map(|p| p.parse::<i64>().ok());
    let (hh, mm, ss) = (t.next()??, t.next()??, t.next()??);
    // days from civil (Howard Hinnant's algorithm)
    let y = if m <= 2 { y - 1 } else { y };
    let era = y.div_euclid(400);
    let yoe = y - era * 400;
    let doy = (153 * ((m + 9) % 12) + 2) / 5 + day - 1;
    let doe = yoe * 365 + yoe / 4 - yoe / 100 + doy;
    let days = era * 146_097 + doe - 719_468;
    Some(((days * 24 + hh) * 60 + mm) * 60_000 + ss * 1000)
}

#[cfg(test)]
mod tests {
    use std::cell::RefCell;

    use super::*;
    use crate::board::{Board, Roles};
    use crate::store::MemStore;

    /// An in-memory forge. `now` stamps every write it receives.
    struct Fake {
        issues: RefCell<Vec<Issue>>,
        now: i64,
    }

    impl Fake {
        fn new(now: i64) -> Self {
            Self {
                issues: RefCell::new(Vec::new()),
                now,
            }
        }
        fn put(&self, number: u64, title: &str, labels: &[&str], closed: bool, updated_ms: i64) {
            self.issues.borrow_mut().push(Issue {
                number,
                title: title.into(),
                body: String::new(),
                labels: labels.iter().map(|s| (*s).into()).collect(),
                closed,
                updated_ms,
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
    }

    impl Forge for Fake {
        fn list_issues(&self) -> Result<Vec<Issue>, SyncError> {
            Ok(self.issues.borrow().clone())
        }
        fn create_issue(&self, d: &Draft) -> Result<Issue, SyncError> {
            let number = self.issues.borrow().len() as u64 + 1;
            self.put(number, &d.title, &[], d.closed, self.now);
            self.update_issue(number, d)
        }
        fn update_issue(&self, number: u64, d: &Draft) -> Result<Issue, SyncError> {
            let mut all = self.issues.borrow_mut();
            let i = all.iter_mut().find(|i| i.number == number).expect("issue");
            (i.title, i.body, i.labels, i.closed, i.updated_ms) = (
                d.title.clone(),
                d.body.clone(),
                d.labels.clone(),
                d.closed,
                self.now,
            );
            Ok(i.clone())
        }
    }

    fn board(clock: fn() -> i64) -> Shared {
        Board::with_clock(Box::new(MemStore::default()), Roles::default(), clock)
    }

    fn human(b: &Shared) -> Actor {
        b.roles().actor("nano", "human").expect("role")
    }

    #[test]
    fn card_becomes_issue_then_run_is_quiet() {
        let b = board(|| 100);
        let card = b
            .add(&human(&b), "Ship", "body", vec!["bbp".into()])
            .expect("add");
        let forge = Fake::new(1000);

        let r = reconcile(&b, &forge).expect("sync");
        assert_eq!((r.issues_created, r.issues_updated), (1, 0));
        let issue = forge.get(1);
        assert_eq!(issue.labels, ["board:backlog", "bbp"]);
        assert_eq!(b.get(card.id).expect("card").issue, Some(1));

        assert!(
            reconcile(&b, &forge).expect("again").is_quiet(),
            "idempotent"
        );
    }

    #[test]
    fn column_label_follows_card_and_foreign_labels_survive() {
        let b = board(|| 100);
        let h = human(&b);
        let card = b.add(&h, "Ship", "", vec![]).expect("add");
        let forge = Fake::new(1000);
        reconcile(&b, &forge).expect("sync");
        forge.issues.borrow_mut()[0].labels.push("bug".into());
        b.move_card(card.id, Column::Ready, &h).expect("move");

        let r = reconcile(&b, &forge).expect("sync");
        assert_eq!(r.issues_updated, 1);
        assert_eq!(forge.get(1).labels, ["bug", "board:ready"]);
    }

    #[test]
    fn done_closes_the_issue() {
        let b = board(|| 100);
        let h = human(&b);
        let card = b.add(&h, "Ship", "", vec![]).expect("add");
        let forge = Fake::new(1000);
        reconcile(&b, &forge).expect("sync");
        for to in [Column::Ready, Column::Building] {
            b.move_card(card.id, to, &h).expect("move");
        }
        b.claim(card.id, &h).expect("claim");
        for to in [Column::Qa] {
            b.move_card(card.id, to, &h).expect("move");
        }
        b.claim(card.id, &h).expect("claim");
        b.move_card(card.id, Column::Review, &h).expect("move");
        b.claim(card.id, &h).expect("claim");
        b.move_card(card.id, Column::Done, &h).expect("done");

        reconcile(&b, &forge).expect("sync");
        assert!(forge.get(1).closed);
        assert_eq!(forge.get(1).labels, ["board:done"]);
    }

    #[test]
    fn open_issues_are_imported_closed_are_not() {
        let b = board(|| 100);
        let forge = Fake::new(1000);
        forge.put(5, "Open one", &["bug", "board:ready"], false, 50);
        forge.put(6, "Old", &[], true, 50);

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
        // The import lands in Backlog, so its label is corrected rather than trusted.
        let r = reconcile(&b, &forge).expect("sync");
        assert_eq!(r.issues_updated, 1);
        assert_eq!(forge.get(5).labels, ["bug", "board:backlog"]);
        assert!(reconcile(&b, &forge).expect("again").is_quiet());
    }

    #[test]
    fn newer_side_wins_title() {
        // Issue newer: pulled into the card.
        let b = board(|| 100);
        let card = b.add(&human(&b), "Old", "", vec![]).expect("add");
        let forge = Fake::new(1000);
        reconcile(&b, &forge).expect("sync");
        forge.issues.borrow_mut()[0].title = "Edited on GitHub".into();
        forge.issues.borrow_mut()[0].updated_ms = 5000;
        let r = reconcile(&b, &forge).expect("sync");
        assert_eq!(r.cards_pulled, 1);
        assert_eq!(b.get(card.id).expect("card").title, "Edited on GitHub");

        // Card newer: pushed to the issue.
        let b = board(|| 9000);
        let card = b.add(&human(&b), "Fresh", "", vec![]).expect("add");
        let forge = Fake::new(1000);
        forge.put(1, "Stale", &["board:backlog"], false, 10);
        b.link_issue(card.id, 1, &human(&b)).expect("link");
        let r = reconcile(&b, &forge).expect("sync");
        assert_eq!((r.cards_pulled, r.issues_updated), (0, 1));
        assert_eq!(forge.get(1).title, "Fresh");
    }

    #[test]
    fn forge_closed_issue_is_drift_not_reopened() {
        let b = board(|| 100);
        let card = b.add(&human(&b), "Ship", "", vec![]).expect("add");
        let forge = Fake::new(1000);
        reconcile(&b, &forge).expect("sync");
        forge.issues.borrow_mut()[0].closed = true;

        let r = reconcile(&b, &forge).expect("sync");
        assert_eq!((r.drift.clone(), r.issues_updated), (vec![1], 0));
        assert!(forge.get(1).closed);
        assert_eq!(b.get(card.id).expect("card").column, Column::Backlog);
    }

    #[test]
    fn deleted_issue_is_reported() {
        let b = board(|| 100);
        let card = b.add(&human(&b), "Ship", "", vec![]).expect("add");
        b.link_issue(card.id, 42, &human(&b)).expect("link");
        let r = reconcile(&b, &Fake::new(1)).expect("sync");
        assert_eq!(r.missing, [42]);
    }

    #[test]
    fn iso_timestamps_parse() {
        assert_eq!(parse_iso_ms("1970-01-01T00:00:00Z"), Some(0));
        assert_eq!(
            parse_iso_ms("2026-10-13T14:48:34Z"),
            Some(1_791_902_914_000)
        );
        assert_eq!(parse_iso_ms("2000-02-29T23:59:59Z"), Some(951_868_799_000));
        assert_eq!(parse_iso_ms("nope"), None);
        assert_eq!(parse_iso_ms("2026-10-13 14:48:34"), None);
    }
}
