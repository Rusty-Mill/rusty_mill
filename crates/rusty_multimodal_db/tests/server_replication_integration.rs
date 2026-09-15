//! Real end-to-end coverage of `Request::FetchSnapshot` (`RPL-FR-001`,
//! ADR-0067, `docs/design/SERVER-REPLICATION-DESIGN.md`) — a real
//! `TcpListener`, a real `SchemaDrivenClient`, and a reopen of the
//! written-back directory via `open_memory_production_stack_portable`
//! to prove the streamed bytes are a real, complete store — the same
//! flagship correctness proof `tests/server_backup_integration.rs`
//! already established for `Request::Backup`. `server` feature only,
//! no `research`.

use rusty_multimodal_db::generic::memory::{
    create_memory_production_stack, open_memory_production_stack_portable, Memory,
};
use rusty_multimodal_db::generic::production::GenericProductionStore;
use rusty_multimodal_db::server::client::{ClientError, SchemaDrivenClient};
use rusty_multimodal_db::server::memory::MemoryConnectionStore;
use rusty_multimodal_db::server::protocol::{ErrorCode, MAX_SNAPSHOT_BYTES};
use rusty_multimodal_db::server::{serve, ServeOptions};
use std::io::Write;
use std::net::{SocketAddr, TcpListener};
use std::path::PathBuf;
use std::sync::Arc;
use std::thread;
use uuid::Uuid;

fn unique_dir(label: &str) -> PathBuf {
    use std::sync::atomic::{AtomicU64, Ordering};
    static COUNTER: AtomicU64 = AtomicU64::new(0);
    let n = COUNTER.fetch_add(1, Ordering::Relaxed);
    std::env::temp_dir().join(format!("{label}_{}_{n}", std::process::id()))
}

fn sample_memories() -> Vec<Memory> {
    vec![
        Memory {
            id: Uuid::from_u128(1),
            content: "first".into(),
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
        },
        Memory {
            id: Uuid::from_u128(2),
            content: "second".into(),
            category: "general".into(),
            tags: vec!["x".into()],
            source: "manual".into(),
            metadata_json: "{}".into(),
            created_at_unix_ms: 2,
            updated_at_unix_ms: 2,
            memory_type: "unclassified".into(),
            status: "active".into(),
            sensitive: false,
            access_count: 0,
            deleted_at_unix_ms: 0,
            node_id: String::new(),
        },
    ]
}

/// A server whose one table has a real `backup_source` (so
/// `fetch_snapshot` can answer at all, independent of whether a caller's
/// token grants `TokenClass::Replication`) — `options` decides the auth
/// story per test, exactly `server_backup_integration.rs`'s
/// `start_server_with_root` shape.
fn start_server(options: ServeOptions) -> (SocketAddr, PathBuf) {
    let source_dir = unique_dir("replication_integration_source");
    std::fs::create_dir_all(&source_dir).unwrap();
    let path = source_dir.join("memories.mmap");
    let stack = create_memory_production_stack(sample_memories(), &[], &path).unwrap();
    let connection_store = Arc::new(
        MemoryConnectionStore::new(GenericProductionStore::new(stack)).with_backup_source(path),
    );
    let listener = TcpListener::bind("127.0.0.1:0").unwrap();
    let addr = listener.local_addr().unwrap();
    thread::spawn(move || serve(listener, connection_store, options));
    (addr, source_dir)
}

/// Write `files` (as `fetch_snapshot` returned them) into a fresh
/// `target_dir`, one file at a time, temp-then-rename — the same
/// crash-safe shape `durability::mmap_store::write_via_rename` and
/// `generic::insert_log`'s own rewrite already use, so a crash mid-write
/// never leaves a half-written file at its final name.
fn write_snapshot(target_dir: &std::path::Path, files: &[(String, Vec<u8>)]) {
    std::fs::create_dir_all(target_dir).unwrap();
    for (name, bytes) in files {
        let final_path = target_dir.join(name);
        let mut tmp_name = name.clone();
        tmp_name.push_str(".rewrite-tmp");
        let tmp_path = target_dir.join(tmp_name);
        let mut f = std::fs::File::create(&tmp_path).unwrap();
        f.write_all(bytes).unwrap();
        f.flush().unwrap();
        drop(f);
        std::fs::rename(&tmp_path, &final_path).unwrap();
    }
}

/// `RPL-FR-001`/`RPL-FR-004`/`RPL-FR-006`: the streamed-back files are a
/// byte-for-byte, complete, reopenable store with the identical record
/// set — the flagship correctness proof, not just "some bytes arrived."
#[test]
fn fetch_snapshot_produces_a_reopenable_directory_with_the_identical_records() {
    let (addr, _source) =
        start_server(ServeOptions::default().with_replication_token("repl-secret".to_string()));
    let mut client = SchemaDrivenClient::connect_authenticated(addr, "repl-secret").unwrap();
    let files = client.fetch_snapshot().unwrap();
    assert!(
        files.len() >= 2,
        "expected at least the slot file and its companion blob, got {}",
        files.len()
    );
    assert!(files.iter().all(|(_, bytes)| !bytes.is_empty()));

    let restored_dir = unique_dir("replication_integration_restored");
    write_snapshot(&restored_dir, &files);
    let restored = open_memory_production_stack_portable(&restored_dir.join("memories.mmap"))
        .expect("the streamed-back files must be a complete, reopenable store");
    let store = GenericProductionStore::new(restored);
    assert_eq!(
        store.get::<Memory>(Uuid::from_u128(1)).map(|m| m.content),
        Some("first".to_string())
    );
    assert_eq!(
        store.get::<Memory>(Uuid::from_u128(2)).map(|m| m.content),
        Some("second".to_string())
    );
}

/// `RPL-FR-002`: no `SERVER_AUTH_REPLICATION_TOKEN`-equivalent
/// configured — the connection starts at `ReadWrite`
/// (`AUTH-FR-007`'s no-tokens-configured default), and `FetchSnapshot`
/// answers `Unauthorized`: zero new network-egress surface by default.
#[test]
fn fetch_snapshot_is_unauthorized_with_no_configured_replication_token() {
    let (addr, _source) = start_server(ServeOptions::default());
    let mut client = SchemaDrivenClient::connect(addr).unwrap();
    match client.fetch_snapshot() {
        Err(ClientError::Server(ErrorCode::Unauthorized, _)) => {}
        other => panic!("expected Unauthorized, got {other:?}"),
    }
}

/// `RPL-FR-002`: the acceptance criterion this whole feature exists to
/// satisfy — reopening `ADR-0065`'s own declined option (c) only behind
/// a *separate* credential. A `ReadWrite` token (even a real,
/// server-configured one) never satisfies `TokenClass::Replication`.
#[test]
fn fetch_snapshot_is_unauthorized_for_a_read_write_token() {
    let (addr, _source) = start_server(ServeOptions::new(
        Some("ro-secret".into()),
        Some("rw-secret".into()),
    ));
    let mut client = SchemaDrivenClient::connect_authenticated(addr, "rw-secret").unwrap();
    match client.fetch_snapshot() {
        Err(ClientError::Server(ErrorCode::Unauthorized, _)) => {}
        other => panic!("expected Unauthorized, got {other:?}"),
    }
}

/// `RPL-FR-002`: a `ReadOnly` token is refused the identical way — the
/// gate is "must be exactly `Replication`," not "must not be
/// `ReadWrite`."
#[test]
fn fetch_snapshot_is_unauthorized_for_a_read_only_token() {
    let (addr, _source) = start_server(ServeOptions::new(
        Some("ro-secret".into()),
        Some("rw-secret".into()),
    ));
    let mut client = SchemaDrivenClient::connect_authenticated(addr, "ro-secret").unwrap();
    match client.fetch_snapshot() {
        Err(ClientError::Server(ErrorCode::Unauthorized, _)) => {}
        other => panic!("expected Unauthorized, got {other:?}"),
    }
}

/// `RPL-FR-007`: a table whose on-disk size exceeds `MAX_SNAPSHOT_BYTES`
/// is refused `TooLarge` — proven by padding the real on-disk blob past
/// the ceiling directly (no need for thousands of inserted records) and
/// confirming the call still returns quickly with nothing streamed,
/// rather than partially reading the oversized file.
#[test]
fn fetch_snapshot_refuses_a_table_over_the_size_ceiling() {
    let (addr, source_dir) =
        start_server(ServeOptions::default().with_replication_token("repl-secret".to_string()));
    let blob_path = source_dir.join("memories.mmap");
    {
        let mut f = std::fs::OpenOptions::new()
            .append(true)
            .open(&blob_path)
            .unwrap();
        let padding = vec![0u8; (MAX_SNAPSHOT_BYTES + 1) as usize];
        f.write_all(&padding).unwrap();
    }
    let mut client = SchemaDrivenClient::connect_authenticated(addr, "repl-secret").unwrap();
    match client.fetch_snapshot() {
        Err(ClientError::Server(ErrorCode::TooLarge, _)) => {}
        other => panic!("expected TooLarge, got {other:?}"),
    }
}
