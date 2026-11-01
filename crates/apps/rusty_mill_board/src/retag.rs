//! Re-filing cards when the workflow's columns change: `qa` and `review` into `v_and_ving`,
//! say. Planning is pure ([`plan`]); [`apply`] and [`undo`] write through the [`Store`] port.
//!
//! The plan is the rollback record, and the caller saves it *before* [`apply`] writes
//! anything, so a crash or a failed write midway still leaves every card's origin on disk.
//! Each entry carries the card's revision when planned: [`apply`] retags a card only at that
//! revision, and [`undo`] restores one only at the very next revision, still in the column the
//! retag put it in. A card anyone has changed since is never touched, and a write whose
//! outcome is unknown is settled by reading the card again: the retag either landed (next
//! revision, undone) or did not (planned revision, nothing to undo).

use std::collections::BTreeMap;

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

/// Retag each planned card still in its planned column at its planned revision.
///
/// # Errors
/// The first storage error. Cards before it are written; the saved plan undoes them.
pub fn apply(store: &dyn Store, plan: &[Retag], now_ms: i64) -> Result<Vec<Outcome>, BoardError> {
    plan.iter()
        .map(|r| {
            write_if(store, r.id, now_ms, |c| {
                (c.column == r.from && c.rev == r.rev)
                    .then(|| r.to.clone())
                    .ok_or_else(|| changed(c, &r.from, r.rev))
            })
        })
        .collect()
}

/// [`apply`], after `save` has made the plan durable: nothing is written unless `save`
/// succeeds, so every retag that lands has its origin on disk first.
///
/// # Errors
/// `save`'s error, with no card touched; or [`apply`]'s, with the saved plan to undo from.
pub fn apply_recorded(
    store: &dyn Store,
    plan: &[Retag],
    now_ms: i64,
    save: impl FnOnce(&[Retag]) -> Result<(), String>,
) -> Result<Vec<Outcome>, String> {
    save(plan).map_err(|e| format!("rollback record not saved, nothing retagged: {e}"))?;
    apply(store, plan, now_ms)
        .map_err(|e| format!("{e}; some cards may be retagged: undo them from the saved record"))
}

/// Put back each planned card the retag moved and nobody has changed since: in `to`, at the
/// revision right after the planned one.
///
/// # Errors
/// The first storage error. Cards before it are restored; running it again finishes the rest.
pub fn undo(store: &dyn Store, plan: &[Retag], now_ms: i64) -> Result<Vec<Outcome>, BoardError> {
    plan.iter()
        .map(|r| {
            write_if(store, r.id, now_ms, |c| {
                (c.column == r.to && c.rev == r.rev + 1)
                    .then(|| r.from.clone())
                    .ok_or_else(|| changed(c, &r.to, r.rev + 1))
            })
        })
        .collect()
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

/// Retag card `id` to the column `decide` names, or skip it with `decide`'s reason.
fn write_if(
    store: &dyn Store,
    id: Uuid,
    now_ms: i64,
    decide: impl Fn(&Card) -> Result<Column, String>,
) -> Result<Outcome, BoardError> {
    let Some(card) = store.get(id)? else {
        return Ok(Outcome::Skipped {
            id,
            why: "no longer on the board".into(),
        });
    };
    let to = match decide(&card) {
        Ok(to) => to,
        Err(why) => return Ok(Outcome::Skipped { id, why }),
    };
    let next = card.retag(&to, now_ms);
    store.replace(&next, Event::of(&next, EventKind::Moved, ACTOR))?;
    Ok(Outcome::Done { id })
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
    fn the_record_is_saved_before_any_card_changes_and_a_failed_save_writes_nothing() {
        let store = MemStore::default();
        let c = card_in(&store, "qa", None);
        let p = factory_plan(&store);
        let refused = apply_recorded(&store, &p, 2, |_| Err("disk full".into()));
        assert!(refused.is_err_and(|e| e.contains("nothing retagged")));
        assert_eq!(column_of(&store, c.id), col("qa"));
        let out = apply_recorded(&store, &p, 3, |saved| {
            assert_eq!(saved, &p[..]);
            assert_eq!(column_of(&store, c.id), col("qa"), "saved before writing");
            Ok(())
        })
        .expect("apply");
        assert_eq!(out.len(), 1);
        assert_eq!(column_of(&store, c.id), col("v_and_ving"));
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
        let p = factory_plan(&store);
        let done = apply(&store, &p, 2).expect("apply");
        assert!(done.iter().all(|o| matches!(o, Outcome::Done { .. })));
        let now = store.get(qa.id).expect("get").expect("card");
        assert_eq!(
            (now.column, now.claim.as_deref()),
            (col("v_and_ving"), Some("tester"))
        );
        undo(&store, &p, 3).expect("undo");
        assert_eq!(column_of(&store, qa.id), col("qa"));
        assert_eq!(column_of(&store, review.id), col("review"));
    }

    #[test]
    fn apply_skips_a_card_that_changed_after_planning() {
        let store = MemStore::default();
        let c = card_in(&store, "qa", None);
        let p = factory_plan(&store);
        let moved = c.retag(&col("building"), 2);
        store
            .replace(&moved, Event::of(&moved, EventKind::Moved, "someone"))
            .expect("move");
        assert!(matches!(
            apply(&store, &p, 3).expect("apply")[..],
            [Outcome::Skipped { .. }]
        ));
        assert_eq!(column_of(&store, c.id), col("building"));
    }

    #[test]
    fn undo_leaves_a_card_worked_on_since_the_retag() {
        let store = MemStore::default();
        let c = card_in(&store, "review", None);
        let p = factory_plan(&store);
        apply(&store, &p, 2).expect("apply");
        // Someone edits it in V&Ving: same column, later revision. Its origin no longer holds.
        let now = store.get(c.id).expect("get").expect("card");
        let edited = now.retag(&col("v_and_ving"), 3);
        store
            .replace(&edited, Event::of(&edited, EventKind::Edited, "someone"))
            .expect("edit");
        assert!(matches!(
            undo(&store, &p, 4).expect("undo")[..],
            [Outcome::Skipped { .. }]
        ));
        assert_eq!(column_of(&store, c.id), col("v_and_ving"));
    }

    #[test]
    fn a_failure_midway_is_recovered_from_the_saved_plan() {
        let store = Failing {
            inner: MemStore::default(),
            calls: AtomicUsize::new(0),
            fail_from: 1,
        };
        let first = card_in(&store, "qa", None);
        let second = card_in(&store, "review", None);
        let p = factory_plan(&store);
        assert!(apply(&store, &p, 2).is_err(), "the second write fails");
        let moved: Vec<Uuid> = [first.id, second.id]
            .into_iter()
            .filter(|id| column_of(&store, *id) == col("v_and_ving"))
            .collect();
        assert_eq!(moved.len(), 1, "one card was retagged before the failure");
        // Undo from the plan saved before writing: the retagged card goes back, the other
        // (never written) is skipped, not moved.
        let fixed = Failing {
            fail_from: usize::MAX,
            ..store
        };
        let out = undo(&fixed, &p, 3).expect("undo");
        assert_eq!(
            out.iter()
                .filter(|o| matches!(o, Outcome::Done { .. }))
                .count(),
            1
        );
        assert_eq!(column_of(&fixed, first.id), col("qa"));
        assert_eq!(column_of(&fixed, second.id), col("review"));
    }
}
