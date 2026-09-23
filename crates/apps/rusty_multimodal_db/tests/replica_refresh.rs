//! `ADR-0118` (`RRF-FR-001`–`004`): the replica-refresh CLI's library
//! against a real served table — a refresh makes a fresh, verified
//! directory the domain's own constructor reopens with the identical
//! records; a wrong token writes nothing; pruning keeps the newest.
#![cfg(feature = "server")]

#[path = "../examples/support/replica_refresh_lib.rs"]
mod replica_refresh;

use replica_refresh::{
    is_plain_file_name, prune, refresh, refresh_loop, snapshots, Domain, RefreshError, Target,
};
use rusty_multimodal_db::generic::memory::{
    create_memory_production_stack, open_memory_production_stack_portable, Memory,
};
use rusty_multimodal_db::generic::production::GenericProductionStore;
use rusty_multimodal_db::server::client::{ClientTlsConfig, ConnectOptions, TrustPolicy};
use rusty_multimodal_db::server::memory::MemoryConnectionStore;
use rusty_multimodal_db::server::{serve, ServeOptions, TlsConfig};
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
    start_server_with(ServeOptions::default().with_replication_token("repl-secret".to_string()))
}

fn start_server_with(options: ServeOptions) -> SocketAddr {
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

    // `RGM-FR-006` (ADR-0121): an operator's own directory that happens
    // to be three numbers is neither listed nor pruned.
    std::fs::create_dir(root.join("2026-09-23")).unwrap();
    std::fs::create_dir(root.join("1700000000-1-1")).unwrap();
    let second = refresh(&target, &root, Domain::Memory).unwrap();
    assert_ne!(second.directory, first.directory);
    assert!(second
        .directory
        .file_name()
        .unwrap()
        .to_string_lossy()
        .starts_with("refresh-"));
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
    assert!(root.join("2026-09-23").exists() && root.join("1700000000-1-1").exists());
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

/// `RGM-FR-008` (ADR-0121): only a single normal path component may be
/// joined under the staging directory; anything a hostile or spoofed
/// server could use to escape it is refused before any write.
#[test]
fn only_plain_file_names_are_accepted_from_a_snapshot() {
    for ok in ["memories.mmap", "memories.mmap.records", "a b.c"] {
        assert!(is_plain_file_name(ok), "{ok}");
    }
    for bad in [
        "",
        ".",
        "..",
        "../memories.mmap",
        "/etc/passwd",
        "dir/memories.mmap",
        "dir\\memories.mmap",
        "memories.mmap/",
    ] {
        assert!(!is_plain_file_name(bad), "{bad:?}");
    }
}

/// `RGT-FR-001` (ADR-0123): a refresh over TLS — the token inside the
/// handshake — installs the same verified directory. The server presents
/// a throwaway self-signed leaf; the library target here trusts it
/// without verification, the CLI's `Target::with_tls` builds the same
/// options with the system trust anchors instead.
#[test]
fn a_refresh_over_tls_installs_a_verified_directory() {
    let rcgen::CertifiedKey { cert, key_pair } =
        rcgen::generate_simple_self_signed(vec!["localhost".to_string()]).unwrap();
    let tls = TlsConfig::new(vec![cert.der().to_vec()], key_pair.serialize_der()).unwrap();
    let addr = start_server_with(
        ServeOptions::default()
            .with_replication_token("repl-secret".to_string())
            .with_tls(tls),
    );
    let root = unique_dir("replica_refresh_tls");
    let target = Target {
        addr: addr.to_string(),
        options: ConnectOptions::new()
            .token("repl-secret")
            .tls(ClientTlsConfig::new(
                "localhost",
                TrustPolicy::DangerNoVerification,
            )),
    };
    let report = refresh(&target, &root, Domain::Memory).unwrap();
    assert_eq!(report.records, 2);
    let with_system_trust = Target::with_tls(addr.to_string(), "repl-secret", "localhost");
    match refresh(&with_system_trust, &root, Domain::Memory) {
        Err(RefreshError::Connect(_)) => {}
        other => panic!("a self-signed leaf is not in the system trust store: {other:?}"),
    }
    assert_eq!(
        snapshots(&root).unwrap().len(),
        1,
        "the refused refresh wrote nothing"
    );
}

/// `RGT-FR-002` (ADR-0123): a snapshot that fails its verification reopen
/// is kept under `.failed-`, never under a name `snapshots` lists — here
/// by asking for the wrong domain, so the fetched memory files have no
/// `entities.mmap` to reopen.
#[test]
fn a_failed_verification_is_kept_aside_and_never_listed() {
    let addr = start_server();
    let root = unique_dir("replica_refresh_failed");
    let target = Target::new(addr.to_string(), "repl-secret");
    match refresh(&target, &root, Domain::Entity) {
        Err(RefreshError::Verification { path, .. }) => {
            assert!(
                path.file_name()
                    .unwrap()
                    .to_string_lossy()
                    .starts_with(".failed-"),
                "{path:?}"
            );
            assert!(path.is_dir(), "kept for inspection: {path:?}");
        }
        other => panic!("expected a verification failure, got {other:?}"),
    }
    assert!(snapshots(&root).unwrap().is_empty());
    assert_eq!(
        prune(&root, 1).unwrap(),
        Vec::<PathBuf>::new(),
        "prune leaves .failed- alone"
    );
}

/// `RGT-FR-003` (ADR-0123): the `--every` loop refreshes, prunes and
/// reports each round until told to stop.
#[test]
fn the_refresh_loop_refreshes_prunes_and_stops_when_told() {
    let addr = start_server();
    let root = unique_dir("replica_refresh_loop");
    let target = Target::new(addr.to_string(), "repl-secret");
    let mut rounds = 0;
    let mut removed_total = 0;
    refresh_loop(
        &target,
        &root,
        Domain::Memory,
        1,
        std::time::Duration::from_millis(1),
        |outcome, pruned| {
            assert!(outcome.is_ok(), "{outcome:?}");
            removed_total += pruned.as_ref().unwrap().len();
            rounds += 1;
            rounds < 3
        },
    );
    assert_eq!(rounds, 3);
    assert_eq!(
        removed_total, 2,
        "each round after the first pruned the previous snapshot"
    );
    assert_eq!(snapshots(&root).unwrap().len(), 1);
}
