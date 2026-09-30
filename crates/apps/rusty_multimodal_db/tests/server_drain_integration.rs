//! `ADR-0127` (`DRN-FR-001`..`005`) over a real socket: a `Shutdown` handle
//! stops `serve` gracefully. Accepting stops, a request already being
//! answered completes, every connection then ends, and `serve` returns.

use rusty_multimodal_db::generic::memory::{create_memory_production_stack, Memory};
use rusty_multimodal_db::generic::production::GenericProductionStore;
use rusty_multimodal_db::server::framing::{read_message, write_message};
use rusty_multimodal_db::server::memory::MemoryConnectionStore;
use rusty_multimodal_db::server::protocol::{
    DomainSchema, ErrorCode, FieldRef, ParentLookup, RecordId, Request, Response, ScanValue,
    Selection, TransactionOp, PROTOCOL_VERSION,
};
use rusty_multimodal_db::server::{serve, ConnectionStore, ServeOptions, Shutdown};
use std::io::{BufReader, BufWriter, Write};
use std::net::{SocketAddr, TcpListener, TcpStream};
use std::sync::Arc;
use std::thread::{self, JoinHandle};
use std::time::{Duration, Instant};
use uuid::Uuid;

/// A `Memory` store whose full scan takes `delay`, so a request can be
/// caught mid-flight. Every other method is the inner store's.
struct Slow {
    inner: MemoryConnectionStore,
    delay: Duration,
}

impl ConnectionStore for Slow {
    fn get(&self, id: RecordId) -> Option<Vec<(FieldRef, ScanValue)>> {
        self.inner.get(id)
    }
    fn filter_eq(&self, field: FieldRef, value: &ScanValue) -> Result<Vec<RecordId>, ErrorCode> {
        self.inner.filter_eq(field, value)
    }
    fn scan_field(&self, field: FieldRef) -> Result<Vec<ScanValue>, ErrorCode> {
        self.inner.scan_field(field)
    }
    fn update_field(
        &self,
        id: RecordId,
        field: FieldRef,
        value: ScanValue,
    ) -> Result<bool, ErrorCode> {
        self.inner.update_field(id, field, value)
    }
    fn parent(&self, id: RecordId) -> Result<ParentLookup, ErrorCode> {
        self.inner.parent(id)
    }
    fn children(&self, id: RecordId) -> Result<Vec<RecordId>, ErrorCode> {
        self.inner.children(id)
    }
    fn neighbors(&self, id: RecordId) -> Result<Vec<RecordId>, ErrorCode> {
        self.inner.neighbors(id)
    }
    fn neighbors_by_relation(
        &self,
        id: RecordId,
        relation: &str,
    ) -> Result<Vec<RecordId>, ErrorCode> {
        self.inner.neighbors_by_relation(id, relation)
    }
    fn list_relation_kinds(&self) -> Vec<String> {
        self.inner.list_relation_kinds()
    }
    fn table_name(&self) -> &str {
        self.inner.table_name()
    }
    fn describe(&self) -> DomainSchema {
        self.inner.describe()
    }
    fn validate_op(&self, op: &TransactionOp) -> Result<(), ErrorCode> {
        self.inner.validate_op(op)
    }
    fn scan_all(&self) -> Vec<(RecordId, Vec<(FieldRef, ScanValue)>)> {
        thread::sleep(self.delay);
        self.inner.scan_all()
    }
    fn apply_transaction(
        &self,
        updates: &[TransactionOp],
        read_set: &[(RecordId, FieldRef, ScanValue)],
    ) -> Result<(), (usize, ErrorCode)> {
        self.inner.apply_transaction(updates, read_set)
    }
}

fn unique_dir(label: &str) -> std::path::PathBuf {
    use std::sync::atomic::{AtomicU64, Ordering};
    static COUNTER: AtomicU64 = AtomicU64::new(0);
    let n = COUNTER.fetch_add(1, Ordering::Relaxed);
    std::env::temp_dir().join(format!("{label}_{}_{n}", std::process::id()))
}

fn memory(i: u128) -> Memory {
    Memory {
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
    }
}

/// A `serve` on a thread, its `Shutdown`, and where to reach it.
fn start(delay: Duration, drain: Duration) -> (SocketAddr, Shutdown, JoinHandle<()>) {
    let dir = unique_dir("drain_integration");
    std::fs::create_dir_all(&dir).unwrap();
    let stack = create_memory_production_stack(
        vec![memory(1), memory(2), memory(3)],
        &[],
        &dir.join("memories.mmap"),
    )
    .unwrap();
    let store = Arc::new(Slow {
        inner: MemoryConnectionStore::new(GenericProductionStore::new(stack)),
        delay,
    });
    let listener = TcpListener::bind("127.0.0.1:0").unwrap();
    let addr = listener.local_addr().unwrap();
    let shutdown = Shutdown::new();
    let options = ServeOptions::new(None, None)
        .with_shutdown(shutdown.clone())
        .with_drain_timeout(drain);
    let handle = thread::spawn(move || serve(listener, store, options));
    (addr, shutdown, handle)
}

fn connect(addr: SocketAddr) -> (BufReader<TcpStream>, BufWriter<TcpStream>) {
    let stream = TcpStream::connect(addr).unwrap();
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
    assert!(matches!(
        read_message::<_, Response>(&mut reader).unwrap(),
        Response::Hello { .. }
    ));
    (reader, writer)
}

fn scan_query() -> Request {
    Request::Query {
        select: Selection::All,
        filter: vec![],
        limit: None,
    }
}

fn join_within(handle: JoinHandle<()>, limit: Duration) {
    let started = Instant::now();
    while !handle.is_finished() {
        assert!(
            started.elapsed() < limit,
            "serve did not return in {limit:?}"
        );
        thread::sleep(Duration::from_millis(10));
    }
    handle.join().unwrap();
}

/// `DRN-FR-001`/`002`/`003`: an idle connection is closed, `serve` returns,
/// and the listener is gone, so a new connect is refused.
#[test]
fn a_shutdown_closes_idle_connections_stops_accepting_and_returns() {
    let (addr, shutdown, handle) = start(Duration::ZERO, Duration::from_secs(5));
    let (mut reader, _writer) = connect(addr);
    assert!(!shutdown.is_requested());
    shutdown.request();
    assert!(shutdown.is_requested());
    join_within(handle, Duration::from_secs(3));
    assert!(
        read_message::<_, Response>(&mut reader).is_err(),
        "the idle connection ended"
    );
    assert!(TcpStream::connect(addr).is_err(), "nothing is listening");
}

/// `DRN-FR-002`: a request in flight when the shutdown lands is answered in
/// full before its connection closes.
#[test]
fn a_request_in_flight_when_shutdown_lands_is_answered_first() {
    let (addr, shutdown, handle) = start(Duration::from_millis(400), Duration::from_secs(5));
    let (mut reader, mut writer) = connect(addr);
    write_message(&mut writer, &scan_query()).unwrap();
    writer.flush().unwrap();
    thread::sleep(Duration::from_millis(100)); // the scan is now sleeping
    shutdown.request();
    match read_message::<_, Response>(&mut reader).unwrap() {
        Response::Rows { rows } => assert_eq!(rows.len(), 3, "the whole answer, not a cut one"),
        other => panic!("expected the in-flight Query's Rows, got {other:?}"),
    }
    assert!(
        read_message::<_, Response>(&mut reader).is_err(),
        "then the connection ends"
    );
    join_within(handle, Duration::from_secs(3));
}

/// `DRN-FR-004`: without a `Shutdown`, `serve` is what it always was; a
/// requested handle before `serve` starts makes it return at once.
#[test]
fn a_shutdown_requested_before_serve_starts_returns_at_once() {
    let listener = TcpListener::bind("127.0.0.1:0").unwrap();
    let dir = unique_dir("drain_integration_early");
    std::fs::create_dir_all(&dir).unwrap();
    let stack =
        create_memory_production_stack(vec![memory(1)], &[], &dir.join("memories.mmap")).unwrap();
    let store = Arc::new(MemoryConnectionStore::new(GenericProductionStore::new(
        stack,
    )));
    let shutdown = Shutdown::new();
    shutdown.request();
    let handle = thread::spawn(move || {
        serve(
            listener,
            store,
            ServeOptions::new(None, None).with_shutdown(shutdown),
        )
    });
    join_within(handle, Duration::from_secs(3));
}
