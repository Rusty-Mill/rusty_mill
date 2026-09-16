//! `ADR-0070`: the CLI's actual support module, verified against real
//! wire-produced backups and independent production reopens.
#![cfg(test)]

#[path = "../examples/support/restore_backup_lib.rs"]
mod restore_backup;

use restore_backup::{restore, Domain, RestoreError};
use std::collections::BTreeMap;
use std::ffi::OsString;
use std::path::{Path, PathBuf};

fn unique_dir() -> PathBuf {
    use std::sync::atomic::{AtomicU64, Ordering};
    static NEXT: AtomicU64 = AtomicU64::new(0);
    let path = std::env::temp_dir().join(format!(
        "restore_backup_{}_{}",
        std::process::id(),
        NEXT.fetch_add(1, Ordering::Relaxed)
    ));
    std::fs::create_dir(&path).unwrap();
    path
}

fn contents(path: &Path) -> BTreeMap<OsString, Vec<u8>> {
    std::fs::read_dir(path)
        .unwrap()
        .map(|entry| {
            let entry = entry.unwrap();
            (entry.file_name(), std::fs::read(entry.path()).unwrap())
        })
        .collect()
}

#[test]
fn domain_parsing_is_exact_and_case_sensitive() {
    for (text, domain) in [
        ("memory", Domain::Memory),
        ("entity", Domain::Entity),
        ("relation", Domain::Relation),
    ] {
        assert_eq!(text.parse::<Domain>(), Ok(domain));
    }
    for bad in [
        "",
        "Memory",
        "ENTITY",
        "relations",
        "dog",
        " memory",
        "memory ",
    ] {
        assert!(bad.parse::<Domain>().is_err(), "{bad:?}");
    }
}

#[test]
fn conflicting_companion_is_refused_before_reading_a_missing_backup() {
    let root = unique_dir();
    let target = root.join("memories.mmap");
    let companion = root.join("memories.mmap.records");
    std::fs::write(&companion, b"existing companion").unwrap();
    let before = contents(&root);
    assert!(matches!(
        restore(&root.join("missing"), &target, Domain::Memory),
        Err(RestoreError::TargetExists { path }) if path == companion
    ));
    assert_eq!(contents(&root), before);
}

#[test]
fn missing_backup_leaves_target_and_staging_names_absent() {
    let root = unique_dir();
    assert!(matches!(
        restore(
            &root.join("missing"),
            &root.join("target/memories.mmap"),
            Domain::Memory
        ),
        Err(RestoreError::StagingIo { .. })
    ));
    assert_eq!(std::fs::read_dir(&root).unwrap().count(), 0);
}

#[test]
fn empty_backup_returns_the_production_verification_error() {
    let root = unique_dir();
    let backup = root.join("empty");
    std::fs::create_dir(&backup).unwrap();
    for (name, domain) in [
        ("memories", Domain::Memory),
        ("entities", Domain::Entity),
        ("relations", Domain::Relation),
    ] {
        let parent = root.join(name);
        assert!(matches!(
            restore(&backup, &parent.join(format!("{name}.mmap")), domain),
            Err(RestoreError::Verification(_))
        ));
        assert!(parent.is_dir());
        assert_eq!(std::fs::read_dir(&parent).unwrap().count(), 0);
    }
}

#[test]
fn target_without_a_filename_is_a_reported_error() {
    assert!(matches!(
        restore(Path::new("missing"), Path::new("/"), Domain::Memory),
        Err(RestoreError::StagingIo { source, .. }) if source.kind() == std::io::ErrorKind::InvalidInput
    ));
}

// The offline tool and tests above need no features. A real Backup
// request needs the existing server feature; no required-features entry
// is attached to this test target. The agreed all-features proof runs all
// of these tests, mirroring server_backup_integration's live fixture.
#[cfg(feature = "server")]
mod live {
    use super::*;
    use rusty_multimodal_db::generic::entity::{
        create_entity_production_stack, open_entity_production_stack_portable, Entity,
        RELATION_LABELS,
    };
    use rusty_multimodal_db::generic::memory::{
        create_memory_production_stack, open_memory_production_stack_portable, Memory,
        MEMORY_RELATION_LABELS,
    };
    use rusty_multimodal_db::generic::production::GenericProductionStore;
    use rusty_multimodal_db::generic::query::{AllIds, GetById, MultiNeighbors};
    use rusty_multimodal_db::generic::relation::{
        create_relation_production_stack, open_relation_production_stack_portable, Relation,
    };
    use rusty_multimodal_db::server::client::SchemaDrivenClient;
    use rusty_multimodal_db::server::entity::EntityConnectionStore;
    use rusty_multimodal_db::server::memory::MemoryConnectionStore;
    use rusty_multimodal_db::server::relation::RelationConnectionStore;
    use rusty_multimodal_db::server::{serve_tables, ConnectionStore, ServeOptions};
    use std::net::TcpListener;
    use std::sync::Arc;
    use uuid::Uuid;

    fn memories() -> Vec<Memory> {
        (1..=3)
            .map(|n| Memory {
                id: Uuid::from_u128(n),
                content: format!("memory {n}: café\nsecond line\0"),
                category: format!("category {n}"),
                tags: vec!["α".into(), format!("tag {n}")],
                source: format!("source {n}"),
                metadata_json: format!("{{\"n\": {n}}}"),
                created_at_unix_ms: n as i64 * 1000,
                updated_at_unix_ms: n as i64 * 2000,
                memory_type: format!("type {n}"),
                status: format!("status {n}"),
                sensitive: n == 2,
                access_count: n as i64 * 3,
                deleted_at_unix_ms: if n == 3 { 9000 } else { 0 },
                node_id: format!("node {n}"),
            })
            .collect()
    }

    fn entities() -> Vec<Entity> {
        (1..=3)
            .map(|n| Entity {
                id: Uuid::from_u128(100 + n),
                label: format!("entity {n}: café"),
                kind: format!("kind {n}"),
                mention_count: n as i64 * 7,
                aliases: vec![format!("alias {n}"), format!("別名 {n}")],
            })
            .collect()
    }

    fn relations() -> Vec<Relation> {
        (1..=3)
            .map(|n| Relation {
                id: Uuid::from_u128(200 + n),
                subject: format!("subject {n}"),
                relation: format!("knows {n}: α"),
                object: format!("object {n}"),
                created_at_unix_ms: n as i64 * 1000,
                updated_at_unix_ms: n as i64 * 2000,
                node_id: format!("node {n}"),
                deleted_at_unix_ms: if n == 3 { 9000 } else { 0 },
            })
            .collect()
    }

    fn mentions() -> Vec<(Uuid, Uuid)> {
        vec![
            (Uuid::from_u128(1), Uuid::from_u128(101)),
            (Uuid::from_u128(1), Uuid::from_u128(102)),
            (Uuid::from_u128(2), Uuid::from_u128(101)),
        ]
    }

    fn stem(domain: Domain) -> &'static str {
        match domain {
            Domain::Memory => "memories.mmap",
            Domain::Entity => "entities.mmap",
            Domain::Relation => "relations.mmap",
        }
    }

    fn backup(root: &Path, domain: Domain) -> (PathBuf, u64) {
        let source = root.join("source");
        std::fs::create_dir(&source).unwrap();
        let path = source.join(stem(domain));
        let store: Arc<dyn ConnectionStore> = match domain {
            Domain::Memory => Arc::new(
                MemoryConnectionStore::new(GenericProductionStore::new(
                    create_memory_production_stack(memories(), &mentions(), &path).unwrap(),
                ))
                .with_backup_source(path),
            ),
            Domain::Entity => Arc::new(
                EntityConnectionStore::new(GenericProductionStore::new(
                    create_entity_production_stack(
                        entities(),
                        &[(Uuid::from_u128(101), Uuid::from_u128(102))],
                        &[(Uuid::from_u128(102), Uuid::from_u128(103))],
                        &path,
                    )
                    .unwrap(),
                ))
                .with_backup_source(path),
            ),
            Domain::Relation => Arc::new(
                RelationConnectionStore::new(GenericProductionStore::new(
                    create_relation_production_stack(relations(), &path).unwrap(),
                ))
                .with_backup_source(path),
            ),
        };
        let backup_root = root.join("backups");
        let options = ServeOptions::default().with_backup_root(backup_root.clone());
        let listener = TcpListener::bind("127.0.0.1:0").unwrap();
        let address = listener.local_addr().unwrap();
        std::thread::spawn(move || {
            serve_tables(listener, vec![("fixture".into(), store)], 0, options)
        });
        let mut client = SchemaDrivenClient::connect(address).unwrap();
        let (files, bytes) = client.backup("nightly").unwrap();
        assert!(files >= 2);
        assert!(bytes > 0);
        (backup_root.join("nightly"), files)
    }

    fn assert_records_and_edges(target: &Path, domain: Domain) {
        match domain {
            Domain::Memory => {
                let stack = open_memory_production_stack_portable(target).unwrap();
                assert_eq!(stack.all_ids().len(), 3);
                for expected in memories() {
                    assert_eq!(stack.get(expected.id), Some(expected.clone()));
                    let mut neighbors = stack
                        .neighbors_by_relation(MEMORY_RELATION_LABELS[0], expected.id)
                        .unwrap_or_default();
                    let mut expected_neighbors: Vec<_> = mentions()
                        .into_iter()
                        .filter_map(|(a, b)| (a == expected.id).then_some(b))
                        .collect();
                    neighbors.sort();
                    expected_neighbors.sort();
                    assert_eq!(neighbors, expected_neighbors);
                }
            }
            Domain::Entity => {
                let stack = open_entity_production_stack_portable(target).unwrap();
                assert_eq!(stack.all_ids().len(), 3);
                for expected in entities() {
                    assert_eq!(stack.get(expected.id), Some(expected));
                }
                for (label, a, b) in [
                    (RELATION_LABELS[0], 101, 102),
                    (RELATION_LABELS[1], 102, 103),
                ] {
                    assert_eq!(
                        stack.neighbors_by_relation(label, Uuid::from_u128(a)),
                        Some(vec![Uuid::from_u128(b)])
                    );
                    assert_eq!(
                        stack.neighbors_by_relation(label, Uuid::from_u128(b)),
                        Some(vec![Uuid::from_u128(a)])
                    );
                }
            }
            Domain::Relation => {
                let stack = open_relation_production_stack_portable(target).unwrap();
                assert_eq!(stack.all_ids().len(), 3);
                for expected in relations() {
                    assert_eq!(stack.get(expected.id), Some(expected));
                }
            }
        }
    }

    fn round_trip(domain: Domain) {
        let root = unique_dir();
        let (backup, files) = backup(&root, domain);
        let backup_before = contents(&backup);
        let parent = if domain == Domain::Relation {
            root.join("new-parent/restored")
        } else {
            root.join("restored")
        };
        // Exercise coexistence with another table, as required by the
        // accepted per-file design, as well as a fresh missing parent.
        if domain == Domain::Entity {
            std::fs::create_dir(&parent).unwrap();
            std::fs::write(parent.join("memories.mmap"), b"sibling table").unwrap();
        }
        let target = parent.join(stem(domain));
        let report = restore(&backup, &target, domain).unwrap();
        assert_eq!(report.files, files);
        assert_eq!(report.records, 3);
        assert_records_and_edges(&target, domain);
        let before_retry = contents(&parent);
        assert!(matches!(
            restore(&backup, &target, domain),
            Err(RestoreError::TargetExists { .. })
        ));
        assert_eq!(contents(&parent), before_retry);
        assert_records_and_edges(&target, domain);
        assert_eq!(contents(&backup), backup_before);
        if domain == Domain::Entity {
            assert_eq!(
                std::fs::read(parent.join("memories.mmap")).unwrap(),
                b"sibling table"
            );
        }
        assert!(!std::fs::read_dir(parent.parent().unwrap())
            .unwrap()
            .any(|e| e
                .unwrap()
                .file_name()
                .to_string_lossy()
                .starts_with(".restore-tmp-")));
        // Retained like server_backup_integration's fixtures: the server
        // thread owns a live mmap until process exit. This path also lets
        // an operator run the example by hand against this real backup.
        println!(
            "real {domain:?} backup for CLI verification: {}",
            backup.display()
        );
    }

    #[test]
    fn memory_backup_restores_every_field_and_mentions_and_refuses_a_retry() {
        round_trip(Domain::Memory);
    }

    #[test]
    fn entity_backup_restores_every_field_and_edges_and_refuses_a_retry() {
        round_trip(Domain::Entity);
    }

    #[test]
    fn relation_backup_restores_every_field_and_refuses_a_retry() {
        round_trip(Domain::Relation);
    }

    #[test]
    fn staging_copy_failure_leaves_no_target_files_and_removes_the_temporary_directory() {
        let root = unique_dir();
        let (backup, _) = backup(&root, Domain::Memory);
        // A directory is reliably uncopyable by fs::copy on both Windows
        // and Linux, even when tests run with elevated filesystem access.
        std::fs::create_dir(backup.join("memories.mmap.unreadable")).unwrap();
        let parent = root.join("restored");
        std::fs::create_dir(&parent).unwrap();
        std::fs::write(parent.join("entities.mmap"), b"sibling table").unwrap();
        let before = contents(&parent);
        assert!(matches!(
            restore(&backup, &parent.join("memories.mmap"), Domain::Memory),
            Err(RestoreError::StagingIo { .. })
        ));
        assert_eq!(contents(&parent), before);
        assert!(!std::fs::read_dir(&root).unwrap().any(|e| e
            .unwrap()
            .file_name()
            .to_string_lossy()
            .starts_with(".restore-tmp-")));
    }

    #[test]
    fn wrong_domain_returns_a_real_durability_error_and_keeps_every_copied_file() {
        let root = unique_dir();
        let (backup, _) = backup(&root, Domain::Entity);
        let before = contents(&backup);
        let target = root.join("restored/entities.mmap");
        let error = restore(&backup, &target, Domain::Memory).unwrap_err();
        match error {
            RestoreError::Verification(
                rusty_multimodal_db::durability::DurabilityError::RecordBlobUnreadable {
                    cause,
                    ..
                },
            ) => {
                assert!(cause.contains("schema tag mismatch"), "{cause}");
            }
            other => panic!("expected the portable opener's schema mismatch, got {other:?}"),
        }
        assert_eq!(contents(target.parent().unwrap()), before);
        assert_eq!(contents(&backup), before);
    }
}
