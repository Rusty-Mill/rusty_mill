//! Real end-to-end coverage of `Request::Metrics` (`MET-FR-001`,
//! ADR-0064, `docs/design/SERVER-METRICS-DESIGN.md`) — a real
//! `TcpListener`, a real `SchemaDrivenClient`. `server` feature only, no
//! `research`.

use rusty_multimodal_db::generic::memory::{create_memory_production_stack, Memory};
use rusty_multimodal_db::generic::production::GenericProductionStore;
use rusty_multimodal_db::server::client::{ClientError, SchemaDrivenClient};
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

fn sample_memories() -> Vec<Memory> {
    vec![Memory {
        id: Uuid::from_u128(1),
        content: "hello".into(),
        category: "general".into(),
        tags: vec![],
        source: "manual".into(),
        metadata_json: "{}".into(),
        created_at_unix_ms: 1,
        updated_at_unix_ms: 1,
        memory_type: "unclassified".into(),
        status: "active".into(),
        sensitive: false,
        access_count: 0,
        deleted_at_unix_ms: 0,
        node_id: String::new(),
    }]
}

fn start_server(options: ServeOptions) -> SocketAddr {
    let dir = unique_dir("metrics_integration");
    std::fs::create_dir_all(&dir).unwrap();
    let path = dir.join("memories.mmap");
    let stack = create_memory_production_stack(sample_memories(), &[], &path).unwrap();
    let connection_store = Arc::new(MemoryConnectionStore::new(GenericProductionStore::new(
        stack,
    )));
    let listener = TcpListener::bind("127.0.0.1:0").unwrap();
    let addr = listener.local_addr().unwrap();
    thread::spawn(move || serve(listener, connection_store, options));
    addr
}

fn metric_value(text: &str, name: &str) -> u64 {
    text.lines()
        .find_map(|line| line.strip_prefix(&format!("{name} ")))
        .unwrap_or_else(|| panic!("{name} not found in:\n{text}"))
        .trim()
        .parse()
        .unwrap()
}

/// `MET-FR-003`: `requests_total`/`ok`/`err` advance by exactly the
/// dispatched requests made between two `Metrics` calls — two
/// successful reads (`GetById` found and not-found, both `Ok` per this
/// crate's own convention) and one refused duplicate insert (`Err`).
/// Measured as a delta, not an absolute count, since `connect()` itself
/// already sends `DescribeSchema`/`DescribeRelations` before the test's
/// own actions.
#[test]
fn metrics_counts_requests_and_the_ok_err_split() {
    let addr = start_server(ServeOptions::default());
    let mut client = SchemaDrivenClient::connect(addr).unwrap();
    let before = client.metrics().unwrap();
    let (total0, ok0, err0) = (
        metric_value(&before, "dogserver_requests_total"),
        metric_value(&before, "dogserver_requests_ok_total"),
        metric_value(&before, "dogserver_requests_err_total"),
    );

    assert!(client.get(Uuid::from_u128(1)).unwrap().is_some());
    assert!(client.get(Uuid::from_u128(999)).unwrap().is_none());
    // A duplicate `Insert` is a genuine server round trip (unlike
    // `update` on a read-only field, which `SchemaDrivenClient` refuses
    // client-side with no frame sent) — the server answers
    // `Err { Duplicate }`.
    let fields: Vec<(&str, ScanValue)> = vec![
        ("content", ScanValue::Str("x".into())),
        ("category", ScanValue::Str("general".into())),
        ("tags", ScanValue::StrList(vec![])),
        ("source", ScanValue::Str("manual".into())),
        ("metadata_json", ScanValue::Str("{}".into())),
        ("created_at_unix_ms", ScanValue::I64(1)),
        ("updated_at_unix_ms", ScanValue::I64(1)),
        ("memory_type", ScanValue::Str("unclassified".into())),
        ("status", ScanValue::Str("active".into())),
        ("sensitive", ScanValue::Bool(false)),
        ("access_count", ScanValue::I64(0)),
        ("deleted_at_unix_ms", ScanValue::I64(0)),
        ("node_id", ScanValue::Str(String::new())),
    ];
    match client.insert(Uuid::from_u128(1), &fields) {
        Err(ClientError::Server(ErrorCode::Duplicate, _)) => {}
        other => panic!("expected Duplicate, got {other:?}"),
    }
    // `before` itself was one dispatched `Metrics` request too.
    let after = client.metrics().unwrap();
    let total = metric_value(&after, "dogserver_requests_total");
    let ok = metric_value(&after, "dogserver_requests_ok_total");
    let err = metric_value(&after, "dogserver_requests_err_total");
    assert_eq!(
        total - total0,
        4,
        "2 GetById + 1 Insert + the first Metrics call itself"
    );
    assert_eq!(
        ok - ok0,
        3,
        "2 GetById (found/not-found, both Ok) + the first Metrics call"
    );
    assert_eq!(err - err0, 1, "the duplicate Insert");
    assert!(
        after.contains("dogserver_connections_active 1\n"),
        "text: {after}"
    );
    assert!(
        after.contains("dogserver_connections_total 1\n"),
        "text: {after}"
    );
}

/// `MET-FR-005`: a `ReadOnly` token can call `Metrics` — the first
/// request this crate gates more permissively than the existing
/// `ReadOnly`/`ReadWrite` split — while a write from the same
/// connection is still refused *by the server*. `access_count` (unlike
/// `content`) has `capabilities.update: true`, so the client sends the
/// request rather than refusing it locally, and the server's own
/// `ReadOnly` gate is what answers `Unauthorized`.
#[test]
fn metrics_is_answerable_by_a_read_only_token() {
    let addr = start_server(ServeOptions::new(
        Some("ro-secret".into()),
        Some("rw-secret".into()),
    ));
    let mut client = SchemaDrivenClient::connect_authenticated(addr, "ro-secret").unwrap();
    assert!(client
        .metrics()
        .unwrap()
        .contains("dogserver_requests_total"));
    match client.update(Uuid::from_u128(1), "access_count", ScanValue::I64(1)) {
        Err(ClientError::Server(ErrorCode::Unauthorized, _)) => {}
        other => panic!("expected Unauthorized, got {other:?}"),
    }
}

/// `connections_active` drops back to `0` once every client
/// disconnects — the drop-guard property, observed from a second
/// connection after the first closes.
#[test]
fn connections_active_returns_to_zero_after_disconnect() {
    let addr = start_server(ServeOptions::default());
    {
        let mut client = SchemaDrivenClient::connect(addr).unwrap();
        let text = client.metrics().unwrap();
        assert!(
            text.contains("dogserver_connections_active 1\n"),
            "text: {text}"
        );
    }
    // Give the server's connection thread a moment to run its `Drop`
    // guards after the socket closes.
    std::thread::sleep(std::time::Duration::from_millis(200));
    let mut client = SchemaDrivenClient::connect(addr).unwrap();
    let text = client.metrics().unwrap();
    assert!(
        text.contains("dogserver_connections_active 1\n"),
        "text: {text}"
    );
    assert!(
        text.contains("dogserver_connections_total 2\n"),
        "text: {text}"
    );
}
