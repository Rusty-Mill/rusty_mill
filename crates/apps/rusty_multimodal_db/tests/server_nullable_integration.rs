//! `ADR-0128` (`NLC-FR-001`..`007`) over a real socket on `Memory`: the two
//! sentinel columns of `ADR-0056` (`deleted_at_unix_ms`: `0`, `node_id`: `""`)
//! read and write as `NULL` from protocol 32, and are still their sentinels
//! at 31 and below — one stored value, two views.

use rusty_multimodal_db::generic::memory::{create_memory_production_stack, Memory};
use rusty_multimodal_db::generic::production::GenericProductionStore;
use rusty_multimodal_db::server::client::{
    ClientError, ConnectOptions, QueryResult, SchemaDrivenClient,
};
use rusty_multimodal_db::server::framing::{read_message, write_message};
use rusty_multimodal_db::server::memory::{MemoryConnectionStore, FIELD_DELETED_AT, FIELD_NODE_ID};
use rusty_multimodal_db::server::protocol::{
    CompareOp, ErrorCode, Predicate, Request, Response, ScanValue, Selection,
};
use rusty_multimodal_db::server::{serve, ServeOptions};
use std::io::{BufReader, BufWriter, Write};
use std::net::{SocketAddr, TcpListener, TcpStream};
use std::sync::Arc;
use std::thread;
use uuid::Uuid;

fn memory(n: u128) -> Memory {
    Memory {
        id: Uuid::from_u128(n),
        content: format!("memory {n}"),
        category: "general".into(),
        tags: vec![],
        source: "manual".into(),
        metadata_json: "{}".into(),
        created_at_unix_ms: 1_000 * n as i64,
        updated_at_unix_ms: 1_000 * n as i64,
        memory_type: "unclassified".into(),
        status: "active".into(),
        sensitive: false,
        access_count: 0,
        deleted_at_unix_ms: 0,
        node_id: String::new(),
    }
}

fn start_server() -> SocketAddr {
    use std::sync::atomic::{AtomicU64, Ordering};
    static COUNTER: AtomicU64 = AtomicU64::new(0);
    let dir = std::env::temp_dir().join(format!(
        "nullable_integration_{}_{}",
        std::process::id(),
        COUNTER.fetch_add(1, Ordering::Relaxed)
    ));
    std::fs::create_dir_all(&dir).unwrap();
    let stack = create_memory_production_stack(
        vec![memory(1), memory(2), memory(3)],
        &[],
        &dir.join("memories.mmap"),
    )
    .unwrap();
    let store = Arc::new(MemoryConnectionStore::new(GenericProductionStore::new(
        stack,
    )));
    let listener = TcpListener::bind("127.0.0.1:0").unwrap();
    let addr = listener.local_addr().unwrap();
    thread::spawn(move || serve(listener, store, ServeOptions::default()));
    addr
}

fn at(addr: SocketAddr, version: u32) -> SchemaDrivenClient {
    SchemaDrivenClient::connect_with(addr, ConnectOptions::new().max_protocol_version(version))
        .unwrap()
}

fn id(n: u128) -> Uuid {
    Uuid::from_u128(n)
}

fn field(fields: &[(String, ScanValue)], name: &str) -> ScanValue {
    fields
        .iter()
        .find(|(n, _)| n == name)
        .unwrap_or_else(|| panic!("no field {name}"))
        .1
        .clone()
}

fn ids(result: QueryResult) -> Vec<Uuid> {
    match result {
        QueryResult::Rows(rows) => {
            let mut ids: Vec<Uuid> = rows.into_iter().map(|(id, _)| id).collect();
            ids.sort();
            ids
        }
        other => panic!("expected Rows, got {other:?}"),
    }
}

/// `NLC-FR-003`/`005`/`007`: at 32 the sentinels read `Null` and
/// `describe_nullable` names the two fields; at 31 the same record reads its
/// sentinels and the describe call is refused locally.
#[test]
fn a_sentinel_reads_null_at_32_and_as_itself_at_31() {
    let addr = start_server();
    let mut now = at(addr, 32);
    let record = now.get(id(1)).unwrap().unwrap();
    assert_eq!(field(&record, "deleted_at_unix_ms"), ScanValue::Null);
    assert_eq!(field(&record, "node_id"), ScanValue::Null);
    assert_eq!(field(&record, "content"), ScanValue::Str("memory 1".into()));
    assert_eq!(
        now.describe_nullable().unwrap(),
        vec!["deleted_at_unix_ms", "node_id"]
    );

    let mut before = at(addr, 31);
    let record = before.get(id(1)).unwrap().unwrap();
    assert_eq!(field(&record, "deleted_at_unix_ms"), ScanValue::I64(0));
    assert_eq!(field(&record, "node_id"), ScanValue::Str(String::new()));
    assert!(matches!(
        before.describe_nullable(),
        Err(ClientError::Unsupported("describe_nullable"))
    ));
}

/// `NLC-FR-002`/`006`: a `Null` written stores the sentinel, a real value
/// reads back as itself, and `IS NULL` / `IS NOT NULL` partition the table.
#[test]
fn null_is_writable_and_is_null_partitions_the_table() {
    let addr = start_server();
    let mut client = at(addr, 32);
    assert_eq!(
        ids(client
            .query("SELECT content FROM memory WHERE deleted_at_unix_ms IS NULL")
            .unwrap()),
        vec![id(1), id(2), id(3)]
    );
    // Tombstone memory 2 with a real stamp; attribute memory 3 to a node;
    // write `Null` back explicitly for memory 1's stamp.
    let write = |client: &mut SchemaDrivenClient, n: u128, deleted: ScanValue, node: ScanValue| {
        let fields = client.get(id(n)).unwrap().unwrap();
        let fields: Vec<(String, ScanValue)> = fields
            .into_iter()
            .map(|(name, value)| match name.as_str() {
                "deleted_at_unix_ms" => (name, deleted.clone()),
                "node_id" => (name, node.clone()),
                _ => (name, value),
            })
            .collect();
        let borrowed: Vec<(&str, ScanValue)> = fields
            .iter()
            .map(|(n, v)| (n.as_str(), v.clone()))
            .collect();
        assert!(client.replace(id(n), &borrowed).unwrap());
    };
    write(&mut client, 2, ScanValue::I64(7_000), ScanValue::Null);
    write(
        &mut client,
        3,
        ScanValue::Null,
        ScanValue::Str("desktop".into()),
    );
    write(&mut client, 1, ScanValue::Null, ScanValue::Null);

    assert_eq!(
        ids(client
            .query("SELECT content FROM memory WHERE deleted_at_unix_ms IS NULL")
            .unwrap()),
        vec![id(1), id(3)]
    );
    assert_eq!(
        ids(client
            .query("SELECT content FROM memory WHERE deleted_at_unix_ms IS NOT NULL")
            .unwrap()),
        vec![id(2)]
    );
    assert_eq!(
        ids(client
            .query("SELECT content FROM memory WHERE node_id IS NOT NULL")
            .unwrap()),
        vec![id(3)]
    );
    let two = client.get(id(2)).unwrap().unwrap();
    assert_eq!(field(&two, "deleted_at_unix_ms"), ScanValue::I64(7_000));
    assert_eq!(field(&two, "node_id"), ScanValue::Null);

    // The stored value is the sentinel: a connection at 31 sees it.
    let mut old = at(addr, 31);
    let three = old.get(id(3)).unwrap().unwrap();
    assert_eq!(field(&three, "deleted_at_unix_ms"), ScanValue::I64(0));
    assert_eq!(field(&three, "node_id"), ScanValue::Str("desktop".into()));
    let one = old.get(id(1)).unwrap().unwrap();
    assert_eq!(field(&one, "node_id"), ScanValue::Str(String::new()));
}

/// `NLC-FR-003`: `GROUP BY` a nullable field keys its group `Null` at 32 and
/// `""` at 31.
#[test]
fn a_group_by_a_nullable_field_keys_null_at_32() {
    let addr = start_server();
    let group_key = |client: &mut SchemaDrivenClient| match client
        .query("SELECT node_id, COUNT(*) FROM memory GROUP BY node_id")
        .unwrap()
    {
        QueryResult::Groups(groups) => field(&groups[0], "node_id"),
        other => panic!("expected Groups, got {other:?}"),
    };
    assert_eq!(group_key(&mut at(addr, 32)), ScanValue::Null);
    assert_eq!(group_key(&mut at(addr, 31)), ScanValue::Str(String::new()));
}

fn raw(addr: SocketAddr, version: u32) -> (BufReader<TcpStream>, BufWriter<TcpStream>) {
    let stream = TcpStream::connect(addr).unwrap();
    let mut reader = BufReader::new(stream.try_clone().unwrap());
    let mut writer = BufWriter::new(stream);
    write_message(
        &mut writer,
        &Request::Hello {
            protocol_version: version,
        },
    )
    .unwrap();
    writer.flush().unwrap();
    assert!(matches!(
        read_message::<_, Response>(&mut reader).unwrap(),
        Response::Hello { .. }
    ));
    (reader, writer)
}

fn send(
    (reader, writer): &mut (BufReader<TcpStream>, BufWriter<TcpStream>),
    request: &Request,
) -> Response {
    write_message(writer, request).unwrap();
    writer.flush().unwrap();
    read_message::<_, Response>(reader).unwrap()
}

fn query(filter: Vec<Predicate>) -> Request {
    Request::Query {
        select: Selection::All,
        filter,
        limit: None,
    }
}

fn err(resp: Response) -> ErrorCode {
    match resp {
        Response::Err { code, .. } => code,
        other => panic!("expected an error, got {other:?}"),
    }
}

/// `NLC-FR-002`/`005`: only equality means "is null"; `Null` for a field
/// that is not nullable is refused as before; below 32 `DescribeNullable`
/// and a `Null` for a nullable field are both `Malformed`.
#[test]
fn only_equality_on_a_nullable_field_takes_null_and_below_32_nothing_does() {
    let addr = start_server();
    let pred = |field, op| Predicate {
        field,
        op,
        value: ScanValue::Null,
    };
    let mut now = raw(addr, 32);
    match send(
        &mut now,
        &query(vec![pred(FIELD_DELETED_AT, CompareOp::Eq)]),
    ) {
        Response::Rows { rows } => assert_eq!(rows.len(), 3),
        other => panic!("expected Rows, got {other:?}"),
    }
    assert_eq!(
        err(send(
            &mut now,
            &query(vec![pred(FIELD_DELETED_AT, CompareOp::Lt)])
        )),
        ErrorCode::Malformed,
        "a bound is not null-ness"
    );
    assert_eq!(
        err(send(&mut now, &query(vec![pred(0, CompareOp::Eq)]))),
        ErrorCode::Malformed,
        "`content` is not nullable"
    );
    match send(&mut now, &Request::DescribeNullable) {
        Response::NullableFields { tags } => {
            assert_eq!(tags, vec![FIELD_DELETED_AT, FIELD_NODE_ID])
        }
        other => panic!("expected NullableFields, got {other:?}"),
    }

    let mut old = raw(addr, 31);
    assert_eq!(
        err(send(&mut old, &Request::DescribeNullable)),
        ErrorCode::Malformed,
        "rule 3: a request from after this connection's version"
    );
    assert_eq!(
        err(send(
            &mut old,
            &query(vec![pred(FIELD_NODE_ID, CompareOp::Eq)])
        )),
        ErrorCode::Malformed,
        "at 31 `Null` is still just a value no kind matches"
    );
}
