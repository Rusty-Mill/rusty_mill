//! Moving the board from rusty_tick to its own SQLite file (ADR-0007).
//!
//! [`migrate`] reads rusty_tick without writing to it, loads the whole board into an empty
//! SQLite store in one transaction, then reads the SQLite store back through the same ports and
//! compares. Any difference is listed in the [`Report`]. A dry run is this against a private
//! in-memory store: nothing is written anywhere.

use std::collections::{BTreeMap, BTreeSet};
use std::path::{Path, PathBuf};

use serde::Serialize;
use serde_json::Value;
use uuid::Uuid;

use crate::domain::{BoardError, Card};
use crate::milestones::MilestoneStore;
use crate::sqlite::{Import, SOURCE_KEY, SqliteStore};
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
    /// Unknown board-document members carried across, in all.
    pub unknown_fields: usize,
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

/// Copy `source` into the empty `target` and check the copy. A source that is already
/// inconsistent is reported and nothing is written.
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
        unknown_fields: history.unknown_fields.iter().map(|(_, m)| m.len()).sum(),
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
    // Every card number must hold a reservation, or a new card could be given it again. A copy
    // known to be wrong is not written at all.
    let reserved: BTreeSet<u64> = history.reservations.iter().map(|(n, _)| *n).collect();
    report.mismatches.extend(
        cards
            .iter()
            .filter(|c| c.number.is_some_and(|n| !reserved.contains(&n)))
            .map(|c| {
                format!(
                    "card {} has number {:?} with no reservation",
                    c.id, c.number
                )
            }),
    );
    report.mismatches.extend(history.unreadable.iter().cloned());
    if !report.exact() {
        return Ok(report);
    }
    let mut numbers = history.reservations.clone();
    numbers.sort_unstable();
    let source_held = held_by_card(&cards, &history);
    // A card's comments and the documents they came from must line up one for one.
    for card in &cards {
        let ids = history
            .comment_ids
            .iter()
            .find(|(task, _)| *task == card.id)
            .map_or(0, |(_, ids)| ids.len());
        if ids != card.comments.len() {
            report.mismatches.push(format!(
                "card {} has {} comments but {ids} comment documents",
                card.id,
                card.comments.len()
            ));
        }
    }
    if !report.exact() {
        return Ok(report);
    }
    let expected = Import {
        cards: &cards,
        numbers: &numbers,
        moves: &moves,
        milestones: &milestones,
        source: &source_held,
    };
    target
        .import(&expected)
        .map_err(|e| format!("writing SQLite: {e}"))?;
    report.mismatches.extend(verify(&expected, target)?);
    Ok(report)
}

/// What the source holds for each card beyond the card: its unknown document members and the
/// ids of its comment documents, by card (cards with neither are left out).
fn held_by_card(
    cards: &[Card],
    history: &crate::tick::TickHistory,
) -> Vec<(Uuid, serde_json::Map<String, Value>)> {
    cards
        .iter()
        .filter_map(|c| {
            let mut held = serde_json::Map::new();
            if let Some((_, extra)) = history.unknown_fields.iter().find(|(id, _)| *id == c.id) {
                held.insert("unknown_fields".into(), Value::Object(extra.clone()));
            }
            if let Some((_, ids)) = history.comment_ids.iter().find(|(id, _)| *id == c.id) {
                held.insert("comment_ids".into(), serde_json::json!(ids));
            }
            (!held.is_empty()).then_some((c.id, held))
        })
        .collect()
}

/// Read `target` back and list every difference from what was loaded: each card whole,
/// milestones, every number reservation, and every move with its time and repeats.
///
/// # Errors
/// A `String` if `target` cannot be read.
pub fn verify(expected: &Import<'_>, target: &SqliteStore) -> Result<Vec<String>, String> {
    let read = |e: BoardError| format!("reading SQLite: {e}");
    let mut found = Vec::new();
    let back = target.list().map_err(read)?;
    if back.len() != expected.cards.len() {
        found.push(format!(
            "{} cards read back, {} written",
            back.len(),
            expected.cards.len()
        ));
    }
    let back: BTreeMap<Uuid, &Card> = back.iter().map(|c| (c.id, c)).collect();
    for card in expected.cards {
        match back.get(&card.id) {
            None => found.push(format!("card {} is missing", card.id)),
            Some(got) if *got != card => found.push(format!(
                "card {} differs in: {}",
                card.id,
                differing(card, got)
            )),
            Some(_) => {}
        }
    }
    let mut want = expected.milestones.to_vec();
    want.sort_by_key(|m| m.id);
    let got = target.milestones().map_err(read)?;
    if got != want {
        found.push(format!(
            "milestones differ: {} read back, {} written",
            got.len(),
            want.len()
        ));
    }
    let mut numbers = expected.numbers.to_vec();
    numbers.sort_unstable();
    let got = target.numbers().map_err(read)?;
    if got != numbers {
        found.push(format!(
            "number reservations differ: {} read back, {} written",
            got.len(),
            numbers.len()
        ));
    }
    for (id, held) in expected.source {
        let stored = target.document(*id).map_err(read)?;
        let got = stored.as_ref().and_then(|d| d.get(SOURCE_KEY));
        if got != Some(&Value::Object(held.clone())) {
            found.push(format!("card {id} lost what the source held for it"));
        }
    }
    let mut moves: Vec<(Uuid, i64)> = expected
        .moves
        .iter()
        .copied()
        .filter(|(id, _)| back.contains_key(id))
        .collect();
    moves.sort_unstable();
    let got = target.moves().map_err(read)?;
    if got != moves {
        found.push(format!(
            "moves differ: {} read back, {} written (card and time compared)",
            got.len(),
            moves.len()
        ));
    }
    Ok(found)
}

/// Copy `source` into a new file at `path`, which appears only if the copy is exact.
///
/// The copy is built in a staging file of this run's own (named for it, in `path`'s directory),
/// then published by a hard link, which fails rather than replace a file that appeared at
/// `path` meanwhile. A failed or interrupted run leaves no file a board could be started on, and
/// removes only its own staging files, so it can simply be run again, even beside another run.
///
/// # Errors
/// A `String` if `path` or a sidecar of it exists (before the copy or when publishing), or as
/// for [`migrate`]. The destination's directory must be under the caller's control while this
/// runs: a file created there between the last check and the link is not detectable.
pub fn migrate_to_file<T: TickApi>(source: &TickStore<T>, path: &Path) -> Result<Report, String> {
    if path.exists() {
        return Err(exists(path));
    }
    no_sidecars(path)?;
    let mut name = path.as_os_str().to_owned();
    name.push(format!(".{}.partial", Uuid::new_v4().simple()));
    let partial = PathBuf::from(name);
    let discard = || {
        for suffix in ["", "-wal", "-shm"] {
            let mut file = partial.as_os_str().to_owned();
            file.push(suffix);
            let _ = std::fs::remove_file(file);
        }
    };
    let outcome = SqliteStore::open(&partial)
        .map_err(|e| format!("{}: {e}", partial.display()))
        .and_then(|target| {
            let report = migrate(source, &target)?;
            // The main file alone must be the whole database: it is all that gets published.
            if report.exact() {
                target
                    .checkpoint()
                    .map_err(|e| format!("making {} self-contained: {e}", partial.display()))?;
            }
            Ok(report)
        });
    let result = match outcome {
        // Checked again here: the copy can take long, and a sidecar that appeared meanwhile
        // would be replayed into the published file.
        Ok(report) if report.exact() => no_sidecars(path).and_then(|()| {
            std::fs::hard_link(&partial, path)
                .map(|()| report)
                .map_err(|e| {
                    if path.exists() {
                        exists(path)
                    } else {
                        format!("publishing {}: {e}", path.display())
                    }
                })
        }),
        other => other,
    };
    discard();
    result
}

/// Refuse a log or rollback journal beside `path`: SQLite would replay it into the new file.
fn no_sidecars(path: &Path) -> Result<(), String> {
    for suffix in ["-wal", "-shm", "-journal"] {
        let mut stray = path.as_os_str().to_owned();
        stray.push(suffix);
        if Path::new(&stray).exists() {
            return Err(format!(
                "{} exists beside {}: remove it or choose another path",
                Path::new(&stray).display(),
                path.display()
            ));
        }
    }
    Ok(())
}

fn exists(path: &Path) -> String {
    format!(
        "{} already exists: the migration writes a new file",
        path.display()
    )
}
