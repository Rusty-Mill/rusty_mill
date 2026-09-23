//! `ADR-0066` (`SCHEMA-MIGRATION-DESIGN`): proves the three-step
//! migration pattern end to end against a synthetic pre-`ADR-0056`
//! (`memory::Memory`, 12-field) `Memory` directory, reopening the
//! result through `Memory`'s **actual, current, unmodified** production
//! code path (`open_memory_production_stack_portable` — the same
//! function `memory_server.rs` calls on every restart), not just
//! checking the migration's own internal state.
//!
//! Shares its migration logic with `examples/migrate_memory_v1_to_v2.rs`
//! via `#[path]` (see that file's own module doc comment for why) — this
//! test exercises the exact code the real CLI runs, not a reimplementation.
#[path = "../examples/support/migrate_memory_v1_to_v2_lib.rs"]
mod migration;

use migration::{migrate, MemoryV1, MigrateError};
use rusty_multimodal_db::durability::DurabilityError;
use rusty_multimodal_db::generic::memory::{
    create_memory_production_stack, open_memory_production_stack_portable, AccessCountField,
    CategoryField, MEMORY_RELATION_LABELS,
};
use rusty_multimodal_db::generic::mmap_store::GenericMmapStore;
use rusty_multimodal_db::generic::query::{GetById, MultiNeighbors};
use rusty_multimodal_db::generic::store::MultiSymmetric;
use std::path::PathBuf;
use uuid::Uuid;

fn unique_dir(label: &str) -> PathBuf {
    use std::sync::atomic::{AtomicU64, Ordering};
    static COUNTER: AtomicU64 = AtomicU64::new(0);
    let n = COUNTER.fetch_add(1, Ordering::Relaxed);
    std::env::temp_dir().join(format!("{label}_{}_{n}", std::process::id()))
}

/// A fresh, not-yet-existing `memories.mmap` path under a fresh
/// directory — the store-path shape every helper in this file takes,
/// matching `create_memory_production_stack`'s own convention.
fn fresh_store_path(label: &str) -> PathBuf {
    unique_dir(label).join("memories.mmap")
}

fn memory_v1(n: u128, category: &str, sensitive: bool) -> MemoryV1 {
    MemoryV1 {
        id: Uuid::from_u128(n),
        content: format!("memory {n}"),
        category: category.into(),
        tags: vec!["t".into(), format!("n{n}")],
        source: "manual".into(),
        metadata_json: "{}".into(),
        created_at_unix_ms: 1_000 * n as i64,
        updated_at_unix_ms: 1_000 * n as i64,
        memory_type: "unclassified".into(),
        status: "active".into(),
        sensitive,
        access_count: n as i64,
    }
}

/// Builds a real, on-disk `memory::Memory`-tagged (pre-`ADR-0056`)
/// store at `path` (a `memories.mmap`-shaped path, not yet existing) —
/// the fixture every test below migrates from. Uses the same public
/// primitives `create_memory_production_stack` itself uses, just typed
/// over `MemoryV1` — there is no `create_memory_v1_production_stack`
/// helper in the library, since a migration only ever *reads* an
/// old-tagged directory, never creates one; this exists purely to
/// build a realistic fixture for this test.
fn create_memory_v1_fixture(
    memories: Vec<MemoryV1>,
    mentions: &[(Uuid, Uuid)],
    path: &std::path::Path,
) -> Result<(), DurabilityError> {
    let core =
        GenericMmapStore::<MemoryV1, CategoryField, AccessCountField>::create(memories, path)?;
    let labeled = [(MEMORY_RELATION_LABELS[0].to_string(), mentions.to_vec())];
    MultiSymmetric::<GenericMmapStore<MemoryV1, CategoryField, AccessCountField>, MemoryV1>::create(
        core, &labeled, path,
    )?;
    Ok(())
}

/// Acceptance criteria 2-4 (`docs/design/SCHEMA-MIGRATION-DESIGN.md`):
/// a migrated directory opens through `Memory`'s real production code
/// path, every field survives byte-for-byte, both sentinels are set,
/// and every `mentions` edge is present, unchanged.
#[test]
fn a_migrated_directory_reopens_through_the_real_production_code_path_with_every_field_and_edge_intact(
) {
    let old_path = fresh_store_path("schema_migration_v1_source");
    let new_path = fresh_store_path("schema_migration_v2_target");
    let entity_a = Uuid::from_u128(1000);
    let entity_b = Uuid::from_u128(1001);
    let memory_1 = memory_v1(1, "general", false);
    let memory_2 = memory_v1(2, "preference", true);
    let mentions = [
        (memory_1.id, entity_a),
        (memory_1.id, entity_b),
        (memory_2.id, entity_a),
    ];
    create_memory_v1_fixture(
        vec![memory_1.clone(), memory_2.clone()],
        &mentions,
        &old_path,
    )
    .unwrap();

    let report = migrate(&old_path, &new_path).unwrap();
    assert_eq!(report.records, 2);
    assert_eq!(report.mentions_edges, 3);

    let stack = open_memory_production_stack_portable(&new_path).unwrap();
    let m1 = stack.get(memory_1.id).unwrap();
    assert_eq!(m1.content, memory_1.content);
    assert_eq!(m1.category, memory_1.category);
    assert_eq!(m1.tags, memory_1.tags);
    assert_eq!(m1.source, memory_1.source);
    assert_eq!(m1.metadata_json, memory_1.metadata_json);
    assert_eq!(m1.created_at_unix_ms, memory_1.created_at_unix_ms);
    assert_eq!(m1.updated_at_unix_ms, memory_1.updated_at_unix_ms);
    assert_eq!(m1.memory_type, memory_1.memory_type);
    assert_eq!(m1.status, memory_1.status);
    assert_eq!(m1.sensitive, memory_1.sensitive);
    assert_eq!(m1.access_count, memory_1.access_count);
    assert_eq!(m1.deleted_at_unix_ms, 0, "ADR-0056's live sentinel");
    assert_eq!(m1.node_id, "", "ADR-0056's unattributed sentinel");

    let m2 = stack.get(memory_2.id).unwrap();
    assert_eq!(m2.sensitive, memory_2.sensitive);
    assert_eq!(m2.deleted_at_unix_ms, 0);
    assert_eq!(m2.node_id, "");

    let mut m1_mentions = stack
        .neighbors_by_relation(MEMORY_RELATION_LABELS[0], memory_1.id)
        .unwrap();
    m1_mentions.sort();
    let mut expected = vec![entity_a, entity_b];
    expected.sort();
    assert_eq!(m1_mentions, expected);
    assert_eq!(
        stack
            .neighbors_by_relation(MEMORY_RELATION_LABELS[0], memory_2.id)
            .unwrap(),
        vec![entity_a]
    );

    let _ = std::fs::remove_dir_all(old_path.parent().unwrap());
    let _ = std::fs::remove_dir_all(new_path.parent().unwrap());
}

/// Acceptance criterion 5: an existing destination is refused before
/// anything is written, and is left byte-for-byte unchanged.
#[test]
fn migrate_refuses_an_existing_destination_and_leaves_it_untouched() {
    let old_path = fresh_store_path("schema_migration_v1_source");
    create_memory_v1_fixture(vec![memory_v1(1, "general", false)], &[], &old_path).unwrap();

    let new_path = fresh_store_path("schema_migration_v2_target");
    std::fs::create_dir_all(new_path.parent().unwrap()).unwrap();
    std::fs::write(&new_path, b"pre-existing, not a real store").unwrap();

    let err = migrate(&old_path, &new_path).unwrap_err();
    assert!(matches!(err, MigrateError::DestinationExists));
    assert_eq!(
        std::fs::read(&new_path).unwrap(),
        b"pre-existing, not a real store"
    );
    assert_eq!(
        std::fs::read_dir(new_path.parent().unwrap())
            .unwrap()
            .count(),
        1,
        "nothing else was written beside the pre-existing destination"
    );

    let _ = std::fs::remove_dir_all(old_path.parent().unwrap());
    let _ = std::fs::remove_dir_all(new_path.parent().unwrap());
}

/// Acceptance criterion 6: a store already tagged `memory::Memory@2`
/// (the current type, not the pre-`ADR-0056` one) is refused with the
/// existing schema-tag-mismatch error — no silent no-op, no partial
/// write, and the same error vocabulary every other `open_portable`
/// call already uses.
#[test]
fn migrate_refuses_a_store_already_tagged_at_the_current_schema() {
    let current_path = fresh_store_path("schema_migration_already_v2_source");
    create_memory_production_stack(Vec::new(), &[], &current_path).unwrap();

    let new_path = fresh_store_path("schema_migration_v2_target");
    let err = migrate(&current_path, &new_path).unwrap_err();
    match err {
        MigrateError::Durability(DurabilityError::RecordBlobUnreadable { cause, .. }) => {
            assert!(cause.contains("schema tag mismatch"), "{cause}");
        }
        other => panic!("expected a schema-tag-mismatch RecordBlobUnreadable, got {other:?}"),
    }
    assert!(!new_path.exists(), "nothing written for a refused source");

    let _ = std::fs::remove_dir_all(current_path.parent().unwrap());
}

/// `SCE-FR-001` (ADR-0116): `memory_server` pointed at a directory
/// written at the pre-`ADR-0056` layout refuses to start — the distinct
/// schema-tag refusal, never a mis-read and never a recreate — and its
/// startup error names the remedy: the migration tool, or a re-push.
#[cfg(feature = "server")]
#[test]
fn memory_server_refuses_an_old_layout_and_names_the_migration_tool() {
    use std::process::{Command, Stdio};

    let dir = unique_dir("schema_migration_server_refusal");
    std::fs::create_dir_all(&dir).unwrap();
    create_memory_v1_fixture(
        vec![memory_v1(1, "general", false)],
        &[],
        &dir.join("memories.mmap"),
    )
    .unwrap();

    let slot_file = dir.join("memories.mmap");
    let before = std::fs::read(&slot_file).unwrap();
    let mut command = Command::new(env!("CARGO_BIN_EXE_memory_server"));
    for (name, _) in std::env::vars_os() {
        if name.to_string_lossy().starts_with("SERVER_") {
            command.env_remove(name);
        }
    }
    let output = command
        .arg("127.0.0.1:0")
        .env("SERVER_DATA_DIR", &dir)
        .stdout(Stdio::null())
        .stderr(Stdio::piped())
        .output()
        .unwrap();
    let stderr = String::from_utf8_lossy(&output.stderr);
    assert!(
        !output.status.success(),
        "memory_server started on an old-layout directory; stderr: {stderr}"
    );
    assert!(
        stderr.contains("schema tag mismatch"),
        "the refusal is the distinct schema-tag one: {stderr}"
    );
    assert!(
        stderr.contains("migrate_memory_v1_to_v2") && stderr.contains("ADR-0066"),
        "the refusal names the migration tool: {stderr}"
    );
    // `RGF-FR-003` (ADR-0120): the hint's paths are the slot files, the
    // variable takes the directory, and the two sibling tables are named.
    assert!(
        stderr.contains("<old_dir>/memories.mmap <new_dir>/memories.mmap")
            && stderr.contains("SERVER_DATA_DIR=<new_dir>")
            && stderr.contains("entities.mmap")
            && stderr.contains("relations.mmap"),
        "the remedy names slot-file paths, the directory, and the sibling tables: {stderr}"
    );
    assert!(
        !stderr.contains("listening on"),
        "refused before listening: {stderr}"
    );
    assert_eq!(
        std::fs::read(&slot_file).unwrap(),
        before,
        "the old slot file is byte-for-byte untouched"
    );
}
