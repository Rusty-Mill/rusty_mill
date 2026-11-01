//! Re-filing cards when the workflow's columns change: `qa` and `review` into `v_and_ving`,
//! say. Planning is pure ([`plan`]); [`apply`] writes the plan through the [`Store`] port.
//!
//! The plan doubles as the rollback record: [`Retag::undo`] swaps each entry's columns, so
//! applying the inverted plan restores the old tags. Only cards still in the column a retag
//! put them in are restored, so a card that has moved on since is left alone.

use std::collections::BTreeMap;

use serde::{Deserialize, Serialize};
use uuid::Uuid;

use crate::domain::{BoardError, Card, Column, Event, EventKind};
use crate::store::Store;

/// The actor recorded on retag events.
pub const ACTOR: &str = "retag";

/// One card to re-file.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct Retag {
    /// The card.
    pub id: Uuid,
    /// Its board number, for people reading the plan.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub number: Option<u64>,
    /// The column it is in.
    pub from: Column,
    /// The column it goes to.
    pub to: Column,
}

impl Retag {
    /// The entry that puts the card back.
    pub fn undo(&self) -> Self {
        Self {
            from: self.to.clone(),
            to: self.from.clone(),
            ..self.clone()
        }
    }
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
                from: c.column.clone(),
                to: to.clone(),
            })
        })
        .collect();
    out.sort_by_key(|r| (r.number, r.id));
    out
}

/// Write `retags` through `store`, skipping a card that is gone or no longer in `from`.
/// Returns the retags actually made, which is the rollback record.
///
/// # Errors
/// The first storage error. Retags already written stay written and are not in the
/// returned list, so the caller must report them: see the error's message.
pub fn apply(store: &dyn Store, retags: &[Retag], now_ms: i64) -> Result<Vec<Retag>, BoardError> {
    let mut done = Vec::new();
    for r in retags {
        let Some(card) = store.get(r.id)? else {
            continue;
        };
        if card.column != r.from {
            continue;
        }
        let next = card.retag(&r.to, now_ms);
        if let Err(e) = store.replace(&next, Event::of(&next, EventKind::Moved, ACTOR)) {
            return Err(BoardError::Storage(format!(
                "{e}; {} card(s) were retagged before it: {}",
                done.len(),
                serde_json::to_string(&done).unwrap_or_default()
            )));
        }
        done.push(r.clone());
    }
    Ok(done)
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::store::MemStore;

    fn col(id: &str) -> Column {
        Column::new(id).expect("id")
    }

    fn card_in(store: &MemStore, column: &str, claim: Option<&str>) -> Card {
        let mut c = Card::new("t", "", vec![], 1).expect("card");
        c.column = col(column);
        c.claim = claim.map(str::to_owned);
        store
            .insert(&c, Event::of(&c, EventKind::Created, "t"))
            .expect("insert");
        c
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
    fn plans_only_mapped_columns() {
        let store = MemStore::default();
        let qa = card_in(&store, "qa", None);
        let review = card_in(&store, "review", None);
        card_in(&store, "ready", None);
        let map = parse_map("qa=v_and_ving,review=v_and_ving").expect("map");
        let p = plan(&store.list().expect("list"), &map);
        let ids: Vec<Uuid> = p.iter().map(|r| r.id).collect();
        assert_eq!(p.len(), 2);
        assert!(ids.contains(&qa.id) && ids.contains(&review.id));
        assert!(p.iter().all(|r| r.to == col("v_and_ving")));
    }

    #[test]
    fn apply_keeps_the_claim_and_undo_restores_the_column() {
        let store = MemStore::default();
        let c = card_in(&store, "review", Some("rev"));
        let map = parse_map("review=v_and_ving").expect("map");
        let made = apply(&store, &plan(&store.list().expect("list"), &map), 2).expect("apply");
        assert_eq!(made.len(), 1);
        let now = store.get(c.id).expect("get").expect("card");
        assert_eq!(
            (now.column, now.claim.as_deref()),
            (col("v_and_ving"), Some("rev"))
        );
        let undo: Vec<Retag> = made.iter().map(Retag::undo).collect();
        apply(&store, &undo, 3).expect("undo");
        assert_eq!(
            store.get(c.id).expect("get").expect("card").column,
            col("review")
        );
    }

    #[test]
    fn apply_skips_a_card_that_moved_on() {
        let store = MemStore::default();
        let c = card_in(&store, "qa", None);
        let stale = Retag {
            id: c.id,
            number: None,
            from: col("review"),
            to: col("v_and_ving"),
        };
        assert_eq!(apply(&store, &[stale], 2).expect("apply"), []);
        assert_eq!(
            store.get(c.id).expect("get").expect("card").column,
            col("qa")
        );
    }
}
