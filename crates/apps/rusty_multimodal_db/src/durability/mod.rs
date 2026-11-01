//! The durable half of `ProductionStore`: [`mmap_store::MmapAgeStore`] (the
//! mmap'd age slot file), the companion record blob, and
//! [`DurabilityError`], re-exported from the engine (ADR-0124).
//!
//! The seven other durability variants ADR-0005 and ADR-0006 compared (WAL,
//! snapshot, hybrid, LSM, embedded engine) were removed by ADR-0138;
//! `RESULTS.md` keeps their measurements and the commit ADR-0138 names
//! keeps their code.

use crate::store::StoreError;
pub(crate) use rusty_multimodal_db_engine::durability::sync_parent_dir;
/// Extracted to `rusty_multimodal_db_engine` with the generic store
/// (ADR-0124); re-exported so every path in this crate is unchanged.
pub use rusty_multimodal_db_engine::durability::DurabilityError;

/// Below this many records, spawning a thread to parallelize index
/// construction in [`mmap_store::MmapAgeStore`] costs more than the work it parallelizes —
/// sequential wins. Measured directly, not guessed: two independent,
/// warm-up-corrected 9-sample runs comparing sequential vs. parallel
/// construction at 500/800/1,000/1,200/1,500/1,800/2,000/3,000/5,000
/// records agreed that ≤800 records reliably favors sequential and
/// ≥1,500 reliably favors parallel, with 1,000–1,200 landing as a genuine
/// coin-flip between the two runs — not a clean crossover number. This
/// threshold sits at the upper, confidently-parallel edge of that
/// measured range, erring toward the known-safe sequential path through
/// the ambiguous 1,000–1,200 band rather than risking the very regression
/// this threshold exists to prevent. (Measured when four construction
/// sites shared it; the other three were removed by ADR-0138.)
pub(crate) const PARALLEL_CONSTRUCTION_THRESHOLD: usize = 1_500;

pub mod mmap_store;
pub(crate) mod record_blob;

pub use mmap_store::MmapAgeStore;

/// Lets every variant's `DogStore::update_age` impl use `?` directly on a
/// `Result<_, DurabilityError>`-returning call, even though the trait
/// method itself must return `Result<(), StoreError>`. An unknown-UUID
/// failure (the one case `StoreError` already models) round-trips as
/// itself; anything else becomes `StoreError::Durability`.
impl From<DurabilityError> for StoreError {
    fn from(error: DurabilityError) -> Self {
        match error {
            DurabilityError::Store(boxed) => match boxed.downcast::<StoreError>() {
                Ok(store_error) => *store_error,
                Err(other) => StoreError::Durability(other.to_string()),
            },
            other => StoreError::Durability(other.to_string()),
        }
    }
}

/// The other half of the round trip: `DurabilityError` now lives in
/// `rusty_multimodal_db_engine`, which cannot name the Dog `StoreError`, so
/// its `Store` variant holds a boxed error and this impl (legal here because
/// `StoreError` is this crate's) keeps every `?` on a `StoreError` working.
impl From<StoreError> for DurabilityError {
    fn from(error: StoreError) -> Self {
        DurabilityError::Store(Box::new(error))
    }
}

#[cfg(test)]
pub(crate) mod test_support {
    use crate::record::DogRecord;
    use uuid::Uuid;

    pub(crate) fn sample_records() -> Vec<DogRecord> {
        vec![
            DogRecord::new(Uuid::from_u128(1), "labrador", 3),
            DogRecord::new(Uuid::from_u128(2), "labrador", 5),
            DogRecord::new(Uuid::from_u128(3), "poodle", 2),
        ]
    }

    pub(crate) fn sample_edges() -> Vec<(Uuid, Uuid)> {
        vec![(Uuid::from_u128(1), Uuid::from_u128(2))]
    }
}
