//! Real end-to-end coverage of the `Memory` domain (`MEM-FR-001`–`008`,
//! ADR-0048, `docs/design/SERVER-MEMORY-DOMAIN-DESIGN.md`) — a real
//! `TcpListener`, a real `SchemaDrivenClient`, and no wire change at all:
//! the consumer's `add`/`get`/`list` shapes over the requests that
//! already exist. `required-features = ["server"]` only, no `research`.

use rusty_multimodal_db::generic::entity::{
    create_entity_production_stack, open_entity_production_stack_portable, Entity,
};
use rusty_multimodal_db::generic::memory::{
    create_memory_production_stack, open_memory_production_stack_portable, Memory,
};
use rusty_multimodal_db::generic::production::GenericProductionStore;
use rusty_multimodal_db::server::client::{ClientError, QueryResult, SchemaDrivenClient};
use rusty_multimodal_db::server::entity::EntityConnectionStore;
use rusty_multimodal_db::server::memory::MemoryConnectionStore;
use rusty_multimodal_db::server::protocol::{ErrorCode, ScanValue};
use rusty_multimodal_db::server::{serve, serve_tables, ConnectionStore, ServeOptions};
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
        create_memory_production_stack(sample_memories(), &[], &path).unwrap()
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

/// `TBL-FR-008` (ADR-0050) on a one-table `Memory` server: the one
/// relation, `mentions`, is listed with its rows in `entity`; a same-
/// table `JOIN memory b ON mentions` is refused client-side; `parent`
/// is `Unsupported`; a one-table server lists its one table and `Use`
/// of any other name is `Malformed`.
#[test]
fn a_one_table_memory_server_lists_mentions_as_foreign_and_itself_as_the_only_table() {
    let addr = start_server();
    let mut client = SchemaDrivenClient::connect(addr).unwrap();
    assert_eq!(client.relations().len(), 1);
    assert_eq!(client.relations()[0].name, "mentions");
    assert_eq!(
        client.relations()[0].target_table.as_deref(),
        Some("entity")
    );
    assert!(matches!(
        client.parent(Uuid::from_u128(1)),
        Err(ClientError::Unsupported(_))
    ));
    assert_eq!(
        client.neighbors(Uuid::from_u128(1)).unwrap(),
        Vec::<Uuid>::new()
    );
    assert!(matches!(
        client.query("SELECT a.content, b.content FROM memory a JOIN memory b ON mentions"),
        Err(ClientError::Sql(_))
    ));
    assert_eq!(
        client.list_tables().unwrap(),
        (vec!["memory".to_string()], "memory".to_string())
    );
    assert!(client.use_table("memory").is_ok());
    match client.use_table("entity") {
        Err(ClientError::Server(ErrorCode::Malformed, _)) => {}
        other => panic!("expected Malformed, got {other:?}"),
    }
    // A link to an entity on a server with no entity table: the server
    // cannot check the far end, so it is `Unsupported` (`TBL-FR-007`).
    match client.link(Uuid::from_u128(1), Uuid::from_u128(0xada), "mentions") {
        Err(ClientError::Server(ErrorCode::Unsupported, _)) => {}
        other => panic!("expected Unsupported, got {other:?}"),
    }
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

fn sample_entities() -> Vec<Entity> {
    let entity = |n: u128, label: &str, kind: &str| Entity {
        id: Uuid::from_u128(n),
        label: label.into(),
        kind: kind.into(),
        mention_count: 0,
        aliases: vec![],
    };
    vec![
        entity(0xada, "Ada Lovelace", "person"),
        entity(0xe1e, "Analytical Engine", "artifact"),
        entity(0x10d, "London", "place"),
    ]
}

/// `TBL-FR-001` (ADR-0050): one listener, two tables — `memory` (primary)
/// with its `mentions` edges seeded, and `entity`. Reopened from the
/// files alone when the directory already holds a store.
fn start_two_table_server_at(dir: std::path::PathBuf) -> SocketAddr {
    std::fs::create_dir_all(&dir).unwrap();
    let memories = dir.join("memories.mmap");
    let entities = dir.join("entities.mmap");
    let memory_stack = if memories.exists() {
        open_memory_production_stack_portable(&memories).unwrap()
    } else {
        create_memory_production_stack(
            sample_memories(),
            &[
                (Uuid::from_u128(1), Uuid::from_u128(0xada)),
                (Uuid::from_u128(2), Uuid::from_u128(0xe1e)),
                (Uuid::from_u128(2), Uuid::from_u128(0xada)),
            ],
            &memories,
        )
        .unwrap()
    };
    let entity_stack = if entities.exists() {
        open_entity_production_stack_portable(&entities).unwrap()
    } else {
        create_entity_production_stack(sample_entities(), &[], &[], &entities).unwrap()
    };
    let memory: Arc<dyn ConnectionStore> = Arc::new(MemoryConnectionStore::new(
        GenericProductionStore::new(memory_stack),
    ));
    let entity: Arc<dyn ConnectionStore> = Arc::new(EntityConnectionStore::new(
        GenericProductionStore::new(entity_stack),
    ));
    let listener = TcpListener::bind("127.0.0.1:0").unwrap();
    let addr = listener.local_addr().unwrap();
    thread::spawn(move || {
        serve_tables(
            listener,
            vec![
                ("memory".to_string(), memory),
                ("entity".to_string(), entity),
            ],
            0,
            ServeOptions::default(),
        )
    });
    addr
}

/// `TBL` acceptance criteria 1–3 (ADR-0050) over a socket — the
/// consumer's `memory_entities` path end to end: `ListTables`; the
/// cross-table `SELECT m.content, e.label FROM memory m JOIN entity e ON
/// mentions` in one round trip, with a right-side `WHERE` on the entity
/// table's own field; `mentions` read from both ends; `use_table`
/// switching every table-less request (the schema, `get`, a `Query`)
/// and back; a runtime `link` to an entity checked against the entity
/// table (`RecordNotFound` for an unknown one, `Malformed` for a label
/// the domain lacks); an unknown table `Malformed`; and a second server
/// on the same directory serving the edge and the join.
#[test]
fn two_tables_on_one_connection_join_across_use_and_link_and_survive_a_restart() {
    let dir = unique_dir("memory_two_tables");
    let addr = start_two_table_server_at(dir.clone());
    let mut client = SchemaDrivenClient::connect(addr).unwrap();
    let (m1, m2, m3) = (Uuid::from_u128(1), Uuid::from_u128(2), Uuid::from_u128(3));
    let (ada, engine, london) = (
        Uuid::from_u128(0xada),
        Uuid::from_u128(0xe1e),
        Uuid::from_u128(0x10d),
    );
    assert_eq!(
        client.list_tables().unwrap(),
        (
            vec!["memory".to_string(), "entity".to_string()],
            "memory".to_string()
        )
    );
    assert_eq!(client.table(), None, "on the primary until a Use");

    // Criterion 3: the cross-table join, one round trip, both sides named.
    let joined = match client
        .query("SELECT m.content, e.label FROM memory m JOIN entity e ON mentions")
        .unwrap()
    {
        QueryResult::Joined(rows) => rows,
        other => panic!("expected Joined, got {other:?}"),
    };
    assert_eq!(joined.len(), 3);
    let mut pairs: Vec<(Uuid, Uuid)> = joined.iter().map(|r| (r.left_id, r.right_id)).collect();
    pairs.sort();
    assert_eq!(pairs, vec![(m1, ada), (m2, ada), (m2, engine)]);
    let ada_row = joined.iter().find(|r| r.right_id == ada).unwrap();
    assert!(ada_row
        .fields
        .iter()
        .any(|(name, v)| name == "e.label" && *v == ScanValue::Str("Ada Lovelace".into())));
    assert!(ada_row.fields.iter().any(|(name, _)| name == "m.content"));
    // A right-side WHERE resolves against the entity table's schema.
    let people = match client
        .query("SELECT m.content, e.label FROM memory m JOIN entity e ON mentions WHERE e.kind = 'artifact'")
        .unwrap()
    {
        QueryResult::Joined(rows) => rows,
        other => panic!("expected Joined, got {other:?}"),
    };
    assert_eq!(people.len(), 1);
    assert_eq!(people[0].right_id, engine);
    // Wrong table for the relation, or the FROM table itself: refused client-side.
    assert!(matches!(
        client.query("SELECT m.content, x.content FROM memory m JOIN memory x ON mentions"),
        Err(ClientError::Sql(_))
    ));

    // Both ends of `mentions`: the entities a memory mentions, and the
    // memories that mention an entity (the consumer's two lookups).
    assert_eq!(
        client.neighbors_by_relation(m1, "mentions").unwrap(),
        vec![ada]
    );
    let mut about_ada = client.neighbors_by_relation(ada, "mentions").unwrap();
    about_ada.sort();
    assert_eq!(about_ada, vec![m1, m2]);

    // Criterion 2: `Use` switches every table-less request, and back.
    client.use_table("entity").unwrap();
    assert_eq!(client.table(), Some("entity"));
    let names: Vec<&str> = client
        .schema()
        .fields
        .iter()
        .map(|f| f.name.as_str())
        .collect();
    assert_eq!(names, vec!["label", "kind", "mention_count", "aliases"]);
    assert_eq!(
        client.get(london).unwrap().unwrap()[0],
        ("label".to_string(), ScanValue::Str("London".into()))
    );
    assert!(
        client.get(m1).unwrap().is_none(),
        "a memory id is not an entity"
    );
    let places = rows(
        client
            .query("SELECT label FROM entity WHERE kind = 'place'")
            .unwrap(),
    );
    assert_eq!(places.len(), 1);
    client.use_table("memory").unwrap();
    assert_eq!(client.table(), Some("memory"));
    assert_eq!(client.schema().fields.len(), 11);
    match client.use_table("customer") {
        Err(ClientError::Server(ErrorCode::Malformed, _)) => {}
        other => panic!("expected Malformed, got {other:?}"),
    }
    assert_eq!(client.table(), Some("memory"), "unchanged on refusal");

    // `TBL-FR-007`: a runtime link, its far end checked in the entity table.
    client.link(m3, london, "mentions").unwrap();
    client.link(m3, london, "mentions").unwrap();
    match client.link(m3, Uuid::from_u128(0xbad), "mentions") {
        Err(ClientError::Server(ErrorCode::RecordNotFound, _)) => {}
        other => panic!("expected RecordNotFound, got {other:?}"),
    }
    match client.link(m3, london, "relates_to") {
        Err(ClientError::Server(ErrorCode::Malformed, _)) => {}
        other => panic!("expected Malformed, got {other:?}"),
    }
    match client.link(Uuid::from_u128(0xbad), london, "mentions") {
        Err(ClientError::Server(ErrorCode::RecordNotFound, _)) => {}
        other => panic!("expected RecordNotFound, got {other:?}"),
    }
    assert_eq!(
        client.neighbors_by_relation(london, "mentions").unwrap(),
        vec![m3]
    );
    drop(client);

    // A second server on the same directory serves the edge and the join.
    let addr = start_two_table_server_at(dir);
    let mut client = SchemaDrivenClient::connect(addr).unwrap();
    let joined = match client
        .query("SELECT m.content, e.label FROM memory m JOIN entity e ON mentions WHERE e.kind = 'place'")
        .unwrap()
    {
        QueryResult::Joined(rows) => rows,
        other => panic!("expected Joined, got {other:?}"),
    };
    assert_eq!(joined.len(), 1);
    assert_eq!((joined[0].left_id, joined[0].right_id), (m3, london));
    client.use_table("entity").unwrap();
    assert!(client.get(london).unwrap().is_some());
}

/// `DEL` acceptance criteria 2–3 (ADR-0051) on the two-table server —
/// the consumer's `delete_memory` and entity delete: deleting a memory
/// removes it from every read and its `mentions` edge from the entity's
/// side; a repeat is `Ok(false)`; deleting an **entity** (`Use entity`)
/// detaches every memory's `mentions` edge to it on the memory table
/// (`DEL-FR-007`), the memories themselves intact, and the cross-table
/// join shrinks accordingly; the deleted ids can be inserted again; and
/// a second server on the same directory serves the deletions.
#[test]
fn delete_a_memory_and_an_entity_across_tables_and_a_restart_serves_it() {
    let dir = unique_dir("memory_delete");
    let addr = start_two_table_server_at(dir.clone());
    let mut client = SchemaDrivenClient::connect(addr).unwrap();
    let (m1, m2) = (Uuid::from_u128(1), Uuid::from_u128(2));
    let (ada, engine) = (Uuid::from_u128(0xada), Uuid::from_u128(0xe1e));
    let joined = |client: &mut SchemaDrivenClient| match client
        .query("SELECT m.content, e.label FROM memory m JOIN entity e ON mentions")
        .unwrap()
    {
        QueryResult::Joined(rows) => {
            let mut pairs: Vec<(Uuid, Uuid)> =
                rows.iter().map(|r| (r.left_id, r.right_id)).collect();
            pairs.sort();
            pairs
        }
        other => panic!("expected Joined, got {other:?}"),
    };
    assert_eq!(
        joined(&mut client),
        vec![(m1, ada), (m2, ada), (m2, engine)]
    );

    // Criterion 2: delete a memory.
    assert!(client.delete(m1).unwrap());
    assert!(client.get(m1).unwrap().is_none());
    assert!(!client.delete(m1).unwrap(), "a repeat is NotFound");
    assert_eq!(
        rows(client.query("SELECT content FROM memory").unwrap()).len(),
        3
    );
    assert_eq!(
        client.neighbors_by_relation(ada, "mentions").unwrap(),
        vec![m2]
    );
    assert_eq!(joined(&mut client), vec![(m2, ada), (m2, engine)]);

    // Criterion 3: delete an entity from the entity table — the memory
    // table's edges to it go (`DEL-FR-007`), the memory stays.
    client.use_table("entity").unwrap();
    assert!(client.delete(ada).unwrap());
    assert!(client.get(ada).unwrap().is_none());
    assert!(!client.delete(ada).unwrap());
    client.use_table("memory").unwrap();
    assert!(client.get(m2).unwrap().is_some(), "the memory is intact");
    assert_eq!(
        client.neighbors_by_relation(m2, "mentions").unwrap(),
        vec![engine]
    );
    assert_eq!(
        client.neighbors_by_relation(ada, "mentions").unwrap(),
        Vec::<Uuid>::new()
    );
    assert_eq!(joined(&mut client), vec![(m2, engine)]);

    // The deleted memory's id can be inserted again — a fresh record, no edges.
    client
        .insert(
            m1,
            &[
                ("content", ScanValue::Str("memory 1, reborn".into())),
                ("category", ScanValue::Str("general".into())),
                ("tags", ScanValue::StrList(vec![])),
                ("source", ScanValue::Str("manual".into())),
                ("metadata_json", ScanValue::Str("{}".into())),
                ("created_at_unix_ms", ScanValue::I64(9_000)),
                ("updated_at_unix_ms", ScanValue::I64(9_000)),
                ("memory_type", ScanValue::Str("unclassified".into())),
                ("status", ScanValue::Str("active".into())),
                ("sensitive", ScanValue::Bool(false)),
                ("access_count", ScanValue::I64(0)),
            ],
        )
        .unwrap();
    assert_eq!(
        client.neighbors_by_relation(m1, "mentions").unwrap(),
        Vec::<Uuid>::new()
    );
    drop(client);

    let addr = start_two_table_server_at(dir);
    let mut client = SchemaDrivenClient::connect(addr).unwrap();
    assert_eq!(
        client.get(m1).unwrap().unwrap()[0],
        (
            "content".to_string(),
            ScanValue::Str("memory 1, reborn".into())
        )
    );
    assert_eq!(
        client.neighbors_by_relation(ada, "mentions").unwrap(),
        Vec::<Uuid>::new()
    );
    assert_eq!(joined(&mut client), vec![(m2, engine)]);
    client.use_table("entity").unwrap();
    assert!(client.get(ada).unwrap().is_none());
    assert!(client.get(engine).unwrap().is_some());
}

/// `CMP` acceptance criterion 2 (ADR-0052) on the two-table server: after
/// inserts, links, and deletes, `compact` on each table reports what it
/// reclaimed, every read before and after agrees, a second `compact`
/// reclaims nothing, and a restart on the compacted files serves the
/// same data.
#[test]
fn compact_each_table_over_the_wire_and_a_restart_serves_the_same_data() {
    let dir = unique_dir("memory_compact");
    let addr = start_two_table_server_at(dir.clone());
    let mut client = SchemaDrivenClient::connect(addr).unwrap();
    let (m1, m3) = (Uuid::from_u128(1), Uuid::from_u128(3));
    let (ada, london) = (Uuid::from_u128(0xada), Uuid::from_u128(0x10d));
    client.link(m3, london, "mentions").unwrap();
    assert!(client.delete(m1).unwrap());
    let joined = |client: &mut SchemaDrivenClient| match client
        .query("SELECT m.content, e.label FROM memory m JOIN entity e ON mentions")
        .unwrap()
    {
        QueryResult::Joined(rows) => {
            let mut pairs: Vec<(Uuid, Uuid)> =
                rows.iter().map(|r| (r.left_id, r.right_id)).collect();
            pairs.sort();
            pairs
        }
        other => panic!("expected Joined, got {other:?}"),
    };
    let before = joined(&mut client);
    let contents_before = rows(client.query("SELECT content FROM memory").unwrap()).len();

    let report = client.compact().unwrap();
    assert_eq!(report.records, 3);
    assert_eq!(report.slots_reclaimed, 1);
    assert_eq!(report.log_entries_folded, 1, "the tombstone");
    assert_eq!(report.edge_logs_folded, 1, "the runtime link");
    assert_eq!(joined(&mut client), before);
    assert_eq!(
        rows(client.query("SELECT content FROM memory").unwrap()).len(),
        contents_before
    );
    let again = client.compact().unwrap();
    assert_eq!(again.records, 3);
    assert_eq!(
        again.slots_reclaimed + again.log_entries_folded + again.edge_logs_folded,
        0
    );

    client.use_table("entity").unwrap();
    assert!(client.delete(ada).unwrap());
    let entity_report = client.compact().unwrap();
    assert_eq!(entity_report.records, 2);
    assert_eq!(entity_report.slots_reclaimed, 1);
    client.use_table("memory").unwrap();
    let after_cascade = joined(&mut client);
    assert!(after_cascade.iter().all(|(_, e)| *e != ada));
    drop(client);

    let addr = start_two_table_server_at(dir);
    let mut client = SchemaDrivenClient::connect(addr).unwrap();
    assert_eq!(joined(&mut client), after_cascade);
    assert!(client.get(m1).unwrap().is_none());
    assert_eq!(
        client.neighbors_by_relation(m3, "mentions").unwrap(),
        vec![london]
    );
}
