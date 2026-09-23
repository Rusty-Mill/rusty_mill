//! `ADR-0118` (`RRF-FR-001`–`004`): the replica-refresh CLI's library
//! against a real served table — a refresh makes a fresh, verified
//! directory the domain's own constructor reopens with the identical
//! records; a wrong token writes nothing; pruning keeps the newest.
#![cfg(feature = "server")]

#[path = "../examples/support/replica_refresh_lib.rs"]
mod replica_refresh;

use replica_refresh::{prune, refresh, snapshots, Domain, RefreshError, Target};
use rusty_multimodal_db::generic::memory::{
    create_memory_production_stack, open_memory_production_stack_portable, Memory,
};
use rusty_multimodal_db::generic::production::GenericProductionStore;
use rusty_multimodal_db::server::memory::MemoryConnectionStore;
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

fn memory(n: u128, content: &str) -> Memory {
    Memory {
        id: Uuid::from_u128(n),
        content: content.into(),
        category: "general".into(),
        tags: vec![],
        source: "manual".into(),
        metadata_json: "{}".into(),
        created_at_unix_ms: 1_000,
        updated_at_unix_ms: 1_000,
        memory_type: "unclassified".into(),
        status: "active".into(),
        sensitive: false,
        access_count: 0,
        deleted_at_unix_ms: 0,
        node_id: String::new(),
    }
}

fn start_server() -> SocketAddr {
    let source = unique_dir("replica_refresh_source");
    std::fs::create_dir_all(&source).unwrap();
    let path = source.join("memories.mmap");
    let stack =
        create_memory_production_stack(vec![memory(1, "first"), memory(2, "second")], &[], &path)
            .unwrap();
    let store = Arc::new(
        MemoryConnectionStore::new(GenericProductionStore::new(stack)).with_backup_source(path),
    );
    let listener = TcpListener::bind("127.0.0.1:0").unwrap();
    let addr = listener.local_addr().unwrap();
    let options = ServeOptions::default().with_replication_token("repl-secret".to_string());
    thread::spawn(move || serve(listener, store, options));
    addr
}

#[test]
fn a_refresh_makes_a_verified_directory_the_domain_reopens_and_pruning_keeps_the_newest() {
    let addr = start_server();
    let root = unique_dir("replica_refresh_root");
    let target = Target::new(addr.to_string(), "repl-secret");

    let first = refresh(&target, &root, Domain::Memory).unwrap();
    assert_eq!(first.records, 2);
    assert!(first.files >= 2, "slot file and blob at least: {first:?}");
    assert!(first.bytes > 0);
    let reopened = open_memory_production_stack_portable(&first.directory.join("memories.mmap"))
        .expect("the refreshed directory is a complete store");
    let store = GenericProductionStore::new(reopened);
    assert_eq!(
        store.get::<Memory>(Uuid::from_u128(2)).map(|m| m.content),
        Some("second".to_string())
    );
    assert!(
        std::fs::read_dir(&root).unwrap().all(|e| !e
            .unwrap()
            .file_name()
            .to_string_lossy()
            .starts_with(".refresh-tmp")),
        "no staging directory is left behind"
    );

    let second = refresh(&target, &root, Domain::Memory).unwrap();
    assert_ne!(second.directory, first.directory);
    assert_eq!(
        snapshots(&root).unwrap(),
        vec![first.directory.clone(), second.directory.clone()],
        "oldest first"
    );
    assert_eq!(
        prune(&root, 0).unwrap(),
        Vec::<PathBuf>::new(),
        "keep 0 removes nothing"
    );
    assert_eq!(prune(&root, 1).unwrap(), vec![first.directory.clone()]);
    assert!(!first.directory.exists());
    assert!(second.directory.exists());
}

#[test]
fn a_wrong_token_writes_nothing() {
    let addr = start_server();
    let root = unique_dir("replica_refresh_unauthorized");
    let target = Target::new(addr.to_string(), "not-the-replication-token");
    match refresh(&target, &root, Domain::Memory) {
        Err(RefreshError::Connect(_)) | Err(RefreshError::Fetch(_)) => {}
        other => panic!("expected a connect or fetch refusal, got {other:?}"),
    }
    assert!(
        !root.exists() || std::fs::read_dir(&root).unwrap().next().is_none(),
        "nothing was written under {root:?}"
    );
}
