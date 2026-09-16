//! Simulation engine for the digital transformation simulator -- the
//! Rust port of `meshed.transformation.models` and
//! `meshed.transformation.engine`.
//!
//! The state machine (see [`SystemStatus`]):
//!
//! ```text
//! Legacy --MigrateTrack--> DualWrite --SunsetLegacy or 2q aging--> Migrated
//! Legacy --SunsetLegacy (skips dual-write)--> Decommissioned  (capability regression)
//! ```
//!
//! Dual-write auto-completes after [`DUAL_WRITE_MIN_QUARTERS`] quarters
//! even without an explicit sunset decision, so a track doesn't stall
//! forever if the operator queues `MigrateTrack` once and moves on.

use super::enums::{CapabilityDimension, DecisionType, SystemStatus};
use rusty_sqlite::rusqlite::{params, Connection, OptionalExtension};
use std::collections::HashMap;

/// Minimum number of quarters a track stays `DualWrite` before it
/// auto-completes to `Migrated` even with no explicit sunset decision.
pub const DUAL_WRITE_MIN_QUARTERS: i64 = 2;

const SCORE_MIN: f64 = 0.0;
const SCORE_MAX: f64 = 5.0;

fn clamp(value: f64) -> f64 {
    value.clamp(SCORE_MIN, SCORE_MAX)
}

fn now_iso() -> String {
    let since_epoch = std::time::SystemTime::now()
        .duration_since(std::time::UNIX_EPOCH)
        .unwrap_or_default();
    // A minimal RFC 3339 UTC formatter -- this crate needs "now, as an
    // ISO-8601 string" only for a `created_at`/`timestamp` audit trail
    // no test asserts the exact value of, so a hand-rolled civil-date
    // conversion (no leap-second table, no calendar library) is
    // sufficient; `rusty_time::DateTime` has no `now()`/`from_timestamp`
    // constructor to build on today (it's a pure calendar/formatting
    // type, not a clock).
    let total_secs = since_epoch.as_secs();
    let mut days = (total_secs / 86_400) as i64;
    let secs_of_day = total_secs % 86_400;
    let (hour, minute, second) = (
        secs_of_day / 3600,
        (secs_of_day % 3600) / 60,
        secs_of_day % 60,
    );

    // Civil-from-days (Howard Hinnant's algorithm), proleptic Gregorian,
    // days since 1970-01-01.
    days += 719_468;
    let era = if days >= 0 { days } else { days - 146_096 } / 146_097;
    let day_of_era = (days - era * 146_097) as u64;
    let year_of_era =
        (day_of_era - day_of_era / 1460 + day_of_era / 36524 - day_of_era / 146_096) / 365;
    let year = year_of_era as i64 + era * 400;
    let day_of_year = day_of_era - (365 * year_of_era + year_of_era / 4 - year_of_era / 100);
    let mp = (5 * day_of_year + 2) / 153;
    let day = (day_of_year - (153 * mp + 2) / 5 + 1) as u32;
    let month = if mp < 10 { mp + 3 } else { mp - 9 } as u32;
    let year = if month <= 2 { year + 1 } else { year };

    format!("{year:04}-{month:02}-{day:02}T{hour:02}:{minute:02}:{second:02}Z")
}

/// A legacy system being strangled and replaced by a mesh data product
/// (`meshed.transformation.models.LegacySystem`).
#[derive(Debug, Clone, PartialEq)]
pub struct LegacySystem {
    pub track: String,
    pub name: String,
    pub target_data_product: String,
    pub status: SystemStatus,
    pub status_since_quarter: i64,
}

/// A decision reference as returned in [`TransformationState`]'s
/// `pending_decisions`/`decision_history` -- `{id, quarter,
/// decision_type, target}`, matching `get_state()`'s dict shape.
#[derive(Debug, Clone, PartialEq)]
pub struct DecisionRef {
    pub id: i64,
    pub quarter: i64,
    pub decision_type: DecisionType,
    pub target: String,
}

/// One `{quarter, maturity_index}` point in the maturity trend --
/// `maturity_index` is the mean of every score recorded that quarter
/// across all tracks/dimensions, rounded to 2 decimals.
#[derive(Debug, Clone, Copy, PartialEq)]
pub struct MaturityPoint {
    pub quarter: i64,
    pub maturity_index: f64,
}

/// A full snapshot for the frontend, matching `get_state()`'s dict
/// shape.
#[derive(Debug, Clone, PartialEq)]
pub struct TransformationState {
    pub quarter: i64,
    pub legacy_systems: Vec<LegacySystem>,
    pub capability: HashMap<String, HashMap<CapabilityDimension, f64>>,
    pub maturity_trend: Vec<MaturityPoint>,
    pub pending_decisions: Vec<DecisionRef>,
    pub decision_history: Vec<DecisionRef>,
}

/// Creates the five transformation tables if they don't already exist.
/// Idempotent, matching `SQLModel.metadata.create_all`'s no-migrations
/// approach.
pub fn ensure_schema(conn: &Connection) -> rusty_sqlite::rusqlite::Result<()> {
    conn.execute_batch(
        "CREATE TABLE IF NOT EXISTS transformation_clock (
            id INTEGER PRIMARY KEY,
            current_quarter INTEGER NOT NULL DEFAULT 0
        );
        CREATE TABLE IF NOT EXISTS legacy_systems (
            id INTEGER PRIMARY KEY AUTOINCREMENT,
            track TEXT NOT NULL UNIQUE,
            name TEXT NOT NULL,
            target_data_product TEXT NOT NULL,
            status TEXT NOT NULL DEFAULT 'legacy',
            status_since_quarter INTEGER NOT NULL DEFAULT 0
        );
        CREATE INDEX IF NOT EXISTS idx_legacy_systems_track ON legacy_systems(track);
        CREATE TABLE IF NOT EXISTS capability_scores (
            id INTEGER PRIMARY KEY AUTOINCREMENT,
            track TEXT NOT NULL,
            dimension TEXT NOT NULL,
            quarter INTEGER NOT NULL,
            score REAL NOT NULL
        );
        CREATE INDEX IF NOT EXISTS idx_capability_scores_track ON capability_scores(track);
        CREATE INDEX IF NOT EXISTS idx_capability_scores_quarter ON capability_scores(quarter);
        CREATE TABLE IF NOT EXISTS transformation_decisions (
            id INTEGER PRIMARY KEY AUTOINCREMENT,
            quarter INTEGER NOT NULL,
            decision_type TEXT NOT NULL,
            target TEXT NOT NULL,
            applied INTEGER NOT NULL DEFAULT 0,
            created_at TEXT NOT NULL
        );
        CREATE INDEX IF NOT EXISTS idx_transformation_decisions_quarter ON transformation_decisions(quarter);
        CREATE TABLE IF NOT EXISTS transformation_events (
            id INTEGER PRIMARY KEY AUTOINCREMENT,
            quarter INTEGER NOT NULL,
            event_type TEXT NOT NULL,
            track TEXT,
            message TEXT NOT NULL,
            timestamp TEXT NOT NULL
        );
        CREATE INDEX IF NOT EXISTS idx_transformation_events_quarter ON transformation_events(quarter);",
    )
}

/// Returns the singleton clock row's `current_quarter`, creating it at
/// quarter 0 if missing.
pub fn get_or_create_clock(conn: &Connection) -> rusty_sqlite::rusqlite::Result<i64> {
    let existing: Option<i64> = conn
        .query_row(
            "SELECT current_quarter FROM transformation_clock LIMIT 1",
            [],
            |row| row.get(0),
        )
        .optional()?;
    if let Some(quarter) = existing {
        return Ok(quarter);
    }
    conn.execute(
        "INSERT INTO transformation_clock (id, current_quarter) VALUES (1, 0)",
        [],
    )?;
    Ok(0)
}

/// Queues a decision for the upcoming quarter (current quarter + 1).
/// `target` is a track slug for `MigrateTrack`/`SunsetLegacy`, or
/// `"platform"`/`"product_teams"` for the two investment decisions.
pub fn queue_decision(
    conn: &Connection,
    decision_type: DecisionType,
    target: &str,
) -> rusty_sqlite::rusqlite::Result<DecisionRef> {
    let current_quarter = get_or_create_clock(conn)?;
    let quarter = current_quarter + 1;
    conn.execute(
        "INSERT INTO transformation_decisions (quarter, decision_type, target, applied, created_at)
         VALUES (?1, ?2, ?3, 0, ?4)",
        params![quarter, decision_type.as_str(), target, now_iso()],
    )?;
    let id = conn.last_insert_rowid();
    Ok(DecisionRef {
        id,
        quarter,
        decision_type,
        target: target.to_string(),
    })
}

fn load_legacy_systems(
    conn: &Connection,
) -> rusty_sqlite::rusqlite::Result<HashMap<String, LegacySystem>> {
    let mut stmt = conn.prepare(
        "SELECT track, name, target_data_product, status, status_since_quarter FROM legacy_systems",
    )?;
    let rows = stmt.query_map([], |row| {
        let status_str: String = row.get(3)?;
        Ok(LegacySystem {
            track: row.get(0)?,
            name: row.get(1)?,
            target_data_product: row.get(2)?,
            status: SystemStatus::parse(&status_str).unwrap_or(SystemStatus::Legacy),
            status_since_quarter: row.get(4)?,
        })
    })?;
    let mut systems = HashMap::new();
    for row in rows {
        let system = row?;
        systems.insert(system.track.clone(), system);
    }
    Ok(systems)
}

fn latest_scores(
    conn: &Connection,
    quarter: i64,
) -> rusty_sqlite::rusqlite::Result<HashMap<String, HashMap<CapabilityDimension, f64>>> {
    let mut stmt =
        conn.prepare("SELECT track, dimension, score FROM capability_scores WHERE quarter = ?1")?;
    let rows = stmt.query_map(params![quarter], |row| {
        let track: String = row.get(0)?;
        let dimension_str: String = row.get(1)?;
        let score: f64 = row.get(2)?;
        Ok((track, dimension_str, score))
    })?;
    let mut scores: HashMap<String, HashMap<CapabilityDimension, f64>> = HashMap::new();
    for row in rows {
        let (track, dimension_str, score) = row?;
        if let Some(dimension) = CapabilityDimension::parse(&dimension_str) {
            scores.entry(track).or_default().insert(dimension, score);
        }
    }
    Ok(scores)
}

fn emit(
    conn: &Connection,
    quarter: i64,
    event_type: &str,
    message: &str,
    track: Option<&str>,
) -> rusty_sqlite::rusqlite::Result<()> {
    conn.execute(
        "INSERT INTO transformation_events (quarter, event_type, track, message, timestamp)
         VALUES (?1, ?2, ?3, ?4, ?5)",
        params![quarter, event_type, track, message, now_iso()],
    )?;
    Ok(())
}

fn add_delta(
    deltas: &mut HashMap<String, HashMap<CapabilityDimension, f64>>,
    track: &str,
    dimension: CapabilityDimension,
    amount: f64,
) {
    *deltas
        .entry(track.to_string())
        .or_default()
        .entry(dimension)
        .or_insert(0.0) += amount;
}

struct PendingDecision {
    id: i64,
    decision_type: DecisionType,
    target: String,
}

/// Applies all pending decisions for the upcoming quarter, evolves
/// legacy-system status, recomputes capability scores, emits narrative
/// events, and advances the clock -- one atomic transaction. Returns
/// the resulting snapshot, same shape as [`get_state`].
pub fn advance_quarter(
    conn: &mut Connection,
) -> rusty_sqlite::rusqlite::Result<TransformationState> {
    let tx = conn.transaction()?;
    let current_quarter = get_or_create_clock(&tx)?;
    let next_q = current_quarter + 1;

    let mut systems = load_legacy_systems(&tx)?;
    let prior_scores = latest_scores(&tx, current_quarter)?;
    let mut deltas: HashMap<String, HashMap<CapabilityDimension, f64>> = HashMap::new();

    let pending: Vec<PendingDecision> = {
        let mut stmt = tx.prepare(
            "SELECT id, decision_type, target FROM transformation_decisions WHERE quarter = ?1 AND applied = 0",
        )?;
        let rows = stmt.query_map(params![next_q], |row| {
            let decision_type_str: String = row.get(1)?;
            Ok(PendingDecision {
                id: row.get(0)?,
                decision_type: DecisionType::parse(&decision_type_str)
                    .unwrap_or(DecisionType::MigrateTrack),
                target: row.get(2)?,
            })
        })?;
        rows.collect::<Result<Vec<_>, _>>()?
    };

    for decision in &pending {
        let target = &decision.target;
        match decision.decision_type {
            DecisionType::MigrateTrack => {
                let eligible = systems
                    .get(target)
                    .is_some_and(|ls| ls.status == SystemStatus::Legacy);
                if eligible {
                    let ls = systems.get_mut(target).unwrap();
                    ls.status = SystemStatus::DualWrite;
                    ls.status_since_quarter = next_q;
                    add_delta(
                        &mut deltas,
                        target,
                        CapabilityDimension::DomainOwnership,
                        0.5,
                    );
                    add_delta(
                        &mut deltas,
                        target,
                        CapabilityDimension::DataAsAProduct,
                        0.5,
                    );
                    emit(
                        &tx,
                        next_q,
                        "wave_started",
                        &format!(
                            "{} begins dual-write to {}",
                            ls.name, ls.target_data_product
                        ),
                        Some(target),
                    )?;
                } else {
                    emit(
                        &tx,
                        next_q,
                        "decision_rejected",
                        &format!("Migrate {target}: already past LEGACY status, decision skipped"),
                        Some(target),
                    )?;
                }
            }
            DecisionType::SunsetLegacy => match systems.get(target).map(|ls| ls.status) {
                Some(SystemStatus::DualWrite) => {
                    let ls = systems.get_mut(target).unwrap();
                    ls.status = SystemStatus::Migrated;
                    ls.status_since_quarter = next_q;
                    add_delta(
                        &mut deltas,
                        target,
                        CapabilityDimension::FederatedGovernance,
                        0.5,
                    );
                    add_delta(
                        &mut deltas,
                        target,
                        CapabilityDimension::DataAsAProduct,
                        0.3,
                    );
                    emit(
                        &tx,
                        next_q,
                        "system_decommissioned",
                        &format!(
                            "{} decommissioned cleanly — {} is now sole source of truth",
                            ls.name, ls.target_data_product
                        ),
                        Some(target),
                    )?;
                }
                Some(SystemStatus::Legacy) => {
                    let ls = systems.get_mut(target).unwrap();
                    ls.status = SystemStatus::Decommissioned;
                    ls.status_since_quarter = next_q;
                    add_delta(
                        &mut deltas,
                        target,
                        CapabilityDimension::FederatedGovernance,
                        -1.0,
                    );
                    add_delta(
                        &mut deltas,
                        target,
                        CapabilityDimension::DataAsAProduct,
                        -0.5,
                    );
                    emit(
                        &tx,
                        next_q,
                        "maturity_regression",
                        &format!(
                            "{} decommissioned without dual-write — consumers lost lineage continuity",
                            ls.name
                        ),
                        Some(target),
                    )?;
                }
                _ => {
                    emit(
                        &tx,
                        next_q,
                        "decision_rejected",
                        &format!("Sunset {target}: no eligible legacy system in LEGACY or DUAL_WRITE status"),
                        Some(target),
                    )?;
                }
            },
            DecisionType::InvestPlatform => {
                for track in systems.keys().cloned().collect::<Vec<_>>() {
                    add_delta(
                        &mut deltas,
                        &track,
                        CapabilityDimension::SelfServePlatform,
                        0.3,
                    );
                }
                emit(
                    &tx,
                    next_q,
                    "decision_applied",
                    "Platform investment lifts self-serve capability mesh-wide",
                    None,
                )?;
            }
            DecisionType::InvestProductTeams => {
                for track in systems.keys().cloned().collect::<Vec<_>>() {
                    add_delta(
                        &mut deltas,
                        &track,
                        CapabilityDimension::DomainOwnership,
                        0.2,
                    );
                    add_delta(
                        &mut deltas,
                        &track,
                        CapabilityDimension::DataAsAProduct,
                        0.2,
                    );
                }
                emit(
                    &tx,
                    next_q,
                    "decision_applied",
                    "Product team investment lifts domain ownership mesh-wide",
                    None,
                )?;
            }
        }
        tx.execute(
            "UPDATE transformation_decisions SET applied = 1 WHERE id = ?1",
            params![decision.id],
        )?;
    }

    // Auto-complete dual-write after the minimum period if not already
    // sunset this quarter.
    for track in systems.keys().cloned().collect::<Vec<_>>() {
        let ls = systems.get(&track).unwrap();
        if ls.status == SystemStatus::DualWrite
            && (next_q - ls.status_since_quarter) >= DUAL_WRITE_MIN_QUARTERS
        {
            let name = ls.name.clone();
            let target_data_product = ls.target_data_product.clone();
            add_delta(
                &mut deltas,
                &track,
                CapabilityDimension::FederatedGovernance,
                0.5,
            );
            add_delta(
                &mut deltas,
                &track,
                CapabilityDimension::DataAsAProduct,
                0.3,
            );
            let ls = systems.get_mut(&track).unwrap();
            ls.status = SystemStatus::Migrated;
            ls.status_since_quarter = next_q;
            emit(
                &tx,
                next_q,
                "system_decommissioned",
                &format!("{name} dual-write period complete — {target_data_product} cut over automatically"),
                Some(&track),
            )?;
        }
    }

    // Write a full forward snapshot of scores for every track/dimension.
    for track in systems.keys() {
        for dimension in CapabilityDimension::ALL {
            let prior = prior_scores
                .get(track)
                .and_then(|scores| scores.get(&dimension))
                .copied()
                .unwrap_or(0.0);
            let delta = deltas
                .get(track)
                .and_then(|d| d.get(&dimension))
                .copied()
                .unwrap_or(0.0);
            let new_score = clamp(prior + delta);
            tx.execute(
                "INSERT INTO capability_scores (track, dimension, quarter, score) VALUES (?1, ?2, ?3, ?4)",
                params![track, dimension.as_str(), next_q, new_score],
            )?;
        }
    }

    for ls in systems.values() {
        tx.execute(
            "UPDATE legacy_systems SET status = ?1, status_since_quarter = ?2 WHERE track = ?3",
            params![ls.status.as_str(), ls.status_since_quarter, ls.track],
        )?;
    }

    tx.execute(
        "UPDATE transformation_clock SET current_quarter = ?1 WHERE id = 1",
        params![next_q],
    )?;
    tx.commit()?;

    get_state(conn)
}

fn maturity_trend(
    conn: &Connection,
    up_to_quarter: i64,
) -> rusty_sqlite::rusqlite::Result<Vec<MaturityPoint>> {
    let mut stmt =
        conn.prepare("SELECT quarter, score FROM capability_scores WHERE quarter <= ?1")?;
    let rows = stmt.query_map(params![up_to_quarter], |row| {
        let quarter: i64 = row.get(0)?;
        let score: f64 = row.get(1)?;
        Ok((quarter, score))
    })?;
    let mut by_quarter: HashMap<i64, Vec<f64>> = HashMap::new();
    for row in rows {
        let (quarter, score) = row?;
        by_quarter.entry(quarter).or_default().push(score);
    }
    let mut points: Vec<MaturityPoint> = by_quarter
        .into_iter()
        .map(|(quarter, scores)| {
            let mean = scores.iter().sum::<f64>() / scores.len() as f64;
            MaturityPoint {
                quarter,
                maturity_index: (mean * 100.0).round() / 100.0,
            }
        })
        .collect();
    points.sort_by_key(|point| point.quarter);
    Ok(points)
}

fn decision_refs(
    conn: &Connection,
    sql: &str,
    params: impl rusty_sqlite::rusqlite::Params,
) -> rusty_sqlite::rusqlite::Result<Vec<DecisionRef>> {
    let mut stmt = conn.prepare(sql)?;
    let rows = stmt.query_map(params, |row| {
        let decision_type_str: String = row.get(2)?;
        Ok(DecisionRef {
            id: row.get(0)?,
            quarter: row.get(1)?,
            decision_type: DecisionType::parse(&decision_type_str)
                .unwrap_or(DecisionType::MigrateTrack),
            target: row.get(3)?,
        })
    })?;
    rows.collect()
}

/// Returns a full snapshot: quarter, legacy systems, capability scores,
/// maturity trend, pending decisions, and recent decision history.
pub fn get_state(conn: &Connection) -> rusty_sqlite::rusqlite::Result<TransformationState> {
    let quarter = get_or_create_clock(conn)?;
    let systems = load_legacy_systems(conn)?;
    let current_scores = latest_scores(conn, quarter)?;

    let mut legacy_systems: Vec<LegacySystem> = systems.into_values().collect();
    legacy_systems.sort_by(|a, b| a.track.cmp(&b.track));

    let pending_decisions = decision_refs(
        conn,
        "SELECT id, quarter, decision_type, target FROM transformation_decisions WHERE applied = 0 ORDER BY id",
        [],
    )?;
    let decision_history = decision_refs(
        conn,
        "SELECT id, quarter, decision_type, target FROM transformation_decisions WHERE applied = 1 \
         ORDER BY quarter DESC LIMIT 20",
        [],
    )?;

    Ok(TransformationState {
        quarter,
        legacy_systems,
        capability: current_scores,
        maturity_trend: maturity_trend(conn, quarter)?,
        pending_decisions,
        decision_history,
    })
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::transformation::seed::seed_transformation_state;

    fn seeded_connection() -> Connection {
        let conn = Connection::open_in_memory().unwrap();
        ensure_schema(&conn).unwrap();
        seed_transformation_state(&conn).unwrap();
        conn
    }

    fn status_of<'a>(state: &'a TransformationState, track: &str) -> &'a LegacySystem {
        state
            .legacy_systems
            .iter()
            .find(|ls| ls.track == track)
            .unwrap()
    }

    #[test]
    fn get_or_create_clock_creates_singleton_row_at_zero() {
        let conn = Connection::open_in_memory().unwrap();
        ensure_schema(&conn).unwrap();
        assert_eq!(get_or_create_clock(&conn).unwrap(), 0);
        // A second call must not create a duplicate row / must return
        // the same value.
        assert_eq!(get_or_create_clock(&conn).unwrap(), 0);
    }

    #[test]
    fn queue_decision_targets_current_quarter_plus_one() {
        let mut conn = seeded_connection();
        advance_quarter(&mut conn).unwrap(); // quarter -> 1
        let decision =
            queue_decision(&conn, DecisionType::MigrateTrack, "personnel-lifecycle").unwrap();
        assert_eq!(decision.quarter, 2);
        assert_eq!(decision.target, "personnel-lifecycle");
    }

    #[test]
    fn migrate_track_on_legacy_system_transitions_to_dual_write() {
        let mut conn = seeded_connection();
        queue_decision(&conn, DecisionType::MigrateTrack, "personnel-lifecycle").unwrap();
        let state = advance_quarter(&mut conn).unwrap();

        let ls = status_of(&state, "personnel-lifecycle");
        assert_eq!(ls.status, SystemStatus::DualWrite);
        assert_eq!(ls.status_since_quarter, 1);
        let scores = &state.capability["personnel-lifecycle"];
        assert_eq!(scores[&CapabilityDimension::DomainOwnership], 1.5); // 1.0 baseline + 0.5
        assert_eq!(scores[&CapabilityDimension::DataAsAProduct], 1.5);
        // Untouched dimensions carry the baseline forward unchanged.
        assert_eq!(scores[&CapabilityDimension::SelfServePlatform], 1.0);
        assert_eq!(scores[&CapabilityDimension::FederatedGovernance], 1.0);
    }

    #[test]
    fn migrate_track_on_non_legacy_system_is_rejected() {
        let mut conn = seeded_connection();
        queue_decision(&conn, DecisionType::MigrateTrack, "personnel-lifecycle").unwrap();
        advance_quarter(&mut conn).unwrap(); // now DUAL_WRITE

        queue_decision(&conn, DecisionType::MigrateTrack, "personnel-lifecycle").unwrap();
        let state = advance_quarter(&mut conn).unwrap(); // quarter 2, decision rejected

        let ls = status_of(&state, "personnel-lifecycle");
        assert_eq!(ls.status, SystemStatus::DualWrite);
        assert_eq!(ls.status_since_quarter, 1); // unchanged since the rejected decision
    }

    #[test]
    fn migrate_track_on_unknown_track_is_rejected_without_error() {
        let mut conn = seeded_connection();
        queue_decision(&conn, DecisionType::MigrateTrack, "no-such-track").unwrap();
        // Must not panic/error even though the target doesn't exist.
        let state = advance_quarter(&mut conn).unwrap();
        assert_eq!(state.quarter, 1);
    }

    #[test]
    fn sunset_legacy_on_dual_write_is_a_clean_cutover() {
        let mut conn = seeded_connection();
        queue_decision(&conn, DecisionType::MigrateTrack, "personnel-lifecycle").unwrap();
        advance_quarter(&mut conn).unwrap(); // quarter 1: DUAL_WRITE

        queue_decision(&conn, DecisionType::SunsetLegacy, "personnel-lifecycle").unwrap();
        let state = advance_quarter(&mut conn).unwrap(); // quarter 2: MIGRATED

        let ls = status_of(&state, "personnel-lifecycle");
        assert_eq!(ls.status, SystemStatus::Migrated);
        assert_eq!(ls.status_since_quarter, 2);
        let scores = &state.capability["personnel-lifecycle"];
        // 1.0 baseline + 0.5 (migrate) + 0.5 (clean cutover) = 2.0
        assert_eq!(scores[&CapabilityDimension::FederatedGovernance], 1.5); // 1.0 + 0.5 cutover only
        assert!((scores[&CapabilityDimension::DataAsAProduct] - 1.8).abs() < 1e-9);
        // 1.0 + 0.5 + 0.3
    }

    #[test]
    fn sunset_legacy_on_legacy_is_a_risky_sunset() {
        let mut conn = seeded_connection();
        queue_decision(&conn, DecisionType::SunsetLegacy, "personnel-lifecycle").unwrap();
        let state = advance_quarter(&mut conn).unwrap();

        let ls = status_of(&state, "personnel-lifecycle");
        assert_eq!(ls.status, SystemStatus::Decommissioned);
        assert_eq!(ls.status_since_quarter, 1);
        let scores = &state.capability["personnel-lifecycle"];
        assert_eq!(scores[&CapabilityDimension::FederatedGovernance], 0.0); // 1.0 - 1.0, not clamped below 0
        assert!((scores[&CapabilityDimension::DataAsAProduct] - 0.5).abs() < 1e-9);
        // 1.0 - 0.5
    }

    #[test]
    fn sunset_legacy_on_already_migrated_or_decommissioned_is_rejected() {
        let mut conn = seeded_connection();
        queue_decision(&conn, DecisionType::SunsetLegacy, "personnel-lifecycle").unwrap();
        advance_quarter(&mut conn).unwrap(); // now DECOMMISSIONED

        queue_decision(&conn, DecisionType::SunsetLegacy, "personnel-lifecycle").unwrap();
        let state = advance_quarter(&mut conn).unwrap(); // rejected, no further change

        let ls = status_of(&state, "personnel-lifecycle");
        assert_eq!(ls.status, SystemStatus::Decommissioned);
        assert_eq!(ls.status_since_quarter, 1);
    }

    #[test]
    fn scores_are_clamped_to_five() {
        let mut conn = seeded_connection();
        // Migrate then sunset repeatedly isn't possible (state machine
        // prevents re-migrating), so drive the clamp via repeated
        // platform investment instead: +0.3 per quarter, from a 1.0
        // baseline, needs many quarters to hit 5.0 -- push well past it.
        for _ in 0..20 {
            queue_decision(&conn, DecisionType::InvestPlatform, "platform").unwrap();
            advance_quarter(&mut conn).unwrap();
        }
        let state = get_state(&conn).unwrap();
        for scores in state.capability.values() {
            assert_eq!(scores[&CapabilityDimension::SelfServePlatform], 5.0);
        }
    }

    #[test]
    fn invest_platform_lifts_self_serve_platform_mesh_wide() {
        let mut conn = seeded_connection();
        queue_decision(&conn, DecisionType::InvestPlatform, "platform").unwrap();
        let state = advance_quarter(&mut conn).unwrap();

        for track in [
            "personnel-lifecycle",
            "position-management",
            "readiness-reporting",
        ] {
            let scores = &state.capability[track];
            assert!((scores[&CapabilityDimension::SelfServePlatform] - 1.3).abs() < 1e-9);
        }
    }

    #[test]
    fn invest_product_teams_lifts_domain_ownership_and_data_as_a_product_mesh_wide() {
        let mut conn = seeded_connection();
        queue_decision(&conn, DecisionType::InvestProductTeams, "product_teams").unwrap();
        let state = advance_quarter(&mut conn).unwrap();

        for track in [
            "personnel-lifecycle",
            "position-management",
            "readiness-reporting",
        ] {
            let scores = &state.capability[track];
            assert!((scores[&CapabilityDimension::DomainOwnership] - 1.2).abs() < 1e-9);
            assert!((scores[&CapabilityDimension::DataAsAProduct] - 1.2).abs() < 1e-9);
        }
    }

    #[test]
    fn dual_write_auto_completes_after_minimum_quarters_with_no_sunset_decision() {
        let mut conn = seeded_connection();
        queue_decision(&conn, DecisionType::MigrateTrack, "personnel-lifecycle").unwrap();
        advance_quarter(&mut conn).unwrap(); // q1: DUAL_WRITE, since=1

        let state = advance_quarter(&mut conn).unwrap(); // q2: 2 - 1 = 1 < 2, not yet
        assert_eq!(
            status_of(&state, "personnel-lifecycle").status,
            SystemStatus::DualWrite
        );

        let state = advance_quarter(&mut conn).unwrap(); // q3: 3 - 1 = 2 >= 2, auto-completes
        let ls = status_of(&state, "personnel-lifecycle");
        assert_eq!(ls.status, SystemStatus::Migrated);
        assert_eq!(ls.status_since_quarter, 3);
    }

    #[test]
    fn advance_quarter_increments_clock_even_with_no_pending_decisions() {
        let mut conn = seeded_connection();
        let state = advance_quarter(&mut conn).unwrap();
        assert_eq!(state.quarter, 1);
    }

    #[test]
    fn advance_quarter_marks_processed_decisions_applied() {
        let mut conn = seeded_connection();
        queue_decision(&conn, DecisionType::MigrateTrack, "personnel-lifecycle").unwrap();
        let state = advance_quarter(&mut conn).unwrap();
        assert!(state.pending_decisions.is_empty());
        assert_eq!(state.decision_history.len(), 1);
        assert_eq!(
            state.decision_history[0].decision_type,
            DecisionType::MigrateTrack
        );
    }

    #[test]
    fn decision_history_is_capped_at_twenty_most_recent() {
        let mut conn = seeded_connection();
        for _ in 0..25 {
            queue_decision(&conn, DecisionType::InvestPlatform, "platform").unwrap();
            advance_quarter(&mut conn).unwrap();
        }
        let state = get_state(&conn).unwrap();
        assert_eq!(state.decision_history.len(), 20);
        // Most recent quarter first.
        assert_eq!(state.decision_history[0].quarter, 25);
    }

    #[test]
    fn maturity_trend_has_one_point_per_quarter_from_zero() {
        let mut conn = seeded_connection();
        advance_quarter(&mut conn).unwrap();
        advance_quarter(&mut conn).unwrap();
        let state = get_state(&conn).unwrap();
        assert_eq!(state.maturity_trend.len(), 3); // quarters 0, 1, 2
        assert_eq!(state.maturity_trend[0].quarter, 0);
        assert_eq!(state.maturity_trend[0].maturity_index, 1.0); // all baseline 1.0
    }

    #[test]
    fn get_state_on_a_fresh_unseeded_database_has_no_legacy_systems() {
        let conn = Connection::open_in_memory().unwrap();
        ensure_schema(&conn).unwrap();
        let state = get_state(&conn).unwrap();
        assert_eq!(state.quarter, 0);
        assert!(state.legacy_systems.is_empty());
        assert!(state.capability.is_empty());
    }
}
