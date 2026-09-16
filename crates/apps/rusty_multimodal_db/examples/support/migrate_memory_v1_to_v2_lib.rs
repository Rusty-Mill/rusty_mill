//! Shared migration logic for `Memory@1` (pre-`ADR-0056`, 12 fields) to
//! `Memory@2` (14 fields — `ADR-0056`'s `deleted_at_unix_ms`/`node_id`
//! sentinels). Included via `#[path]`, not published, by both
//! `examples/migrate_memory_v1_to_v2.rs` (the real CLI) and
//! `tests/schema_migration.rs` (the regression test), so both exercise
//! the exact same code rather than two hand-kept-in-sync copies.
//!
//! See `docs/design/SCHEMA-MIGRATION-DESIGN.md` (`ADR-0066`) for the
//! full design. Why this lives outside `src/`, not as a new library
//! module: the whole pattern is composed entirely from primitives
//! `crate::generic` already exposes publicly (`GenericMmapStore::
//! open_portable`, `MultiSymmetric::open_portable`,
//! `create_memory_production_stack`) — this file adds a caller-defined
//! old-layout type and a hand-written field mapping, the two pieces
//! `AGENTS.md`'s "no speculative generality" rule says stay bespoke per
//! migration, not a reusable library abstraction built from one real
//! case (`MIG-FR-001`/`002`/`007`).
#![allow(dead_code)] // the CLI and the test each use only part of this file

use rusty_multimodal_db::durability::DurabilityError;
use rusty_multimodal_db::generic::memory::{
    create_memory_production_stack, AccessCountField, CategoryField, Memory, MEMORY_RELATION_LABELS,
};
use rusty_multimodal_db::generic::mmap_store::GenericMmapStore;
use rusty_multimodal_db::generic::query::{AllIds, GetById, MultiNeighbors};
use rusty_multimodal_db::generic::store::MultiSymmetric;
use rusty_multimodal_db::generic::traits::{IndexedField, Record, ScannableField, SchemaTag};
use std::path::Path;
use uuid::Uuid;

/// `Memory`'s layout before `ADR-0056` appended `deleted_at_unix_ms`/
/// `node_id` — the same twelve fields, in the same order, as `Memory`'s
/// own current struct minus those two.
///
/// **Reconstructed, not recovered**: this crate's `git subtree` import
/// into the `rusty_mill` monorepo squashed the standalone repo's
/// history, so the exact pre-bump tag literal (below) cannot be
/// independently verified from this repository. It is grounded in
/// `ADR-0056`'s own "bump `Memory`'s schema tag *to* `memory::
/// Memory@2`" phrasing and every other never-bumped type's identical
/// no-suffix `module::Type` convention (`entity::Entity`,
/// `order_customer::Order`, `relation::Relation`, `reminder::
/// Reminder`) — see `docs/design/SCHEMA-MIGRATION-DESIGN.md`'s Context
/// for the full account. `ADR-0056` itself records that no deployment
/// holds a layout-1 `Memory` directory today, so there is nothing to
/// recover — this type is a synthetic, realistic stand-in for
/// `ADR-0056`'s exact field diff, proving the migration *mechanism*,
/// not a recovered historical artifact.
#[derive(Debug, Clone, PartialEq, serde::Serialize, serde::Deserialize)]
pub struct MemoryV1 {
    pub id: Uuid,
    pub content: String,
    pub category: String,
    pub tags: Vec<String>,
    pub source: String,
    pub metadata_json: String,
    pub created_at_unix_ms: i64,
    pub updated_at_unix_ms: i64,
    pub memory_type: String,
    pub status: String,
    pub sensitive: bool,
    pub access_count: i64,
}

impl Record for MemoryV1 {
    type Id = Uuid;
    fn id(&self) -> Uuid {
        self.id
    }
}

impl SchemaTag for MemoryV1 {
    const SCHEMA_TAG: &'static str = "memory::Memory";
}

// Reuses `Memory`'s own marker types directly: `category`/
// `access_count` are byte-for-byte unchanged across the `@1` -> `@2`
// bump, so there is no need for `MemoryV1` to mint its own markers — a
// marker is a zero-sized tag, not tied to one record type.
impl IndexedField<CategoryField> for MemoryV1 {
    type IndexValue = String;
    fn indexed_value(&self) -> &String {
        &self.category
    }
}

impl ScannableField<AccessCountField> for MemoryV1 {
    type ScanValue = i64;
    fn scannable_value(&self) -> i64 {
        self.access_count
    }
    fn set_scannable_value(&mut self, value: i64) {
        self.access_count = value;
    }
}

/// `open_memory_production_stack_portable`'s own three-line body, typed
/// over `MemoryV1` instead of `Memory` — the read half of the pattern.
/// No `Ordered` wrapper: migration only needs `AllIds`/`GetById`/
/// `MultiNeighbors`, all present without it, and `Ordered` writes no
/// companion file of its own to skip reading.
///
/// # Errors
///
/// Returns [`DurabilityError::RecordBlobUnreadable`] if `path` is not a
/// `memory::Memory`-tagged (pre-`ADR-0056`) directory — the same error,
/// unchanged, any other `open_portable` call reports on a schema-tag
/// mismatch.
pub fn open_memory_v1_stack_portable(
    path: &Path,
) -> Result<
    MultiSymmetric<GenericMmapStore<MemoryV1, CategoryField, AccessCountField>, MemoryV1>,
    DurabilityError,
> {
    let core = GenericMmapStore::<MemoryV1, CategoryField, AccessCountField>::open_portable(path)?;
    MultiSymmetric::open_portable(core, path, &MEMORY_RELATION_LABELS)
}

/// Every field named explicitly, not derived — `ADR-0056`'s own
/// documented sentinels: `0` for "live", `""` for "unattributed".
pub fn migrate_memory_v1_to_v2(old: MemoryV1) -> Memory {
    Memory {
        id: old.id,
        content: old.content,
        category: old.category,
        tags: old.tags,
        source: old.source,
        metadata_json: old.metadata_json,
        created_at_unix_ms: old.created_at_unix_ms,
        updated_at_unix_ms: old.updated_at_unix_ms,
        memory_type: old.memory_type,
        status: old.status,
        sensitive: old.sensitive,
        access_count: old.access_count,
        deleted_at_unix_ms: 0,
        node_id: String::new(),
    }
}

/// What one migration run did — printed by the CLI, asserted by the
/// regression test.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct MigrationReport {
    pub records: usize,
    pub mentions_edges: usize,
}

/// Error [`migrate`] can return, before any write against `new_path`
/// beyond the existence check.
#[derive(Debug)]
pub enum MigrateError {
    /// `new_path` already exists — refused before anything is written.
    DestinationExists,
    Durability(DurabilityError),
}

impl std::fmt::Display for MigrateError {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        match self {
            MigrateError::DestinationExists => {
                write!(f, "destination already exists; refusing to overwrite it")
            }
            MigrateError::Durability(e) => write!(f, "{e}"),
        }
    }
}

impl std::error::Error for MigrateError {}

impl From<DurabilityError> for MigrateError {
    fn from(e: DurabilityError) -> Self {
        MigrateError::Durability(e)
    }
}

/// The whole pattern, end to end: read every `MemoryV1` record and
/// every `mentions` edge from `old_path` (the insert log already
/// folded in, `INS-FR-004`), convert each record, then write a fresh
/// `Memory@2` directory at `new_path` via the existing, **unmodified**
/// `create_memory_production_stack` — reusing the exact write path
/// every real `Memory` deployment already goes through, not a new one.
///
/// `old_path` is never opened for writing. `new_path` must not already
/// exist: a botched or interrupted run can corrupt at most the (fresh,
/// disposable) destination, never the only copy of the source data —
/// the same refusal `Backup` (`ADR-0065`) already established for a
/// server-produced target directory.
///
/// # Errors
///
/// [`MigrateError::DestinationExists`] if `new_path` already exists;
/// [`MigrateError::Durability`] wrapping whatever reading `old_path` or
/// writing `new_path` returns.
pub fn migrate(old_path: &Path, new_path: &Path) -> Result<MigrationReport, MigrateError> {
    if new_path.exists() {
        return Err(MigrateError::DestinationExists);
    }
    let old_stack = open_memory_v1_stack_portable(old_path)?;
    let ids = old_stack.all_ids();
    let mut records = Vec::with_capacity(ids.len());
    let mut edges = Vec::new();
    for id in ids {
        let old = old_stack
            .get(id)
            .expect("id came from all_ids() on this same stack");
        // Memory ids only ever appear on the "near" side of `mentions`
        // (the far side is `Entity` ids, never enumerated by
        // `MemoryV1::all_ids()`), so no dedup is needed.
        if let Some(mentioned) = old_stack.neighbors_by_relation(MEMORY_RELATION_LABELS[0], id) {
            edges.extend(mentioned.into_iter().map(|entity_id| (id, entity_id)));
        }
        records.push(migrate_memory_v1_to_v2(old));
    }
    let report = MigrationReport {
        records: records.len(),
        mentions_edges: edges.len(),
    };
    create_memory_production_stack(records, &edges, new_path)?;
    Ok(report)
}
