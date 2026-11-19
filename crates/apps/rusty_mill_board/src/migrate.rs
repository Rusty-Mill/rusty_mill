//! Moving the board from rusty_tick to its own SQLite file (ADR-0007).
//!
//! [`migrate`] reads rusty_tick without writing to it, loads the whole board into an empty
//! SQLite store in one transaction, then reads the SQLite store back through the same ports and
//! compares. Any difference is listed in the [`Report`]. A dry run is this against a private
//! in-memory store: nothing is written anywhere.

use std::collections::{BTreeMap, BTreeSet};

use serde::Serialize;
use serde_json::Value;
use uuid::Uuid;

use crate::domain::{BoardError, Card, Span};
use crate::milestones::MilestoneStore;
use crate::sqlite::{Import, SqliteStore};
use crate::store::Store;
use crate::tick::{TickApi, TickStore};

/// What was moved, and every way the copy differs from the source.
#[derive(Debug, Default, Serialize)]
pub struct Report {
    /// Cards read from rusty_tick.
    pub cards: usize,
    /// Cards with a card number.
    pub numbered: usize,
    /// Comments across all cards.
    pub comments: usize,
    /// Number reservations, gaps included.
    pub reservations: usize,
    /// Milestones.
    pub milestones: usize,
    /// Column entries in the move history, and the ones for tasks no longer on the board.
    pub moves: usize,
    /// Moves of a task that is not a card: left out.
    pub moves_dropped: usize,
    /// Cards per column.
    pub per_column: BTreeMap<String, usize>,
    /// Each difference found, as text. Empty when the copy is exact.
    pub mismatches: Vec<String>,
}

impl Report {
    /// Whether the copy equals the source.
    #[must_use]
    pub fn exact(&self) -> bool {
        self.mismatches.is_empty()
    }
}

fn read(e: BoardError) -> String {
    format!("reading rusty_tick: {e}")
}

/// The members of two cards' JSON that differ, so a mismatch says what, not just which.
fn differing(a: &Card, b: &Card) -> String {
    let (Ok(Value::Object(a)), Ok(Value::Object(b))) =
        (serde_json::to_value(a), serde_json::to_value(b))
    else {
        return "unserializable".into();
    };
    let keys: BTreeSet<&String> = a.keys().chain(b.keys()).collect();
    let fields: Vec<&str> = keys
        .into_iter()
        .filter(|k| a.get(*k) != b.get(*k))
        .map(String::as_str)
        .collect();
    fields.join(", ")
}

/// Copy `source` into the empty `target` and check the copy.
///
/// # Errors
/// A `String` if rusty_tick cannot be read or `target` cannot be written (a non-empty `target`
/// included); differences in the copy are in the [`Report`], not errors.
pub fn migrate<T: TickApi>(source: &TickStore<T>, target: &SqliteStore) -> Result<Report, String> {
    let cards = source.list().map_err(read)?;
    let milestones = source.milestones().map_err(read)?;
    let history = source.history().map_err(read)?;
    let ids: BTreeSet<Uuid> = cards.iter().map(|c| c.id).collect();
    let moves: Vec<(Uuid, i64)> = history
        .moves
        .iter()
        .copied()
        .filter(|(id, _)| ids.contains(id))
        .collect();
    let mut report = Report {
        cards: cards.len(),
        numbered: cards.iter().filter(|c| c.number.is_some()).count(),
        comments: cards.iter().map(|c| c.comments.len()).sum(),
        reservations: history.reservations.len(),
        milestones: milestones.len(),
        moves: moves.len(),
        moves_dropped: history.moves.len() - moves.len(),
        ..Report::default()
    };
    for card in &cards {
        *report
            .per_column
            .entry(card.column.to_string())
            .or_default() += 1;
    }
    // Every card number must hold a reservation, or a new card could be given it again.
    let reserved: BTreeSet<u64> = history.reservations.iter().map(|(n, _)| *n).collect();
    for card in cards
        .iter()
        .filter(|c| c.number.is_some_and(|n| !reserved.contains(&n)))
    {
        report.mismatches.push(format!(
            "card {} has number {:?} with no reservation",
            card.id, card.number
        ));
    }
    let mut numbers = history.reservations.clone();
    numbers.sort_unstable();
    target
        .import(&Import {
            cards: &cards,
            numbers: &numbers,
            moves: &moves,
            milestones: &milestones,
        })
        .map_err(|e| format!("writing SQLite: {e}"))?;
    verify(
        &mut report,
        &cards,
        &milestones,
        &history.reservations,
        &moves,
        target,
    )?;
    Ok(report)
}

/// Read `target` back through the ports and record every difference from what was loaded.
fn verify(
    report: &mut Report,
    cards: &[Card],
    milestones: &[crate::milestones::Milestone],
    reservations: &[(u64, Uuid)],
    moves: &[(Uuid, i64)],
    target: &SqliteStore,
) -> Result<(), String> {
    let back = target.list().map_err(|e| format!("reading SQLite: {e}"))?;
    if back.len() != cards.len() {
        report.mismatches.push(format!(
            "{} cards read back, {} written",
            back.len(),
            cards.len()
        ));
    }
    let back: BTreeMap<Uuid, &Card> = back.iter().map(|c| (c.id, c)).collect();
    for card in cards {
        match back.get(&card.id) {
            None => report
                .mismatches
                .push(format!("card {} is missing", card.id)),
            Some(got) if *got != card => report.mismatches.push(format!(
                "card {} differs in: {}",
                card.id,
                differing(card, got)
            )),
            Some(_) => {}
        }
    }
    let mut want = milestones.to_vec();
    want.sort_by_key(|m| m.id);
    let got = target
        .milestones()
        .map_err(|e| format!("reading SQLite: {e}"))?;
    if got != want {
        report.mismatches.push(format!(
            "milestones differ: {} read back, {} written",
            got.len(),
            want.len()
        ));
    }
    let top = reservations.iter().map(|(n, _)| *n).max().unwrap_or(0);
    let got_top = target
        .highest_reserved()
        .map_err(|e| format!("reading SQLite: {e}"))?;
    if got_top != top {
        report.mismatches.push(format!(
            "highest reserved number is {got_top}, expected {top}"
        ));
    }
    let moved = target
        .moved_in(Span::default())
        .map_err(|e| format!("reading SQLite: {e}"))?;
    let expected: BTreeSet<Uuid> = moves.iter().map(|(id, _)| *id).collect();
    if moved != expected {
        report.mismatches.push(format!(
            "{} cards with moves read back, {} expected",
            moved.len(),
            expected.len()
        ));
    }
    Ok(())
}
