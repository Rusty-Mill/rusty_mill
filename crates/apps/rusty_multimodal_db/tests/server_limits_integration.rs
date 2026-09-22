//! `ADR-0093` (`docs/design/SERVER-CONNECTION-LIMITS-DESIGN.md`) over a
//! real socket on `Memory`: the idle timeout (`LIM-FR-001`), the
//! connection cap (`LIM-FR-002`), and the row cap (`LIM-FR-003`) —
//! each opt-in through `ServeOptions`, each observed from the client
//! side exactly as a deployment would see it.

use rusty_multimodal_db::generic::memory::{create_memory_production_stack, Memory};
use rusty_multimodal_db::generic::production::GenericProductionStore;
use rusty_multimodal_db::server::client::{ClientError, QueryResult, SchemaDrivenClient};
use rusty_multimodal_db::server::framing::{read_message, write_message};
use rusty_multimodal_db::server::memory::{MemoryConnectionStore, FIELD_UPDATED_AT};
use rusty_multimodal_db::server::protocol::{
    ErrorCode, Request, Response, Selection, PROTOCOL_VERSION,
};
use rusty_multimodal_db::server::{serve, ServeOptions};
use std::io::{BufReader, BufWriter, Read, Write};
use std::net::{SocketAddr, TcpListener, TcpStream};
use std::sync::Arc;
use std::thread;
use std::time::Duration;
use uuid::Uuid;

fn unique_dir(label: &str) -> std::path::PathBuf {
    use std::sync::atomic::{AtomicU64, Ordering};
    static COUNTER: AtomicU64 = AtomicU64::new(0);
    let n = COUNTER.fetch_add(1, Ordering::Relaxed);
    std::env::temp_dir().join(format!("{label}_{}_{n}", std::process::id()))
}

fn sample_memories(n: u128) -> Vec<Memory> {
    (1..=n)
        .map(|i| Memory {
            id: Uuid::from_u128(i),
            content: format!("memory {i}"),
            category: "general".into(),
            tags: vec![],
            source: "manual".into(),
            metadata_json: "{}".into(),
            created_at_unix_ms: i as i64,
            updated_at_unix_ms: i as i64,
            memory_type: "unclassified".into(),
            status: "active".into(),
            sensitive: false,
            access_count: 0,
            deleted_at_unix_ms: 0,
            node_id: String::new(),
        })
        .collect()
}

fn start_server(options: ServeOptions) -> SocketAddr {
    let dir = unique_dir("limits_integration");
    std::fs::create_dir_all(&dir).unwrap();
    let path = dir.join("memories.mmap");
    let stack = create_memory_production_stack(sample_memories(5), &[], &path).unwrap();
    let connection_store = Arc::new(MemoryConnectionStore::new(GenericProductionStore::new(
        stack,
    )));
    let listener = TcpListener::bind("127.0.0.1:0").unwrap();
    let addr = listener.local_addr().unwrap();
    thread::spawn(move || serve(listener, connection_store, options));
    addr
}

/// A raw, negotiated connection — `Hello` at `PROTOCOL_VERSION` sent
/// and acknowledged — so the tests can send one exact `Request` shape
/// and read one exact `Response`.
fn raw_connection(addr: SocketAddr) -> (BufReader<TcpStream>, BufWriter<TcpStream>) {
    let stream = TcpStream::connect(addr).unwrap();
    stream.set_nodelay(true).unwrap();
    stream
        .set_read_timeout(Some(Duration::from_secs(5)))
        .unwrap();
    let mut reader = BufReader::new(stream.try_clone().unwrap());
    let mut writer = BufWriter::new(stream);
    write_message(
        &mut writer,
        &Request::Hello {
            protocol_version: PROTOCOL_VERSION,
        },
    )
    .unwrap();
    writer.flush().unwrap();
    match read_message::<_, Response>(&mut reader).unwrap() {
        Response::Hello { .. } => {}
        other => panic!("expected Hello, got {other:?}"),
    }
    (reader, writer)
}

fn round_trip(
    reader: &mut BufReader<TcpStream>,
    writer: &mut BufWriter<TcpStream>,
    request: &Request,
) -> Response {
    write_message(writer, request).unwrap();
    writer.flush().unwrap();
    read_message::<_, Response>(reader).unwrap()
}

fn query(limit: Option<usize>) -> Request {
    Request::Query {
        select: Selection::All,
        filter: vec![],
        limit,
    }
}

fn page(limit: u64) -> Request {
    Request::Page {
        order_by: FIELD_UPDATED_AT,
        after: None,
        limit,
    }
}

fn error_code(resp: Response) -> ErrorCode {
    match resp {
        Response::Err { code, .. } => code,
        other => panic!("expected an error, got {other:?}"),
    }
}

/// `LIM-FR-001`: a connection that sends nothing for longer than the
/// idle timeout is closed by the server; one that keeps talking is not.
#[test]
fn an_idle_connection_is_closed_at_the_idle_timeout_and_a_busy_one_is_not() {
    let addr =
        start_server(ServeOptions::new(None, None).with_idle_timeout(Duration::from_millis(300)));

    // Busy: three requests spaced under the timeout all answer.
    let (mut reader, mut writer) = raw_connection(addr);
    for _ in 0..3 {
        thread::sleep(Duration::from_millis(100));
        match round_trip(&mut reader, &mut writer, &query(Some(1))) {
            Response::Rows { rows } => assert_eq!(rows.len(), 1),
            other => panic!("expected Rows, got {other:?}"),
        }
    }

    // Idle: past the timeout the server has closed its side — the next
    // read sees EOF (or a reset), never a response.
    thread::sleep(Duration::from_millis(700));
    let mut probe = [0u8; 1];
    let closed = match reader.get_mut().read(&mut probe) {
        Ok(0) => true,
        Ok(_) => false,
        Err(e) => e.kind() != std::io::ErrorKind::WouldBlock,
    };
    assert!(closed, "the idle connection was not closed by the server");
}

/// `LIM-FR-002`: with a cap of one, a second concurrent connection is
/// closed at accept with nothing written, the refusal is counted, and
/// once the first connection ends a new one is admitted.
#[test]
fn a_connection_past_the_cap_is_closed_at_accept_and_counted() {
    let addr = start_server(ServeOptions::new(None, None).with_max_connections(1));

    let mut first = SchemaDrivenClient::connect(addr).unwrap();
    let text = first.metrics().unwrap();
    assert!(
        text.contains("dogserver_connections_active 1\n"),
        "text: {text}"
    );

    // The second: the server closes it before reading a byte, so the
    // `Hello` the client sends is answered by EOF or a reset.
    let refused = TcpStream::connect(addr).unwrap();
    refused
        .set_read_timeout(Some(Duration::from_secs(5)))
        .unwrap();
    let mut writer = BufWriter::new(refused.try_clone().unwrap());
    let _ = write_message(
        &mut writer,
        &Request::Hello {
            protocol_version: PROTOCOL_VERSION,
        },
    );
    let _ = writer.flush();
    let mut reader = BufReader::new(refused);
    assert!(
        read_message::<_, Response>(&mut reader).is_err(),
        "a connection past the cap was answered"
    );

    let text = first.metrics().unwrap();
    assert!(
        text.contains("dogserver_connections_refused_total 1\n"),
        "text: {text}"
    );
    assert!(
        text.contains("dogserver_connections_total 1\n"),
        "a refused accept must not count as accepted; text: {text}"
    );

    drop(first);
    // Give the connection thread a moment to release its slot.
    thread::sleep(Duration::from_millis(200));
    let mut third = SchemaDrivenClient::connect(addr).unwrap();
    let text = third.metrics().unwrap();
    assert!(
        text.contains("dogserver_connections_active 1\n"),
        "text: {text}"
    );
}

/// `LIM-FR-003` as amended by `CLP-FR-001` (ADR-0102): under a row cap
/// of two, a `Query` with no `limit` answers the first two rows (clamped,
/// and counted), a `Query` with a `limit` of three is `TooLarge` before
/// any read, a `Query` at the cap answers, and a page is capped the same
/// way; the schema-driven client surfaces the code unchanged.
#[test]
fn a_read_asking_for_more_rows_than_the_cap_is_too_large_before_any_read() {
    let addr = start_server(ServeOptions::new(None, None).with_max_query_rows(2));
    let (mut reader, mut writer) = raw_connection(addr);

    match round_trip(&mut reader, &mut writer, &query(None)) {
        Response::Rows { rows } => assert_eq!(rows.len(), 2, "clamped to the cap"),
        other => panic!("expected Rows clamped to the cap, got {other:?}"),
    }
    assert_eq!(
        error_code(round_trip(&mut reader, &mut writer, &query(Some(3)))),
        ErrorCode::TooLarge
    );
    match round_trip(&mut reader, &mut writer, &query(Some(2))) {
        Response::Rows { rows } => assert_eq!(rows.len(), 2),
        other => panic!("expected Rows, got {other:?}"),
    }
    assert_eq!(
        error_code(round_trip(&mut reader, &mut writer, &page(3))),
        ErrorCode::TooLarge
    );
    match round_trip(&mut reader, &mut writer, &page(2)) {
        Response::Rows { rows } => assert_eq!(rows.len(), 2),
        other => panic!("expected Rows, got {other:?}"),
    }
    // A refused read is an error the metrics see; the clamped one is
    // counted on its own family; a `Metrics` request itself is never
    // capped.
    let mut client = SchemaDrivenClient::connect(addr).unwrap();
    let text = client.metrics().unwrap();
    assert!(
        text.contains("dogserver_requests_err_total 2\n"),
        "text: {text}"
    );
    assert!(
        text.contains("dogserver_query_rows_clamped_total 1\n"),
        "text: {text}"
    );
    // `CLP-FR-001` through the client: a `SELECT` with no `LIMIT` is
    // answered, clamped; one with a `LIMIT` above the cap is refused.
    match client.query("SELECT * FROM memory").unwrap() {
        QueryResult::Rows(rows) => assert_eq!(rows.len(), 2, "clamped through the client"),
        other => panic!("expected Rows, got {other:?}"),
    }
    match client.query("SELECT * FROM memory LIMIT 3") {
        Err(ClientError::Server(code, _)) => assert_eq!(code, ErrorCode::TooLarge),
        other => panic!("expected TooLarge through the client, got {other:?}"),
    }
}
