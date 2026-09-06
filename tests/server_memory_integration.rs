//! Real end-to-end coverage of the `Memory` domain (`MEM-FR-001`–`008`,
//! ADR-0048, `docs/design/SERVER-MEMORY-DOMAIN-DESIGN.md`) — a real
//! `TcpListener`, a real `SchemaDrivenClient`, and no wire change at all:
//! the consumer's `add`/`get`/`list` shapes over the requests that
//! already exist. `required-features = ["server"]` only, no `research`.

use rusty_multimodal_db::generic::memory::{
    create_memory_production_stack, open_memory_production_stack_portable, Memory,
};
use rusty_multimodal_db::generic::production::GenericProductionStore;
use rusty_multimodal_db::server::client::{ClientError, QueryResult, SchemaDrivenClient};
use rusty_multimodal_db::server::memory::MemoryConnectionStore;
use rusty_multimodal_db::server::protocol::{ErrorCode, ScanValue};
use rusty_multimodal_db::server::{serve, ServeOptions};
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

fn memory(n: u128, content: &str, category: &str, sensitive: bool) -> Memory {
    Memory {
        id: Uuid::from_u128(n),
        content: content.into(),
        category: category.into(),
        tags: vec!["sample".into(), format!("n{n}")],
        source: if n % 2 == 0 {
            "import".into()
        } else {
            "manual".into()
        },
        metadata_json: "{}".into(),
        created_at_unix_ms: 1_000 * n as i64,
        updated_at_unix_ms: 1_000 * n as i64,
        memory_type: "unclassified".into(),
        status: "active".into(),
        sensitive,
        access_count: 0,
    }
}

fn sample_memories() -> Vec<Memory> {
    vec![
        memory(1, "Prefers merge commits", "preference", false),
        memory(2, "Records are insertable since ADR-0046", "fact", false),
        memory(3, "A private note", "general", true),
        memory(4, "Another general note", "general", false),
    ]
}

fn start_server() -> SocketAddr {
    start_server_at(unique_dir("memory_integration"))
}

fn start_server_at(dir: std::path::PathBuf) -> SocketAddr {
    std::fs::create_dir_all(&dir).unwrap();
    let path = dir.join("memories.mmap");
    let stack = if path.exists() {
        open_memory_production_stack_portable(&path).unwrap()
    } else {
        create_memory_production_stack(sample_memories(), &path).unwrap()
    };
    let connection_store = Arc::new(MemoryConnectionStore::new(GenericProductionStore::new(
        stack,
    )));
    let listener = TcpListener::bind("127.0.0.1:0").unwrap();
    let addr = listener.local_addr().unwrap();
    thread::spawn(move || serve(listener, connection_store, ServeOptions::default()));
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

/// `MEM` acceptance criterion 2: `GetById` returns all eleven fields by
/// name with `tags` as a list; the consumer's `list` filters — category,
/// source, the sensitive gate — are `FilterEq` and `Query`; `GROUP BY
/// category` counts match.
#[test]
fn get_list_filters_and_group_by_category_match_the_consumers_shapes() {
    let addr = start_server();
    let mut client = SchemaDrivenClient::connect(addr).unwrap();
    let names: Vec<&str> = client
        .schema()
        .fields
        .iter()
        .map(|f| f.name.as_str())
        .collect();
    assert_eq!(
        names,
        vec![
            "content",
            "category",
            "tags",
            "source",
            "metadata_json",
            "created_at_unix_ms",
            "updated_at_unix_ms",
            "memory_type",
            "status",
            "sensitive",
            "access_count"
        ]
    );
    let fields = client.get(Uuid::from_u128(3)).unwrap().unwrap();
    assert_eq!(
        fields[0],
        (
            "content".to_string(),
            ScanValue::Str("A private note".into())
        )
    );
    assert_eq!(
        fields[2],
        (
            "tags".to_string(),
            ScanValue::StrList(vec!["sample".into(), "n3".into()])
        )
    );
    assert_eq!(fields[9], ("sensitive".to_string(), ScanValue::Bool(true)));

    // `list(category = general)` — `FilterEq` on the indexed field.
    let mut general = client
        .filter_eq("category", ScanValue::Str("general".into()))
        .unwrap();
    general.sort();
    assert_eq!(general, vec![Uuid::from_u128(3), Uuid::from_u128(4)]);
    // The default list hides sensitive memories — a `WHERE` on a Bool.
    let visible = rows(
        client
            .query("SELECT content FROM memory WHERE sensitive = false")
            .unwrap(),
    );
    assert_eq!(visible.len(), 3);
    // `list(source = import)` — a read-only field is still queryable.
    let imported = rows(
        client
            .query("SELECT content FROM memory WHERE source = 'import'")
            .unwrap(),
    );
    assert_eq!(imported.len(), 2);
    // `GROUP BY category`.
    let counts = groups(
        client
            .query("SELECT category, COUNT(*) FROM memory GROUP BY category")
            .unwrap(),
    );
    let general_count = counts
        .iter()
        .find(|g| g[0] == ("category".to_string(), ScanValue::Str("general".into())))
        .unwrap();
    assert_eq!(
        general_count[1],
        ("COUNT(*)".to_string(), ScanValue::I64(2))
    );
}

/// `MEM` acceptance criterion 3: the consumer's `add_memory` is one
/// `insert` with every field; the retrieval counter is the one
/// `update`; content is read-only; and a server restarted on the same
/// directory serves the inserted memory with its tags and counter.
#[test]
fn insert_a_memory_bump_its_access_count_and_a_restart_serves_it() {
    let dir = unique_dir("memory_insert");
    let addr = start_server_at(dir.clone());
    let mut client = SchemaDrivenClient::connect(addr).unwrap();
    let id = Uuid::from_u128(77);
    client
        .insert(
            id,
            &[
                ("content", ScanValue::Str("The Memory domain exists".into())),
                ("category", ScanValue::Str("fact".into())),
                (
                    "tags",
                    ScanValue::StrList(vec!["adr-0048".into(), "milestone".into()]),
                ),
                ("source", ScanValue::Str("claude-code".into())),
                ("metadata_json", ScanValue::Str(r#"{"pr":200}"#.into())),
                ("created_at_unix_ms", ScanValue::I64(77_000)),
                ("updated_at_unix_ms", ScanValue::I64(77_000)),
                ("memory_type", ScanValue::Str("decision".into())),
                ("status", ScanValue::Str("active".into())),
                ("sensitive", ScanValue::Bool(false)),
                ("access_count", ScanValue::I64(0)),
            ],
        )
        .unwrap();
    assert!(client
        .update(id, "access_count", ScanValue::I64(1))
        .unwrap());
    match client.update(id, "access_count", ScanValue::I64(-1)) {
        Err(ClientError::Server(ErrorCode::Malformed, _)) => {}
        other => panic!("expected Malformed, got {other:?}"),
    }
    assert!(matches!(
        client.update(id, "content", ScanValue::Str("x".into())),
        Err(ClientError::Unsupported(_))
    ));
    let facts = rows(
        client
            .query("SELECT content, access_count FROM memory WHERE category = 'fact'")
            .unwrap(),
    );
    assert_eq!(facts.len(), 2);
    drop(client);

    let addr = start_server_at(dir);
    let mut client = SchemaDrivenClient::connect(addr).unwrap();
    let got = client.get(id).unwrap().unwrap();
    assert_eq!(
        got[0],
        (
            "content".to_string(),
            ScanValue::Str("The Memory domain exists".into())
        )
    );
    assert_eq!(
        got[2],
        (
            "tags".to_string(),
            ScanValue::StrList(vec!["adr-0048".into(), "milestone".into()])
        )
    );
    assert_eq!(
        got[4],
        (
            "metadata_json".to_string(),
            ScanValue::Str(r#"{"pr":200}"#.into())
        )
    );
    assert_eq!(got[10], ("access_count".to_string(), ScanValue::I64(1)));
}

/// `MEM-FR-005`: no relation of either kind — every relation request and
/// `link` are refused client-side, and the schema says so.
#[test]
fn every_relation_request_is_unsupported() {
    let addr = start_server();
    let mut client = SchemaDrivenClient::connect(addr).unwrap();
    assert!(matches!(
        client.neighbors(Uuid::from_u128(1)),
        Err(ClientError::Unsupported(_))
    ));
    assert!(matches!(
        client.parent(Uuid::from_u128(1)),
        Err(ClientError::Unsupported(_))
    ));
    assert!(matches!(
        client.link(Uuid::from_u128(1), Uuid::from_u128(2), "x"),
        Err(ClientError::Unsupported(_))
    ));
    assert!(client.relations().is_empty());
}

/// `REP` acceptance criterion 2 (ADR-0049) on `Memory` over a socket —
/// the consumer's `update_memory`: `replace` with every field rewrites
/// `content`, `category`, `tags`, `metadata_json`, `sensitive`, and the
/// counter at once; every read sees the new version (`get`, `filter_eq`
/// on the new and the old category, `WHERE sensitive`); an unknown id
/// is `Ok(false)` with nothing created; a malformed list and a negative
/// counter are `Malformed` with nothing written; `upsert` replaces an
/// existing record and creates a missing one (`SessionOpen` inside a
/// session is `tests/server_transaction_integration.rs`'s); and a server restarted on the same directory serves
/// the replaced version.
#[test]
fn replace_a_memory_over_the_wire_every_read_sees_it_and_a_restart_serves_it() {
    let dir = unique_dir("memory_replace");
    let addr = start_server_at(dir.clone());
    let mut client = SchemaDrivenClient::connect(addr).unwrap();
    let id = Uuid::from_u128(1);
    let before = client.get(id).unwrap().unwrap();
    assert_eq!(
        before[1],
        ("category".to_string(), ScanValue::Str("preference".into()))
    );
    let edited: Vec<(&str, ScanValue)> = vec![
        ("content", ScanValue::Str("memory 1, revised".into())),
        ("category", ScanValue::Str("decision".into())),
        ("tags", ScanValue::StrList(vec!["revised".into()])),
        ("source", ScanValue::Str("manual".into())),
        ("metadata_json", ScanValue::Str(r#"{"edited":true}"#.into())),
        ("created_at_unix_ms", ScanValue::I64(1_000)),
        ("updated_at_unix_ms", ScanValue::I64(2_000)),
        ("memory_type", ScanValue::Str("decision".into())),
        ("status", ScanValue::Str("active".into())),
        ("sensitive", ScanValue::Bool(true)),
        ("access_count", ScanValue::I64(4)),
    ];
    assert!(client.replace(id, &edited).unwrap());
    let got = client.get(id).unwrap().unwrap();
    assert_eq!(
        got[0],
        (
            "content".to_string(),
            ScanValue::Str("memory 1, revised".into())
        )
    );
    assert_eq!(got[9], ("sensitive".to_string(), ScanValue::Bool(true)));
    assert_eq!(got[10], ("access_count".to_string(), ScanValue::I64(4)));
    assert_eq!(
        client
            .filter_eq("category", ScanValue::Str("decision".into()))
            .unwrap(),
        vec![id]
    );
    assert!(!client
        .filter_eq("category", ScanValue::Str("preference".into()))
        .unwrap()
        .contains(&id));
    let sensitive = rows(
        client
            .query("SELECT content FROM memory WHERE sensitive = true")
            .unwrap(),
    );
    assert_eq!(sensitive.len(), 2, "memory 1 joined memory 3");
    assert_eq!(
        rows(client.query("SELECT content FROM memory").unwrap()).len(),
        4,
        "no new record"
    );

    // An unknown id: `Ok(false)`, nothing created.
    assert!(!client.replace(Uuid::from_u128(99), &edited).unwrap());
    assert!(client.get(Uuid::from_u128(99)).unwrap().is_none());
    // Refusals write nothing.
    let mut negative = edited.clone();
    negative[10] = ("access_count", ScanValue::I64(-1));
    match client.replace(id, &negative) {
        Err(ClientError::Server(ErrorCode::Malformed, _)) => {}
        other => panic!("expected Malformed, got {other:?}"),
    }
    match client.replace(id, &edited[..10]) {
        Err(ClientError::Server(ErrorCode::Malformed, _)) => {}
        other => panic!("expected Malformed, got {other:?}"),
    }
    assert!(matches!(
        client.replace(id, &[("no_such_field", ScanValue::I64(0))]),
        Err(ClientError::UnknownField(_))
    ));
    assert_eq!(
        client.get(id).unwrap().unwrap(),
        got,
        "nothing written on refusal"
    );

    // `upsert`: an existing id is replaced (`false`), a new one created (`true`).
    let mut again = edited.clone();
    again[10] = ("access_count", ScanValue::I64(5));
    assert!(!client.upsert(id, &again).unwrap());
    assert_eq!(
        client.get(id).unwrap().unwrap()[10],
        ("access_count".to_string(), ScanValue::I64(5))
    );
    assert!(client.upsert(Uuid::from_u128(99), &edited).unwrap());
    assert!(client.get(Uuid::from_u128(99)).unwrap().is_some());

    drop(client);

    let addr = start_server_at(dir);
    let mut client = SchemaDrivenClient::connect(addr).unwrap();
    let got = client.get(id).unwrap().unwrap();
    assert_eq!(
        got[0],
        (
            "content".to_string(),
            ScanValue::Str("memory 1, revised".into())
        )
    );
    assert_eq!(
        got[2],
        (
            "tags".to_string(),
            ScanValue::StrList(vec!["revised".into()])
        )
    );
    assert_eq!(got[10], ("access_count".to_string(), ScanValue::I64(5)));
    let mut decisions = client
        .filter_eq("category", ScanValue::Str("decision".into()))
        .unwrap();
    decisions.sort();
    assert_eq!(decisions, vec![id, Uuid::from_u128(99)], "both survived");
    assert!(client.get(Uuid::from_u128(99)).unwrap().is_some());
}
