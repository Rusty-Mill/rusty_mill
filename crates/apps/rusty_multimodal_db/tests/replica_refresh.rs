//! `ADR-0118` (`RRF-FR-001`–`004`): the replica-refresh CLI's library
//! against a real served table — a refresh makes a fresh, verified
//! directory the domain's own constructor reopens with the identical
//! records; a wrong token writes nothing; pruning keeps the newest.
#![cfg(feature = "server")]

#[path = "../examples/support/replica_refresh_lib.rs"]
mod replica_refresh;

use replica_refresh::{
    first_real_failure, follow, is_plain_file_name, prune, read_position, refresh, refresh_at,
    refresh_loop, snapshots, write_position, Domain, FollowError, Position, RefreshError, Target,
};
use rusty_multimodal_db::generic::memory::{
    create_memory_production_stack, open_memory_production_stack_portable, Memory,
};
use rusty_multimodal_db::generic::production::GenericProductionStore;
use rusty_multimodal_db::generic::query::AllIds;
use rusty_multimodal_db::server::changelog::{ChangeLog, DEFAULT_RETAIN_BYTES};
use rusty_multimodal_db::server::changelogged::ChangeLogged;
use rusty_multimodal_db::server::client::{BatchOp, SchemaDrivenClient};
use rusty_multimodal_db::server::client::{
    ClientError, ClientTlsConfig, ConnectOptions, TrustPolicy,
};
use rusty_multimodal_db::server::memory::MemoryConnectionStore;
use rusty_multimodal_db::server::protocol::{ErrorCode, ScanValue, WriteOp};
use rusty_multimodal_db::server::{serve, serve_tables, ConnectionStore, ServeOptions, TlsConfig};
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
        "C:escape",
        "C:\\escape",
        "\\\\server\\share",
        "x:stream",
        "NUL",
        "COM1.txt",
        "trailing.",
        "trailing ",
        "wild*card",
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

/// A `Memory` primary with a change log, on a real socket.
fn start_logged_server() -> SocketAddr {
    start_logged_server_with(0, None)
}

/// [`start_logged_server`] with a companion file of `pad` bytes (a table over
/// the legacy snapshot ceiling that still reopens) and, if given, chunked
/// snapshots staged under that directory.
fn start_logged_server_with(pad: usize, staging: Option<&std::path::Path>) -> SocketAddr {
    let dir = unique_dir("replica_follow_source");
    std::fs::create_dir_all(&dir).unwrap();
    let path = dir.join("memories.mmap");
    let stack =
        create_memory_production_stack(vec![memory(1, "first"), memory(2, "second")], &[], &path)
            .unwrap();
    std::fs::write(dir.join("memories.mmap.pad"), vec![7u8; pad]).unwrap();
    let store = Arc::new(
        MemoryConnectionStore::new(GenericProductionStore::new(stack)).with_backup_source(path),
    );
    let log = ChangeLog::open(&dir.join("memory.changes"), DEFAULT_RETAIN_BYTES).unwrap();
    let served: Arc<dyn ConnectionStore> = Arc::new(ChangeLogged::new(store, Arc::new(log)));
    let mut options = ServeOptions::new(Some("ro".to_string()), Some("rw".to_string()))
        .with_replication_token("repl-secret".to_string());
    if let Some(staging) = staging {
        options = options
            .with_snapshot_staging(staging.to_path_buf(), 64)
            .unwrap();
    }
    let listener = TcpListener::bind("127.0.0.1:0").unwrap();
    let addr = listener.local_addr().unwrap();
    thread::spawn(move || serve_tables(listener, vec![("memory".to_string(), served)], 0, options));
    addr
}

/// `ADR-0136`: a table over the frame cap refreshes through chunks, the log
/// position rides along so `follow` can continue, and the server's staged copy
/// is gone afterwards; without staging the refusal says so.
#[test]
fn a_table_over_the_legacy_ceiling_refreshes_in_chunks_with_its_position() {
    let staging = unique_dir("replica_chunk_staging");
    let addr = start_logged_server_with(9 << 20, Some(&staging));
    let root = unique_dir("replica_chunk_root");
    let target = Target::new(addr.to_string(), "repl-secret");
    let report = refresh_at(&target, &root, Domain::Memory).unwrap();
    assert!(report.bytes > 9 << 20, "{}", report.bytes);
    assert_eq!(report.records, 2);
    assert!(report.position.is_some(), "a logged table gives a position");
    assert_eq!(
        read_position(&report.directory).unwrap(),
        report.position.unwrap()
    );
    let pad = report.directory.join("memories.mmap.pad");
    assert_eq!(std::fs::metadata(pad).unwrap().len(), 9 << 20);
    for _ in 0..100 {
        if !staging.join("snap-0").exists() {
            return;
        }
        thread::sleep(std::time::Duration::from_millis(20));
    }
    panic!("the server's staged copy was not freed");
}

#[test]
fn a_table_over_the_legacy_ceiling_is_refused_where_staging_is_off() {
    let addr = start_logged_server_with(9 << 20, None);
    let root = unique_dir("replica_chunk_root");
    let target = Target::new(addr.to_string(), "repl-secret");
    match refresh_at(&target, &root, Domain::Memory) {
        Err(RefreshError::Fetch(ClientError::Server(ErrorCode::TooLarge, _))) => {}
        other => panic!("expected the legacy TooLarge, got {other:?}"),
    }
    assert!(
        snapshots(&root).unwrap().is_empty(),
        "nothing was left behind"
    );
}

/// `ADR-0131` phase 2: a refresh records the log position, `follow`
/// applies what the primary wrote afterwards, and the standby directory
/// reopens with the primary's records; a stale epoch is `Resync`.
#[test]
fn follow_applies_the_primarys_later_writes_to_a_refreshed_directory() {
    let addr = start_logged_server();
    let root = unique_dir("replica_follow_root");
    let target = Target::new(addr.to_string(), "repl-secret");
    let report = refresh_at(&target, &root, Domain::Memory).unwrap();
    let start = report.position.expect("a logged table gives a position");
    assert_eq!(read_position(&report.directory).unwrap(), start);

    let mut writer = SchemaDrivenClient::connect_authenticated(addr, "rw").unwrap();
    let mut fields: Vec<(String, ScanValue)> = writer.get(Uuid::from_u128(1)).unwrap().unwrap();
    for (name, value) in fields.iter_mut() {
        if name == "content" {
            *value = ScanValue::Str("first, edited".into());
        }
    }
    let borrowed: Vec<(&str, ScanValue)> = fields
        .iter()
        .map(|(n, v)| (n.as_str(), v.clone()))
        .collect();
    writer
        .write_batch(
            &[
                BatchOp::Delete {
                    id: Uuid::from_u128(2),
                },
                BatchOp::Replace {
                    id: Uuid::from_u128(1),
                    fields: &borrowed,
                },
            ],
            true,
        )
        .unwrap();

    let mut seen = (0, 0);
    follow(
        &target,
        &report.directory,
        Domain::Memory,
        std::time::Duration::from_millis(10),
        |applied, head| {
            seen = (applied, head);
            applied < head || applied == start.seq
        },
    )
    .unwrap();
    assert!(seen.0 > start.seq && seen.0 == seen.1, "{seen:?}");
    assert_eq!(read_position(&report.directory).unwrap().seq, seen.0);

    let standby =
        open_memory_production_stack_portable(&report.directory.join("memories.mmap")).unwrap();
    assert_eq!(standby.all_ids(), vec![Uuid::from_u128(1)]);

    write_position(
        &report.directory,
        Position {
            epoch: start.epoch + 1,
            seq: start.seq,
        },
    )
    .unwrap();
    let stale = follow(
        &target,
        &report.directory,
        Domain::Memory,
        std::time::Duration::from_millis(10),
        |_, _| false,
    );
    assert!(matches!(stale, Err(FollowError::Resync)), "{stale:?}");
}

/// Review 2.3 (R2): a logged `[Link(A, B), Delete(A)]` applied, then applied
/// again after a crash ahead of the position write, is not divergence: the
/// replayed link fails `RecordNotFound` because the batch itself deletes A.
/// A link to a missing record that nothing deletes still is.
#[test]
fn a_replayed_link_then_delete_batch_is_not_divergence() {
    let dir = unique_dir("replica_follow_link_delete");
    std::fs::create_dir_all(&dir).unwrap();
    let stack = create_memory_production_stack(
        vec![memory(1, "a"), memory(2, "b")],
        &[],
        &dir.join("memories"),
    )
    .unwrap();
    let standby = MemoryConnectionStore::new(GenericProductionStore::new(stack));
    let (a, b) = (Uuid::from_u128(1), Uuid::from_u128(2));
    let entry = vec![
        WriteOp::Link {
            left: a,
            right: b,
            relation: "mentions".into(),
        },
        WriteOp::Delete { id: a },
    ];
    let first = standby.write_batch(&entry, false).unwrap();
    assert_eq!(first_real_failure(&entry, &first), None);
    let replayed = standby.write_batch(&entry, false).unwrap();
    assert_eq!(first_real_failure(&entry, &replayed), None);

    let dangling = vec![WriteOp::Link {
        left: a,
        right: b,
        relation: "mentions".into(),
    }];
    let results = standby.write_batch(&dangling, false).unwrap();
    assert_eq!(
        first_real_failure(&dangling, &results),
        Some((0, ErrorCode::RecordNotFound))
    );
}
