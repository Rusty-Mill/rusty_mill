//! Re-filing cards when the workflow's columns change: `qa` and `review` into `v_and_ving`,
//! say. Planning is pure ([`plan`]); [`apply`] and [`undo`] write through the [`Store`] port.
//!
//! The rollback record ([`RecordFile`]) is a JSON-lines file: the plan first, saved and
//! synced before any card changes, then a `begin` line before each write and a `done` line
//! after it, each synced. So a crash or a failed write midway still leaves on disk every
//! card's origin and which cards the retag touched. Each plan entry carries the card's
//! revision: [`apply`] retags a card only at that revision, and [`undo`] restores only a card
//! the record says the retag wrote (or began to write), still in the column it put it in at
//! the very next revision. A card anyone has changed since, or the retag never reached, is
//! left alone and reported.

use std::collections::{BTreeMap, BTreeSet};
use std::fs::{File, OpenOptions};
use std::io::Write;
use std::path::Path;

use serde::{Deserialize, Serialize};
use uuid::Uuid;

use crate::domain::{BoardError, Card, Column, Event, EventKind};
use crate::store::Store;

/// The actor recorded on retag events.
pub const ACTOR: &str = "retag";

/// One card to re-file.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct Retag {
    /// The card.
    pub id: Uuid,
    /// Its board number, for people reading the plan.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub number: Option<u64>,
    /// The card's revision when planned.
    pub rev: u64,
    /// The column it was in.
    pub from: Column,
    /// The column it goes to.
    pub to: Column,
}

/// What happened to one planned card.
#[derive(Debug, Clone, PartialEq, Eq, Serialize)]
#[serde(rename_all = "snake_case", tag = "outcome")]
pub enum Outcome {
    /// Written.
    Done {
        /// The card.
        id: Uuid,
    },
    /// Left alone, and why.
    Skipped {
        /// The card.
        id: Uuid,
        /// Why.
        why: String,
    },
}

/// Parse a mapping like `qa=v_and_ving,review=v_and_ving`.
///
/// # Errors
/// A malformed pair, an invalid column id, a repeated source, or a mapping that touches
/// Done or Blocked (their meaning is fixed, so their cards never change column this way).
pub fn parse_map(spec: &str) -> Result<BTreeMap<Column, Column>, String> {
    let mut map = BTreeMap::new();
    for pair in spec.split(',').map(str::trim).filter(|p| !p.is_empty()) {
        let (from, to) = pair
            .split_once('=')
            .ok_or_else(|| format!("{pair:?}: expected from=to"))?;
        let col = |id: &str| Column::new(id.trim()).map_err(|e| format!("{pair:?}: {e}"));
        let (from, to) = (col(from)?, col(to)?);
        if [&from, &to]
            .iter()
            .any(|c| **c == Column::DONE || **c == Column::BLOCKED)
        {
            return Err(format!(
                "{pair:?}: done and blocked cards keep their column"
            ));
        }
        if from == to {
            return Err(format!("{pair:?}: maps a column to itself"));
        }
        if map.insert(from, to).is_some() {
            return Err(format!("{pair:?}: column mapped twice"));
        }
    }
    if map.is_empty() {
        return Err("no columns to retag".into());
    }
    Ok(map)
}

/// The retags `map` makes on `cards`, ordered by card number then id.
pub fn plan(cards: &[Card], map: &BTreeMap<Column, Column>) -> Vec<Retag> {
    let mut out: Vec<Retag> = cards
        .iter()
        .filter_map(|c| {
            map.get(&c.column).map(|to| Retag {
                id: c.id,
                number: c.number,
                rev: c.rev,
                from: c.column.clone(),
                to: to.clone(),
            })
        })
        .collect();
    out.sort_by_key(|r| (r.number, r.id));
    out
}

/// One line of a rollback record.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum Line {
    /// The whole plan, the record's first line.
    Plan(Vec<Retag>),
    /// About to write this card.
    Begin(Uuid),
    /// This card's write landed.
    Done(Uuid),
}

/// A rollback record as read back: the plan and how far the retag got.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct Record {
    /// The plan.
    pub plan: Vec<Retag>,
    begun: BTreeSet<Uuid>,
    done: BTreeSet<Uuid>,
}

impl Record {
    /// Parse a record. A last line cut short by a crash is ignored: it was a progress mark
    /// whose write had not yet started or whose outcome the `begin` before it already covers.
    ///
    /// # Errors
    /// No plan first, or a malformed line before the last.
    pub fn parse(text: &str) -> Result<Self, String> {
        let lines: Vec<&str> = text.lines().filter(|l| !l.trim().is_empty()).collect();
        let mut parsed = Vec::new();
        for (i, l) in lines.iter().enumerate() {
            match serde_json::from_str::<Line>(l) {
                Ok(line) => parsed.push(line),
                Err(_) if i + 1 == lines.len() && i > 0 => {}
                Err(e) => return Err(format!("line {}: {e}", i + 1)),
            }
        }
        let mut parsed = parsed.into_iter();
        let Some(Line::Plan(plan)) = parsed.next() else {
            return Err("a retag record starts with its plan".into());
        };
        let (mut begun, mut done) = (BTreeSet::new(), BTreeSet::new());
        for line in parsed {
            match line {
                Line::Begin(id) => {
                    begun.insert(id);
                }
                Line::Done(id) => {
                    done.insert(id);
                }
                Line::Plan(_) => return Err("a retag record has one plan".into()),
            }
        }
        Ok(Self { plan, begun, done })
    }
}

/// A rollback record being written: created exclusively, every line synced.
pub struct RecordFile(File);

impl RecordFile {
    /// Publish `plan` at `path` as a new file, durably, refusing to replace anything there.
    ///
    /// The file is created exclusively (`create_new`), so of two runs naming the same record
    /// only one gets it, and an existing record is never touched. The plan is synced, then the
    /// directory, before this returns, and [`apply`] writes no card before that. Unix only:
    /// elsewhere the directory cannot be synced and this fails, so nothing is retagged.
    ///
    /// # Errors
    /// The path exists, the platform is not Unix, or any I/O error creating, writing or
    /// syncing it.
    pub fn create(path: &Path, plan: &[Retag]) -> Result<Self, String> {
        let file = OpenOptions::new()
            .write(true)
            .create_new(true)
            .open(path)
            .map_err(|e| e.to_string())?;
        let mut record = Self(file);
        record.mark(&Line::Plan(plan.to_vec()))?;
        sync_parent_dir(path).map_err(|e| format!("syncing its directory: {e}"))?;
        Ok(record)
    }

    /// Append `line` and sync it.
    ///
    /// # Errors
    /// Any I/O error writing or syncing.
    pub fn mark(&mut self, line: &Line) -> Result<(), String> {
        let mut text = serde_json::to_string(line).map_err(|e| e.to_string())?;
        text.push('\n');
        self.0
            .write_all(text.as_bytes())
            .map_err(|e| e.to_string())?;
        self.0.sync_all().map_err(|e| e.to_string())
    }
}

#[cfg(unix)]
fn sync_parent_dir(path: &Path) -> std::io::Result<()> {
    let dir = match path.parent() {
        Some(dir) if !dir.as_os_str().is_empty() => dir,
        _ => Path::new("."),
    };
    File::open(dir)?.sync_all()
}

/// Elsewhere this code cannot make a new file's directory entry durable, so it refuses to
/// write a record, and with it any retag.
#[cfg(not(unix))]
fn sync_parent_dir(_path: &Path) -> std::io::Result<()> {
    Err(std::io::Error::new(
        std::io::ErrorKind::Unsupported,
        "retag records are only made durable on Unix",
    ))
}

/// Retag each planned card still in its planned column at its planned revision, calling
/// `mark` with `begin` before and `done` after each write; a card is never written unless its
/// `begin` was recorded.
///
/// # Errors
/// The first storage or `mark` error. Cards before it are written and recorded; undo them
/// from the record.
pub fn apply(
    store: &dyn Store,
    plan: &[Retag],
    now_ms: i64,
    mark: &mut dyn FnMut(&Line) -> Result<(), String>,
) -> Result<Vec<Outcome>, String> {
    let mut out = Vec::new();
    for r in plan {
        let card = store.get(r.id).map_err(|e| e.to_string())?;
        let Some(card) = card else {
            out.push(skipped(r.id, "no longer on the board"));
            continue;
        };
        if card.column != r.from || card.rev != r.rev {
            out.push(skipped(r.id, &changed(&card, &r.from, r.rev)));
            continue;
        }
        mark(&Line::Begin(r.id))?;
        let next = card.retag(&r.to, now_ms);
        store
            .replace(&next, Event::of(&next, EventKind::Moved, ACTOR))
            .map_err(|e| e.to_string())?;
        mark(&Line::Done(r.id))?;
        out.push(Outcome::Done { id: r.id });
    }
    Ok(out)
}

/// Put back each card the record says the retag wrote that nobody has changed since: in
/// `to`, at the revision right after the planned one. A card whose write began but is not
/// recorded as done may or may not have been written, and someone may have moved it into
/// `to` since, so it is reported for a person to check, never moved.
///
/// # Errors
/// The first storage error. Cards before it are restored; running it again finishes the rest.
pub fn undo(store: &dyn Store, record: &Record, now_ms: i64) -> Result<Vec<Outcome>, BoardError> {
    let mut out = Vec::new();
    for r in &record.plan {
        if !record.begun.contains(&r.id) {
            out.push(skipped(r.id, "the retag never reached it"));
            continue;
        }
        let Some(card) = store.get(r.id)? else {
            out.push(skipped(r.id, "no longer on the board"));
            continue;
        };
        if !record.done.contains(&r.id) {
            let why = format!(
                "its retag's outcome is unknown; it is in {} at revision {} (planned {} in {}): check it and move it by hand",
                card.column, card.rev, r.rev, r.from
            );
            out.push(skipped(r.id, &why));
            continue;
        }
        if card.column != r.to || card.rev != r.rev + 1 {
            out.push(skipped(r.id, &changed(&card, &r.to, r.rev + 1)));
            continue;
        }
        let back = card.retag(&r.from, now_ms);
        store.replace(&back, Event::of(&back, EventKind::Moved, ACTOR))?;
        out.push(Outcome::Done { id: r.id });
    }
    Ok(out)
}

fn skipped(id: Uuid, why: &str) -> Outcome {
    Outcome::Skipped {
        id,
        why: why.to_owned(),
    }
}

fn changed(card: &Card, column: &Column, rev: u64) -> String {
    if card.rev == rev {
        format!("in {}, not {column}", card.column)
    } else {
        format!(
            "changed since (revision {}, expected {rev}; now in {})",
            card.rev, card.column
        )
    }
}

#[cfg(test)]
mod tests {
    use std::sync::atomic::{AtomicUsize, Ordering};

    use super::*;
    use crate::store::MemStore;

    fn col(id: &str) -> Column {
        Column::new(id).expect("id")
    }

    fn card_in(store: &dyn Store, column: &str, claim: Option<&str>) -> Card {
        let mut c = Card::new("t", "", vec![], 1).expect("card");
        c.column = col(column);
        c.claim = claim.map(str::to_owned);
        store
            .insert(&c, Event::of(&c, EventKind::Created, "t"))
            .expect("insert");
        c
    }

    fn column_of(store: &dyn Store, id: Uuid) -> Column {
        store.get(id).expect("get").expect("card").column
    }

    fn factory_plan(store: &dyn Store) -> Vec<Retag> {
        let map = parse_map("qa=v_and_ving,review=v_and_ving").expect("map");
        plan(&store.list().expect("list"), &map)
    }

    /// Apply with an in-memory journal; returns the result and the record it leaves.
    fn run(store: &dyn Store, plan: &[Retag]) -> (Result<Vec<Outcome>, String>, Record) {
        let mut lines = vec![Line::Plan(plan.to_vec())];
        let res = apply(store, plan, 2, &mut |l| {
            lines.push(l.clone());
            Ok(())
        });
        (res, record_of(&lines))
    }

    fn record_of(lines: &[Line]) -> Record {
        let text: String = lines
            .iter()
            .map(|l| serde_json::to_string(l).expect("json") + "\n")
            .collect();
        Record::parse(&text).expect("record")
    }

    fn move_by_hand(store: &dyn Store, id: Uuid, to: &str) {
        let c = store.get(id).expect("get").expect("card");
        let next = c.retag(&col(to), 9);
        store
            .replace(&next, Event::of(&next, EventKind::Moved, "someone"))
            .expect("move");
    }

    /// A store whose `replace` fails from the `fail_from`-th call on (0-based).
    struct Failing {
        inner: MemStore,
        calls: AtomicUsize,
        fail_from: usize,
    }

    impl Store for Failing {
        fn get(&self, id: Uuid) -> Result<Option<Card>, BoardError> {
            self.inner.get(id)
        }
        fn list(&self) -> Result<Vec<Card>, BoardError> {
            self.inner.list()
        }
        fn insert(&self, card: &Card, event: Event) -> Result<u64, BoardError> {
            self.inner.insert(card, event)
        }
        fn replace(&self, card: &Card, event: Event) -> Result<u64, BoardError> {
            if self.calls.fetch_add(1, Ordering::SeqCst) >= self.fail_from {
                return Err(BoardError::Storage("down".into()));
            }
            self.inner.replace(card, event)
        }
        fn events_since(&self, a: Option<Uuid>, l: usize) -> Result<Vec<Event>, BoardError> {
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
    fn parses_a_mapping_and_refuses_bad_ones() {
        let m = parse_map("qa=v_and_ving, review=v_and_ving").expect("map");
        assert_eq!(m.get(&col("qa")), Some(&col("v_and_ving")));
        assert_eq!(m.len(), 2);
        for bad in [
            "",
            "qa",
            "qa=QA",
            "qa=done",
            "blocked=ready",
            "qa=qa",
            "qa=a,qa=b",
        ] {
            assert!(parse_map(bad).is_err(), "{bad:?} should be refused");
        }
    }

    #[test]
    fn plans_only_mapped_columns_with_their_revision() {
        let store = MemStore::default();
        let qa = card_in(&store, "qa", None);
        let review = card_in(&store, "review", None);
        card_in(&store, "ready", None);
        let p = factory_plan(&store);
        let ids: Vec<Uuid> = p.iter().map(|r| r.id).collect();
        assert_eq!(p.len(), 2);
        assert!(ids.contains(&qa.id) && ids.contains(&review.id));
        assert!(
            p.iter()
                .all(|r| r.to == col("v_and_ving") && r.rev == qa.rev)
        );
    }

    #[test]
    fn apply_keeps_the_claim_and_undo_restores_each_origin() {
        let store = MemStore::default();
        let qa = card_in(&store, "qa", Some("tester"));
        let review = card_in(&store, "review", None);
        let (res, record) = run(&store, &factory_plan(&store));
        assert!(
            res.expect("apply")
                .iter()
                .all(|o| matches!(o, Outcome::Done { .. }))
        );
        let now = store.get(qa.id).expect("get").expect("card");
        assert_eq!(
            (now.column, now.claim.as_deref()),
            (col("v_and_ving"), Some("tester"))
        );
        undo(&store, &record, 3).expect("undo");
        assert_eq!(column_of(&store, qa.id), col("qa"));
        assert_eq!(column_of(&store, review.id), col("review"));
    }

    #[test]
    fn apply_skips_a_card_that_changed_after_planning() {
        let store = MemStore::default();
        let c = card_in(&store, "qa", None);
        let p = factory_plan(&store);
        move_by_hand(&store, c.id, "building");
        let (res, _) = run(&store, &p);
        assert!(matches!(res.expect("apply")[..], [Outcome::Skipped { .. }]));
        assert_eq!(column_of(&store, c.id), col("building"));
    }

    #[test]
    fn undo_leaves_a_card_worked_on_since_the_retag() {
        let store = MemStore::default();
        let c = card_in(&store, "review", None);
        let (_, record) = run(&store, &factory_plan(&store));
        move_by_hand(&store, c.id, "building");
        move_by_hand(&store, c.id, "v_and_ving");
        assert!(matches!(
            undo(&store, &record, 4).expect("undo")[..],
            [Outcome::Skipped { .. }]
        ));
        assert_eq!(column_of(&store, c.id), col("v_and_ving"));
    }

    #[test]
    fn undo_never_moves_a_card_the_retag_did_not_reach() {
        // The retag fails before the second card; later someone moves that card into the
        // retag's destination by hand, landing at exactly "planned revision + 1".
        let store = Failing {
            inner: MemStore::default(),
            calls: AtomicUsize::new(0),
            fail_from: 1,
        };
        card_in(&store, "qa", None);
        card_in(&store, "review", None);
        let p = factory_plan(&store);
        let (res, record) = run(&store, &p);
        assert!(res.is_err(), "the second write fails");
        let store = Failing {
            fail_from: usize::MAX,
            ..store
        };
        let (written, failed) = (&p[0], &p[1]);
        assert_eq!(column_of(&store, written.id), col("v_and_ving"));
        assert_eq!(column_of(&store, failed.id), failed.from);
        move_by_hand(&store, failed.id, "v_and_ving");
        let out = undo(&store, &record, 5).expect("undo");
        assert_eq!(
            column_of(&store, written.id),
            written.from,
            "written: put back"
        );
        assert_eq!(
            column_of(&store, failed.id),
            col("v_and_ving"),
            "begun, never done: reported, not moved"
        );
        assert!(matches!(out[1], Outcome::Skipped { .. }));
    }

    #[test]
    fn a_card_never_begun_is_not_undone() {
        let store = MemStore::default();
        let c = card_in(&store, "qa", None);
        let p = factory_plan(&store);
        // Only the plan was saved (say the run stopped right after): nothing is undone, even
        // once the card reaches the destination some other way.
        move_by_hand(&store, c.id, "v_and_ving");
        let record = record_of(&[Line::Plan(p)]);
        assert!(matches!(
            undo(&store, &record, 3).expect("undo")[..],
            [Outcome::Skipped { .. }]
        ));
        assert_eq!(column_of(&store, c.id), col("v_and_ving"));
    }

    #[test]
    fn a_failed_begin_mark_writes_nothing() {
        let store = MemStore::default();
        let c = card_in(&store, "qa", None);
        let p = factory_plan(&store);
        let res = apply(&store, &p, 2, &mut |_| Err("disk full".into()));
        assert!(res.is_err());
        assert_eq!(column_of(&store, c.id), col("qa"));
    }

    #[cfg(unix)]
    #[test]
    fn a_record_file_is_created_once_and_read_back() {
        let dir = std::env::temp_dir().join(format!("mill-retag-{}", Uuid::new_v4()));
        std::fs::create_dir_all(&dir).expect("dir");
        let path = dir.join("record.json");
        let store = MemStore::default();
        let c = card_in(&store, "qa", None);
        let p = factory_plan(&store);
        let mut file = RecordFile::create(&path, &p).expect("the first run reserves it");
        let saved = std::fs::read_to_string(&path).expect("read");
        assert!(
            RecordFile::create(&path, &[]).is_err(),
            "a second run is refused"
        );
        assert_eq!(std::fs::read_to_string(&path).expect("read"), saved);
        apply(&store, &p, 2, &mut |l| file.mark(l)).expect("apply");
        // A crash mid-append leaves a cut-off last line, which parsing ignores.
        let text = std::fs::read_to_string(&path).expect("read") + "{\"beg";
        let record = Record::parse(&text).expect("parse");
        assert_eq!(record.plan, p);
        undo(&store, &record, 3).expect("undo");
        assert_eq!(column_of(&store, c.id), col("qa"));
        assert!(Record::parse("{\"begin\":\"x\"}").is_err(), "no plan first");
        std::fs::remove_dir_all(&dir).expect("clean up");
    }

    #[cfg(not(unix))]
    #[test]
    fn a_record_is_refused_where_it_cannot_be_made_durable() {
        let path = std::env::temp_dir().join(format!("mill-retag-{}.json", Uuid::new_v4()));
        assert!(RecordFile::create(&path, &[]).is_err());
        let _ = std::fs::remove_file(&path);
    }
}
