//! The re-scoped `DIFFERENTIAL-TEST-SUITE-PORT` unit (`ADR-0062` item 5).
//! The original plan — port `rusty_remind_me`'s own ~60-test
//! SQLite/Postgres differential suite — was abandoned: that suite lives
//! in a separate repo, and the 2026-09-07 hub spike's own
//! `MultimodalHubStore` adapter was never merged anywhere in this one.
//! Re-scoped, owner-approved: build the *equivalent* differential/
//! behavioral coverage inside this crate's own `tests/`, against a real
//! `memory_server`-shaped `SchemaDrivenClient` round trip — no storage
//! layer shortcuts, matching `tests/server_memory_integration.rs`'s own
//! precedent exactly (`required-features = ["server"]` only, no
//! `research`; every assertion over a real `TcpListener`).
//!
//! Eight portable consumer behaviors were named as the target list. Six
//! get a new test here; two are cited as already fully proven elsewhere
//! and deliberately not duplicated:
//!
//! 1. LWW push semantics (`ReplaceIf` guard on `updated_at_unix_ms`) —
//!    already fully proven by
//!    `a_guarded_replace_is_last_writer_wins_over_the_wire_and_survives_a_restart`
//!    (`tests/server_memory_integration.rs`). Not duplicated.
//! 2. Sensitive-flag round trip on insert —
//!    [`a_sensitive_memory_inserted_over_the_wire_reads_back_true`].
//! 3. Malformed-op isolation in a *pipelined* write batch —
//!    [`a_malformed_op_in_a_pipelined_batch_is_isolated_and_reports_failed`].
//! 4. `node_id` isolation via `Query` —
//!    [`node_id_isolates_records_by_origin_node_over_a_query`].
//! 5. The entity alias union-merge primitive (`replace` with a client-
//!    computed union, idempotent on reapply) —
//!    [`entity_aliases_union_merge_round_trips_via_replace_and_is_idempotent`].
//! 6. `entity_relations` (`Relation` domain) round trip — `FilterEq
//!    subject`/`Query object` retrieval and directed-edge semantics are
//!    already fully proven by
//!    `the_relation_table_serves_directed_open_label_edges_over_the_wire`
//!    (`tests/server_memory_integration.rs`); only the untested
//!    composition (an identical-edge reinsert is a refused duplicate,
//!    not a second edge) gets a new test here:
//!    [`reinserting_the_identical_relation_edge_is_a_refused_duplicate_not_a_second_edge`].
//! 7. Stats/count breakdowns — `count_by_category`-equivalent
//!    (`GROUP BY category, COUNT(*)`) is already fully proven on
//!    `Memory` by
//!    `get_list_filters_and_group_by_category_match_the_consumers_shapes`
//!    (`tests/server_memory_integration.rs`); `tests/server_sql_integration.rs`
//!    proves the same `WHERE`+`COUNT(*)`/`GROUP BY` grammar generically
//!    but only against `Dog`. The one genuinely missing composition — a
//!    `count_since`-equivalent (`WHERE updated_at_unix_ms > t`, no
//!    `GROUP BY`) against `Memory` specifically — is new here:
//!    [`count_since_equivalent_via_a_plain_where_and_count_star_on_memory`].
//! 8. Tombstone compaction — checked directly first: `Compact`
//!    (`ConnectionStore::compact`/`GenericProductionStore::compact`,
//!    `docs/design/SERVER-COMPACT-DESIGN.md`) reclaims *hard*-deleted
//!    (`Request::Delete`) slots and folds insert/edge logs; it has no
//!    relationship at all to `deleted_at_unix_ms`, which is a plain
//!    scannable field a soft delete only ever stamps via whole-record
//!    `Replace`. So the one real, honest composition to prove is: a
//!    soft-deleted record is excluded from a live-records `Query`
//!    (`WHERE deleted_at_unix_ms = 0`) while a live one is not, and
//!    `Compact` neither purges the tombstone nor drops the live record —
//!    [`a_soft_deleted_record_is_excluded_from_live_query_but_never_touched_by_compact`].
//!
//! Explicitly out of scope, per the re-scoped plan, and not attempted
//! here: `hub_seq` monotonic-write-sequence ordering (no backend
//! primitive exists) and anything Postgres-specific (planner estimates,
//! `TIMESTAMPTZ` migration — no SQL engine on this backend at all).

use rusty_multimodal_db::generic::entity::{
    create_entity_production_stack, open_entity_production_stack_portable,
};
use rusty_multimodal_db::generic::memory::{
    create_memory_production_stack, open_memory_production_stack_portable,
};
use rusty_multimodal_db::generic::production::GenericProductionStore;
use rusty_multimodal_db::generic::relation::open_or_create_relation_production_stack;
use rusty_multimodal_db::server::client::{BatchOp, ClientError, QueryResult, SchemaDrivenClient};
use rusty_multimodal_db::server::entity::EntityConnectionStore;
use rusty_multimodal_db::server::memory::MemoryConnectionStore;
use rusty_multimodal_db::server::protocol::{ErrorCode, ScanValue, WriteResult};
use rusty_multimodal_db::server::relation::RelationConnectionStore;
use rusty_multimodal_db::server::{serve_tables, ConnectionStore, ServeOptions};
use std::net::{SocketAddr, TcpListener};
use std::sync::Arc;
use std::thread;
use uuid::Uuid;

fn unique_dir(label: &str) -> std::path::PathBuf {
    use std::sync::atomic::{AtomicU64, Ordering};
    static COUNTER: AtomicU64 = AtomicU64::new(0);
    let n = COUNTER.fetch_add(1, Ordering::Relaxed);
    std::env::temp_dir().join(format!("{label}_{}_{n}", std::process::id()))
}

/// One full `Memory` field list in wire order (tags 0-12), every test
/// below's only source of a well-formed record — `stamp` fills both
/// `created_at_unix_ms`/`updated_at_unix_ms`, `node` the sync-fields
/// origin (`""` unattributed, matching `ADR-0056`'s sentinel).
fn full(
    content: &str,
    category: &str,
    sensitive: bool,
    stamp: i64,
    node: &str,
) -> Vec<(&'static str, ScanValue)> {
    vec![
        ("content", ScanValue::Str(content.into())),
        ("category", ScanValue::Str(category.into())),
        ("tags", ScanValue::StrList(vec!["diff-suite".into()])),
        ("source", ScanValue::Str("test".into())),
        ("metadata_json", ScanValue::Str("{}".into())),
        ("created_at_unix_ms", ScanValue::I64(stamp)),
        ("updated_at_unix_ms", ScanValue::I64(stamp)),
        ("memory_type", ScanValue::Str("unclassified".into())),
        ("status", ScanValue::Str("active".into())),
        ("sensitive", ScanValue::Bool(sensitive)),
        ("access_count", ScanValue::I64(0)),
        ("deleted_at_unix_ms", ScanValue::I64(0)),
        ("node_id", ScanValue::Str(node.into())),
    ]
}

/// A three-table (`memory`, `entity`, `relation`) server on an empty
/// store — every test below seeds its own records via `insert` so each
/// is self-contained and never depends on another test's fixture shape,
/// mirroring `tests/server_memory_integration.rs`'s own
/// `start_three_table_server_at` construction exactly, minus the sample
/// seed data.
fn start_server() -> SocketAddr {
    start_server_at(unique_dir("hub_differential"))
}

fn start_server_at(dir: std::path::PathBuf) -> SocketAddr {
    std::fs::create_dir_all(&dir).unwrap();
    let memories = dir.join("memories.mmap");
    let entities = dir.join("entities.mmap");
    let memory_stack = if memories.exists() {
        open_memory_production_stack_portable(&memories).unwrap()
    } else {
        create_memory_production_stack(Vec::new(), &[], &memories).unwrap()
    };
    let entity_stack = if entities.exists() {
        open_entity_production_stack_portable(&entities).unwrap()
    } else {
        create_entity_production_stack(Vec::new(), &[], &[], &entities).unwrap()
    };
    let relation_stack =
        open_or_create_relation_production_stack(&dir.join("relations.mmap")).unwrap();
    let memory: Arc<dyn ConnectionStore> = Arc::new(MemoryConnectionStore::new(
        GenericProductionStore::new(memory_stack),
    ));
    let entity: Arc<dyn ConnectionStore> = Arc::new(EntityConnectionStore::new(
        GenericProductionStore::new(entity_stack),
    ));
    let relation: Arc<dyn ConnectionStore> = Arc::new(RelationConnectionStore::new(
        GenericProductionStore::new(relation_stack),
    ));
    let listener = TcpListener::bind("127.0.0.1:0").unwrap();
    let addr = listener.local_addr().unwrap();
    thread::spawn(move || {
        serve_tables(
            listener,
            vec![
                ("memory".to_string(), memory),
                ("entity".to_string(), entity),
                ("relation".to_string(), relation),
            ],
            0,
            ServeOptions::default(),
        )
    });
    addr
}

fn rows(result: QueryResult) -> Vec<(Uuid, Vec<(String, ScanValue)>)> {
    match result {
        QueryResult::Rows(rows) => rows,
        other => panic!("expected Rows, got {other:?}"),
    }
}

fn groups(result: QueryResult) -> Vec<Vec<(String, ScanValue)>> {
    match result {
        QueryResult::Groups(groups) => groups,
        other => panic!("expected Groups, got {other:?}"),
    }
}

/// Behavior 2: the consumer's own historical regression class
/// (`rusty_remind_me`'s `hub_compat_test.rs`, bug #265) — a `Memory`
/// inserted with `sensitive: true` over the wire reads back `true`, not
/// silently dropped/coerced to `false`, and a `sensitive`-filtered
/// `Query` reports it in the right bucket.
#[test]
fn a_sensitive_memory_inserted_over_the_wire_reads_back_true() {
    let addr = start_server();
    let mut client = SchemaDrivenClient::connect(addr).unwrap();
    let id = Uuid::from_u128(500);
    client
        .insert(id, &full("a secret note", "general", true, 500_000, ""))
        .unwrap();

    let got = client.get(id).unwrap().unwrap();
    assert_eq!(got[9], ("sensitive".to_string(), ScanValue::Bool(true)));

    let sensitive_only = rows(
        client
            .query("SELECT content FROM memory WHERE sensitive = true")
            .unwrap(),
    );
    assert!(sensitive_only.iter().any(|(rid, _)| *rid == id));
    let visible_only = rows(
        client
            .query("SELECT content FROM memory WHERE sensitive = false")
            .unwrap(),
    );
    assert!(!visible_only.iter().any(|(rid, _)| *rid == id));
}

/// Behavior 3: `write_batch_pipelined_applies_each_and_atomic_is_all_or_nothing`
/// (`tests/server_memory_integration.rs`) exercises a pipelined batch's
/// `NotFound` (a soft failure, not malformed) and an atomic batch's
/// all-or-nothing abort — never a pipelined (non-atomic) batch carrying
/// a genuinely malformed op. This proves that missing case: a malformed
/// insert (a short field list) sandwiched between two valid ones,
/// pipelined, reports `WriteResult::Failed(Malformed)` for the bad op
/// alone, and both valid ops land uncorrupted and unblocked.
#[test]
fn a_malformed_op_in_a_pipelined_batch_is_isolated_and_reports_failed() {
    let addr = start_server();
    let mut client = SchemaDrivenClient::connect(addr).unwrap();
    let id = Uuid::from_u128;
    let a = full("first", "general", false, 1_000, "");
    let short: Vec<(&str, ScanValue)> = vec![("content", ScanValue::Str("incomplete".into()))];
    let b = full("second", "general", false, 2_000, "");

    let results = client
        .write_batch(
            &[
                BatchOp::Insert {
                    id: id(601),
                    fields: &a,
                },
                BatchOp::Insert {
                    id: id(602),
                    fields: &short,
                },
                BatchOp::Insert {
                    id: id(603),
                    fields: &b,
                },
            ],
            false,
        )
        .unwrap();
    assert_eq!(results[0], WriteResult::Inserted);
    assert_eq!(results[1], WriteResult::Failed(ErrorCode::Malformed));
    assert_eq!(results[2], WriteResult::Inserted);

    assert!(client.get(id(601)).unwrap().is_some());
    assert!(
        client.get(id(602)).unwrap().is_none(),
        "the malformed op wrote nothing"
    );
    assert!(
        client.get(id(603)).unwrap().is_some(),
        "not corrupted or blocked by the failure before it"
    );
}

/// Behavior 4: the consumer's own choice to never echo `node_id` when
/// unattributed is a client-side convention, not a backend gap — the
/// field is real and filterable. Two memories written from different
/// origin nodes; a `Query` on `node_id` isolates each node's own
/// records and never leaks the other's.
#[test]
fn node_id_isolates_records_by_origin_node_over_a_query() {
    let addr = start_server();
    let mut client = SchemaDrivenClient::connect(addr).unwrap();
    let id = Uuid::from_u128;
    client
        .insert(
            id(701),
            &full("from laptop", "general", false, 1_000, "laptop"),
        )
        .unwrap();
    client
        .insert(
            id(702),
            &full("from desktop", "general", false, 2_000, "desktop"),
        )
        .unwrap();
    client
        .insert(
            id(703),
            &full("also laptop", "general", false, 3_000, "laptop"),
        )
        .unwrap();

    let mut laptop_ids: Vec<Uuid> = rows(
        client
            .query("SELECT content FROM memory WHERE node_id = 'laptop'")
            .unwrap(),
    )
    .into_iter()
    .map(|(rid, _)| rid)
    .collect();
    laptop_ids.sort();
    assert_eq!(laptop_ids, vec![id(701), id(703)]);

    let desktop_ids: Vec<Uuid> = rows(
        client
            .query("SELECT content FROM memory WHERE node_id = 'desktop'")
            .unwrap(),
    )
    .into_iter()
    .map(|(rid, _)| rid)
    .collect();
    assert_eq!(desktop_ids, vec![id(702)]);
}

/// Behavior 5: entity alias union-merge is adapter logic (read, compute
/// a union client-side, write), not a single primitive — this proves
/// the real primitive it would be built from: `aliases` (a `StrList`)
/// is readable via `get` and, despite every per-field wire capability
/// on it being `false` (`ENT4-FR-002`), genuinely writable through a
/// whole-record `replace` (`EntityConnectionStore::replace_record`
/// shares `entity_from_fields` with `insert_record` — no field-level
/// `Unsupported` gate applies to it). The union round-trips correctly
/// and reapplying the identical union is idempotent — no growth.
#[test]
fn entity_aliases_union_merge_round_trips_via_replace_and_is_idempotent() {
    let addr = start_server();
    let mut client = SchemaDrivenClient::connect(addr).unwrap();
    client.use_table("entity").unwrap();
    let id = Uuid::from_u128(900);
    client
        .insert(
            id,
            &[
                ("label", ScanValue::Str("Ada Lovelace".into())),
                ("kind", ScanValue::Str("person".into())),
                ("mention_count", ScanValue::I64(0)),
                ("aliases", ScanValue::StrList(vec!["Ada".into()])),
            ],
        )
        .unwrap();

    let read_aliases = |client: &mut SchemaDrivenClient| match &client.get(id).unwrap().unwrap()[3]
    {
        (name, ScanValue::StrList(v)) => {
            assert_eq!(name, "aliases");
            v.clone()
        }
        other => panic!("{other:?}"),
    };
    let union_with = |current: &[String], candidates: &[&str]| -> Vec<String> {
        let mut union = current.to_vec();
        for candidate in candidates {
            if !union.iter().any(|a| a == candidate) {
                union.push((*candidate).to_string());
            }
        }
        union
    };
    let replace_with = |aliases: Vec<String>| -> Vec<(&'static str, ScanValue)> {
        vec![
            ("label", ScanValue::Str("Ada Lovelace".into())),
            ("kind", ScanValue::Str("person".into())),
            ("mention_count", ScanValue::I64(0)),
            ("aliases", ScanValue::StrList(aliases)),
        ]
    };

    // The union-merge: existing aliases ("Ada") union a newly-seen set
    // ("Countess of Lovelace", "Ada" again) — one genuinely new entry.
    let current = read_aliases(&mut client);
    let union = union_with(&current, &["Countess of Lovelace", "Ada"]);
    assert_eq!(
        union,
        vec!["Ada".to_string(), "Countess of Lovelace".to_string()]
    );
    assert!(client.replace(id, &replace_with(union.clone())).unwrap());
    assert_eq!(read_aliases(&mut client), union);

    // Reapplying the identical union is idempotent: no growth.
    let stored = read_aliases(&mut client);
    let reunion = union_with(&stored, &["Countess of Lovelace", "Ada"]);
    assert_eq!(reunion, stored, "no growth computing the same union again");
    assert!(client.replace(id, &replace_with(reunion.clone())).unwrap());
    assert_eq!(read_aliases(&mut client), union);
}

/// Behavior 6: `the_relation_table_serves_directed_open_label_edges_over_the_wire`
/// (`tests/server_memory_integration.rs`) already proves out-edges by
/// `FilterEq subject`, in-edges by `Query object`, paging, a
/// last-writer-wins `ReplaceIf`, a count, and a delete — not duplicated
/// here. The one composition it never exercises: reinserting the
/// identical `(subject, relation, object)` triple under its own id a
/// second time is refused as `ErrorCode::Duplicate` — nothing written,
/// no second edge — the store's real idempotency guarantee against a
/// repeated sync push of the same edge.
#[test]
fn reinserting_the_identical_relation_edge_is_a_refused_duplicate_not_a_second_edge() {
    let addr = start_server();
    let mut client = SchemaDrivenClient::connect(addr).unwrap();
    client.use_table("relation").unwrap();
    let id = Uuid::from_u128(950);
    let edge = |updated_at: i64| -> Vec<(&'static str, ScanValue)> {
        vec![
            ("subject", ScanValue::Str("alice".into())),
            ("relation", ScanValue::Str("knows".into())),
            ("object", ScanValue::Str("bob".into())),
            ("created_at_unix_ms", ScanValue::I64(1_000)),
            ("updated_at_unix_ms", ScanValue::I64(updated_at)),
            ("node_id", ScanValue::Str("laptop".into())),
            ("deleted_at_unix_ms", ScanValue::I64(0)),
        ]
    };
    client.insert(id, &edge(1_000)).unwrap();
    match client.insert(id, &edge(1_000)) {
        Err(ClientError::Server(ErrorCode::Duplicate, _)) => {}
        other => panic!("expected Duplicate, got {other:?}"),
    }

    let counted = groups(
        client
            .query("SELECT COUNT(*) FROM relation WHERE subject = 'alice'")
            .unwrap(),
    );
    assert_eq!(
        counted[0][0].1,
        ScanValue::I64(1),
        "the reinsert created no second edge"
    );
    assert_eq!(
        client.get(id).unwrap().unwrap()[4].1,
        ScanValue::I64(1_000),
        "the original record, unreplaced"
    );
}

/// Behavior 7: `count_by_category`-equivalent (`GROUP BY category,
/// COUNT(*)`) is already proven on `Memory` by
/// `get_list_filters_and_group_by_category_match_the_consumers_shapes`
/// (`tests/server_memory_integration.rs`); the same `WHERE`+`COUNT(*)`
/// grammar is proven generically in `tests/server_sql_integration.rs`,
/// but only against `Dog`. Missing specifically: a `Memory`-domain
/// `count_since`-equivalent — a plain `WHERE updated_at_unix_ms > t`
/// folded into one `COUNT(*)`, no `GROUP BY`.
#[test]
fn count_since_equivalent_via_a_plain_where_and_count_star_on_memory() {
    let addr = start_server();
    let mut client = SchemaDrivenClient::connect(addr).unwrap();
    let id = Uuid::from_u128;
    client
        .insert(id(801), &full("a", "general", false, 1_000, ""))
        .unwrap();
    client
        .insert(id(802), &full("b", "general", false, 2_000, ""))
        .unwrap();
    client
        .insert(id(803), &full("c", "general", false, 3_000, ""))
        .unwrap();
    client
        .insert(id(804), &full("d", "general", false, 4_000, ""))
        .unwrap();

    let counted = groups(
        client
            .query("SELECT COUNT(*) FROM memory WHERE updated_at_unix_ms > 2000")
            .unwrap(),
    );
    assert_eq!(counted.len(), 1);
    assert_eq!(counted[0][0], ("COUNT(*)".to_string(), ScanValue::I64(2)));
}

/// Behavior 8: checked directly first — `Compact`
/// (`ConnectionStore::compact`/`GenericProductionStore::compact`,
/// `docs/design/SERVER-COMPACT-DESIGN.md`) reclaims *hard*-deleted
/// (`Request::Delete`) slots and folds insert/edge logs; it has no
/// relationship at all to `deleted_at_unix_ms`, a plain scannable field
/// a soft delete only ever stamps via whole-record `Replace`. So the
/// real, honest composition to prove is: a soft-deleted record is
/// excluded from a live-records `Query` (`WHERE deleted_at_unix_ms =
/// 0`) while a live one is not, and `Compact` neither purges the
/// tombstone nor lets the live record disappear — `sync_fields_carry_
/// sentinels_and_serve_the_purge_set_over_the_wire` and `compact_each_
/// table_over_the_wire_and_a_restart_serves_the_same_data`
/// (`tests/server_memory_integration.rs`) each prove one half; this
/// proves their composition.
#[test]
fn a_soft_deleted_record_is_excluded_from_live_query_but_never_touched_by_compact() {
    let addr = start_server();
    let mut client = SchemaDrivenClient::connect(addr).unwrap();
    let id = Uuid::from_u128;
    client
        .insert(id(851), &full("alive", "general", false, 1_000, ""))
        .unwrap();
    client
        .insert(id(852), &full("tombstoned", "general", false, 2_000, ""))
        .unwrap();
    let mut tombstoned = full("tombstoned", "general", false, 2_000, "");
    tombstoned[11] = ("deleted_at_unix_ms", ScanValue::I64(9_000));
    assert!(client.replace(id(852), &tombstoned).unwrap());

    let live_ids: Vec<Uuid> = rows(
        client
            .query("SELECT content FROM memory WHERE deleted_at_unix_ms = 0")
            .unwrap(),
    )
    .into_iter()
    .map(|(rid, _)| rid)
    .collect();
    assert!(live_ids.contains(&id(851)));
    assert!(
        !live_ids.contains(&id(852)),
        "soft-deleted, excluded from the live view"
    );

    // `Compact` never touches either — it has no relationship to
    // `deleted_at_unix_ms` at all.
    let report = client.compact().unwrap();
    assert_eq!(
        report.records, 2,
        "compact does not purge soft-deleted records"
    );
    assert!(
        client.get(id(852)).unwrap().is_some(),
        "a soft-deleted record is never compacted away"
    );
    assert_eq!(
        client.get(id(852)).unwrap().unwrap()[11].1,
        ScanValue::I64(9_000),
        "the tombstone stamp survives compact untouched"
    );
    assert!(
        client.get(id(851)).unwrap().is_some(),
        "the live record is never touched"
    );
}
