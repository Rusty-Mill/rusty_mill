//! Real end-to-end coverage of `Request::Query`/`SchemaDrivenClient::query`
//! (`SQL-FR-001`–`010`, ADR-0034,
//! `docs/design/SERVER-SQL-SELECT-DESIGN.md`) and of
//! `Request::Aggregate`/the same `SchemaDrivenClient::query`
//! (`AGG-FR-001`–`010`, ADR-0035,
//! `docs/design/SERVER-SQL-AGGREGATE-DESIGN.md`) — a real `TcpListener`,
//! real client `TcpStream`s, a real SQL string parsed client-side and
//! answered by the server as a full-scan-then-filter(-then-bucket-then-
//! reduce, for `GROUP BY`/an aggregate function). All three domains in
//! one file, `required-features = ["server", "research"]`, matching
//! `tests/server_schema_driven_client.rs`'s own precedent for a target
//! that needs every domain adapter.

use rusty_multimodal_db::generic::entity::{create_entity_production_stack, Entity};
use rusty_multimodal_db::generic::memory::{create_memory_production_stack, Memory};
use rusty_multimodal_db::generic::order_customer::{
    create_order_production_stack, Order, OrderStatus,
};
use rusty_multimodal_db::generic::production::GenericProductionStore;
use rusty_multimodal_db::generic::relation::{create_relation_production_stack, Relation};
use rusty_multimodal_db::generic::reminder::{
    create_reminder_production_stack, Reminder, ReminderStatus,
};
use rusty_multimodal_db::generic_spike::employee_impl::{
    create_employee_production_stack, Department, Employee,
};
use rusty_multimodal_db::record::DogRecord;
use rusty_multimodal_db::server::client::{
    ClientError, QueryResult, SchemaDrivenClient, SessionOptions,
};
use rusty_multimodal_db::server::dog::DogConnectionStore;
use rusty_multimodal_db::server::employee::EmployeeConnectionStore;
use rusty_multimodal_db::server::entity::EntityConnectionStore;
use rusty_multimodal_db::server::memory::MemoryConnectionStore;
use rusty_multimodal_db::server::order::OrderConnectionStore;
use rusty_multimodal_db::server::protocol::ScanValue;
use rusty_multimodal_db::server::relation::RelationConnectionStore;
use rusty_multimodal_db::server::reminder::ReminderConnectionStore;
use rusty_multimodal_db::server::{serve, ServeOptions};
use rusty_multimodal_db::ProductionStore;
use std::net::{SocketAddr, TcpListener};
use std::sync::Arc;
use std::thread;
use uuid::Uuid;

/// See `tests/server_dog_integration.rs`'s identical helper for why this
/// needs both the process id and a monotonic counter, not just one.
fn unique_dir(label: &str) -> std::path::PathBuf {
    use std::sync::atomic::{AtomicU64, Ordering};
    static COUNTER: AtomicU64 = AtomicU64::new(0);
    let n = COUNTER.fetch_add(1, Ordering::Relaxed);
    std::env::temp_dir().join(format!("{label}_{}_{n}", std::process::id()))
}

fn sample_dogs() -> Vec<DogRecord> {
    vec![
        DogRecord::new(Uuid::from_u128(1), "labrador", 3),
        DogRecord::new(Uuid::from_u128(2), "poodle", 5),
        DogRecord::new(Uuid::from_u128(3), "labrador", 9),
    ]
}

fn start_dog_server() -> SocketAddr {
    let dir = unique_dir("sql_integration_dog");
    std::fs::create_dir_all(&dir).unwrap();
    let path = dir.join("dogs.mmap");
    let store = ProductionStore::create(sample_dogs(), Vec::new(), &path).unwrap();
    let connection_store = Arc::new(DogConnectionStore::new(store));
    let listener = TcpListener::bind("127.0.0.1:0").unwrap();
    let addr = listener.local_addr().unwrap();
    thread::spawn(move || serve(listener, connection_store, ServeOptions::default()));
    addr
}

/// Like [`start_dog_server`], but with `n` generated dogs
/// (`bench_support::build_dataset`) instead of the fixed three-record
/// sample — large enough to force `SchemaDrivenClient::query`'s
/// `ORDER BY`-with-no-`LIMIT` path (`OBY-FR-004`, ADR-0061) across
/// several internal `Request::Page` calls.
fn start_dog_server_n(n: usize) -> SocketAddr {
    use rusty_multimodal_db::bench_support::build_dataset;

    let dir = unique_dir("sql_integration_dog_n");
    std::fs::create_dir_all(&dir).unwrap();
    let path = dir.join("dogs.mmap");
    let records = build_dataset(n).records;
    let store = ProductionStore::create(records, Vec::new(), &path).unwrap();
    let connection_store = Arc::new(DogConnectionStore::new(store));
    let listener = TcpListener::bind("127.0.0.1:0").unwrap();
    let addr = listener.local_addr().unwrap();
    thread::spawn(move || serve(listener, connection_store, ServeOptions::default()));
    addr
}

fn start_order_server() -> SocketAddr {
    let dir = unique_dir("sql_integration_order");
    std::fs::create_dir_all(&dir).unwrap();
    let path = dir.join("amount.mmap");
    let orders = vec![
        Order {
            id: Uuid::from_u128(1),
            customer_id: Uuid::from_u128(100),
            amount_cents: 2_500,
            status: OrderStatus::Shipped,
            created_at_unix_ms: 1_000,
            discount_cents: 0,
        },
        Order {
            id: Uuid::from_u128(2),
            customer_id: Uuid::from_u128(100),
            amount_cents: 4_200,
            status: OrderStatus::Pending,
            created_at_unix_ms: 2_000,
            discount_cents: 0,
        },
    ];
    let stack = create_order_production_stack(orders, &path).unwrap();
    let connection_store = Arc::new(OrderConnectionStore::new(GenericProductionStore::new(
        stack,
    )));
    let listener = TcpListener::bind("127.0.0.1:0").unwrap();
    let addr = listener.local_addr().unwrap();
    thread::spawn(move || serve(listener, connection_store, ServeOptions::default()));
    addr
}

fn start_employee_server() -> SocketAddr {
    let dir = unique_dir("sql_integration_employee");
    std::fs::create_dir_all(&dir).unwrap();
    let path = dir.join("salary.mmap");
    let employees = vec![
        Employee {
            id: Uuid::from_u128(1),
            name: "Alex".into(),
            department: Department::Engineering,
            salary_cents: 1_200_000,
            manager_id: None,
        },
        Employee {
            id: Uuid::from_u128(2),
            name: "Bel".into(),
            department: Department::Sales,
            salary_cents: 950_000,
            manager_id: Some(Uuid::from_u128(1)),
        },
    ];
    let edges = vec![];
    let stack = create_employee_production_stack(employees, &edges, &path).unwrap();
    let connection_store = Arc::new(EmployeeConnectionStore::new(GenericProductionStore::new(
        stack,
    )));
    let listener = TcpListener::bind("127.0.0.1:0").unwrap();
    let addr = listener.local_addr().unwrap();
    thread::spawn(move || serve(listener, connection_store, ServeOptions::default()));
    addr
}

fn age_of(rows: &[(Uuid, Vec<(String, ScanValue)>)], id: Uuid) -> u32 {
    let (_, fields) = rows.iter().find(|(row_id, _)| *row_id == id).unwrap();
    match &fields.iter().find(|(name, _)| name == "age").unwrap().1 {
        ScanValue::U32(age) => *age,
        other => panic!("expected U32, got {other:?}"),
    }
}

/// Every test in this file below that runs a plain `SELECT` (no `GROUP
/// BY`/aggregate function) expects `QueryResult::Rows` — this unwraps it,
/// panicking on `Groups` (a bug in the test itself, not a case these
/// tests exercise). Aggregate-specific tests match on `QueryResult`
/// directly instead.
fn rows(result: QueryResult) -> Vec<(Uuid, Vec<(String, ScanValue)>)> {
    match result {
        QueryResult::Rows(rows) => rows,
        QueryResult::Groups(groups) => {
            panic!("expected QueryResult::Rows, got Groups: {groups:?}")
        }
        QueryResult::Joined(joined) => panic!("expected Rows or Groups, got Joined: {joined:?}"),
    }
}

/// Acceptance criterion 3 (`*`, no `WHERE`): every row, every field.
#[test]
fn select_star_with_no_where_returns_every_row_and_field() {
    let addr = start_dog_server();
    let mut client = SchemaDrivenClient::connect(addr).unwrap();
    let mut result = rows(client.query("SELECT * FROM dog").unwrap());
    result.sort_by_key(|(id, _)| *id);
    assert_eq!(result.len(), 3);
    assert_eq!(result[0].1.len(), 2, "both breed and age come back");
    assert_eq!(age_of(&result, Uuid::from_u128(1)), 3);
}

/// Acceptance criterion 3: named columns project exactly that subset.
#[test]
fn named_columns_project_exactly_that_subset() {
    let addr = start_dog_server();
    let mut client = SchemaDrivenClient::connect(addr).unwrap();
    let result = rows(client.query("SELECT age FROM dog WHERE age = 3").unwrap());
    assert_eq!(result.len(), 1);
    assert_eq!(result[0].1, vec![("age".to_string(), ScanValue::U32(3))]);
}

/// Acceptance criterion 3: every comparator kind, against a real `U32`
/// field.
#[test]
fn every_comparator_kind_filters_correctly() {
    let addr = start_dog_server();
    let mut client = SchemaDrivenClient::connect(addr).unwrap();

    let eq = rows(client.query("SELECT * FROM dog WHERE age = 5").unwrap());
    assert_eq!(eq.len(), 1);
    assert_eq!(age_of(&eq, Uuid::from_u128(2)), 5);

    let ne = rows(client.query("SELECT * FROM dog WHERE age != 5").unwrap());
    assert_eq!(ne.len(), 2);

    let lt = rows(client.query("SELECT * FROM dog WHERE age < 5").unwrap());
    assert_eq!(lt.len(), 1);
    assert_eq!(age_of(&lt, Uuid::from_u128(1)), 3);

    let le = rows(client.query("SELECT * FROM dog WHERE age <= 5").unwrap());
    assert_eq!(le.len(), 2);

    let gt = rows(client.query("SELECT * FROM dog WHERE age > 5").unwrap());
    assert_eq!(gt.len(), 1);
    assert_eq!(age_of(&gt, Uuid::from_u128(3)), 9);

    let ge = rows(client.query("SELECT * FROM dog WHERE age >= 5").unwrap());
    assert_eq!(ge.len(), 2);
}

/// Acceptance criterion 3: two `AND`-ed conditions, both across
/// different fields — only rows matching both come back.
#[test]
fn two_and_ed_conditions_across_two_fields() {
    let addr = start_dog_server();
    let mut client = SchemaDrivenClient::connect(addr).unwrap();
    let result = rows(
        client
            .query("SELECT * FROM dog WHERE breed = 'labrador' AND age > 5")
            .unwrap(),
    );
    assert_eq!(result.len(), 1);
    assert_eq!(age_of(&result, Uuid::from_u128(3)), 9);
}

/// Acceptance criterion 7: `LIMIT` truncates the row count and nothing
/// else; omitting it returns every matching row.
#[test]
fn limit_truncates_and_omitting_it_returns_every_match() {
    let addr = start_dog_server();
    let mut client = SchemaDrivenClient::connect(addr).unwrap();
    let limited = rows(client.query("SELECT * FROM dog LIMIT 1").unwrap());
    assert_eq!(limited.len(), 1);
    let unbounded = rows(client.query("SELECT * FROM dog").unwrap());
    assert_eq!(unbounded.len(), 3);
}

/// Acceptance criterion 4: an unknown column name is a client-side
/// `ClientError::UnknownField`.
#[test]
fn unknown_column_is_a_client_side_error() {
    let addr = start_dog_server();
    let mut client = SchemaDrivenClient::connect(addr).unwrap();
    assert!(matches!(
        client.query("SELECT weight FROM dog"),
        Err(ClientError::UnknownField(name)) if name == "weight"
    ));
    assert!(matches!(
        client.query("SELECT * FROM dog WHERE weight = 5"),
        Err(ClientError::UnknownField(name)) if name == "weight"
    ));
}

/// Acceptance criterion 4: a `WHERE` literal that doesn't match its
/// field's real type is a client-side error; so is an ordering
/// comparator against a non-orderable field.
#[test]
fn kind_mismatch_and_bad_ordering_comparator_are_client_side_errors() {
    let addr = start_dog_server();
    let mut client = SchemaDrivenClient::connect(addr).unwrap();
    assert!(matches!(
        client.query("SELECT * FROM dog WHERE age = 'old'"),
        Err(ClientError::Sql(_))
    ));
    assert!(matches!(
        client.query("SELECT * FROM dog WHERE breed > 'labrador'"),
        Err(ClientError::Sql(_))
    ));
}

/// A syntax error never reaches the wire either — the same client-side
/// posture as an unknown field or a kind mismatch.
#[test]
fn a_syntax_error_is_a_client_side_error() {
    let addr = start_dog_server();
    let mut client = SchemaDrivenClient::connect(addr).unwrap();
    assert!(matches!(
        client.query("SELECT * dog"),
        Err(ClientError::Sql(_))
    ));
}

/// Acceptance criterion 5: `Dog::breed` has every capability flag
/// (`filter_eq`/`scan`/`update`) `false`, yet is fully selectable and
/// filterable via `Query` — a full scan needs no index.
#[test]
fn a_field_with_every_capability_flag_false_is_still_queryable() {
    let addr = start_dog_server();
    let mut client = SchemaDrivenClient::connect(addr).unwrap();
    assert!(!client
        .schema()
        .fields
        .iter()
        .any(|f| f.name == "breed" && (f.capabilities.filter_eq || f.capabilities.scan)));
    let result = rows(
        client
            .query("SELECT breed FROM dog WHERE breed = 'poodle'")
            .unwrap(),
    );
    assert_eq!(result.len(), 1);
    assert_eq!(
        result[0].1,
        vec![("breed".to_string(), ScanValue::Str("poodle".into()))]
    );
}

/// A hand-rolled server that always negotiates a fixed protocol
/// version, however high a version the client's own `Hello` asks for —
/// the moral equivalent of a real older build, without needing to
/// check one out. Every other request is answered by the real
/// `dispatch` over a real `DogConnectionStore`, so a client that never
/// calls a version-gated method is served exactly as normal.
fn start_versioned_dog_server(version: u32) -> SocketAddr {
    use rusty_multimodal_db::server::dispatch;
    use rusty_multimodal_db::server::framing::{read_message, write_message};
    use rusty_multimodal_db::server::protocol::{Request, Response};

    let dir = unique_dir("sql_integration_versioned_dog");
    std::fs::create_dir_all(&dir).unwrap();
    let path = dir.join("dogs.mmap");
    let store = ProductionStore::create(sample_dogs(), Vec::new(), &path).unwrap();
    let connection_store = Arc::new(DogConnectionStore::new(store));
    let listener = TcpListener::bind("127.0.0.1:0").unwrap();
    let addr = listener.local_addr().unwrap();
    thread::spawn(move || {
        for stream in listener.incoming() {
            let Ok(mut stream) = stream else { break };
            let connection_store = Arc::clone(&connection_store);
            thread::spawn(move || loop {
                let req: Request = match read_message(&mut stream) {
                    Ok(req) => req,
                    Err(_) => return,
                };
                let resp = match req {
                    Request::Hello { .. } => Response::Hello {
                        protocol_version: version,
                    },
                    other => dispatch(connection_store.as_ref(), other),
                };
                if write_message(&mut stream, &resp).is_err() {
                    return;
                }
            });
        }
    });
    addr
}

fn start_version_7_dog_server() -> SocketAddr {
    start_versioned_dog_server(7)
}

/// `SQL-FR-010`: `query` requires protocol version 8 — against a
/// connection negotiated at 7, `query` is `ClientError::Unsupported`
/// with no frame sent, and the connection keeps working normally for
/// everything else (`get`).
#[test]
fn query_requires_protocol_version_8() {
    let addr = start_version_7_dog_server();
    let mut client = SchemaDrivenClient::connect(addr).unwrap();
    assert_eq!(client.server_protocol_version(), 7);
    assert!(matches!(
        client.query("SELECT * FROM dog"),
        Err(ClientError::Unsupported("sql query"))
    ));
    // The connection is still usable — the refusal above sent no frame.
    assert!(client.get(Uuid::from_u128(1)).unwrap().is_some());
}

// `OBY-FR-001`–`006`, ADR-0061, `docs/design/SERVER-SQL-ORDER-BY-DESIGN.md`.

/// Acceptance criterion 2: a `LIMIT`-bearing `ORDER BY` returns exactly
/// that many rows, strictly ascending by the ordered field, matching
/// `SchemaDrivenClient::page`'s own result for the identical
/// field/limit.
#[test]
fn order_by_with_limit_returns_rows_ascending_matching_page() {
    let addr = start_dog_server();
    let mut client = SchemaDrivenClient::connect(addr).unwrap();

    let ordered = rows(
        client
            .query("SELECT * FROM dog ORDER BY age LIMIT 2")
            .unwrap(),
    );
    assert_eq!(ordered.len(), 2);
    assert_eq!(age_of(&ordered, ordered[0].0), 3);
    assert_eq!(age_of(&ordered, ordered[1].0), 5);

    let paged = client.page("age", None, 2).unwrap();
    assert_eq!(
        ordered.iter().map(|(id, _)| *id).collect::<Vec<_>>(),
        paged.iter().map(|(id, _)| *id).collect::<Vec<_>>(),
        "ORDER BY compiles to the identical Request::Page walk page() uses directly"
    );
}

/// Acceptance criterion 2: a named `SELECT` column list projects down
/// correctly even though `Request::Page` itself always returns every
/// field of each record.
#[test]
fn order_by_projects_named_columns_even_though_page_returns_every_field() {
    let addr = start_dog_server();
    let mut client = SchemaDrivenClient::connect(addr).unwrap();
    let result = rows(
        client
            .query("SELECT age FROM dog ORDER BY age LIMIT 1")
            .unwrap(),
    );
    assert_eq!(result.len(), 1);
    assert_eq!(result[0].1, vec![("age".to_string(), ScanValue::U32(3))]);
}

/// Acceptance criterion 2: an `ORDER BY` with no `LIMIT` walks the whole
/// table across several internal `Request::Page` calls (2,500 records,
/// well over the client's internal chunk size), returning every row
/// exactly once, strictly ascending, with no duplicate or missing row
/// across a chunk seam.
#[test]
fn order_by_with_no_limit_walks_every_record_across_several_pages() {
    let addr = start_dog_server_n(2_500);
    let mut client = SchemaDrivenClient::connect(addr).unwrap();
    let result = rows(client.query("SELECT * FROM dog ORDER BY age").unwrap());
    assert_eq!(result.len(), 2_500);

    let ages: Vec<u32> = result.iter().map(|(id, _)| age_of(&result, *id)).collect();
    assert!(
        ages.windows(2).all(|pair| pair[0] <= pair[1]),
        "strictly ascending by age (ties broken by id, unchecked here)"
    );

    let mut ids: Vec<Uuid> = result.iter().map(|(id, _)| *id).collect();
    let before = ids.len();
    ids.sort();
    ids.dedup();
    assert_eq!(ids.len(), before, "no duplicate row across a chunk seam");
}

/// Acceptance criterion 3: every client-side refusal, no frame sent —
/// a non-`U32`/`I64` order field and a zero `LIMIT`
/// (`ClientError::Unsupported`), and an unknown order field
/// (`ClientError::UnknownField`). `WHERE` combined with `ORDER BY` is no
/// longer refused (`FPG-FR-005`, ADR-0068) — see the `FilteredPage`
/// tests below; `JOIN`/`GROUP BY` combined with `ORDER BY` are still
/// syntax errors, unit-tested directly in `sql.rs`
/// (`order_by_with_join_is_a_syntax_error`/
/// `order_by_with_group_by_or_aggregate_is_a_syntax_error`).
#[test]
fn order_by_client_side_refusals() {
    let addr = start_dog_server();
    let mut client = SchemaDrivenClient::connect(addr).unwrap();

    assert!(matches!(
        client.query("SELECT * FROM dog ORDER BY breed"),
        Err(ClientError::Unsupported("order by"))
    ));
    assert!(matches!(
        client.query("SELECT * FROM dog ORDER BY age LIMIT 0"),
        Err(ClientError::Unsupported("order by limit"))
    ));
    assert!(matches!(
        client.query("SELECT * FROM dog ORDER BY weight"),
        Err(ClientError::UnknownField(name)) if name == "weight"
    ));
    // The connection is still usable after every refusal above — none
    // of them sent a frame.
    assert!(client.get(Uuid::from_u128(1)).unwrap().is_some());
}

/// Acceptance criterion 3: `ORDER BY` requires protocol version 20 —
/// against a connection negotiated below it, `ORDER BY` is
/// `ClientError::Unsupported("order by")` with no frame sent, and the
/// connection keeps working normally for everything else.
#[test]
fn order_by_requires_protocol_version_20() {
    let addr = start_versioned_dog_server(19);
    let mut client = SchemaDrivenClient::connect(addr).unwrap();
    assert_eq!(client.server_protocol_version(), 19);
    assert!(matches!(
        client.query("SELECT * FROM dog ORDER BY age"),
        Err(ClientError::Unsupported("order by"))
    ));
    assert!(client.get(Uuid::from_u128(1)).unwrap().is_some());
}

// `FPG-FR-001`–`007`, ADR-0068, `docs/design/SERVER-FILTERED-PAGE-DESIGN.md`.

/// Acceptance criteria 1/2: `WHERE` combined with `ORDER BY` and
/// `LIMIT` returns exactly the matching rows, strictly ascending by the
/// ordered field — row-for-row identical (ignoring order) to a
/// client-side filter-then-sort over the equivalent unordered `Query`
/// result, and identical to `page()`'s own walk once its own result is
/// filtered down to the same predicate by hand.
#[test]
fn filtered_order_by_with_limit_matches_page_and_query() {
    let addr = start_dog_server();
    let mut client = SchemaDrivenClient::connect(addr).unwrap();

    let filtered = rows(
        client
            .query("SELECT * FROM dog WHERE breed = 'labrador' ORDER BY age LIMIT 10")
            .unwrap(),
    );
    // This file's dogs: id1 age=3 labrador, id2 age=5 poodle, id3 age=9
    // labrador — exactly two labradors, ascending by age.
    assert_eq!(filtered.len(), 2);
    assert_eq!(filtered[0].0, Uuid::from_u128(1));
    assert_eq!(filtered[1].0, Uuid::from_u128(3));

    // Acceptance criterion 2: the same *set* of ids as an unordered
    // `Query` with the identical filter.
    let mut via_query: Vec<Uuid> = rows(
        client
            .query("SELECT * FROM dog WHERE breed = 'labrador'")
            .unwrap(),
    )
    .into_iter()
    .map(|(id, _)| id)
    .collect();
    via_query.sort();
    let mut via_filtered_page: Vec<Uuid> = filtered.iter().map(|(id, _)| *id).collect();
    via_filtered_page.sort();
    assert_eq!(via_query, via_filtered_page);

    // The unfiltered `page()` walk, filtered down by hand — the same
    // rows in the same order (`FilteredPage`'s own contract: a strict
    // subset-and-reorder of the unfiltered walk).
    let paged: Vec<Uuid> = client
        .page("age", None, 10)
        .unwrap()
        .into_iter()
        .filter(|(_, fields)| {
            fields
                .iter()
                .any(|(name, v)| name == "breed" && *v == ScanValue::Str("labrador".into()))
        })
        .map(|(id, _)| id)
        .collect();
    assert_eq!(
        filtered.iter().map(|(id, _)| *id).collect::<Vec<_>>(),
        paged,
        "FilteredPage matches Page's own walk filtered down by hand, in the same order"
    );
}

/// Acceptance criterion 1: a named `SELECT` column list still projects
/// down correctly through the filtered path.
#[test]
fn filtered_order_by_projects_named_columns() {
    let addr = start_dog_server();
    let mut client = SchemaDrivenClient::connect(addr).unwrap();
    let result = rows(
        client
            .query("SELECT age FROM dog WHERE breed = 'labrador' ORDER BY age LIMIT 1")
            .unwrap(),
    );
    assert_eq!(result.len(), 1);
    assert_eq!(result[0].1, vec![("age".to_string(), ScanValue::U32(3))]);
}

/// Acceptance criterion 1: a filtered `ORDER BY` with no `LIMIT` walks
/// the whole matching subset across several internal
/// `Request::FilteredPage` calls (2,500 records, `age >= 10` matching
/// well over the client's internal chunk size), returning every
/// matching row exactly once, strictly ascending, with no duplicate or
/// missing row across a chunk seam — and the identical id set an
/// unordered `Query` with the same filter returns.
#[test]
fn filtered_order_by_with_no_limit_walks_every_matching_record_across_several_pages() {
    let addr = start_dog_server_n(2_500);
    let mut client = SchemaDrivenClient::connect(addr).unwrap();
    let result = rows(
        client
            .query("SELECT * FROM dog WHERE age >= 10 ORDER BY age")
            .unwrap(),
    );
    // Ages are uniformly drawn from 0..=20 (`MIN_AGE`/`MAX_AGE`); >= 10
    // is about half of 2,500 — comfortably over `ORDER_BY_PAGE_CHUNK`
    // (1,000) and comfortably under the full table, so this exercises a
    // real multi-page filtered walk without depending on an exact count.
    assert!(result.len() > 1_000 && result.len() < 2_500);

    let ages: Vec<u32> = result.iter().map(|(id, _)| age_of(&result, *id)).collect();
    assert!(
        ages.iter().all(|age| *age >= 10),
        "every returned row matches the WHERE clause"
    );
    assert!(
        ages.windows(2).all(|pair| pair[0] <= pair[1]),
        "strictly ascending by age (ties broken by id, unchecked here)"
    );

    let mut ids: Vec<Uuid> = result.iter().map(|(id, _)| *id).collect();
    let before = ids.len();
    ids.sort();
    ids.dedup();
    assert_eq!(ids.len(), before, "no duplicate row across a chunk seam");

    let mut via_query: Vec<Uuid> = rows(client.query("SELECT * FROM dog WHERE age >= 10").unwrap())
        .into_iter()
        .map(|(id, _)| id)
        .collect();
    via_query.sort();
    assert_eq!(
        ids, via_query,
        "identical id set to an unordered Query with the same filter"
    );
}

/// Acceptance criterion 3: every client-side refusal a filtered `ORDER
/// BY` can produce, no frame sent — an unknown `WHERE` field, a kind
/// mismatch, and an ordering comparator against a non-orderable field
/// (`resolve_filter`'s existing checks, reused unchanged for this
/// request) — plus `order by`'s own existing refusals still apply.
#[test]
fn filtered_order_by_client_side_refusals() {
    let addr = start_dog_server();
    let mut client = SchemaDrivenClient::connect(addr).unwrap();

    assert!(matches!(
        client.query("SELECT * FROM dog WHERE weight = 5 ORDER BY age"),
        Err(ClientError::UnknownField(name)) if name == "weight"
    ));
    assert!(matches!(
        client.query("SELECT * FROM dog WHERE age = 'old' ORDER BY age"),
        Err(ClientError::Sql(_))
    ));
    assert!(matches!(
        client.query("SELECT * FROM dog WHERE breed > 'labrador' ORDER BY age"),
        Err(ClientError::Sql(_))
    ));
    assert!(matches!(
        client.query("SELECT * FROM dog WHERE breed = 'labrador' ORDER BY breed"),
        Err(ClientError::Unsupported("order by"))
    ));
    assert!(matches!(
        client.query("SELECT * FROM dog WHERE breed = 'labrador' ORDER BY age LIMIT 0"),
        Err(ClientError::Unsupported("order by limit"))
    ));
    // The connection is still usable after every refusal above — none
    // of them sent a frame.
    assert!(client.get(Uuid::from_u128(1)).unwrap().is_some());
}

/// Acceptance criterion 4: a filtered `ORDER BY` requires protocol
/// version 26 — against a connection negotiated at 25 (where plain,
/// unfiltered `ORDER BY` already works, since it only needs 20), a
/// filtered one is `ClientError::Unsupported("order by with filter")`
/// with no frame sent, and the connection keeps working normally for
/// everything else, including the still-legal unfiltered `ORDER BY`.
#[test]
fn filtered_order_by_requires_protocol_version_26() {
    let addr = start_versioned_dog_server(25);
    let mut client = SchemaDrivenClient::connect(addr).unwrap();
    assert_eq!(client.server_protocol_version(), 25);
    assert!(matches!(
        client.query("SELECT * FROM dog WHERE breed = 'labrador' ORDER BY age"),
        Err(ClientError::Unsupported("order by with filter"))
    ));
    let unfiltered = rows(client.query("SELECT * FROM dog ORDER BY age").unwrap());
    assert_eq!(unfiltered.len(), 3);
    assert!(client.get(Uuid::from_u128(1)).unwrap().is_some());
}

/// Acceptance criterion 5: `Request::Page` (unfiltered) is unaffected —
/// same result before and after this round for the identical
/// field/limit, run again here (redundant with
/// `order_by_with_limit_returns_rows_ascending_matching_page` above,
/// stated explicitly as this round's own regression check).
#[test]
fn unfiltered_page_is_unaffected_by_filtered_page() {
    let addr = start_dog_server();
    let mut client = SchemaDrivenClient::connect(addr).unwrap();
    let paged = client.page("age", None, 10).unwrap();
    assert_eq!(paged.len(), 3);
    assert_eq!(
        paged.iter().map(|(id, _)| *id).collect::<Vec<_>>(),
        vec![Uuid::from_u128(1), Uuid::from_u128(2), Uuid::from_u128(3)]
    );
}

/// Acceptance criterion 6 (read-your-writes half): a read-your-writes
/// session's own `Query` still sees committed state only — a staged
/// write to a field it reads is never reflected, even though
/// `Session::get` on the same id and field *does* show it.
#[test]
fn query_inside_a_read_your_writes_session_sees_committed_state_only() {
    let addr = start_dog_server();
    let mut client = SchemaDrivenClient::connect(addr).unwrap();
    let mut session = client
        .begin_with(SessionOptions::new().read_your_writes())
        .unwrap();
    session
        .update(Uuid::from_u128(1), "age", ScanValue::U32(99))
        .unwrap();

    // The session's own point read sees the staged write...
    let overlaid = session.get(Uuid::from_u128(1)).unwrap().unwrap();
    assert!(overlaid
        .iter()
        .any(|(name, value)| name == "age" && *value == ScanValue::U32(99)));

    // ...but Query, on the very same open session, does not.
    let result = rows(session.query("SELECT age FROM dog WHERE age = 3").unwrap());
    assert_eq!(
        result.len(),
        1,
        "Query is never overlaid with staged writes, even inside the session that staged them"
    );
    assert!(rows(session.query("SELECT age FROM dog WHERE age = 99").unwrap()).is_empty());

    session.rollback().unwrap();
}

/// Verification plan: "a real client round trip on each of the three
/// domains" — the order domain, covering an `I64` field (`amount_cents`)
/// and an enum-backed `U32` field (`status`).
#[test]
fn order_domain_query_round_trip() {
    let addr = start_order_server();
    let mut client = SchemaDrivenClient::connect(addr).unwrap();

    // status = 1 (Shipped) matches only the first order.
    let shipped = rows(
        client
            .query("SELECT amount_cents FROM order WHERE status = 1")
            .unwrap(),
    );
    assert_eq!(shipped.len(), 1);
    assert_eq!(
        shipped[0].1,
        vec![("amount_cents".to_string(), ScanValue::I64(2_500))]
    );

    // amount_cents > 3000 matches only the second order.
    let mut big = rows(
        client
            .query("SELECT * FROM order WHERE amount_cents > 3000")
            .unwrap(),
    );
    big.sort_by_key(|(id, _)| *id);
    assert_eq!(big.len(), 1);
    assert_eq!(big[0].0, Uuid::from_u128(2));
}

/// Verification plan: "a real client round trip on each of the three
/// domains" — the employee domain, covering a `Str` field (`name`) and an
/// enum-backed `U32` field (`department`).
#[test]
fn employee_domain_query_round_trip() {
    let addr = start_employee_server();
    let mut client = SchemaDrivenClient::connect(addr).unwrap();

    // department = 0 (Engineering) matches only Alex.
    let engineers = rows(
        client
            .query("SELECT name FROM employee WHERE department = 0")
            .unwrap(),
    );
    assert_eq!(engineers.len(), 1);
    assert_eq!(
        engineers[0].1,
        vec![("name".to_string(), ScanValue::Str("Alex".into()))]
    );

    // salary_cents > 1_000_000 matches only Alex too.
    let high_earners = rows(
        client
            .query("SELECT * FROM employee WHERE salary_cents > 1000000")
            .unwrap(),
    );
    assert_eq!(high_earners.len(), 1);
    assert_eq!(high_earners[0].0, Uuid::from_u128(1));
}

/// Acceptance criterion 6 (snapshot-isolation half): a snapshot-isolated
/// session's own `Query` is never tracked into its read set — an
/// external commit to a field the session's `Query` read never causes a
/// spurious conflict at `Commit`, unlike the identical field read via
/// `GetById`.
#[test]
fn query_inside_a_snapshot_isolation_session_is_not_read_set_tracked() {
    let addr = start_dog_server();
    let mut client = SchemaDrivenClient::connect(addr).unwrap();
    let mut other = SchemaDrivenClient::connect(addr).unwrap();

    let mut session = client
        .begin_with(SessionOptions::new().snapshot_isolation())
        .unwrap();
    // A Query reading the exact field an external commit is about to
    // change.
    let result = rows(session.query("SELECT age FROM dog WHERE age = 3").unwrap());
    assert_eq!(result.len(), 1);

    assert!(other
        .update(Uuid::from_u128(1), "age", ScanValue::U32(50))
        .unwrap());

    // A staged write to an unrelated record still commits cleanly —
    // proving the Query above added nothing to the read set (a real
    // GetById on id 1 would have made this Commit fail with Conflict,
    // exactly as `tests/server_transaction_integration.rs`'s own
    // `snapshot_isolation_detects_a_conflicting_commit_from_another_connection`
    // proves for GetById).
    session
        .update(Uuid::from_u128(2), "age", ScanValue::U32(6))
        .unwrap();
    session.commit().unwrap();
}

// `AGG-FR-001`–`010`, ADR-0035, `docs/design/SERVER-SQL-AGGREGATE-DESIGN.md`.

/// This file's dogs, for reference: id1 age=3 breed=labrador, id2 age=5
/// breed=poodle, id3 age=9 breed=labrador.
fn groups(result: QueryResult) -> Vec<Vec<(String, ScanValue)>> {
    match result {
        QueryResult::Groups(groups) => groups,
        QueryResult::Rows(rows) => panic!("expected QueryResult::Groups, got Rows: {rows:?}"),
        QueryResult::Joined(joined) => panic!("expected Rows or Groups, got Joined: {joined:?}"),
    }
}

/// Acceptance criterion 3: `SELECT COUNT(*) FROM dog` with no `GROUP BY`
/// returns exactly one group whose `key` is empty and whose value is the
/// whole table's count.
#[test]
fn count_star_with_no_group_by_returns_one_whole_table_group() {
    let addr = start_dog_server();
    let mut client = SchemaDrivenClient::connect(addr).unwrap();
    let result = groups(client.query("SELECT COUNT(*) FROM dog").unwrap());
    assert_eq!(result.len(), 1);
    assert_eq!(result[0], vec![("COUNT(*)".to_string(), ScanValue::I64(3))]);
}

/// Acceptance criterion 2: `GROUP BY` on one field.
#[test]
fn group_by_one_field() {
    let addr = start_dog_server();
    let mut client = SchemaDrivenClient::connect(addr).unwrap();
    let result = groups(
        client
            .query("SELECT breed, COUNT(*) FROM dog GROUP BY breed")
            .unwrap(),
    );
    assert_eq!(result.len(), 2, "labrador and poodle");
    let labrador = result
        .iter()
        .find(|row| row[0] == ("breed".to_string(), ScanValue::Str("labrador".into())))
        .unwrap();
    assert_eq!(labrador[1], ("COUNT(*)".to_string(), ScanValue::I64(2)));
    let poodle = result
        .iter()
        .find(|row| row[0] == ("breed".to_string(), ScanValue::Str("poodle".into())))
        .unwrap();
    assert_eq!(poodle[1], ("COUNT(*)".to_string(), ScanValue::I64(1)));
}

/// Acceptance criterion 2: `GROUP BY` on two fields — one group per
/// distinct (breed, age) pair, since every dog here has a distinct age.
#[test]
fn group_by_two_fields() {
    let addr = start_dog_server();
    let mut client = SchemaDrivenClient::connect(addr).unwrap();
    let result = groups(
        client
            .query("SELECT breed, age, COUNT(*) FROM dog GROUP BY breed, age")
            .unwrap(),
    );
    assert_eq!(
        result.len(),
        3,
        "every dog has a distinct (breed, age) pair"
    );
}

/// Acceptance criterion 2: every aggregate function, verified against a
/// hand-computed filter-then-bucket-then-reduce over the same rows —
/// labrador = {age 3, age 9}.
#[test]
fn every_aggregate_function_matches_a_hand_computed_reduction() {
    let addr = start_dog_server();
    let mut client = SchemaDrivenClient::connect(addr).unwrap();
    let result = groups(
        client
            .query(
                "SELECT breed, COUNT(*), SUM(age), AVG(age), MIN(age), MAX(age) \
                 FROM dog GROUP BY breed",
            )
            .unwrap(),
    );
    let labrador = result
        .iter()
        .find(|row| row[0] == ("breed".to_string(), ScanValue::Str("labrador".into())))
        .unwrap();
    assert_eq!(labrador[1], ("COUNT(*)".to_string(), ScanValue::I64(2)));
    assert_eq!(labrador[2], ("SUM(age)".to_string(), ScanValue::I64(12)));
    assert_eq!(labrador[3], ("AVG(age)".to_string(), ScanValue::F64(6.0)));
    assert_eq!(labrador[4], ("MIN(age)".to_string(), ScanValue::U32(3)));
    assert_eq!(labrador[5], ("MAX(age)".to_string(), ScanValue::U32(9)));
}

/// Acceptance criterion 6: `AVG` equals that same group's `SUM` divided
/// by its `COUNT`, verified directly against a separate `SUM`+`COUNT`
/// query on the identical `GROUP BY`.
#[test]
fn avg_equals_sum_divided_by_count() {
    let addr = start_dog_server();
    let mut client = SchemaDrivenClient::connect(addr).unwrap();
    let avg_result = groups(
        client
            .query("SELECT breed, AVG(age) FROM dog GROUP BY breed")
            .unwrap(),
    );
    let sum_count_result = groups(
        client
            .query("SELECT breed, SUM(age), COUNT(*) FROM dog GROUP BY breed")
            .unwrap(),
    );
    for avg_row in &avg_result {
        let breed = &avg_row[0];
        let ScanValue::F64(avg) = avg_row[1].1 else {
            panic!("expected F64")
        };
        let sum_count_row = sum_count_result
            .iter()
            .find(|row| &row[0] == breed)
            .unwrap();
        let ScanValue::I64(sum) = sum_count_row[1].1 else {
            panic!("expected I64")
        };
        let ScanValue::I64(count) = sum_count_row[2].1 else {
            panic!("expected I64")
        };
        assert_eq!(avg, sum as f64 / count as f64, "for {breed:?}");
    }
}

/// Acceptance criterion 2: a `WHERE` clause combined with `GROUP BY` —
/// only rows passing the filter are bucketed and reduced.
#[test]
fn where_clause_combines_with_group_by() {
    let addr = start_dog_server();
    let mut client = SchemaDrivenClient::connect(addr).unwrap();
    let result = groups(
        client
            .query("SELECT breed, COUNT(*) FROM dog WHERE age > 3 GROUP BY breed")
            .unwrap(),
    );
    // Only id2 (poodle, age 5) and id3 (labrador, age 9) survive age > 3.
    assert_eq!(result.len(), 2);
    let labrador = result
        .iter()
        .find(|row| row[0] == ("breed".to_string(), ScanValue::Str("labrador".into())))
        .unwrap();
    assert_eq!(labrador[1], ("COUNT(*)".to_string(), ScanValue::I64(1)));
}

/// Acceptance criterion 4: a filter matching zero rows for a given
/// `GROUP BY` key produces zero groups for that key, not a group with a
/// zero/null aggregate value.
#[test]
fn a_filter_matching_nothing_for_a_key_produces_no_group_for_that_key() {
    let addr = start_dog_server();
    let mut client = SchemaDrivenClient::connect(addr).unwrap();
    let result = groups(
        client
            .query("SELECT breed, COUNT(*) FROM dog WHERE age > 100 GROUP BY breed")
            .unwrap(),
    );
    assert!(result.is_empty());
}

/// Acceptance criterion 3/4: `SELECT COUNT(*) FROM dog WHERE false`-
/// equivalent still returns exactly one group whose `Count` is `0` — the
/// implicit whole-table bucket always exists when `group_by` is empty.
#[test]
fn count_star_with_a_filter_matching_nothing_still_returns_one_zero_group() {
    let addr = start_dog_server();
    let mut client = SchemaDrivenClient::connect(addr).unwrap();
    let result = groups(
        client
            .query("SELECT COUNT(*) FROM dog WHERE age > 100")
            .unwrap(),
    );
    assert_eq!(result.len(), 1);
    assert_eq!(result[0], vec![("COUNT(*)".to_string(), ScanValue::I64(0))]);
}

/// Acceptance criterion 2: `LIMIT` on a grouped result truncates the
/// group count only.
#[test]
fn limit_on_a_grouped_result_truncates_the_group_count() {
    let addr = start_dog_server();
    let mut client = SchemaDrivenClient::connect(addr).unwrap();
    let result = groups(
        client
            .query("SELECT breed, COUNT(*) FROM dog GROUP BY breed LIMIT 1")
            .unwrap(),
    );
    assert_eq!(result.len(), 1);
}

/// Acceptance criterion 5: a plain column missing from `GROUP BY` is a
/// client-side `ClientError::Sql`, no round trip.
#[test]
fn a_plain_column_missing_from_group_by_is_a_client_side_error() {
    let addr = start_dog_server();
    let mut client = SchemaDrivenClient::connect(addr).unwrap();
    assert!(matches!(
        client.query("SELECT breed, age, COUNT(*) FROM dog GROUP BY breed"),
        Err(ClientError::Sql(_))
    ));
}

/// Acceptance criterion 5: `SELECT *` alongside `GROUP BY` is rejected
/// the same way.
#[test]
fn select_star_alongside_group_by_is_a_client_side_error() {
    let addr = start_dog_server();
    let mut client = SchemaDrivenClient::connect(addr).unwrap();
    assert!(matches!(
        client.query("SELECT * FROM dog GROUP BY breed"),
        Err(ClientError::Sql(_))
    ));
}

/// Acceptance criterion 5: `SUM`/`AVG`/`MIN`/`MAX` against a `Str` field
/// is a clean client-side rejection, never silently accepted.
#[test]
fn an_aggregate_function_against_a_non_numeric_field_is_a_client_side_error() {
    let addr = start_dog_server();
    let mut client = SchemaDrivenClient::connect(addr).unwrap();
    assert!(matches!(
        client.query("SELECT SUM(breed) FROM dog"),
        Err(ClientError::Sql(_))
    ));
}

/// Acceptance criterion 1: a connection negotiated below 9 gets
/// `ClientError::Unsupported("sql aggregate")` with no frame sent; a
/// plain `SELECT` with no aggregate content still negotiates and runs at
/// 8 unchanged, on the very same connection.
#[test]
fn aggregate_requires_protocol_version_9_but_plain_select_still_runs_at_8() {
    let addr = start_version_7_dog_server();
    let mut client = SchemaDrivenClient::connect(addr).unwrap();
    assert_eq!(client.server_protocol_version(), 7);
    assert!(matches!(
        client.query("SELECT COUNT(*) FROM dog"),
        Err(ClientError::Unsupported("sql aggregate"))
    ));
    // A plain SELECT is refused too, since this fixture negotiates 7 —
    // below both the version-8 and version-9 gates. `query_requires_
    // protocol_version_8` above already covers that half in isolation.
    assert!(matches!(
        client.query("SELECT * FROM dog"),
        Err(ClientError::Unsupported("sql query"))
    ));
}

/// Acceptance criterion 7 (read-your-writes half): `Aggregate` inside a
/// read-your-writes session sees committed state only.
#[test]
fn aggregate_inside_a_read_your_writes_session_sees_committed_state_only() {
    let addr = start_dog_server();
    let mut client = SchemaDrivenClient::connect(addr).unwrap();
    let mut session = client
        .begin_with(SessionOptions::new().read_your_writes())
        .unwrap();
    session
        .update(Uuid::from_u128(1), "age", ScanValue::U32(99))
        .unwrap();

    let result = groups(
        session
            .query("SELECT COUNT(*) FROM dog WHERE age = 99")
            .unwrap(),
    );
    assert_eq!(
        result.len(),
        1,
        "Aggregate is never overlaid with staged writes"
    );
    assert_eq!(result[0], vec![("COUNT(*)".to_string(), ScanValue::I64(0))]);

    session.rollback().unwrap();
}

/// Acceptance criterion 7 (snapshot-isolation half): `Aggregate` inside a
/// snapshot-isolation session is never added to its read set — an
/// external commit to a field the session's `Aggregate` read never
/// causes a spurious conflict at `Commit`.
#[test]
fn aggregate_inside_a_snapshot_isolation_session_is_not_read_set_tracked() {
    let addr = start_dog_server();
    let mut client = SchemaDrivenClient::connect(addr).unwrap();
    let mut other = SchemaDrivenClient::connect(addr).unwrap();

    let mut session = client
        .begin_with(SessionOptions::new().snapshot_isolation())
        .unwrap();
    let result = groups(
        session
            .query("SELECT COUNT(*) FROM dog WHERE age = 3")
            .unwrap(),
    );
    assert_eq!(result.len(), 1);

    assert!(other
        .update(Uuid::from_u128(1), "age", ScanValue::U32(50))
        .unwrap());

    session
        .update(Uuid::from_u128(2), "age", ScanValue::U32(6))
        .unwrap();
    session.commit().unwrap();
}

// ---------------------------------------------------------------------
// `ADR-0073` / `docs/design/SERVER-QUERY-PLANNER-DESIGN.md`, acceptance
// criterion 3 (`QPL-FR-005`): an equality `Query` on every shipped
// `filter_eq: true` field returns exactly the full scan's rows — before
// and after runtime writes — and `Entity::label`'s normalized index is
// corrected to exact `Eq`. The oracle is the unfiltered `SELECT *`,
// filtered here by exact `ScanValue` equality: there is deliberately no
// server-side hook to force a plan, so the comparison is between what the
// planner returns and what the table actually holds.
// ---------------------------------------------------------------------

fn memory(n: u128, category: &str) -> Memory {
    Memory {
        id: Uuid::from_u128(n),
        content: format!("memory {n}"),
        category: category.into(),
        tags: vec!["sample".into()],
        source: "manual".into(),
        metadata_json: "{}".into(),
        created_at_unix_ms: 1_000 * n as i64,
        updated_at_unix_ms: 1_000 * n as i64,
        memory_type: "unclassified".into(),
        status: "active".into(),
        sensitive: false,
        access_count: n as i64,
        deleted_at_unix_ms: 0,
        node_id: String::new(),
    }
}

/// The full field list `Insert`/`Replace` need for a `Memory`, by wire
/// name, in `MemoryConnectionStore`'s own tag order.
fn memory_fields(n: u128, category: &str) -> Vec<(&'static str, ScanValue)> {
    vec![
        ("content", ScanValue::Str(format!("memory {n}"))),
        ("category", ScanValue::Str(category.into())),
        ("tags", ScanValue::StrList(vec!["sample".into()])),
        ("source", ScanValue::Str("manual".into())),
        ("metadata_json", ScanValue::Str("{}".into())),
        ("created_at_unix_ms", ScanValue::I64(1_000 * n as i64)),
        ("updated_at_unix_ms", ScanValue::I64(1_000 * n as i64)),
        ("memory_type", ScanValue::Str("unclassified".into())),
        ("status", ScanValue::Str("active".into())),
        ("sensitive", ScanValue::Bool(false)),
        ("access_count", ScanValue::I64(n as i64)),
        ("deleted_at_unix_ms", ScanValue::I64(0)),
        ("node_id", ScanValue::Str(String::new())),
    ]
}

/// Five memories over three categories: `general` is shared by three
/// records, so an indexed equality is neither the whole table nor one
/// row.
fn start_memory_server() -> SocketAddr {
    let dir = unique_dir("sql_integration_memory");
    std::fs::create_dir_all(&dir).unwrap();
    let path = dir.join("memories.mmap");
    let memories = vec![
        memory(1, "general"),
        memory(2, "preference"),
        memory(3, "general"),
        memory(4, "decision"),
        memory(5, "general"),
    ];
    let stack = create_memory_production_stack(memories, &[], &path).unwrap();
    let connection_store = Arc::new(MemoryConnectionStore::new(GenericProductionStore::new(
        stack,
    )));
    let listener = TcpListener::bind("127.0.0.1:0").unwrap();
    let addr = listener.local_addr().unwrap();
    thread::spawn(move || serve(listener, connection_store, ServeOptions::default()));
    addr
}

fn entity(n: u128, label: &str, kind: &str, mention_count: i64) -> Entity {
    Entity {
        id: Uuid::from_u128(n),
        label: label.into(),
        kind: kind.into(),
        mention_count,
        aliases: vec![],
    }
}

/// Four entities: three `person`s (one with a two-word label whose
/// case/whitespace variants the `label` index also matches) and one
/// `decision`, plus two symmetric `relates_to` edges (1—2, 2—3) so a
/// `JOIN … ON relates_to` has pairs whose left side is and is not a
/// `person`.
fn start_entity_server() -> SocketAddr {
    let dir = unique_dir("sql_integration_entity");
    std::fs::create_dir_all(&dir).unwrap();
    let path = dir.join("entities.mmap");
    let entities = vec![
        entity(1, "Grace Hopper", "person", 7),
        entity(2, "Ada Lovelace", "person", 3),
        entity(3, "ADR-0046", "decision", 1),
        entity(4, "Alan Turing", "person", 12),
    ];
    let relates_to = [
        (Uuid::from_u128(1), Uuid::from_u128(2)),
        (Uuid::from_u128(2), Uuid::from_u128(3)),
    ];
    let stack = create_entity_production_stack(entities, &relates_to, &[], &path).unwrap();
    let connection_store = Arc::new(EntityConnectionStore::new(GenericProductionStore::new(
        stack,
    )));
    let listener = TcpListener::bind("127.0.0.1:0").unwrap();
    let addr = listener.local_addr().unwrap();
    thread::spawn(move || serve(listener, connection_store, ServeOptions::default()));
    addr
}

fn relation(n: u128, subject: &str, label: &str, object: &str) -> Relation {
    Relation {
        id: Uuid::from_u128(n),
        subject: subject.into(),
        relation: label.into(),
        object: object.into(),
        created_at_unix_ms: 1_000 * n as i64,
        updated_at_unix_ms: 1_000 * n as i64,
        node_id: String::new(),
        deleted_at_unix_ms: 0,
    }
}

fn start_relation_server() -> SocketAddr {
    let dir = unique_dir("sql_integration_relation");
    std::fs::create_dir_all(&dir).unwrap();
    let path = dir.join("relations.mmap");
    let relations = vec![
        relation(1, "ada", "authored", "adr-0046"),
        relation(2, "adr-0047", "follows", "adr-0046"),
        relation(3, "ada", "mentions", "grace"),
    ];
    let stack = create_relation_production_stack(relations, &path).unwrap();
    let connection_store = Arc::new(RelationConnectionStore::new(GenericProductionStore::new(
        stack,
    )));
    let listener = TcpListener::bind("127.0.0.1:0").unwrap();
    let addr = listener.local_addr().unwrap();
    thread::spawn(move || serve(listener, connection_store, ServeOptions::default()));
    addr
}

fn start_reminder_server() -> SocketAddr {
    let dir = unique_dir("sql_integration_reminder");
    std::fs::create_dir_all(&dir).unwrap();
    let path = dir.join("reminders.mmap");
    let reminders = vec![
        Reminder {
            id: Uuid::from_u128(1),
            title: "a".into(),
            due_at_unix_ms: 1_000,
            status: ReminderStatus::Pending,
        },
        Reminder {
            id: Uuid::from_u128(2),
            title: "b".into(),
            due_at_unix_ms: 2_000,
            status: ReminderStatus::Done,
        },
        Reminder {
            id: Uuid::from_u128(3),
            title: "c".into(),
            due_at_unix_ms: 1_000,
            status: ReminderStatus::Snoozed,
        },
    ];
    let stack = create_reminder_production_stack(reminders, &path).unwrap();
    let connection_store = Arc::new(ReminderConnectionStore::new(GenericProductionStore::new(
        stack,
    )));
    let listener = TcpListener::bind("127.0.0.1:0").unwrap();
    let addr = listener.local_addr().unwrap();
    thread::spawn(move || serve(listener, connection_store, ServeOptions::default()));
    addr
}

/// `QPL-FR-005`'s oracle: `SELECT * FROM table WHERE field = literal`
/// (the planner's index path when `field` is `filter_eq: true`) must
/// equal the unfiltered `SELECT *` filtered here by exact `ScanValue`
/// equality on `field` — same ids, same fields, compared sorted by id
/// since both plans return rows in their own unspecified order
/// (`QPL-FR-006`). Returns the match count so a caller can assert the
/// case is neither empty nor the whole table.
fn assert_indexed_eq_matches_full_scan(
    client: &mut SchemaDrivenClient,
    table: &str,
    field: &str,
    literal: &str,
    value: &ScanValue,
) -> usize {
    let mut oracle = rows(client.query(&format!("SELECT * FROM {table}")).unwrap());
    let total = oracle.len();
    oracle.retain(|(_, fields)| fields.iter().any(|(name, v)| name == field && v == value));
    oracle.sort_by_key(|(id, _)| *id);
    let mut indexed = rows(
        client
            .query(&format!("SELECT * FROM {table} WHERE {field} = {literal}"))
            .unwrap(),
    );
    indexed.sort_by_key(|(id, _)| *id);
    assert_eq!(
        indexed, oracle,
        "{table}.{field} = {literal}: the index path must return exactly the full scan's rows"
    );
    assert!(
        !indexed.is_empty() && indexed.len() < total,
        "{table}.{field} = {literal}: a meaningful case is neither empty nor the whole table \
         (got {} of {total})",
        indexed.len()
    );
    indexed.len()
}

/// Acceptance criterion 3, every shipped `filter_eq: true` field —
/// `Memory::category`, `Entity::kind`/`label`, `Relation::subject`,
/// `Order::status`, `Employee::department`, `Reminder::due_at_unix_ms` —
/// plus `Dog::breed` as the control that never plans an index
/// (`filter_eq: false` on every `Dog` field) and answers identically.
#[test]
fn indexed_equality_query_matches_the_full_scan_on_every_shipped_index() {
    let mut memory = SchemaDrivenClient::connect(start_memory_server()).unwrap();
    assert_eq!(
        assert_indexed_eq_matches_full_scan(
            &mut memory,
            "memory",
            "category",
            "'general'",
            &ScanValue::Str("general".into()),
        ),
        3
    );

    let mut entity = SchemaDrivenClient::connect(start_entity_server()).unwrap();
    assert_eq!(
        assert_indexed_eq_matches_full_scan(
            &mut entity,
            "entity",
            "kind",
            "'person'",
            &ScanValue::Str("person".into()),
        ),
        3
    );
    assert_eq!(
        assert_indexed_eq_matches_full_scan(
            &mut entity,
            "entity",
            "label",
            "'Grace Hopper'",
            &ScanValue::Str("Grace Hopper".into()),
        ),
        1
    );

    let mut relation = SchemaDrivenClient::connect(start_relation_server()).unwrap();
    assert_eq!(
        assert_indexed_eq_matches_full_scan(
            &mut relation,
            "relation",
            "subject",
            "'ada'",
            &ScanValue::Str("ada".into()),
        ),
        2
    );

    let mut order = SchemaDrivenClient::connect(start_order_server()).unwrap();
    assert_eq!(
        assert_indexed_eq_matches_full_scan(&mut order, "order", "status", "1", &ScanValue::U32(1)),
        1
    );

    let mut employee = SchemaDrivenClient::connect(start_employee_server()).unwrap();
    assert_eq!(
        assert_indexed_eq_matches_full_scan(
            &mut employee,
            "employee",
            "department",
            "0",
            &ScanValue::U32(0),
        ),
        1
    );

    let mut reminder = SchemaDrivenClient::connect(start_reminder_server()).unwrap();
    assert_eq!(
        assert_indexed_eq_matches_full_scan(
            &mut reminder,
            "reminder",
            "due_at_unix_ms",
            "1000",
            &ScanValue::I64(1_000),
        ),
        2
    );

    let mut dog = SchemaDrivenClient::connect(start_dog_server()).unwrap();
    assert_eq!(
        assert_indexed_eq_matches_full_scan(
            &mut dog,
            "dog",
            "breed",
            "'labrador'",
            &ScanValue::Str("labrador".into()),
        ),
        2
    );
}

/// `QPL-FR-003`, acceptance criterion 3: `Entity::label`'s index is
/// case- and whitespace-insensitive (`ENT3-FR-005`, ADR-0040) — a
/// *superset* of exact `Eq`. `Request::FilterEq` returns the record for
/// a variant; `Query`'s `Eq` must not, because the re-check runs the
/// exact comparison over whatever the index handed back.
#[test]
fn entity_label_index_superset_is_corrected_to_exact_equality_by_query() {
    let mut client = SchemaDrivenClient::connect(start_entity_server()).unwrap();
    let grace = Uuid::from_u128(1);

    for variant in [
        "grace hopper",
        "GRACE HOPPER",
        "Grace  Hopper",
        " Grace Hopper ",
    ] {
        let via_filter_eq = client
            .filter_eq("label", ScanValue::Str(variant.into()))
            .unwrap();
        assert!(
            via_filter_eq.contains(&grace),
            "FilterEq's normalized index matches {variant:?}"
        );
        let via_query = rows(
            client
                .query(&format!("SELECT * FROM entity WHERE label = '{variant}'"))
                .unwrap(),
        );
        assert!(
            via_query.is_empty(),
            "Query's Eq is exact: {variant:?} must match nothing, got {via_query:?}"
        );
    }

    let exact = rows(
        client
            .query("SELECT label FROM entity WHERE label = 'Grace Hopper'")
            .unwrap(),
    );
    assert_eq!(exact.len(), 1);
    assert_eq!(exact[0].0, grace);
}

/// `QPL-FR-001`/`003`: with an indexed `Eq` and a second, unindexed
/// predicate, the index narrows the read and the re-check applies both —
/// exactly the rows satisfying the conjunction, and no others.
#[test]
fn indexed_equality_composes_with_a_second_predicate() {
    let mut client = SchemaDrivenClient::connect(start_entity_server()).unwrap();
    let mut result = rows(
        client
            .query("SELECT label FROM entity WHERE kind = 'person' AND mention_count > 5")
            .unwrap(),
    );
    result.sort_by_key(|(id, _)| *id);
    assert_eq!(
        result.iter().map(|(id, _)| *id).collect::<Vec<_>>(),
        vec![Uuid::from_u128(1), Uuid::from_u128(4)]
    );

    // The same conjunction with the predicates reversed plans the same
    // index (`plan_query` picks the first *eligible* predicate) and
    // answers identically.
    let mut reversed = rows(
        client
            .query("SELECT label FROM entity WHERE mention_count > 5 AND kind = 'person'")
            .unwrap(),
    );
    reversed.sort_by_key(|(id, _)| *id);
    assert_eq!(reversed, result);
}

/// `QPL-FR-005`, acceptance criterion 3: the generic equality index is
/// maintained under runtime `Insert`, `Replace` (a record moving into
/// and another out of the indexed value), and `Delete` — the index path
/// tracks each write exactly as the full scan does.
#[test]
fn indexed_equality_query_tracks_runtime_insert_replace_and_delete() {
    let mut client = SchemaDrivenClient::connect(start_memory_server()).unwrap();
    let general = ScanValue::Str("general".into());
    let preference = ScanValue::Str("preference".into());
    let check = |client: &mut SchemaDrivenClient, literal: &str, value: &ScanValue| {
        assert_indexed_eq_matches_full_scan(client, "memory", "category", literal, value)
    };

    assert_eq!(check(&mut client, "'general'", &general), 3);
    assert_eq!(check(&mut client, "'preference'", &preference), 1);

    // Insert a fourth `general`.
    client
        .insert(Uuid::from_u128(6), &memory_fields(6, "general"))
        .unwrap();
    assert_eq!(check(&mut client, "'general'", &general), 4);

    // Replace moves id 1 out of `general` into `preference`.
    client
        .replace(Uuid::from_u128(1), &memory_fields(1, "preference"))
        .unwrap();
    assert_eq!(check(&mut client, "'general'", &general), 3);
    assert_eq!(check(&mut client, "'preference'", &preference), 2);

    // Delete one `general` record.
    assert!(client.delete(Uuid::from_u128(3)).unwrap());
    assert_eq!(check(&mut client, "'general'", &general), 2);

    // A value no record holds (any more, or ever) is an empty result on
    // either plan — never an error.
    let gone = rows(
        client
            .query("SELECT * FROM memory WHERE category = 'never'")
            .unwrap(),
    );
    assert!(gone.is_empty());
}

// ---------------------------------------------------------------------
// `ADR-0074` / `docs/design/SERVER-QUERY-PLANNER-CONSUMERS-DESIGN.md`,
// acceptance criterion 2 (`QPC-FR-005`): the same candidate step behind
// `Aggregate`, `FilteredPage`, and `Join`'s left side returns exactly
// what the full scan returns — groups as a set, a page as an exact
// sequence, pairs as a set — proven against oracles computed in the test
// from unfiltered requests (which plan `FullScan`, having no predicate).
// ---------------------------------------------------------------------

/// The `(id, fields)` rows of `SELECT * FROM {table}`, filtered here by
/// exact `ScanValue` equality on `field` — the oracle every proof below
/// starts from. The unfiltered `SELECT *` has no predicate to plan on,
/// so it is always a full scan.
fn exact_eq_rows(
    client: &mut SchemaDrivenClient,
    table: &str,
    field: &str,
    value: &ScanValue,
) -> Vec<(Uuid, Vec<(String, ScanValue)>)> {
    let mut all = rows(client.query(&format!("SELECT * FROM {table}")).unwrap());
    all.retain(|(_, fields)| fields.iter().any(|(name, v)| name == field && v == value));
    all
}

fn i64_field(fields: &[(String, ScanValue)], name: &str) -> i64 {
    match &fields.iter().find(|(n, _)| n == name).unwrap().1 {
        ScanValue::I64(v) => *v,
        other => panic!("expected I64 for {name}, got {other:?}"),
    }
}

/// `QPC-FR-002`: `Aggregate` through the index path — `COUNT(*)` and
/// `SUM(access_count)` grouped by the indexed field, the implicit single
/// bucket with no `GROUP BY`, and the `Entity::label` superset case —
/// each equal to the tally over the exact-equality oracle rows, before
/// and after a runtime `Insert` into the indexed value.
#[test]
fn aggregate_over_an_indexed_equality_matches_the_full_scan_tally() {
    let mut client = SchemaDrivenClient::connect(start_memory_server()).unwrap();
    let general = ScanValue::Str("general".into());

    let check = |client: &mut SchemaDrivenClient| {
        let oracle = exact_eq_rows(client, "memory", "category", &general);
        let expected_count = oracle.len() as i64;
        let expected_sum: i64 = oracle
            .iter()
            .map(|(_, f)| i64_field(f, "access_count"))
            .sum();

        let grouped = groups(
            client
                .query(
                    "SELECT category, COUNT(*), SUM(access_count) FROM memory \
                     WHERE category = 'general' GROUP BY category",
                )
                .unwrap(),
        );
        assert_eq!(grouped.len(), 1, "one group: general");
        assert_eq!(
            grouped[0][0],
            ("category".to_string(), ScanValue::Str("general".into()))
        );
        assert_eq!(grouped[0][1].1, ScanValue::I64(expected_count), "COUNT(*)");
        assert_eq!(
            grouped[0][2].1,
            ScanValue::I64(expected_sum),
            "SUM(access_count)"
        );

        let implicit = groups(
            client
                .query("SELECT COUNT(*) FROM memory WHERE category = 'general'")
                .unwrap(),
        );
        assert_eq!(implicit.len(), 1, "the implicit bucket always exists");
        assert_eq!(implicit[0][0].1, ScanValue::I64(expected_count));
        expected_count
    };

    assert_eq!(check(&mut client), 3);
    client
        .insert(Uuid::from_u128(6), &memory_fields(6, "general"))
        .unwrap();
    assert_eq!(check(&mut client), 4);

    // A value nothing holds: the implicit bucket still exists, at zero,
    // on either plan (`AGG` acceptance criterion 4, unchanged).
    let none = groups(
        client
            .query("SELECT COUNT(*) FROM memory WHERE category = 'never'")
            .unwrap(),
    );
    assert_eq!(
        none,
        vec![vec![("COUNT(*)".to_string(), ScanValue::I64(0))]]
    );

    // `Entity::label`'s normalized index is a superset of exact `Eq`:
    // `FilterEq` finds the record for a case variant, the aggregate's
    // own re-filter counts nothing for it.
    let mut entity = SchemaDrivenClient::connect(start_entity_server()).unwrap();
    assert!(entity
        .filter_eq("label", ScanValue::Str("grace hopper".into()))
        .unwrap()
        .contains(&Uuid::from_u128(1)));
    let variant = groups(
        entity
            .query("SELECT COUNT(*) FROM entity WHERE label = 'grace hopper'")
            .unwrap(),
    );
    assert_eq!(variant[0][0].1, ScanValue::I64(0));
    let exact = groups(
        entity
            .query("SELECT COUNT(*) FROM entity WHERE label = 'Grace Hopper'")
            .unwrap(),
    );
    assert_eq!(exact[0][0].1, ScanValue::I64(1));
    let people = groups(
        entity
            .query("SELECT kind, COUNT(*) FROM entity WHERE kind = 'person' GROUP BY kind")
            .unwrap(),
    );
    assert_eq!(
        people,
        vec![vec![
            ("kind".to_string(), ScanValue::Str("person".into())),
            ("COUNT(*)".to_string(), ScanValue::I64(3)),
        ]]
    );
}

/// `QPC-FR-003`: `WHERE <indexed> = v ORDER BY <field>` compiles to
/// `FilteredPage`; its rows must be the **exact sequence** `page_rows`
/// would produce over the exact-equality oracle rows — sorted by
/// `(order_by value, id)`, truncated to `LIMIT` — with and without a
/// `LIMIT`, and tracking a runtime `Insert` into and `Replace` out of the
/// indexed value. Order is this consumer's contract, so the sequence is
/// compared, not the set (`QPC-FR-006`: no observable difference).
#[test]
fn filtered_page_over_an_indexed_equality_returns_the_exact_sorted_sequence() {
    let mut client = SchemaDrivenClient::connect(start_memory_server()).unwrap();
    let general = ScanValue::Str("general".into());

    let expected = |client: &mut SchemaDrivenClient, limit: Option<usize>| {
        let mut oracle = exact_eq_rows(client, "memory", "category", &general);
        oracle.sort_by_key(|(id, f)| (i64_field(f, "updated_at_unix_ms"), *id));
        if let Some(n) = limit {
            oracle.truncate(n);
        }
        oracle
    };
    let actual = |client: &mut SchemaDrivenClient, limit: Option<usize>| {
        let sql = match limit {
            Some(n) => format!(
                "SELECT * FROM memory WHERE category = 'general' ORDER BY updated_at_unix_ms LIMIT {n}"
            ),
            None => "SELECT * FROM memory WHERE category = 'general' ORDER BY updated_at_unix_ms"
                .to_string(),
        };
        rows(client.query(&sql).unwrap())
    };
    let check = |client: &mut SchemaDrivenClient, limit: Option<usize>| {
        let want = expected(client, limit);
        let got = actual(client, limit);
        assert_eq!(
            got, want,
            "LIMIT {limit:?}: exact sequence, fields included"
        );
        got.into_iter().map(|(id, _)| id).collect::<Vec<_>>()
    };

    assert_eq!(
        check(&mut client, Some(2)),
        vec![Uuid::from_u128(1), Uuid::from_u128(3)]
    );
    assert_eq!(
        check(&mut client, None),
        vec![Uuid::from_u128(1), Uuid::from_u128(3), Uuid::from_u128(5)]
    );

    client
        .insert(Uuid::from_u128(6), &memory_fields(6, "general"))
        .unwrap();
    assert_eq!(
        check(&mut client, None),
        vec![
            Uuid::from_u128(1),
            Uuid::from_u128(3),
            Uuid::from_u128(5),
            Uuid::from_u128(6)
        ]
    );
    client
        .replace(Uuid::from_u128(1), &memory_fields(1, "preference"))
        .unwrap();
    assert_eq!(
        check(&mut client, None),
        vec![Uuid::from_u128(3), Uuid::from_u128(5), Uuid::from_u128(6)]
    );
    assert_eq!(check(&mut client, Some(1)), vec![Uuid::from_u128(3)]);
}

/// One joined row as `(left_id, right_id, projected fields)`.
type JoinedPair = (Uuid, Uuid, Vec<(String, ScanValue)>);

fn joined_pairs(result: QueryResult) -> Vec<JoinedPair> {
    match result {
        QueryResult::Joined(rows) => {
            let mut pairs: Vec<JoinedPair> = rows
                .into_iter()
                .map(|r| (r.left_id, r.right_id, r.fields))
                .collect();
            pairs.sort_by_key(|(l, r, _)| (*l, *r));
            pairs
        }
        other => panic!("expected Joined, got {other:?}"),
    }
}

/// `QPC-FR-004`: a `JOIN` whose left-side `WHERE` is an indexed equality
/// returns exactly the pairs of the unfiltered join (no left filter →
/// `FullScan`, the oracle) whose left row matches by exact equality —
/// for `kind` (the generic index) and for `label` (the normalized
/// superset index, where a case variant matches `FilterEq` but yields no
/// pairs). Pairs are compared as a set: pair order is unspecified.
#[test]
fn join_with_an_indexed_left_filter_matches_the_full_scan_pairs() {
    let mut client = SchemaDrivenClient::connect(start_entity_server()).unwrap();
    let oracle_all = joined_pairs(
        client
            .query("SELECT a.kind, a.label, b.label FROM entity a JOIN entity b ON relates_to")
            .unwrap(),
    );
    assert_eq!(oracle_all.len(), 4, "edges 1—2 and 2—3, both orientations");

    let oracle_for = |field: &str, value: &str| {
        let mut o = oracle_all.clone();
        o.retain(|(_, _, f)| {
            f.iter()
                .any(|(n, v)| n == field && *v == ScanValue::Str(value.into()))
        });
        o
    };

    let people = joined_pairs(
        client
            .query(
                "SELECT a.kind, a.label, b.label FROM entity a JOIN entity b ON relates_to \
                 WHERE a.kind = 'person'",
            )
            .unwrap(),
    );
    assert_eq!(people, oracle_for("a.kind", "person"));
    assert_eq!(
        people.iter().map(|(l, r, _)| (*l, *r)).collect::<Vec<_>>(),
        vec![
            (Uuid::from_u128(1), Uuid::from_u128(2)),
            (Uuid::from_u128(2), Uuid::from_u128(1)),
            (Uuid::from_u128(2), Uuid::from_u128(3)),
        ],
        "left rows 1 and 2 are persons; 3 (a decision) contributes no left pair"
    );

    let ada = joined_pairs(
        client
            .query(
                "SELECT a.kind, a.label, b.label FROM entity a JOIN entity b ON relates_to \
                 WHERE a.label = 'Ada Lovelace'",
            )
            .unwrap(),
    );
    assert_eq!(ada, oracle_for("a.label", "Ada Lovelace"));
    assert_eq!(ada.len(), 2);

    // The superset case: `FilterEq` matches the variant, the join's own
    // left re-check does not.
    assert!(client
        .filter_eq("label", ScanValue::Str("ada lovelace".into()))
        .unwrap()
        .contains(&Uuid::from_u128(2)));
    let variant = joined_pairs(
        client
            .query(
                "SELECT a.label, b.label FROM entity a JOIN entity b ON relates_to \
                 WHERE a.label = 'ada lovelace'",
            )
            .unwrap(),
    );
    assert!(variant.is_empty());

    // `LIMIT` still bounds the pair count on the index path.
    let limited = joined_pairs(
        client
            .query(
                "SELECT a.label, b.label FROM entity a JOIN entity b ON relates_to \
                 WHERE a.kind = 'person' LIMIT 2",
            )
            .unwrap(),
    );
    assert_eq!(limited.len(), 2);
    let people_ids: Vec<(Uuid, Uuid)> = people.iter().map(|(l, r, _)| (*l, *r)).collect();
    assert!(
        limited
            .iter()
            .all(|(l, r, _)| people_ids.contains(&(*l, *r))),
        "every limited pair is one of the unlimited pairs (projections differ, so ids compare)"
    );
}

// ---------------------------------------------------------------------
// `ADR-0075` / `docs/design/SERVER-QUERY-PLANNER-RANGE-DESIGN.md`,
// acceptance criterion 5 (`QPR-FR-005`): a range on the one field
// `Memory`/`Relation` keep an `Ordered` index over is walked, not
// scanned, and returns exactly the full scan's result — proven against
// two oracles: the unfiltered `SELECT *` filtered here, and the identical
// predicate on `created_at_unix_ms`, which the fixtures set to the same
// value per record and which is never range-indexed (`FullScan`). There
// is deliberately no server-side hook to force a plan.
// ---------------------------------------------------------------------

/// The full field list `Insert`/`Replace` need for a `Relation`, by wire
/// name, in `RelationConnectionStore`'s own tag order, with the stamp
/// chosen by the caller.
fn relation_fields(
    n: u128,
    subject: &str,
    label: &str,
    object: &str,
    updated_at: i64,
) -> Vec<(&'static str, ScanValue)> {
    vec![
        ("subject", ScanValue::Str(subject.into())),
        ("relation", ScanValue::Str(label.into())),
        ("object", ScanValue::Str(object.into())),
        ("created_at_unix_ms", ScanValue::I64(1_000 * n as i64)),
        ("updated_at_unix_ms", ScanValue::I64(updated_at)),
        ("node_id", ScanValue::Str(String::new())),
        ("deleted_at_unix_ms", ScanValue::I64(0)),
    ]
}

/// [`memory_fields`] with `updated_at_unix_ms` overridden — a record
/// re-keyed across a bound.
fn memory_fields_updated_at(
    n: u128,
    category: &str,
    updated_at: i64,
) -> Vec<(&'static str, ScanValue)> {
    let mut fields = memory_fields(n, category);
    for (name, value) in &mut fields {
        if *name == "updated_at_unix_ms" {
            *value = ScanValue::I64(updated_at);
        }
    }
    fields
}

fn compares(value: i64, op: &str, literal: i64) -> bool {
    match op {
        ">" => value > literal,
        ">=" => value >= literal,
        "<" => value < literal,
        "<=" => value <= literal,
        "=" => value == literal,
        other => panic!("unknown comparator {other}"),
    }
}

/// `QPR-FR-005`'s oracle for one table: `SELECT * FROM {table} WHERE
/// updated_at_unix_ms <clause>` (the `Ordered` walk) must equal the
/// unfiltered `SELECT *` filtered here, compared sorted by id (row order
/// is unspecified on either plan, `QPR-FR-006`); and, while every record
/// still carries `created_at == updated_at` (`control`), the identical
/// clause on `created_at_unix_ms` — never range-indexed, so a full scan —
/// must return the same rows. `clause` is `AND`-ed `(op, literal)`
/// pairs. Returns the match count.
fn assert_range_matches_full_scan(
    client: &mut SchemaDrivenClient,
    table: &str,
    clause: &[(&str, i64)],
    control: bool,
) -> usize {
    let where_on = |field: &str| {
        clause
            .iter()
            .map(|(op, k)| format!("{field} {op} {k}"))
            .collect::<Vec<_>>()
            .join(" AND ")
    };
    let mut oracle = rows(client.query(&format!("SELECT * FROM {table}")).unwrap());
    oracle.retain(|(_, f)| {
        clause
            .iter()
            .all(|(op, k)| compares(i64_field(f, "updated_at_unix_ms"), op, *k))
    });
    oracle.sort_by_key(|(id, _)| *id);
    let walked_sql = format!(
        "SELECT * FROM {table} WHERE {}",
        where_on("updated_at_unix_ms")
    );
    let mut walked = rows(client.query(&walked_sql).unwrap());
    walked.sort_by_key(|(id, _)| *id);
    assert_eq!(
        walked, oracle,
        "{walked_sql}: the range walk must return exactly the full scan's rows"
    );
    if control {
        let scanned_sql = format!(
            "SELECT * FROM {table} WHERE {}",
            where_on("created_at_unix_ms")
        );
        let mut scanned = rows(client.query(&scanned_sql).unwrap());
        scanned.sort_by_key(|(id, _)| *id);
        assert_eq!(
            walked, scanned,
            "{scanned_sql}: the never-indexed twin must agree with the walk"
        );
    }
    walked.len()
}

/// `QPR-FR-005`, acceptance criterion 5: every comparator, a two-sided
/// range, `=` as a one-key walk, and the two contradictory ranges, on
/// `Memory` (stamps 1_000–5_000) and `Relation` (1_000–3_000), each
/// against both oracles; then runtime `Insert` into the range, `Replace`
/// re-keying a record across a bound in both directions, and `Delete`,
/// each tracked by the walk (the `created_at` control is dropped once a
/// stamp is re-keyed, since the twin no longer holds the same value).
#[test]
fn range_query_on_the_ordered_field_matches_the_full_scan_on_memory_and_relation() {
    for (table, addr, n) in [
        ("memory", start_memory_server(), 5usize),
        ("relation", start_relation_server(), 3usize),
    ] {
        let mut client = SchemaDrivenClient::connect(addr).unwrap();
        let check = |client: &mut SchemaDrivenClient, clause: &[(&str, i64)]| {
            assert_range_matches_full_scan(client, table, clause, true)
        };
        assert_eq!(check(&mut client, &[(">", 2_000)]), n - 2, "{table} > 2000");
        assert_eq!(
            check(&mut client, &[(">=", 2_000)]),
            n - 1,
            "{table} >= 2000"
        );
        assert_eq!(check(&mut client, &[("<", 2_000)]), 1, "{table} < 2000");
        assert_eq!(check(&mut client, &[("<=", 2_000)]), 2, "{table} <= 2000");
        assert_eq!(
            check(&mut client, &[(">=", 2_000), ("<", 4_000)]),
            2,
            "{table}: 2000 <= stamp < 4000"
        );
        assert_eq!(check(&mut client, &[("=", 2_000)]), 1, "{table} = 2000");
        // ADR-0091 (protocol 29): an inverted range and equal exclusive
        // bounds are contradictions — `Malformed` before any read (they
        // were "empty, never an error" from ADR-0075 to ADR-0090).
        assert_malformed(
            &mut client,
            &format!("SELECT * FROM {table} WHERE updated_at_unix_ms > 4000 AND updated_at_unix_ms < 2000"),
        );
        assert_malformed(
            &mut client,
            &format!("SELECT * FROM {table} WHERE updated_at_unix_ms > 2000 AND updated_at_unix_ms < 2000"),
        );
        assert_eq!(
            check(&mut client, &[(">", 1_000), (">", 3_000)]),
            n.saturating_sub(3),
            "{table}: two lower bounds — the walk takes the first, the re-check the second"
        );
        assert_eq!(
            check(&mut client, &[(">", 1_000), ("<=", 5_000), (">=", 1_000)]),
            n - 1,
            "{table}: three bounds, only the first of each side supplies the walk"
        );
    }

    // Runtime writes on `Memory`.
    let mut client = SchemaDrivenClient::connect(start_memory_server()).unwrap();
    let check = |client: &mut SchemaDrivenClient, clause: &[(&str, i64)], control: bool| {
        assert_range_matches_full_scan(client, "memory", clause, control)
    };
    client
        .insert(Uuid::from_u128(6), &memory_fields(6, "general"))
        .unwrap();
    assert_eq!(
        check(&mut client, &[(">", 2_000)], true),
        4,
        "insert at 6000"
    );
    // Re-key id 1 from 1_000 up to 9_000: into `> 4000`.
    client
        .replace(
            Uuid::from_u128(1),
            &memory_fields_updated_at(1, "general", 9_000),
        )
        .unwrap();
    assert_eq!(check(&mut client, &[(">", 4_000)], false), 3, "ids 5, 6, 1");
    assert_eq!(
        check(&mut client, &[("<", 2_000)], false),
        0,
        "id 1 left the low end"
    );
    // Re-key id 5 from 5_000 down to 500: out of `> 4000`.
    client
        .replace(
            Uuid::from_u128(5),
            &memory_fields_updated_at(5, "general", 500),
        )
        .unwrap();
    assert_eq!(check(&mut client, &[(">", 4_000)], false), 2, "ids 6, 1");
    assert_eq!(
        check(&mut client, &[("<", 2_000)], false),
        1,
        "id 5 arrived"
    );
    assert!(client.delete(Uuid::from_u128(6)).unwrap());
    assert_eq!(check(&mut client, &[(">", 4_000)], false), 1, "id 1 alone");
    assert_eq!(check(&mut client, &[("=", 9_000)], false), 1);
    assert_eq!(check(&mut client, &[("=", 6_000)], false), 0, "deleted");

    // Runtime writes on `Relation`.
    let mut client = SchemaDrivenClient::connect(start_relation_server()).unwrap();
    let check = |client: &mut SchemaDrivenClient, clause: &[(&str, i64)], control: bool| {
        assert_range_matches_full_scan(client, "relation", clause, control)
    };
    client
        .insert(
            Uuid::from_u128(4),
            &relation_fields(4, "grace", "mentions", "ada", 500),
        )
        .unwrap();
    assert_eq!(check(&mut client, &[("<", 2_000)], false), 2, "ids 1 and 4");
    client
        .replace(
            Uuid::from_u128(4),
            &relation_fields(4, "grace", "mentions", "ada", 9_000),
        )
        .unwrap();
    assert_eq!(check(&mut client, &[("<", 2_000)], false), 1);
    assert_eq!(
        check(&mut client, &[(">", 3_000)], false),
        1,
        "id 4 re-keyed up"
    );
    assert!(client.delete(Uuid::from_u128(4)).unwrap());
    assert_eq!(check(&mut client, &[(">", 3_000)], false), 0);
}

/// `QPR-FR-005`, acceptance criterion 5: `Aggregate` through the range
/// walk — `COUNT(*)`/`SUM`/`MIN`/`MAX` over the implicit bucket and
/// `COUNT(*)` grouped by `category` — equal to the tally over the oracle
/// rows and to the never-indexed `created_at` twin; groups compared as a
/// set (`QPR-FR-006`).
#[test]
fn aggregate_over_a_range_matches_the_full_scan_tally() {
    let mut client = SchemaDrivenClient::connect(start_memory_server()).unwrap();
    let mut oracle = rows(client.query("SELECT * FROM memory").unwrap());
    oracle.retain(|(_, f)| {
        let stamp = i64_field(f, "updated_at_unix_ms");
        (2_000..5_000).contains(&stamp)
    });
    assert_eq!(oracle.len(), 3, "ids 2, 3, 4");
    let counts: Vec<i64> = oracle
        .iter()
        .map(|(_, f)| i64_field(f, "access_count"))
        .collect();
    let expected_implicit = vec![
        ScanValue::I64(counts.len() as i64),
        ScanValue::I64(counts.iter().sum()),
        ScanValue::I64(*counts.iter().min().unwrap()),
        ScanValue::I64(*counts.iter().max().unwrap()),
    ];
    let mut expected_grouped: Vec<(ScanValue, i64)> = Vec::new();
    for (_, f) in &oracle {
        let category = f
            .iter()
            .find(|(n, _)| n == "category")
            .map(|(_, v)| v.clone())
            .unwrap();
        match expected_grouped.iter_mut().find(|(c, _)| *c == category) {
            Some((_, n)) => *n += 1,
            None => expected_grouped.push((category, 1)),
        }
    }
    expected_grouped.sort_by(|a, b| format!("{:?}", a.0).cmp(&format!("{:?}", b.0)));

    for field in ["updated_at_unix_ms", "created_at_unix_ms"] {
        let implicit = groups(
            client
                .query(&format!(
                    "SELECT COUNT(*), SUM(access_count), MIN(access_count), MAX(access_count) \
                     FROM memory WHERE {field} >= 2000 AND {field} < 5000"
                ))
                .unwrap(),
        );
        assert_eq!(implicit.len(), 1, "{field}: the implicit bucket");
        let values: Vec<ScanValue> = implicit[0].iter().map(|(_, v)| v.clone()).collect();
        assert_eq!(values, expected_implicit, "{field}: COUNT/SUM/MIN/MAX");

        let mut grouped: Vec<(ScanValue, i64)> = groups(
            client
                .query(&format!(
                    "SELECT category, COUNT(*) FROM memory \
                     WHERE {field} >= 2000 AND {field} < 5000 GROUP BY category"
                ))
                .unwrap(),
        )
        .into_iter()
        .map(|g| match (&g[0].1, &g[1].1) {
            (category, ScanValue::I64(n)) => (category.clone(), *n),
            other => panic!("unexpected group shape {other:?}"),
        })
        .collect();
        grouped.sort_by(|a, b| format!("{:?}", a.0).cmp(&format!("{:?}", b.0)));
        assert_eq!(
            grouped, expected_grouped,
            "{field}: COUNT(*) GROUP BY category"
        );
    }

    // An empty range: the implicit bucket still exists, at zero.
    let none = groups(
        client
            .query("SELECT COUNT(*) FROM memory WHERE updated_at_unix_ms > 9000")
            .unwrap(),
    );
    assert_eq!(
        none,
        vec![vec![("COUNT(*)".to_string(), ScanValue::I64(0))]]
    );
}

/// `QPR-FR-005`/`QPR-FR-006` (3), acceptance criterion 5: `WHERE
/// updated_at_unix_ms > v ORDER BY access_count` compiles to
/// `FilteredPage`; its rows must be the **exact sequence** `page_rows`
/// produces over the oracle rows and the identical sequence the
/// never-indexed `created_at` twin returns — with and without `LIMIT`,
/// after a runtime `Insert` into the range and a `Replace` re-keying a
/// record out of it.
#[test]
fn filtered_page_over_a_range_returns_the_exact_sorted_sequence() {
    let mut client = SchemaDrivenClient::connect(start_memory_server()).unwrap();
    let expected = |client: &mut SchemaDrivenClient, limit: Option<usize>| {
        let mut oracle = rows(client.query("SELECT * FROM memory").unwrap());
        oracle.retain(|(_, f)| i64_field(f, "updated_at_unix_ms") > 1_000);
        oracle.sort_by_key(|(id, f)| (i64_field(f, "access_count"), *id));
        if let Some(n) = limit {
            oracle.truncate(n);
        }
        oracle
    };
    let actual = |client: &mut SchemaDrivenClient, field: &str, limit: Option<usize>| {
        let sql = match limit {
            Some(n) => {
                format!("SELECT * FROM memory WHERE {field} > 1000 ORDER BY access_count LIMIT {n}")
            }
            None => format!("SELECT * FROM memory WHERE {field} > 1000 ORDER BY access_count"),
        };
        rows(client.query(&sql).unwrap())
    };
    let check = |client: &mut SchemaDrivenClient, limit: Option<usize>, control: bool| {
        let want = expected(client, limit);
        let got = actual(client, "updated_at_unix_ms", limit);
        assert_eq!(
            got, want,
            "LIMIT {limit:?}: exact sequence, fields included"
        );
        if control {
            assert_eq!(
                actual(client, "created_at_unix_ms", limit),
                want,
                "LIMIT {limit:?}: the never-indexed twin"
            );
        }
        got.into_iter().map(|(id, _)| id).collect::<Vec<_>>()
    };
    let id = Uuid::from_u128;
    assert_eq!(check(&mut client, Some(2), true), vec![id(2), id(3)]);
    assert_eq!(
        check(&mut client, None, true),
        vec![id(2), id(3), id(4), id(5)]
    );
    client
        .insert(Uuid::from_u128(6), &memory_fields(6, "general"))
        .unwrap();
    assert_eq!(
        check(&mut client, None, true),
        vec![id(2), id(3), id(4), id(5), id(6)]
    );
    // Re-key id 2 below the bound: it leaves the page.
    client
        .replace(id(2), &memory_fields_updated_at(2, "preference", 500))
        .unwrap();
    assert_eq!(
        check(&mut client, None, false),
        vec![id(3), id(4), id(5), id(6)]
    );
    assert_eq!(check(&mut client, Some(1), false), vec![id(3)]);
}

/// `QPR-FR-007`, acceptance criterion 5: a domain with no range field
/// (`Dog`, `Entity`) still answers an ordering predicate from the full
/// scan, exactly as before this round.
#[test]
fn an_ordering_predicate_on_a_domain_with_no_range_field_still_matches_the_full_scan() {
    let mut dog = SchemaDrivenClient::connect(start_dog_server()).unwrap();
    let all = rows(dog.query("SELECT * FROM dog").unwrap());
    let pivot = age_of(&all, all[0].0);
    let mut oracle = all.clone();
    oracle.retain(|(id, _)| age_of(&all, *id) > pivot);
    oracle.sort_by_key(|(id, _)| *id);
    let mut got = rows(
        dog.query(&format!("SELECT * FROM dog WHERE age > {pivot}"))
            .unwrap(),
    );
    got.sort_by_key(|(id, _)| *id);
    assert_eq!(got, oracle);

    let mut entity = SchemaDrivenClient::connect(start_entity_server()).unwrap();
    let all = rows(entity.query("SELECT * FROM entity").unwrap());
    let mut oracle = all.clone();
    oracle.retain(|(_, f)| i64_field(f, "mention_count") >= 1);
    oracle.sort_by_key(|(id, _)| *id);
    let mut got = rows(
        entity
            .query("SELECT * FROM entity WHERE mention_count >= 1")
            .unwrap(),
    );
    got.sort_by_key(|(id, _)| *id);
    assert_eq!(got, oracle);
}

// ---------------------------------------------------------------------
// `ADR-0076` / `docs/design/SERVER-FILTERED-PAGE-WALK-DESIGN.md`,
// acceptance criterion 4 (`FPW-FR-004`/`005`): a page ordered by
// `updated_at_unix_ms` whose `WHERE` is only bounds on that field is the
// bounded walk of the `Ordered` index, and returns exactly the sequence
// `page_rows` produces over the oracle rows; `!=` and a mixed filter
// still answer exactly, through the default.
// ---------------------------------------------------------------------

/// The oracle sequence for `WHERE updated_at_unix_ms <clause> ORDER BY
/// updated_at_unix_ms [LIMIT n]`: the unfiltered `SELECT *` filtered
/// here, sorted by `(updated_at, id)`, truncated.
fn since_page_oracle(
    client: &mut SchemaDrivenClient,
    table: &str,
    clause: &[(&str, i64)],
    limit: Option<usize>,
) -> Vec<(Uuid, Vec<(String, ScanValue)>)> {
    let mut oracle = rows(client.query(&format!("SELECT * FROM {table}")).unwrap());
    oracle.retain(|(_, f)| {
        clause
            .iter()
            .all(|(op, k)| compares(i64_field(f, "updated_at_unix_ms"), op, *k))
    });
    oracle.sort_by_key(|(id, f)| (i64_field(f, "updated_at_unix_ms"), *id));
    if let Some(n) = limit {
        oracle.truncate(n);
    }
    oracle
}

fn since_page(
    client: &mut SchemaDrivenClient,
    table: &str,
    clause: &[(&str, i64)],
    limit: Option<usize>,
) -> Vec<(Uuid, Vec<(String, ScanValue)>)> {
    let where_ = clause
        .iter()
        .map(|(op, k)| format!("updated_at_unix_ms {op} {k}"))
        .collect::<Vec<_>>()
        .join(" AND ");
    let sql = match limit {
        Some(n) => {
            format!("SELECT * FROM {table} WHERE {where_} ORDER BY updated_at_unix_ms LIMIT {n}")
        }
        None => format!("SELECT * FROM {table} WHERE {where_} ORDER BY updated_at_unix_ms"),
    };
    rows(client.query(&sql).unwrap())
}

/// `FPW-FR-004`: every comparator, two-sided, `=`, with and without
/// `LIMIT`, on `Memory` and `Relation`, then after runtime `Insert`,
/// re-keying `Replace`, and `Delete`; `FPW-FR-005`: `!=` and a mixed
/// filter through the default, still exact.
#[test]
fn since_shaped_filtered_page_walks_the_index_and_returns_the_exact_sequence() {
    for (table, addr) in [
        ("memory", start_memory_server()),
        ("relation", start_relation_server()),
    ] {
        let mut client = SchemaDrivenClient::connect(addr).unwrap();
        let check =
            |client: &mut SchemaDrivenClient, clause: &[(&str, i64)], limit: Option<usize>| {
                let want = since_page_oracle(client, table, clause, limit);
                let got = since_page(client, table, clause, limit);
                assert_eq!(got, want, "{table}: {clause:?} LIMIT {limit:?}");
                got.len()
            };
        for limit in [None, Some(1), Some(2), Some(10)] {
            check(&mut client, &[(">", 1_000)], limit);
            check(&mut client, &[(">=", 2_000)], limit);
            check(&mut client, &[("<", 3_000)], limit);
            check(&mut client, &[("<=", 2_000)], limit);
            check(&mut client, &[("=", 2_000)], limit);
            check(&mut client, &[(">=", 2_000), ("<", 3_000)], limit);
        }
        assert_eq!(check(&mut client, &[(">", 1_000)], Some(1)), 1);
        // ADR-0091 (protocol 29): the inverted range is `Malformed`.
        assert_malformed(
            &mut client,
            &format!(
                "SELECT * FROM {table} WHERE updated_at_unix_ms > 3000 AND updated_at_unix_ms < 2000 \
                 ORDER BY updated_at_unix_ms"
            ),
        );
    }

    // Runtime writes on `Memory` (ids 1–5 at 1_000 * n).
    let mut client = SchemaDrivenClient::connect(start_memory_server()).unwrap();
    let id = Uuid::from_u128;
    let ids = |rows: Vec<(Uuid, Vec<(String, ScanValue)>)>| {
        rows.into_iter().map(|(id, _)| id).collect::<Vec<_>>()
    };
    let check = |client: &mut SchemaDrivenClient, clause: &[(&str, i64)], limit: Option<usize>| {
        let want = since_page_oracle(client, "memory", clause, limit);
        let got = since_page(client, "memory", clause, limit);
        assert_eq!(got, want, "memory: {clause:?} LIMIT {limit:?}");
        ids(got)
    };
    client
        .insert(id(6), &memory_fields_updated_at(6, "general", 2_500))
        .unwrap();
    assert_eq!(
        check(&mut client, &[(">", 1_000)], Some(3)),
        vec![id(2), id(6), id(3)],
        "the insert lands by its stamp"
    );
    client
        .replace(id(2), &memory_fields_updated_at(2, "general", 9_000))
        .unwrap();
    assert_eq!(
        check(&mut client, &[(">", 1_000)], None),
        vec![id(6), id(3), id(4), id(5), id(2)],
        "re-keyed to the end"
    );
    assert!(client.delete(id(3)).unwrap());
    assert_eq!(
        check(&mut client, &[(">=", 2_000), ("<=", 5_000)], Some(2)),
        vec![id(6), id(4)]
    );

    // `FPW-FR-005`: ineligible shapes answer exactly through the default.
    let mut oracle = rows(client.query("SELECT * FROM memory").unwrap());
    oracle.retain(|(_, f)| i64_field(f, "updated_at_unix_ms") != 4_000);
    oracle.sort_by_key(|(id, f)| (i64_field(f, "updated_at_unix_ms"), *id));
    let got = rows(
        client
            .query(
                "SELECT * FROM memory WHERE updated_at_unix_ms != 4000 ORDER BY updated_at_unix_ms",
            )
            .unwrap(),
    );
    assert_eq!(got, oracle, "!= through the default");
    let mut oracle = rows(client.query("SELECT * FROM memory").unwrap());
    oracle.retain(|(_, f)| {
        i64_field(f, "updated_at_unix_ms") > 1_000
            && f.iter()
                .any(|(n, v)| n == "category" && *v == ScanValue::Str("general".into()))
    });
    oracle.sort_by_key(|(id, f)| (i64_field(f, "updated_at_unix_ms"), *id));
    oracle.truncate(2);
    let got = rows(
        client
            .query(
                "SELECT * FROM memory WHERE updated_at_unix_ms > 1000 AND category = 'general' \
                 ORDER BY updated_at_unix_ms LIMIT 2",
            )
            .unwrap(),
    );
    assert_eq!(got, oracle, "a mixed filter through the default");
}

// ---------------------------------------------------------------------
// `ADR-0077` / `docs/design/SERVER-FILTERED-PAGE-WALK-MIXED-DESIGN.md`,
// acceptance criterion 4 (`FPM-FR-004`/`005`): a page ordered by
// `updated_at_unix_ms` whose `WHERE` mixes bounds on that field with
// predicates on other, unindexed fields (or `!=` on it) walks the
// `Ordered` index past the rejects and returns exactly the sorted
// oracle's sequence; an equality on the declared index still answers
// exactly, through the default's bucket.
// ---------------------------------------------------------------------

/// The oracle for `SELECT * FROM {table} WHERE {where} ORDER BY
/// updated_at_unix_ms [LIMIT n]`: every row, kept by `keep`, sorted by
/// `(updated_at, id)`, truncated.
fn mixed_page_oracle(
    client: &mut SchemaDrivenClient,
    table: &str,
    keep: impl Fn(&[(String, ScanValue)]) -> bool,
    limit: Option<usize>,
) -> Vec<(Uuid, Vec<(String, ScanValue)>)> {
    let mut oracle = rows(client.query(&format!("SELECT * FROM {table}")).unwrap());
    oracle.retain(|(_, f)| keep(f));
    oracle.sort_by_key(|(id, f)| (i64_field(f, "updated_at_unix_ms"), *id));
    if let Some(n) = limit {
        oracle.truncate(n);
    }
    oracle
}

/// A row-level predicate the oracle keeps rows by.
type Keep<'a> = &'a dyn Fn(&[(String, ScanValue)]) -> bool;

fn str_field(fields: &[(String, ScanValue)], name: &str) -> String {
    match fields.iter().find(|(n, _)| n == name) {
        Some((_, ScanValue::Str(s))) => s.clone(),
        other => panic!("{name}: expected a Str field, got {other:?}"),
    }
}

/// `FPM-FR-004`: on `Memory` (`content` unique per record, `source`
/// shared by every record, neither indexed) and `Relation` (`object`
/// unindexed, `subject` the declared index), with and without `LIMIT`,
/// then after runtime `Insert`, re-keying `Replace`, and `Delete`;
/// `FPM-FR-005`: an equality on the declared index, still exact.
#[test]
fn mixed_filtered_page_walks_past_rejects_and_returns_the_exact_sequence() {
    let mut client = SchemaDrivenClient::connect(start_memory_server()).unwrap();
    let id = Uuid::from_u128;
    let ids = |rows: Vec<(Uuid, Vec<(String, ScanValue)>)>| {
        rows.into_iter().map(|(id, _)| id).collect::<Vec<_>>()
    };
    let check = |client: &mut SchemaDrivenClient,
                 where_: &str,
                 keep: Keep,
                 limit: Option<usize>| {
        let want = mixed_page_oracle(client, "memory", keep, limit);
        let sql = match limit {
            Some(n) => {
                format!("SELECT * FROM memory WHERE {where_} ORDER BY updated_at_unix_ms LIMIT {n}")
            }
            None => format!("SELECT * FROM memory WHERE {where_} ORDER BY updated_at_unix_ms"),
        };
        let got = rows(client.query(&sql).unwrap());
        assert_eq!(got, want, "memory: {where_} LIMIT {limit:?}");
        ids(got)
    };
    let updated = |f: &[(String, ScanValue)]| i64_field(f, "updated_at_unix_ms");
    let content = |f: &[(String, ScanValue)]| str_field(f, "content");
    let source = |f: &[(String, ScanValue)]| str_field(f, "source");

    for limit in [None, Some(1), Some(2), Some(10)] {
        // Every record's `source` is "manual": rejects nothing, walks on.
        check(
            &mut client,
            "updated_at_unix_ms > 1000 AND source = 'manual'",
            &|f| updated(f) > 1_000 && source(f) == "manual",
            limit,
        );
        // Rejects everything: an empty page after the whole walk.
        check(
            &mut client,
            "updated_at_unix_ms > 1000 AND source != 'manual'",
            &|f| updated(f) > 1_000 && source(f) != "manual",
            limit,
        );
        // Walks past 2 and 3 to 4, then 5 is rejected and the index ends.
        check(
            &mut client,
            "updated_at_unix_ms > 1000 AND content = 'memory 4'",
            &|f| updated(f) > 1_000 && content(f) == "memory 4",
            limit,
        );
        // `!=` on the walked field is a reject, not a cut.
        check(
            &mut client,
            "updated_at_unix_ms != 3000",
            &|f| updated(f) != 3_000,
            limit,
        );
        check(
            &mut client,
            "updated_at_unix_ms >= 2000 AND updated_at_unix_ms != 3000 AND content != 'memory 5'",
            &|f| updated(f) >= 2_000 && updated(f) != 3_000 && content(f) != "memory 5",
            limit,
        );
        // `FPM-FR-005`: the declared `category` index, equality-first.
        check(
            &mut client,
            "updated_at_unix_ms > 1000 AND category = 'general'",
            &|f| updated(f) > 1_000 && str_field(f, "category") == "general",
            limit,
        );
    }
    assert_eq!(
        check(
            &mut client,
            "updated_at_unix_ms > 1000 AND content = 'memory 4'",
            &|f| updated(f) > 1_000 && content(f) == "memory 4",
            Some(2),
        ),
        vec![id(4)]
    );
    assert_eq!(
        check(
            &mut client,
            "updated_at_unix_ms != 3000",
            &|f| updated(f) != 3_000,
            Some(3),
        ),
        vec![id(1), id(2), id(4)]
    );

    // Runtime writes: an insert the rejects admit lands by its stamp, a
    // re-keyed record moves, a deleted one is walked past.
    client
        .insert(id(6), &memory_fields_updated_at(6, "general", 2_500))
        .unwrap();
    assert_eq!(
        check(
            &mut client,
            "updated_at_unix_ms > 1000 AND source = 'manual' AND content != 'memory 3'",
            &|f| updated(f) > 1_000 && source(f) == "manual" && content(f) != "memory 3",
            Some(3),
        ),
        vec![id(2), id(6), id(4)]
    );
    client
        .replace(id(2), &memory_fields_updated_at(2, "general", 9_000))
        .unwrap();
    assert!(client.delete(id(3)).unwrap());
    assert_eq!(
        check(
            &mut client,
            "updated_at_unix_ms > 1000 AND content != 'memory 4'",
            &|f| updated(f) > 1_000 && content(f) != "memory 4",
            None,
        ),
        vec![id(6), id(5), id(2)]
    );

    // `Relation`: `object` is unindexed (a reject), `subject` the
    // declared index (equality-first, the default's bucket).
    let mut client = SchemaDrivenClient::connect(start_relation_server()).unwrap();
    for (where_, keep) in [
        (
            "updated_at_unix_ms >= 1000 AND object = 'adr-0046'",
            &(|f: &[(String, ScanValue)]| {
                updated(f) >= 1_000 && str_field(f, "object") == "adr-0046"
            }) as Keep,
        ),
        (
            "updated_at_unix_ms < 3000 AND object != 'adr-0046'",
            &|f: &[(String, ScanValue)]| updated(f) < 3_000 && str_field(f, "object") != "adr-0046",
        ),
        (
            "updated_at_unix_ms >= 1000 AND subject = 'ada'",
            &|f: &[(String, ScanValue)]| updated(f) >= 1_000 && str_field(f, "subject") == "ada",
        ),
    ] {
        for limit in [None, Some(1), Some(5)] {
            let want = mixed_page_oracle(&mut client, "relation", keep, limit);
            let sql = match limit {
                Some(n) => format!(
                    "SELECT * FROM relation WHERE {where_} ORDER BY updated_at_unix_ms LIMIT {n}"
                ),
                None => {
                    format!("SELECT * FROM relation WHERE {where_} ORDER BY updated_at_unix_ms")
                }
            };
            let got = rows(client.query(&sql).unwrap());
            assert_eq!(got, want, "relation: {where_} LIMIT {limit:?}");
        }
    }
    let got = rows(
        client
            .query(
                "SELECT * FROM relation WHERE updated_at_unix_ms >= 1000 AND object = 'adr-0046' \
                 ORDER BY updated_at_unix_ms LIMIT 1",
            )
            .unwrap(),
    );
    assert_eq!(ids(got), vec![id(1)]);
}

// ---------------------------------------------------------------------
// `ADR-0078` / `docs/design/SERVER-QUERY-PLANNER-INTERSECT-DESIGN.md`,
// acceptance criterion 4 (`QPI-FR-003`/`004`): a `WHERE` carrying both
// an equality on the declared index and a bound on the range field
// answers `Query`, `COUNT(*)`, and a page ordered by the range field
// exactly as before — the intersection changes what is read, never what
// is returned — on `Memory` (`category` × `updated_at_unix_ms`) and
// `Relation` (`subject` × `updated_at_unix_ms`), through runtime writes.
// ---------------------------------------------------------------------

/// `QPI-FR-004`: the intersected shapes as the exact oracle's set,
/// count, and sequence.
#[test]
fn an_indexed_equality_with_a_range_returns_the_exact_set_count_and_sequence() {
    let id = Uuid::from_u128;
    let updated = |f: &[(String, ScanValue)]| i64_field(f, "updated_at_unix_ms");
    let sorted = |mut rows: Vec<(Uuid, Vec<(String, ScanValue)>)>| {
        rows.sort_by_key(|(id, _)| *id);
        rows
    };
    let check = |client: &mut SchemaDrivenClient,
                 table: &str,
                 where_: &str,
                 keep: Keep,
                 pinned: Option<&[Uuid]>| {
        let mut oracle = rows(client.query(&format!("SELECT * FROM {table}")).unwrap());
        oracle.retain(|(_, f)| keep(f));
        let got = rows(
            client
                .query(&format!("SELECT * FROM {table} WHERE {where_}"))
                .unwrap(),
        );
        assert_eq!(
            sorted(got.clone()),
            sorted(oracle.clone()),
            "{table}: {where_}"
        );
        let counted = groups(
            client
                .query(&format!("SELECT COUNT(*) FROM {table} WHERE {where_}"))
                .unwrap(),
        );
        assert_eq!(
            counted[0],
            vec![("COUNT(*)".to_string(), ScanValue::I64(oracle.len() as i64))],
            "{table}: COUNT(*) WHERE {where_}"
        );
        let paged = rows(
            client
                .query(&format!(
                    "SELECT * FROM {table} WHERE {where_} ORDER BY updated_at_unix_ms LIMIT 2"
                ))
                .unwrap(),
        );
        oracle.sort_by_key(|(id, f)| (updated(f), *id));
        oracle.truncate(2);
        assert_eq!(paged, oracle, "{table}: paged WHERE {where_}");
        if let Some(pinned) = pinned {
            let mut ids: Vec<Uuid> = got.into_iter().map(|(id, _)| id).collect();
            ids.sort();
            assert_eq!(ids, pinned, "{table}: {where_}");
        }
    };

    let mut client = SchemaDrivenClient::connect(start_memory_server()).unwrap();
    let general = |f: &[(String, ScanValue)]| str_field(f, "category") == "general";
    check(
        &mut client,
        "memory",
        "category = 'general' AND updated_at_unix_ms >= 3000",
        &|f| general(f) && updated(f) >= 3_000,
        Some(&[id(3), id(5)]),
    );
    check(
        &mut client,
        "memory",
        "updated_at_unix_ms > 1000 AND category = 'general' AND updated_at_unix_ms < 5000",
        &|f| general(f) && updated(f) > 1_000 && updated(f) < 5_000,
        Some(&[id(3)]),
    );
    check(
        &mut client,
        "memory",
        "category = 'general' AND updated_at_unix_ms = 1000",
        &|f| general(f) && updated(f) == 1_000,
        Some(&[id(1)]),
    );
    check(
        &mut client,
        "memory",
        "category = 'decision' AND updated_at_unix_ms >= 5000",
        &|f| str_field(f, "category") == "decision" && updated(f) >= 5_000,
        Some(&[]),
    );
    check(
        &mut client,
        "memory",
        "category = 'nobody' AND updated_at_unix_ms >= 1000",
        &|f| str_field(f, "category") == "nobody" && updated(f) >= 1_000,
        Some(&[]),
    );
    // Runtime writes: an insert into the bucket lands in the range, a
    // re-key moves a record out of it, a delete removes one.
    client
        .insert(id(6), &memory_fields_updated_at(6, "general", 3_500))
        .unwrap();
    client
        .replace(id(5), &memory_fields_updated_at(5, "general", 500))
        .unwrap();
    assert!(client.delete(id(3)).unwrap());
    check(
        &mut client,
        "memory",
        "category = 'general' AND updated_at_unix_ms >= 3000",
        &|f| general(f) && updated(f) >= 3_000,
        Some(&[id(6)]),
    );

    let mut client = SchemaDrivenClient::connect(start_relation_server()).unwrap();
    check(
        &mut client,
        "relation",
        "subject = 'ada' AND updated_at_unix_ms >= 2000",
        &|f| str_field(f, "subject") == "ada" && updated(f) >= 2_000,
        Some(&[id(3)]),
    );
    check(
        &mut client,
        "relation",
        "updated_at_unix_ms <= 2000 AND subject = 'ada'",
        &|f| str_field(f, "subject") == "ada" && updated(f) <= 2_000,
        Some(&[id(1)]),
    );
}

// ---------------------------------------------------------------------
// `ADR-0080` / `docs/design/SERVER-REMINDER-DUE-INDEX-DESIGN.md`,
// acceptance criterion 4 (`RDO-FR-004`): `Reminder`'s `due_at_unix_ms`
// is range-indexed — the consumer's "what is due" shapes over a real
// socket answer exactly the sorted oracle, through runtime writes.
// Fixture: 1 at 1_000 pending, 2 at 2_000 done, 3 at 1_000 snoozed.
// ---------------------------------------------------------------------

#[test]
fn reminder_due_at_range_and_page_return_the_exact_sequence() {
    let id = Uuid::from_u128;
    let due = |f: &[(String, ScanValue)]| i64_field(f, "due_at_unix_ms");
    let status = |f: &[(String, ScanValue)]| match f.iter().find(|(n, _)| n == "status") {
        Some((_, ScanValue::U32(s))) => *s,
        other => panic!("status: {other:?}"),
    };
    let mut client = SchemaDrivenClient::connect(start_reminder_server()).unwrap();
    let ids = |rows: Vec<(Uuid, Vec<(String, ScanValue)>)>| {
        rows.into_iter().map(|(id, _)| id).collect::<Vec<_>>()
    };
    let check =
        |client: &mut SchemaDrivenClient, where_: &str, keep: Keep, limit: Option<usize>| {
            let mut oracle = rows(client.query("SELECT * FROM reminder").unwrap());
            oracle.retain(|(_, f)| keep(f));
            oracle.sort_by_key(|(id, f)| (due(f), *id));
            if let Some(n) = limit {
                oracle.truncate(n);
            }
            let sql = match limit {
                Some(n) => format!(
                    "SELECT * FROM reminder WHERE {where_} ORDER BY due_at_unix_ms LIMIT {n}"
                ),
                None => format!("SELECT * FROM reminder WHERE {where_} ORDER BY due_at_unix_ms"),
            };
            let got = rows(client.query(&sql).unwrap());
            assert_eq!(got, oracle, "reminder: {where_} LIMIT {limit:?}");
            let counted = groups(
                client
                    .query(&format!("SELECT COUNT(*) FROM reminder WHERE {where_}"))
                    .unwrap(),
            );
            let mut all = rows(client.query("SELECT * FROM reminder").unwrap());
            all.retain(|(_, f)| keep(f));
            assert_eq!(
                counted[0],
                vec![("COUNT(*)".to_string(), ScanValue::I64(all.len() as i64))]
            );
            ids(got)
        };
    for limit in [None, Some(1), Some(5)] {
        // The due-now listing: a bound and a `status` reject.
        check(
            &mut client,
            "due_at_unix_ms <= 1500 AND status = 0",
            &|f| due(f) <= 1_500 && status(f) == 0,
            limit,
        );
        check(
            &mut client,
            "due_at_unix_ms <= 1500 AND status != 0",
            &|f| due(f) <= 1_500 && status(f) != 0,
            limit,
        );
        check(
            &mut client,
            "due_at_unix_ms >= 2000",
            &|f| due(f) >= 2_000,
            limit,
        );
        check(
            &mut client,
            "due_at_unix_ms > 500 AND due_at_unix_ms < 1500",
            &|f| due(f) > 500 && due(f) < 1_500,
            limit,
        );
        // The `due_at` equality is the bucket (equality-first), with and
        // without a bound beside it (the intersection).
        check(
            &mut client,
            "due_at_unix_ms = 1000",
            &|f| due(f) == 1_000,
            limit,
        );
        check(
            &mut client,
            "due_at_unix_ms = 1000 AND due_at_unix_ms >= 1000 AND status = 2",
            &|f| due(f) == 1_000 && status(f) == 2,
            limit,
        );
    }
    assert_eq!(
        check(
            &mut client,
            "due_at_unix_ms <= 1500 AND status = 0",
            &|f| due(f) <= 1_500 && status(f) == 0,
            Some(5),
        ),
        vec![id(1)]
    );
    assert_eq!(
        check(
            &mut client,
            "due_at_unix_ms <= 1500 AND status != 0",
            &|f| due(f) <= 1_500 && status(f) != 0,
            None,
        ),
        vec![id(3)]
    );
    // A status update (the one runtime write this domain takes) is
    // reflected; the order is untouched.
    client.update(id(1), "status", ScanValue::U32(1)).unwrap();
    assert_eq!(
        check(
            &mut client,
            "due_at_unix_ms <= 1500 AND status = 0",
            &|f| due(f) <= 1_500 && status(f) == 0,
            None,
        ),
        Vec::<Uuid>::new()
    );
    assert_eq!(
        check(
            &mut client,
            "due_at_unix_ms <= 2000",
            &|f| due(f) <= 2_000,
            None
        ),
        vec![id(1), id(3), id(2)]
    );
}

// ---------------------------------------------------------------------
// `ADR-0081` / `docs/design/SERVER-QUERY-PLANNER-COUNTED-WALK-DESIGN.md`,
// acceptance criterion 4 (`QCW-FR-005`): a `COUNT(*)` whose filter is
// only bounds on the range field answers from the sorted index with
// no record read, and is exactly the decode path's count — on `Memory`,
// `Relation`, and `Reminder`, with and without a filter, through
// runtime writes; every ineligible count shape still exact.
// ---------------------------------------------------------------------

#[test]
fn a_pure_range_count_from_the_index_matches_the_decoded_count_exactly() {
    let count_where = |client: &mut SchemaDrivenClient, table: &str, where_: &str| -> i64 {
        let sql = if where_.is_empty() {
            format!("SELECT COUNT(*) FROM {table}")
        } else {
            format!("SELECT COUNT(*) FROM {table} WHERE {where_}")
        };
        match &groups(client.query(&sql).unwrap())[0][0] {
            (_, ScanValue::I64(n)) => *n,
            other => panic!("{other:?}"),
        }
    };
    let oracle = |client: &mut SchemaDrivenClient, table: &str, keep: Keep| -> i64 {
        let mut all = rows(client.query(&format!("SELECT * FROM {table}")).unwrap());
        all.retain(|(_, f)| keep(f));
        all.len() as i64
    };
    let updated = |f: &[(String, ScanValue)]| i64_field(f, "updated_at_unix_ms");
    let due = |f: &[(String, ScanValue)]| i64_field(f, "due_at_unix_ms");

    let mut client = SchemaDrivenClient::connect(start_memory_server()).unwrap();
    let id = Uuid::from_u128;
    type Row = [(String, ScanValue)];
    for (where_, keep) in [
        ("", &(|_: &Row| true) as Keep),
        ("updated_at_unix_ms >= 2000", &|f: &Row| updated(f) >= 2_000),
        (
            "updated_at_unix_ms > 2000 AND updated_at_unix_ms <= 4000",
            &|f: &Row| updated(f) > 2_000 && updated(f) <= 4_000,
        ),
        ("updated_at_unix_ms = 3000", &|f: &Row| updated(f) == 3_000),
        ("updated_at_unix_ms < 1000", &|f: &Row| updated(f) < 1_000),
        // ADR-0091 (protocol 29): the inverted range (`> 5000 AND < 1000`)
        // that once counted zero is `Malformed`, asserted below the loop.
        // Ineligible: still the decoded count.
        (
            "updated_at_unix_ms >= 2000 AND category = 'general'",
            &|f: &Row| updated(f) >= 2_000 && str_field(f, "category") == "general",
        ),
        ("updated_at_unix_ms != 2000", &|f: &Row| updated(f) != 2_000),
        (
            "updated_at_unix_ms >= 1000 AND updated_at_unix_ms >= 3000",
            &|f: &Row| updated(f) >= 3_000,
        ),
    ] {
        assert_eq!(
            count_where(&mut client, "memory", where_),
            oracle(&mut client, "memory", keep),
            "memory: COUNT(*) WHERE {where_}"
        );
    }
    assert_malformed(
        &mut client,
        "SELECT COUNT(*) FROM memory WHERE updated_at_unix_ms > 5000 AND updated_at_unix_ms < 1000",
    );
    assert_eq!(
        count_where(&mut client, "memory", "updated_at_unix_ms >= 2000"),
        4
    );
    // Runtime writes move the count with the index.
    client
        .insert(id(6), &memory_fields_updated_at(6, "general", 2_500))
        .unwrap();
    assert_eq!(
        count_where(&mut client, "memory", "updated_at_unix_ms >= 2000"),
        5
    );
    client
        .replace(id(6), &memory_fields_updated_at(6, "general", 500))
        .unwrap();
    assert_eq!(
        count_where(&mut client, "memory", "updated_at_unix_ms >= 2000"),
        4
    );
    assert!(client.delete(id(5)).unwrap());
    assert_eq!(
        count_where(&mut client, "memory", "updated_at_unix_ms >= 2000"),
        3
    );
    assert_eq!(count_where(&mut client, "memory", ""), 5);

    let mut client = SchemaDrivenClient::connect(start_relation_server()).unwrap();
    for (where_, keep) in [
        ("", &(|_: &[(String, ScanValue)]| true) as Keep),
        ("updated_at_unix_ms <= 2000", &|f: &[(
            String,
            ScanValue,
        )]| {
            updated(f) <= 2_000
        }),
    ] {
        assert_eq!(
            count_where(&mut client, "relation", where_),
            oracle(&mut client, "relation", keep),
            "relation: {where_}"
        );
    }

    let mut client = SchemaDrivenClient::connect(start_reminder_server()).unwrap();
    for (where_, keep) in [
        ("", &(|_: &[(String, ScanValue)]| true) as Keep),
        ("due_at_unix_ms <= 1500", &|f: &[(String, ScanValue)]| {
            due(f) <= 1_500
        }),
        ("due_at_unix_ms <= 1500 AND status = 0", &|f: &[(
            String,
            ScanValue,
        )]| {
            due(f) <= 1_500
                && matches!(
                    f.iter().find(|(n, _)| n == "status"),
                    Some((_, ScanValue::U32(0)))
                )
        }),
        // `due_at = k` plans the equality bucket, not the walk: decoded.
        ("due_at_unix_ms = 1000", &|f: &[(String, ScanValue)]| {
            due(f) == 1_000
        }),
    ] {
        assert_eq!(
            count_where(&mut client, "reminder", where_),
            oracle(&mut client, "reminder", keep),
            "reminder: {where_}"
        );
    }
    assert_eq!(
        count_where(&mut client, "reminder", "due_at_unix_ms <= 1500"),
        2
    );
}

// ---------------------------------------------------------------------
// `ADR-0082` / `docs/design/SERVER-QUERY-PLANNER-KEYED-WALK-DESIGN.md`,
// acceptance criterion 4 (`QKW-FR-005`): `SUM`/`AVG`/`MIN`/`MAX` of the
// range field over a pure range answer from the walked keys, no record
// read, and equal the decode path exactly — on `Memory` and `Reminder`,
// over a range, everything, and nothing; an aggregate over another
// field, or a second predicate, still decodes and is still exact.
// ---------------------------------------------------------------------

#[test]
fn aggregates_over_the_range_field_from_the_walked_keys_match_the_decoded_answers() {
    let scalar = |client: &mut SchemaDrivenClient, sql: &str| -> ScanValue {
        groups(client.query(sql).unwrap())[0][0].1.clone()
    };
    let keys =
        |client: &mut SchemaDrivenClient, table: &str, field: &str, keep: Keep| -> Vec<i64> {
            let mut all = rows(client.query(&format!("SELECT * FROM {table}")).unwrap());
            all.retain(|(_, f)| keep(f));
            all.iter().map(|(_, f)| i64_field(f, field)).collect()
        };
    let expect = |ks: &[i64], func: &str| -> ScanValue {
        match func {
            "SUM" => ScanValue::I64(ks.iter().sum()),
            "AVG" if ks.is_empty() => ScanValue::F64(0.0),
            "AVG" => ScanValue::F64(ks.iter().sum::<i64>() as f64 / ks.len() as f64),
            "MIN" => ScanValue::I64(ks.iter().copied().min().unwrap_or(0)),
            "MAX" => ScanValue::I64(ks.iter().copied().max().unwrap_or(0)),
            other => panic!("{other}"),
        }
    };
    type Row = [(String, ScanValue)];
    let updated = |f: &Row| i64_field(f, "updated_at_unix_ms");
    let due = |f: &Row| i64_field(f, "due_at_unix_ms");

    let mut client = SchemaDrivenClient::connect(start_memory_server()).unwrap();
    for (where_, keep) in [
        ("", &(|_: &Row| true) as Keep),
        ("WHERE updated_at_unix_ms >= 2000", &|f: &Row| {
            updated(f) >= 2_000
        }),
        (
            "WHERE updated_at_unix_ms > 2000 AND updated_at_unix_ms < 5000",
            &|f: &Row| updated(f) > 2_000 && updated(f) < 5_000,
        ),
        ("WHERE updated_at_unix_ms > 9000", &|f: &Row| {
            updated(f) > 9_000
        }),
        // Ineligible (a second predicate): decoded, still exact.
        (
            "WHERE updated_at_unix_ms >= 2000 AND category = 'general'",
            &|f: &Row| updated(f) >= 2_000 && str_field(f, "category") == "general",
        ),
    ] {
        let ks = keys(&mut client, "memory", "updated_at_unix_ms", keep);
        for func in ["SUM", "AVG", "MIN", "MAX"] {
            assert_eq!(
                scalar(
                    &mut client,
                    &format!("SELECT {func}(updated_at_unix_ms) FROM memory {where_}")
                ),
                expect(&ks, func),
                "memory: {func} {where_}"
            );
        }
        // An aggregate over another field beside a pure range: decoded.
        let created: Vec<i64> = {
            let mut all = rows(client.query("SELECT * FROM memory").unwrap());
            all.retain(|(_, f)| keep(f));
            all.iter()
                .map(|(_, f)| i64_field(f, "created_at_unix_ms"))
                .collect()
        };
        assert_eq!(
            scalar(
                &mut client,
                &format!("SELECT MAX(created_at_unix_ms) FROM memory {where_}")
            ),
            expect(&created, "MAX"),
            "memory: MAX(created_at) {where_}"
        );
    }
    assert_eq!(
        scalar(
            &mut client,
            "SELECT MIN(updated_at_unix_ms) FROM memory WHERE updated_at_unix_ms > 2000"
        ),
        ScanValue::I64(3_000)
    );
    // A runtime re-key moves the extreme with the index.
    client
        .replace(
            Uuid::from_u128(1),
            &memory_fields_updated_at(1, "general", 9_000),
        )
        .unwrap();
    assert_eq!(
        scalar(&mut client, "SELECT MAX(updated_at_unix_ms) FROM memory"),
        ScanValue::I64(9_000)
    );

    let mut client = SchemaDrivenClient::connect(start_reminder_server()).unwrap();
    for (where_, keep) in [
        ("", &(|_: &Row| true) as Keep),
        ("WHERE due_at_unix_ms > 1000", &|f: &Row| due(f) > 1_000),
        ("WHERE due_at_unix_ms > 5000", &|f: &Row| due(f) > 5_000),
    ] {
        let ks = keys(&mut client, "reminder", "due_at_unix_ms", keep);
        for func in ["SUM", "AVG", "MIN", "MAX"] {
            assert_eq!(
                scalar(
                    &mut client,
                    &format!("SELECT {func}(due_at_unix_ms) FROM reminder {where_}")
                ),
                expect(&ks, func),
                "reminder: {func} {where_}"
            );
        }
    }
    assert_eq!(
        scalar(
            &mut client,
            "SELECT MIN(due_at_unix_ms) FROM reminder WHERE due_at_unix_ms > 1000"
        ),
        ScanValue::I64(2_000),
        "the next due"
    );
}

// ---------------------------------------------------------------------
// `ADR-0083` / `docs/design/SERVER-QUERY-PLANNER-BOUND-TIGHTENING-DESIGN.md`,
// acceptance criterion 4 (`QBT-FR-004`): several bounds on one side of
// the range field answer exactly — `Query`, `COUNT(*)`, the page — the
// walk between the two tightest, on `Memory` and `Reminder`.
// ---------------------------------------------------------------------

#[test]
fn redundant_and_contradictory_bounds_on_the_range_field_answer_exactly() {
    type Row = [(String, ScanValue)];
    let updated = |f: &Row| i64_field(f, "updated_at_unix_ms");
    let due = |f: &Row| i64_field(f, "due_at_unix_ms");
    let check =
        |client: &mut SchemaDrivenClient, table: &str, order: &str, where_: &str, keep: Keep| {
            let mut oracle = rows(client.query(&format!("SELECT * FROM {table}")).unwrap());
            oracle.retain(|(_, f)| keep(f));
            let mut got = rows(
                client
                    .query(&format!("SELECT * FROM {table} WHERE {where_}"))
                    .unwrap(),
            );
            got.sort_by_key(|(id, _)| *id);
            oracle.sort_by_key(|(id, _)| *id);
            assert_eq!(got, oracle, "{table}: {where_}");
            let counted = groups(
                client
                    .query(&format!("SELECT COUNT(*) FROM {table} WHERE {where_}"))
                    .unwrap(),
            );
            assert_eq!(
                counted[0][0].1,
                ScanValue::I64(oracle.len() as i64),
                "{table}: COUNT(*) WHERE {where_}"
            );
            oracle.sort_by_key(|(id, f)| (i64_field(f, order), *id));
            oracle.truncate(2);
            let paged = rows(
                client
                    .query(&format!(
                        "SELECT * FROM {table} WHERE {where_} ORDER BY {order} LIMIT 2"
                    ))
                    .unwrap(),
            );
            assert_eq!(paged, oracle, "{table}: paged WHERE {where_}");
            got.len()
        };
    let mut client = SchemaDrivenClient::connect(start_memory_server()).unwrap();
    let u = "updated_at_unix_ms";
    assert_eq!(
        check(
            &mut client,
            "memory",
            u,
            "updated_at_unix_ms >= 1000 AND updated_at_unix_ms >= 3000",
            &|f: &Row| updated(f) >= 3_000
        ),
        3
    );
    assert_eq!(
        check(
            &mut client,
            "memory",
            u,
            "updated_at_unix_ms > 3000 AND updated_at_unix_ms >= 3000",
            &|f: &Row| updated(f) > 3_000
        ),
        2
    );
    assert_eq!(
        check(&mut client, "memory", u, "updated_at_unix_ms <= 4000 AND updated_at_unix_ms < 4000 AND updated_at_unix_ms <= 9000", &|f: &Row| updated(f) < 4_000),
        3
    );
    assert_eq!(
        check(&mut client, "memory", u, "updated_at_unix_ms >= 1000 AND updated_at_unix_ms = 3000 AND updated_at_unix_ms <= 5000", &|f: &Row| updated(f) == 3_000),
        1
    );
    // ADR-0091 (protocol 29): the contradiction is `Malformed` (it was
    // "empty, no error" when this test was written under ADR-0083).
    assert_malformed(
        &mut client,
        "SELECT * FROM memory WHERE updated_at_unix_ms = 3000 AND updated_at_unix_ms > 3000",
    );
    assert_eq!(
        check(
            &mut client,
            "memory",
            u,
            "updated_at_unix_ms >= 2000 AND updated_at_unix_ms >= 4000 AND category = 'general'",
            &|f: &Row| updated(f) >= 4_000 && str_field(f, "category") == "general"
        ),
        1,
        "with a second field: still exact through the decode path"
    );
    let mut client = SchemaDrivenClient::connect(start_reminder_server()).unwrap();
    assert_eq!(
        check(
            &mut client,
            "reminder",
            "due_at_unix_ms",
            "due_at_unix_ms > 500 AND due_at_unix_ms > 1500 AND due_at_unix_ms < 9000",
            &|f: &Row| due(f) > 1_500
        ),
        1
    );
}

// ---------------------------------------------------------------------
// `ADR-0084` / `docs/design/SERVER-QUERY-PLANNER-KEYED-GROUPS-DESIGN.md`,
// acceptance criterion 3 (`QKG-FR-004`): `GROUP BY` the range field from
// the walked keys — every column the decode path's, on `Memory` and
// `Reminder` (whose fixture already repeats a due stamp).
// ---------------------------------------------------------------------

#[test]
fn a_group_by_of_the_range_field_from_the_walked_keys_matches_the_decoded_groups() {
    type Row = [(String, ScanValue)];
    let oracle = |client: &mut SchemaDrivenClient, table: &str, field: &str, keep: Keep| {
        let mut all = rows(client.query(&format!("SELECT * FROM {table}")).unwrap());
        all.retain(|(_, f)| keep(f));
        let mut keys: Vec<i64> = all.iter().map(|(_, f)| i64_field(f, field)).collect();
        keys.sort_unstable();
        keys.dedup();
        keys.into_iter()
            .map(|k| {
                let run: Vec<i64> = all
                    .iter()
                    .map(|(_, f)| i64_field(f, field))
                    .filter(|v| *v == k)
                    .collect();
                vec![
                    (field.to_string(), ScanValue::I64(k)),
                    ("COUNT(*)".to_string(), ScanValue::I64(run.len() as i64)),
                    (format!("SUM({field})"), ScanValue::I64(run.iter().sum())),
                    (
                        format!("AVG({field})"),
                        ScanValue::F64(run.iter().sum::<i64>() as f64 / run.len() as f64),
                    ),
                    (format!("MIN({field})"), ScanValue::I64(k)),
                    (format!("MAX({field})"), ScanValue::I64(k)),
                ]
            })
            .collect::<Vec<_>>()
    };
    let check =
        |client: &mut SchemaDrivenClient, table: &str, field: &str, where_: &str, keep: Keep| {
            let expected = oracle(client, table, field, keep);
            let mut got = groups(
                client
                    .query(&format!(
                        "SELECT {field}, COUNT(*), SUM({field}), AVG({field}), MIN({field}), \
                     MAX({field}) FROM {table} {where_} GROUP BY {field}"
                    ))
                    .unwrap(),
            );
            // Column names are the SQL layer's; compare the values by position.
            let values = |g: &mut Vec<Vec<(String, ScanValue)>>| -> Vec<Vec<ScanValue>> {
                g.sort_by_key(|group| match &group[0].1 {
                    ScanValue::I64(k) => *k,
                    other => panic!("{other:?}"),
                });
                g.iter()
                    .map(|group| group.iter().map(|(_, v)| v.clone()).collect())
                    .collect()
            };
            let mut expected = expected;
            assert_eq!(values(&mut got), values(&mut expected), "{table} {where_}");
            got.len()
        };
    let updated = |f: &Row| i64_field(f, "updated_at_unix_ms");
    let due = |f: &Row| i64_field(f, "due_at_unix_ms");

    let mut client = SchemaDrivenClient::connect(start_memory_server()).unwrap();
    // Re-key two memories onto one stamp so a run of equal keys exists.
    for n in [2u128, 4] {
        client
            .replace(
                Uuid::from_u128(n),
                &memory_fields_updated_at(n, "general", 3_000),
            )
            .unwrap();
    }
    for (where_, keep) in [
        ("", &(|_: &Row| true) as Keep),
        ("WHERE updated_at_unix_ms >= 3000", &|f: &Row| {
            updated(f) >= 3_000
        }),
        (
            "WHERE updated_at_unix_ms > 1000 AND updated_at_unix_ms < 5000",
            &|f: &Row| updated(f) > 1_000 && updated(f) < 5_000,
        ),
        ("WHERE updated_at_unix_ms > 9000", &|f: &Row| {
            updated(f) > 9_000
        }),
    ] {
        check(&mut client, "memory", "updated_at_unix_ms", where_, keep);
    }
    assert_eq!(
        check(
            &mut client,
            "memory",
            "updated_at_unix_ms",
            "WHERE updated_at_unix_ms = 3000",
            &|f: &Row| updated(f) == 3_000
        ),
        1,
        "one run of three"
    );
    // Ineligible shapes decode and stay exact: a second field in the
    // filter; the key grouped beside another field.
    check(
        &mut client,
        "memory",
        "updated_at_unix_ms",
        "WHERE updated_at_unix_ms >= 1000 AND category = 'general'",
        &|f: &Row| updated(f) >= 1_000 && str_field(f, "category") == "general",
    );
    let beside = groups(
        client
            .query(
                "SELECT updated_at_unix_ms, category, COUNT(*) FROM memory \
                 WHERE updated_at_unix_ms >= 1000 GROUP BY updated_at_unix_ms, category",
            )
            .unwrap(),
    );
    assert_eq!(
        beside.len(),
        3,
        "every memory is now general: (1000), (3000) ×3 collapsed, (5000)"
    );
    // A `LIMIT` with a bound truncates the ascending groups; with no
    // bound the request decodes and truncates the decode path's own.
    let limited = groups(
        client
            .query(
                "SELECT updated_at_unix_ms, COUNT(*) FROM memory \
                 WHERE updated_at_unix_ms >= 1000 GROUP BY updated_at_unix_ms LIMIT 2",
            )
            .unwrap(),
    );
    assert_eq!(
        limited
            .iter()
            .map(|g| (g[0].1.clone(), g[1].1.clone()))
            .collect::<Vec<_>>(),
        vec![
            (ScanValue::I64(1_000), ScanValue::I64(1)),
            (ScanValue::I64(3_000), ScanValue::I64(3)),
        ]
    );
    assert_eq!(
        groups(
            client
                .query("SELECT updated_at_unix_ms, COUNT(*) FROM memory GROUP BY updated_at_unix_ms LIMIT 2")
                .unwrap()
        )
        .len(),
        2
    );

    let mut client = SchemaDrivenClient::connect(start_reminder_server()).unwrap();
    for (where_, keep) in [
        ("", &(|_: &Row| true) as Keep),
        ("WHERE due_at_unix_ms <= 1000", &|f: &Row| due(f) <= 1_000),
        ("WHERE due_at_unix_ms > 1000", &|f: &Row| due(f) > 1_000),
        ("WHERE due_at_unix_ms > 9000", &|f: &Row| due(f) > 9_000),
    ] {
        check(&mut client, "reminder", "due_at_unix_ms", where_, keep);
    }
}

// ---------------------------------------------------------------------
// `ADR-0085` / `docs/design/SERVER-AGGREGATE-HASHED-BUCKET-DESIGN.md`,
// acceptance criterion 2 (`AGB-FR-002`): the decode path's `GROUP BY`
// — the shapes the walk cannot answer — buckets exactly as before over
// a real socket: a second predicate beside the key, a `Str` key, a
// composite key, `LIMIT` in first-seen order, and the whole-table bucket.
// ---------------------------------------------------------------------

#[test]
fn group_by_through_the_decode_path_buckets_exactly_over_a_socket() {
    type Row = [(String, ScanValue)];
    let due = |f: &Row| i64_field(f, "due_at_unix_ms");
    let status = |f: &Row| match f.iter().find(|(n, _)| n == "status") {
        Some((_, ScanValue::U32(s))) => *s,
        other => panic!("{other:?}"),
    };
    let mut client = SchemaDrivenClient::connect(start_reminder_server()).unwrap();
    let all = rows(client.query("SELECT * FROM reminder").unwrap());

    // The key beside a second predicate: one group per due stamp among
    // the pending, in the order the walk names them (ascending).
    let pending: Vec<&Row> = all
        .iter()
        .map(|(_, f)| f.as_slice())
        .filter(|f| status(f) == 0)
        .collect();
    let mut stamps: Vec<i64> = pending.iter().map(|f| due(f)).collect();
    stamps.sort_unstable();
    stamps.dedup();
    let got = groups(
        client
            .query(
                "SELECT due_at_unix_ms, COUNT(*) FROM reminder \
                 WHERE due_at_unix_ms >= 0 AND status = 0 GROUP BY due_at_unix_ms",
            )
            .unwrap(),
    );
    assert_eq!(
        got.iter()
            .map(|g| (g[0].1.clone(), g[1].1.clone()))
            .collect::<Vec<_>>(),
        stamps
            .iter()
            .map(|k| (
                ScanValue::I64(*k),
                ScanValue::I64(pending.iter().filter(|f| due(f) == *k).count() as i64)
            ))
            .collect::<Vec<_>>()
    );

    // A composite key over the whole table (a scan): every distinct
    // (status, due) pair once, each count the tally.
    let composite = groups(
        client
            .query("SELECT status, due_at_unix_ms, COUNT(*) FROM reminder GROUP BY status, due_at_unix_ms")
            .unwrap(),
    );
    let mut pairs: Vec<(u32, i64)> = all.iter().map(|(_, f)| (status(f), due(f))).collect();
    pairs.sort_unstable();
    pairs.dedup();
    assert_eq!(composite.len(), pairs.len());
    for g in &composite {
        let (s, d) = match (&g[0].1, &g[1].1) {
            (ScanValue::U32(s), ScanValue::I64(d)) => (*s, *d),
            other => panic!("{other:?}"),
        };
        let tally = all
            .iter()
            .filter(|(_, f)| status(f) == s && due(f) == d)
            .count() as i64;
        assert_eq!(g[2].1, ScanValue::I64(tally), "({s}, {d})");
    }

    // `LIMIT` keeps first-seen order: the first group of the unlimited
    // answer is the one group of `LIMIT 1`.
    let unlimited = groups(
        client
            .query("SELECT status, COUNT(*) FROM reminder GROUP BY status")
            .unwrap(),
    );
    let one = groups(
        client
            .query("SELECT status, COUNT(*) FROM reminder GROUP BY status LIMIT 1")
            .unwrap(),
    );
    assert_eq!(one, vec![unlimited[0].clone()]);

    // The whole-table bucket is untouched by the hash: one group, the
    // row count, even over nothing.
    assert_eq!(
        groups(client.query("SELECT COUNT(*) FROM reminder").unwrap())[0][0].1,
        ScanValue::I64(all.len() as i64)
    );
    assert_eq!(
        groups(
            client
                .query("SELECT COUNT(*) FROM reminder WHERE status = 7")
                .unwrap()
        )[0][0]
            .1,
        ScanValue::I64(0)
    );
}

// ---------------------------------------------------------------------
// `ADR-0087` / `docs/design/SERVER-QUERY-PLANNER-RANGE-FOLD-DESIGN.md`,
// acceptance criterion 3 (`QRF-FR-004`): the ungrouped reductions of the
// range field — now one fold, no key materialized — still answer
// exactly over a real socket, on `Memory` and `Reminder`, two-sided,
// one-sided, over everything, and over nothing.
// ---------------------------------------------------------------------

#[test]
fn ungrouped_reductions_from_one_fold_match_the_decoded_answers() {
    type Row = [(String, ScanValue)];
    let row = |client: &mut SchemaDrivenClient, sql: &str| -> Vec<ScanValue> {
        groups(client.query(sql).unwrap())[0]
            .iter()
            .map(|(_, v)| v.clone())
            .collect()
    };
    let expect = |ks: &[i64]| -> Vec<ScanValue> {
        let n = ks.len() as i64;
        let sum: i64 = ks.iter().sum();
        vec![
            ScanValue::I64(n),
            ScanValue::I64(sum),
            if n == 0 {
                ScanValue::F64(0.0)
            } else {
                ScanValue::F64(sum as f64 / n as f64)
            },
            ScanValue::I64(ks.iter().copied().min().unwrap_or(0)),
            ScanValue::I64(ks.iter().copied().max().unwrap_or(0)),
        ]
    };
    let check =
        |client: &mut SchemaDrivenClient, table: &str, field: &str, where_: &str, keep: Keep| {
            let mut all = rows(client.query(&format!("SELECT * FROM {table}")).unwrap());
            all.retain(|(_, f)| keep(f));
            let ks: Vec<i64> = all.iter().map(|(_, f)| i64_field(f, field)).collect();
            assert_eq!(
                row(
                    client,
                    &format!(
                        "SELECT COUNT(*), SUM({field}), AVG({field}), MIN({field}), MAX({field}) \
                     FROM {table} {where_}"
                    )
                ),
                expect(&ks),
                "{table} {where_}"
            );
        };
    let updated = |f: &Row| i64_field(f, "updated_at_unix_ms");
    let due = |f: &Row| i64_field(f, "due_at_unix_ms");

    let mut client = SchemaDrivenClient::connect(start_memory_server()).unwrap();
    for (where_, keep) in [
        ("", &(|_: &Row| true) as Keep),
        ("WHERE updated_at_unix_ms >= 2000", &|f: &Row| {
            updated(f) >= 2_000
        }),
        (
            "WHERE updated_at_unix_ms > 1000 AND updated_at_unix_ms <= 4000",
            &|f: &Row| updated(f) > 1_000 && updated(f) <= 4_000,
        ),
        ("WHERE updated_at_unix_ms > 9000", &|f: &Row| {
            updated(f) > 9_000
        }),
    ] {
        check(&mut client, "memory", "updated_at_unix_ms", where_, keep);
    }
    let mut client = SchemaDrivenClient::connect(start_reminder_server()).unwrap();
    for (where_, keep) in [
        ("", &(|_: &Row| true) as Keep),
        ("WHERE due_at_unix_ms <= 1000", &|f: &Row| due(f) <= 1_000),
        ("WHERE due_at_unix_ms > 9000", &|f: &Row| due(f) > 9_000),
    ] {
        check(&mut client, "reminder", "due_at_unix_ms", where_, keep);
    }
}

// ---------------------------------------------------------------------
// `ADR-0089` / `docs/design/SERVER-PAGE-DESCENDING-DESIGN.md`, acceptance
// criterion 3 (`PGD-FR-006`): `ORDER BY … DESC` over a real socket — the
// exact reverse of the ascending answer, with `LIMIT`, with `WHERE`, on
// the indexed field (`Memory`'s `updated_at_unix_ms`, `Reminder`'s
// `due_at_unix_ms`) and on a scanned one; a cursor walk through
// `page_desc` is disjoint and complete; `ASC` is the default spelled out.
// ---------------------------------------------------------------------

#[test]
fn order_by_desc_is_the_exact_reverse_of_the_ascending_answer() {
    let ids = |rows: &[(Uuid, Vec<(String, ScanValue)>)]| -> Vec<Uuid> {
        rows.iter().map(|(id, _)| *id).collect()
    };
    for (start, table, indexed, scanned, where_) in [
        (
            start_memory_server as fn() -> SocketAddr,
            "memory",
            "updated_at_unix_ms",
            "created_at_unix_ms",
            "WHERE updated_at_unix_ms >= 2000",
        ),
        (
            start_reminder_server as fn() -> SocketAddr,
            "reminder",
            "due_at_unix_ms",
            "status",
            "WHERE due_at_unix_ms <= 2000",
        ),
    ] {
        let mut client = SchemaDrivenClient::connect(start()).unwrap();
        for field in [indexed, scanned] {
            let asc = rows(
                client
                    .query(&format!("SELECT * FROM {table} ORDER BY {field}"))
                    .unwrap(),
            );
            let mut expected = ids(&asc);
            expected.reverse();
            let desc = rows(
                client
                    .query(&format!("SELECT * FROM {table} ORDER BY {field} DESC"))
                    .unwrap(),
            );
            assert_eq!(ids(&desc), expected, "{table} {field}");
            assert_eq!(
                ids(&rows(
                    client
                        .query(&format!("SELECT * FROM {table} ORDER BY {field} ASC"))
                        .unwrap()
                )),
                ids(&asc),
                "ASC is the default"
            );
            let top2 = rows(
                client
                    .query(&format!(
                        "SELECT * FROM {table} ORDER BY {field} DESC LIMIT 2"
                    ))
                    .unwrap(),
            );
            assert_eq!(
                ids(&top2),
                expected[..2].to_vec(),
                "{table} {field} LIMIT 2"
            );
        }
        // With a filter: the reverse of the filtered ascending page.
        let asc = rows(
            client
                .query(&format!(
                    "SELECT * FROM {table} {where_} ORDER BY {indexed}"
                ))
                .unwrap(),
        );
        let mut expected = ids(&asc);
        expected.reverse();
        let desc = rows(
            client
                .query(&format!(
                    "SELECT * FROM {table} {where_} ORDER BY {indexed} DESC"
                ))
                .unwrap(),
        );
        assert_eq!(ids(&desc), expected, "{table} {where_}");
        // A cursor walk of two rows at a time through `page_desc`:
        // disjoint pages, greatest first, the whole table.
        let mut walked = Vec::new();
        let mut before = None;
        loop {
            let page = client.page_desc(indexed, before.take(), 2).unwrap();
            if page.is_empty() {
                break;
            }
            let (last_id, last_fields) = page.last().unwrap();
            before = Some((
                last_fields
                    .iter()
                    .find(|(n, _)| n == indexed)
                    .map(|(_, v)| v.clone())
                    .unwrap(),
                *last_id,
            ));
            walked.extend(ids(&page));
            if page.len() < 2 {
                break;
            }
        }
        let mut all = ids(&rows(
            client
                .query(&format!("SELECT * FROM {table} ORDER BY {indexed}"))
                .unwrap(),
        ));
        all.reverse();
        assert_eq!(
            walked, all,
            "{table}: page_desc walks everything, greatest first"
        );
    }
}

// ---------------------------------------------------------------------
// `ADR-0090` / `docs/design/SERVER-SQL-ID-PREDICATE-DESIGN.md`,
// acceptance criterion 2 (`SID-FR-003`): `WHERE id = '<uuid>'` over a
// real socket — one point read, exactly the scan's row for that id; the
// other predicates re-checked; the `SELECT` list applied; an unknown id
// empty; `LIMIT 0` empty; `!=`, a non-UUID, and two ids refused
// client-side with no frame.
// ---------------------------------------------------------------------

#[test]
fn where_id_equals_is_a_point_read_that_matches_the_scan() {
    let mut client = SchemaDrivenClient::connect(start_memory_server()).unwrap();
    let all = rows(client.query("SELECT * FROM memory").unwrap());
    let (target, fields) = all
        .iter()
        .find(|(id, _)| *id == Uuid::from_u128(3))
        .unwrap();
    let category = str_field(fields, "category");
    let got = rows(
        client
            .query(&format!("SELECT * FROM memory WHERE id = '{target}'"))
            .unwrap(),
    );
    assert_eq!(got, vec![(*target, fields.clone())]);
    // The other predicates are re-checked: one that holds, one that does not.
    assert_eq!(
        rows(
            client
                .query(&format!(
                    "SELECT * FROM memory WHERE id = '{target}' AND category = '{category}'"
                ))
                .unwrap()
        )
        .len(),
        1
    );
    assert!(rows(
        client
            .query(&format!(
                "SELECT * FROM memory WHERE category = 'no-such' AND id = '{target}'"
            ))
            .unwrap()
    )
    .is_empty());
    // The SELECT list applies.
    let projected = rows(
        client
            .query(&format!(
                "SELECT content, category FROM memory WHERE id = '{target}'"
            ))
            .unwrap(),
    );
    assert_eq!(
        projected[0]
            .1
            .iter()
            .map(|(n, _)| n.as_str())
            .collect::<Vec<_>>(),
        vec!["content", "category"]
    );
    // Unknown id, LIMIT 0.
    assert!(rows(
        client
            .query(&format!(
                "SELECT * FROM memory WHERE id = '{}'",
                Uuid::from_u128(999)
            ))
            .unwrap()
    )
    .is_empty());
    assert!(rows(
        client
            .query(&format!(
                "SELECT * FROM memory WHERE id = '{target}' LIMIT 0"
            ))
            .unwrap()
    )
    .is_empty());
    // Refused client-side, no frame: the wrong comparator, a non-UUID, two ids.
    for sql in [
        format!("SELECT * FROM memory WHERE id != '{target}'"),
        "SELECT * FROM memory WHERE id = 'not-a-uuid'".to_string(),
        format!(
            "SELECT * FROM memory WHERE id = '{target}' AND id = '{}'",
            Uuid::from_u128(1)
        ),
    ] {
        assert!(
            matches!(client.query(&sql), Err(ClientError::Sql(_))),
            "{sql}"
        );
    }
}

// ---------------------------------------------------------------------
// `ADR-0091` / `docs/design/SERVER-QUERY-CONTRADICTION-DESIGN.md`,
// acceptance criterion 2 (`QCX-FR-003`): over a real socket at protocol
// 29 a contradictory `WHERE` is `Malformed` before any read — `Query`,
// `COUNT(*)`, the ordered page, on the range field and on a `Str`
// field — a satisfiable one still answers; and a connection negotiated
// at 28 gets the empty answer every earlier version gave.
// ---------------------------------------------------------------------

#[test]
fn a_contradictory_filter_is_malformed_at_29_and_empty_below() {
    use rusty_multimodal_db::server::framing::{read_message, write_message};
    use rusty_multimodal_db::server::protocol::{
        CompareOp, ErrorCode, Predicate, Request, Response, Selection,
    };
    use std::io::{BufReader, BufWriter, Write};
    use std::net::TcpStream;

    let addr = start_memory_server();
    let mut client = SchemaDrivenClient::connect(addr).unwrap();
    for sql in [
        "SELECT * FROM memory WHERE updated_at_unix_ms > 5000 AND updated_at_unix_ms < 3000",
        "SELECT * FROM memory WHERE updated_at_unix_ms = 3000 AND updated_at_unix_ms > 3000",
        "SELECT COUNT(*) FROM memory WHERE updated_at_unix_ms >= 5000 AND updated_at_unix_ms < 5000",
        "SELECT * FROM memory WHERE updated_at_unix_ms > 5000 AND updated_at_unix_ms < 3000 \
         ORDER BY updated_at_unix_ms",
        "SELECT * FROM memory WHERE category = 'general' AND category = 'decision'",
        "SELECT * FROM memory WHERE category = 'general' AND category != 'general'",
        "SELECT * FROM memory WHERE created_at_unix_ms > 5000 AND created_at_unix_ms < 3000",
    ] {
        assert!(
            matches!(
                client.query(sql),
                Err(ClientError::Server(ErrorCode::Malformed, _))
            ),
            "{sql}"
        );
    }
    // Satisfiable pairs still answer.
    assert_eq!(
        rows(client
            .query("SELECT * FROM memory WHERE updated_at_unix_ms >= 3000 AND updated_at_unix_ms <= 3000")
            .unwrap())
        .len(),
        1
    );
    assert!(client
        .query("SELECT * FROM memory WHERE updated_at_unix_ms > 3000 AND updated_at_unix_ms < 5000")
        .is_ok());

    // At 28 the same request is the empty answer, no error.
    let stream = TcpStream::connect(addr).unwrap();
    let mut reader = BufReader::new(stream.try_clone().unwrap());
    let mut writer = BufWriter::new(stream);
    let send =
        |writer: &mut BufWriter<TcpStream>, reader: &mut BufReader<TcpStream>, req: &Request| {
            write_message(writer, req).unwrap();
            writer.flush().unwrap();
            read_message::<_, Response>(reader).unwrap()
        };
    assert!(matches!(
        send(
            &mut writer,
            &mut reader,
            &Request::Hello {
                protocol_version: 28
            }
        ),
        Response::Hello {
            protocol_version: 28
        }
    ));
    let updated_at = client
        .schema()
        .fields
        .iter()
        .find(|f| f.name == "updated_at_unix_ms")
        .unwrap()
        .tag;
    let contradiction = vec![
        Predicate {
            field: updated_at,
            op: CompareOp::Gt,
            value: ScanValue::I64(5_000),
        },
        Predicate {
            field: updated_at,
            op: CompareOp::Lt,
            value: ScanValue::I64(3_000),
        },
    ];
    match send(
        &mut writer,
        &mut reader,
        &Request::Query {
            select: Selection::All,
            filter: contradiction,
            limit: None,
        },
    ) {
        Response::Rows { rows } => assert!(rows.is_empty(), "below 29: the empty answer"),
        other => panic!("below 29 expected empty Rows, got {other:?}"),
    }
}

/// `QCX-FR-003` (ADR-0091): `sql` is refused with `Malformed` at
/// protocol 29 before any read — the assertion the four pre-existing
/// contradictory shapes below moved to, named in the ADR.
fn assert_malformed(client: &mut SchemaDrivenClient, sql: &str) {
    assert!(
        matches!(
            client.query(sql),
            Err(ClientError::Server(
                rusty_multimodal_db::server::protocol::ErrorCode::Malformed,
                _
            ))
        ),
        "{sql}: a contradiction is Malformed since ADR-0091"
    );
}
