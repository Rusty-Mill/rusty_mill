//! Idempotent seed data for the digital transformation simulator -- the
//! Rust port of `meshed.transformation.seed`.
//!
//! Seeds the three manpower tracks with their legacy predecessors --
//! the systems that existed before the corresponding meshed data
//! product took over -- plus a baseline (low) capability score across
//! all four dimensions at quarter 0.

use super::enums::{CapabilityDimension, SystemStatus};
use rusty_sqlite::rusqlite::{params, Connection};

/// `(track, legacy system name, target data product)` -- one per
/// manpower track.
const LEGACY_SYSTEMS: [(&str, &str, &str); 3] = [
    (
        "personnel-lifecycle",
        "Personnel Legacy DB",
        "personnel-lifecycle",
    ),
    (
        "position-management",
        "Position Management Spreadsheets",
        "position-management",
    ),
    (
        "readiness-reporting",
        "Readiness Manual Reports",
        "readiness-reporting",
    ),
];

/// Baseline maturity: pre-transformation systems score low on every
/// principle.
const BASELINE_SCORE: f64 = 1.0;

/// Seeds legacy systems, baseline scores, and the clock if not already
/// present. Safe to call on every app startup -- no-ops if a
/// `transformation_clock` row already exists.
pub fn seed_transformation_state(conn: &Connection) -> rusty_sqlite::rusqlite::Result<()> {
    let already_seeded: bool = conn.query_row(
        "SELECT EXISTS(SELECT 1 FROM transformation_clock)",
        [],
        |row| row.get(0),
    )?;
    if already_seeded {
        return Ok(());
    }

    conn.execute(
        "INSERT INTO transformation_clock (id, current_quarter) VALUES (1, 0)",
        [],
    )?;

    for (track, name, target_product) in LEGACY_SYSTEMS {
        conn.execute(
            "INSERT INTO legacy_systems (track, name, target_data_product, status, status_since_quarter)
             VALUES (?1, ?2, ?3, ?4, 0)",
            params![track, name, target_product, SystemStatus::Legacy.as_str()],
        )?;
        for dimension in CapabilityDimension::ALL {
            conn.execute(
                "INSERT INTO capability_scores (track, dimension, quarter, score) VALUES (?1, ?2, 0, ?3)",
                params![track, dimension.as_str(), BASELINE_SCORE],
            )?;
        }
    }

    Ok(())
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::transformation::engine::{ensure_schema, get_state};

    fn seeded_connection() -> Connection {
        let conn = Connection::open_in_memory().unwrap();
        ensure_schema(&conn).unwrap();
        seed_transformation_state(&conn).unwrap();
        conn
    }

    #[test]
    fn seeds_exactly_three_legacy_systems() {
        let conn = seeded_connection();
        let state = get_state(&conn).unwrap();
        assert_eq!(state.legacy_systems.len(), 3);

        let by_track: std::collections::HashMap<_, _> = state
            .legacy_systems
            .iter()
            .map(|ls| (ls.track.as_str(), ls))
            .collect();
        let personnel = by_track["personnel-lifecycle"];
        assert_eq!(personnel.name, "Personnel Legacy DB");
        assert_eq!(personnel.target_data_product, "personnel-lifecycle");
        assert_eq!(personnel.status, SystemStatus::Legacy);
        assert_eq!(personnel.status_since_quarter, 0);

        assert!(by_track.contains_key("position-management"));
        assert!(by_track.contains_key("readiness-reporting"));
    }

    #[test]
    fn seeds_baseline_score_for_every_track_and_dimension() {
        let conn = seeded_connection();
        let state = get_state(&conn).unwrap();
        assert_eq!(state.capability.len(), 3);
        for scores in state.capability.values() {
            assert_eq!(scores.len(), 4);
            for score in scores.values() {
                assert_eq!(*score, 1.0);
            }
        }
    }

    #[test]
    fn seeds_the_clock_at_quarter_zero() {
        let conn = seeded_connection();
        let state = get_state(&conn).unwrap();
        assert_eq!(state.quarter, 0);
    }

    #[test]
    fn is_idempotent_when_already_seeded() {
        let conn = Connection::open_in_memory().unwrap();
        ensure_schema(&conn).unwrap();
        seed_transformation_state(&conn).unwrap();

        // A second call must not duplicate rows.
        seed_transformation_state(&conn).unwrap();
        let state = get_state(&conn).unwrap();
        assert_eq!(state.legacy_systems.len(), 3);
    }

    #[test]
    fn does_not_reseed_after_the_clock_has_advanced() {
        let conn = seeded_connection();
        conn.execute(
            "UPDATE transformation_clock SET current_quarter = 5 WHERE id = 1",
            [],
        )
        .unwrap();

        seed_transformation_state(&conn).unwrap();
        let state = get_state(&conn).unwrap();
        assert_eq!(state.quarter, 5);
        assert_eq!(state.legacy_systems.len(), 3);
    }
}
