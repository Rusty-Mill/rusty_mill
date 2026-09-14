//! Real end-to-end coverage of `Request::Backup` (`BAK-FR-001`,
//! ADR-0065, `docs/design/SERVER-BACKUP-DESIGN.md`) — a real
//! `TcpListener`, a real `SchemaDrivenClient`, and a reopen of the
//! produced directory via `open_memory_production_stack_portable` to
//! prove the copy is real, not just "some bytes moved." `server`
//! feature only, no `research`.

use rusty_multimodal_db::generic::memory::{
    create_memory_production_stack, open_memory_production_stack_portable, Memory,
};
use rusty_multimodal_db::generic::production::GenericProductionStore;
use rusty_multimodal_db::server::client::{ClientError, SchemaDrivenClient};
use rusty_multimodal_db::server::memory::MemoryConnectionStore;
use rusty_multimodal_db::server::protocol::ErrorCode;
use rusty_multimodal_db::server::{serve, ServeOptions};
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

/// `backup_root: None` — a server with no configured root, matching
/// `Unsupported` (`BAK-FR-003`) unless a test opts in via
/// `start_server_with_root`.
fn start_server() -> (SocketAddr, PathBuf) {
    let source_dir = unique_dir("backup_integration_source");
    std::fs::create_dir_all(&source_dir).unwrap();
    let path = source_dir.join("memories.mmap");
    let stack = create_memory_production_stack(sample_memories(), &[], &path).unwrap();
    let connection_store = Arc::new(MemoryConnectionStore::new(GenericProductionStore::new(
        stack,
    )));
    let listener = TcpListener::bind("127.0.0.1:0").unwrap();
    let addr = listener.local_addr().unwrap();
    thread::spawn(move || serve(listener, connection_store, ServeOptions::default()));
    (addr, source_dir)
}

fn start_server_with_root(options: ServeOptions) -> (SocketAddr, PathBuf) {
    let source_dir = unique_dir("backup_integration_source");
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

/// `BAK-FR-006`: the produced directory is byte-for-byte a reopenable
/// store with the identical record set — the flagship correctness
/// proof, not just "some files landed."
#[test]
fn backup_produces_a_reopenable_directory_with_the_identical_records() {
    let root = unique_dir("backup_integration_root");
    let (addr, _source) =
        start_server_with_root(ServeOptions::default().with_backup_root(root.clone()));
    let mut client = SchemaDrivenClient::connect(addr).unwrap();
    let (files, bytes) = client.backup("nightly").unwrap();
    assert!(
        files >= 2,
        "expected at least the slot file and its companion blob, got {files}"
    );
    assert!(bytes > 0);

    let restored = open_memory_production_stack_portable(&root.join("nightly/memories.mmap"))
        .expect("the backup directory must be a complete, reopenable store");
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

/// `BAK-FR-004`: a `name` containing a path separator or `..` is
/// refused before any I/O — the target root is never touched.
#[test]
fn backup_refuses_a_name_that_is_not_a_single_path_component() {
    let root = unique_dir("backup_integration_root");
    let (addr, _source) =
        start_server_with_root(ServeOptions::default().with_backup_root(root.clone()));
    let mut client = SchemaDrivenClient::connect(addr).unwrap();
    for bad in ["../escape", "a/b", "a\\b", "..", "."] {
        match client.backup(bad) {
            Err(ClientError::Server(ErrorCode::Malformed, _)) => {}
            other => panic!("name {bad:?}: expected Malformed, got {other:?}"),
        }
    }
    assert!(
        !root.exists() || std::fs::read_dir(&root).unwrap().next().is_none(),
        "a refused name must never create anything under the backup root"
    );
}

/// `BAK-FR-005`: a name that already names an existing directory is
/// refused, not silently overwritten.
#[test]
fn backup_refuses_an_existing_target() {
    let root = unique_dir("backup_integration_root");
    let (addr, _source) =
        start_server_with_root(ServeOptions::default().with_backup_root(root.clone()));
    let mut client = SchemaDrivenClient::connect(addr).unwrap();
    client.backup("nightly").unwrap();
    match client.backup("nightly") {
        Err(ClientError::Server(ErrorCode::Storage, _)) => {}
        other => panic!("expected Storage on a repeat name, got {other:?}"),
    }
}

/// `BAK-FR-003`: no `SERVER_BACKUP_ROOT`-equivalent configured — every
/// `Backup` request answers `Unsupported`, zero new filesystem-write
/// surface by default.
#[test]
fn backup_is_unsupported_with_no_configured_root() {
    let (addr, _source) = start_server();
    let mut client = SchemaDrivenClient::connect(addr).unwrap();
    match client.backup("nightly") {
        Err(ClientError::Server(ErrorCode::Unsupported, _)) => {}
        other => panic!("expected Unsupported, got {other:?}"),
    }
}

/// `BAK-FR-001`: gated like `Compact` — `Unauthorized` for a `ReadOnly`
/// token.
#[test]
fn backup_is_unauthorized_for_a_read_only_token() {
    let root = unique_dir("backup_integration_root");
    let (addr, _source) = start_server_with_root(
        ServeOptions::new(Some("ro-secret".into()), Some("rw-secret".into()))
            .with_backup_root(root),
    );
    let mut client = SchemaDrivenClient::connect_authenticated(addr, "ro-secret").unwrap();
    match client.backup("nightly") {
        Err(ClientError::Server(ErrorCode::Unauthorized, _)) => {}
        other => panic!("expected Unauthorized, got {other:?}"),
    }
}
