//! `ADR-0131` (`CHL-FR-001`..`006`) over a real socket on `Memory`: a table's
//! writes are recorded in a change log, a standby restores a snapshot taken at
//! a known position and tails the log, and reaches the primary's state.

use rusty_multimodal_db::generic::memory::{
    create_memory_production_stack, open_memory_production_stack_portable, Memory,
};
use rusty_multimodal_db::generic::production::GenericProductionStore;
use rusty_multimodal_db::server::changelog::{ChangeLog, DEFAULT_RETAIN_BYTES};
use rusty_multimodal_db::server::changelogged::ChangeLogged;
use rusty_multimodal_db::server::client::{
    BatchOp, ClientError, ConnectOptions, SchemaDrivenClient,
};
use rusty_multimodal_db::server::memory::MemoryConnectionStore;
use rusty_multimodal_db::server::protocol::{ErrorCode, ScanValue};
use rusty_multimodal_db::server::{serve_tables, ConnectionStore, ServeOptions};
use std::net::{SocketAddr, TcpListener};
use std::path::{Path, PathBuf};
use std::sync::Arc;
use std::thread;
use uuid::Uuid;

const TOKEN: &str = "repl-secret";

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

fn unique_dir(label: &str) -> PathBuf {
    use std::sync::atomic::{AtomicU64, Ordering};
    static COUNTER: AtomicU64 = AtomicU64::new(0);
    let d = std::env::temp_dir().join(format!(
        "replication_{label}_{}_{}",
        std::process::id(),
        COUNTER.fetch_add(1, Ordering::Relaxed)
    ));
    std::fs::create_dir_all(&d).unwrap();
    d
}

struct Primary {
    addr: SocketAddr,
    store: Arc<MemoryConnectionStore>,
}

/// A `Memory` primary with a change log (or without one) on a real socket.
fn start(logged: bool) -> Primary {
    let dir = unique_dir("primary");
    let path = dir.join("memories.mmap");
    let stack =
        create_memory_production_stack(vec![memory(1), memory(2), memory(3)], &[], &path).unwrap();
    let store = Arc::new(
        MemoryConnectionStore::new(GenericProductionStore::new(stack)).with_backup_source(path),
    );
    let served: Arc<dyn ConnectionStore> = if logged {
        let log = ChangeLog::open(&dir.join("memory.changes"), DEFAULT_RETAIN_BYTES).unwrap();
        Arc::new(ChangeLogged::new(store.clone(), Arc::new(log)))
    } else {
        store.clone()
    };
    let listener = TcpListener::bind("127.0.0.1:0").unwrap();
    let addr = listener.local_addr().unwrap();
    thread::spawn(move || {
        serve_tables(
            listener,
            vec![("memory".to_string(), served)],
            0,
            ServeOptions::new(Some("ro".to_string()), Some("rw".to_string()))
                .with_replication_token(TOKEN.to_string()),
        )
    });
    Primary { addr, store }
}

fn id(n: u128) -> Uuid {
    Uuid::from_u128(n)
}

fn fields_like(
    client: &mut SchemaDrivenClient,
    n: u128,
    content: &str,
) -> Vec<(String, ScanValue)> {
    client
        .get(id(n))
        .unwrap()
        .unwrap()
        .into_iter()
        .map(|(name, value)| match name.as_str() {
            "content" => (name, ScanValue::Str(content.into())),
            _ => (name, value),
        })
        .collect()
}

fn borrowed(fields: &[(String, ScanValue)]) -> Vec<(&str, ScanValue)> {
    fields
        .iter()
        .map(|(n, v)| (n.as_str(), v.clone()))
        .collect()
}

/// Restore a snapshot's files into a fresh directory and open it as a store.
fn restore(files: &[(String, Vec<u8>)]) -> MemoryConnectionStore {
    let dir = unique_dir("replica");
    for (name, bytes) in files {
        std::fs::write(dir.join(name), bytes).unwrap();
    }
    let stack =
        open_memory_production_stack_portable(&Path::new(&dir).join("memories.mmap")).unwrap();
    MemoryConnectionStore::new(GenericProductionStore::new(stack))
}

fn sorted(store: &dyn ConnectionStore) -> Vec<(Uuid, Vec<(u16, ScanValue)>)> {
    let mut rows = store.scan_all();
    rows.sort_by_key(|(id, _)| *id);
    rows
}

/// A snapshot taken mid-stream carries the position it agrees with; the
/// standby restores it, tails the log from there, and equals the primary.
#[test]
fn a_standby_restores_a_snapshot_then_tails_the_log_to_the_primarys_state() {
    let primary = start(true);
    let mut writer = SchemaDrivenClient::connect_authenticated(primary.addr, "rw").unwrap();
    let mut repl = SchemaDrivenClient::connect_authenticated(primary.addr, TOKEN).unwrap();

    // Before the snapshot: some writes of each kind.
    let inserted = fields_like(&mut writer, 1, "inserted before the snapshot");
    writer.insert(id(10), &borrowed(&inserted)).unwrap();
    writer
        .update(id(1), "access_count", ScanValue::I64(2))
        .unwrap();

    let snapshot = repl.fetch_snapshot_at().unwrap();
    let (epoch, seq) = snapshot
        .position
        .expect("a table with a log reports its position");
    assert_eq!(seq, 2, "two entries were committed before the copy");
    let replica = restore(&snapshot.files);
    assert_eq!(
        sorted(&replica),
        sorted(primary.store.as_ref()),
        "the snapshot is the state at seq"
    );

    // After the snapshot: every write path.
    let replaced = fields_like(&mut writer, 2, "replaced after the snapshot");
    writer.replace(id(2), &borrowed(&replaced)).unwrap();
    writer.delete(id(3)).unwrap();
    let mut session = writer.begin().unwrap();
    let staged = fields_like(&mut repl, 1, "inserted in a session");
    session.insert(id(11), &borrowed(&staged)).unwrap();
    session
        .update(id(11), "access_count", ScanValue::I64(9))
        .unwrap();
    session
        .update(id(2), "access_count", ScanValue::I64(4))
        .unwrap();
    session.commit().unwrap();
    writer
        .write_batch(
            &[
                BatchOp::UpdateField {
                    id: id(10),
                    field: "access_count",
                    value: ScanValue::I64(6),
                },
                BatchOp::Delete { id: id(10) },
            ],
            true,
        )
        .unwrap();
    // A duplicate insert and a missing update change nothing: not logged.
    let dup = fields_like(&mut repl, 1, "duplicate");
    assert!(writer.insert(id(1), &borrowed(&dup)).is_err());
    assert!(!writer
        .update(id(99), "access_count", ScanValue::I64(1))
        .unwrap());

    let batch = repl.fetch_since(epoch, seq, 100).unwrap();
    assert_eq!(batch.epoch, epoch);
    assert_eq!(batch.first, seq + 1);
    assert_eq!(
        batch.head,
        seq + batch.entries.len() as u64,
        "every entry is one commit unit"
    );
    assert_eq!(
        batch.entries.len(),
        4,
        "replace, delete, the session, the batch"
    );
    for ops in &batch.entries {
        replica.write_batch(ops, true).unwrap();
    }
    assert_eq!(
        sorted(&replica),
        sorted(primary.store.as_ref()),
        "the standby caught up"
    );

    // Caught up: nothing further; a position past the head is refused.
    assert!(repl
        .fetch_since(epoch, batch.head, 100)
        .unwrap()
        .entries
        .is_empty());
    assert!(matches!(
        repl.fetch_since(epoch, batch.head + 1, 100),
        Err(ClientError::Server(ErrorCode::Malformed, _))
    ));
}

/// `CHL-FR-005`: another epoch is `Gone`; the class, the version and a table
/// with no log are each refused before any entry is read.
#[test]
fn the_wrong_epoch_class_version_or_table_is_refused() {
    let primary = start(true);
    let mut repl = SchemaDrivenClient::connect_authenticated(primary.addr, TOKEN).unwrap();
    assert!(matches!(
        repl.fetch_since(99, 0, 10),
        Err(ClientError::Server(ErrorCode::Gone, _))
    ));

    let mut plain = SchemaDrivenClient::connect_authenticated(primary.addr, "ro").unwrap();
    assert!(matches!(
        plain.fetch_since(1, 0, 10),
        Err(ClientError::Server(ErrorCode::Unauthorized, _))
    ));

    let mut old = SchemaDrivenClient::connect_with(
        primary.addr,
        ConnectOptions::new().token(TOKEN).max_protocol_version(33),
    )
    .unwrap();
    assert!(matches!(
        old.fetch_since(1, 0, 10),
        Err(ClientError::Unsupported("fetch_since"))
    ));
    // A 33 connection still gets a plain snapshot, no position.
    assert_eq!(old.fetch_snapshot_at().unwrap().position, None);

    let unlogged = start(false);
    let mut repl = SchemaDrivenClient::connect_authenticated(unlogged.addr, TOKEN).unwrap();
    assert!(matches!(
        repl.fetch_since(1, 0, 10),
        Err(ClientError::Server(ErrorCode::Unsupported, _))
    ));
    assert_eq!(
        repl.fetch_snapshot_at().unwrap().position,
        None,
        "no log, a plain snapshot"
    );
}
