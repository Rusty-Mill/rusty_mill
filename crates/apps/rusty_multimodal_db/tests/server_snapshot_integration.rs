//! `ADR-0136` (`CSN-FR-001`–`005`): chunked snapshots end to end — a real
//! server, a table padded past `MAX_SNAPSHOT_BYTES` (a companion file the
//! reopen ignores, so the directory stays a real store), and both the client's
//! `fetch_snapshot_chunked` and raw frames for what the client never sends.
#![cfg(feature = "server")]

use rusty_multimodal_db::generic::memory::{
    create_memory_production_stack, open_memory_production_stack_portable, Memory,
};
use rusty_multimodal_db::generic::production::GenericProductionStore;
use rusty_multimodal_db::generic::query::AllIds;
use rusty_multimodal_db::server::client::{ClientError, SchemaDrivenClient};
use rusty_multimodal_db::server::framing::{read_message, write_message};
use rusty_multimodal_db::server::memory::MemoryConnectionStore;
use rusty_multimodal_db::server::protocol::{
    DomainSchema, ErrorCode, RelationCapabilities, Request, Response, MAX_CHUNK_BYTES,
    MAX_SNAPSHOT_BYTES, PROTOCOL_VERSION,
};
use rusty_multimodal_db::server::{serve, ServeOptions};
use sha2::{Digest, Sha256};
use std::io::{BufReader, BufWriter, Write};
use std::net::{SocketAddr, TcpListener, TcpStream};
use std::path::{Path, PathBuf};
use std::sync::{mpsc, Arc};
use std::thread;
use std::time::{Duration, Instant};
use uuid::Uuid;

const PAD_BYTES: usize = MAX_SNAPSHOT_BYTES as usize + 1024 * 1024;

fn unique_dir(label: &str) -> PathBuf {
    use std::sync::atomic::{AtomicU64, Ordering};
    static COUNTER: AtomicU64 = AtomicU64::new(0);
    let n = COUNTER.fetch_add(1, Ordering::Relaxed);
    std::env::temp_dir().join(format!("{label}_{}_{n}", std::process::id()))
}

fn memory(n: u128) -> Memory {
    Memory {
        id: Uuid::from_u128(n),
        content: format!("memory {n}"),
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
    }
}

/// A server over a table padded past the legacy ceiling; `staging` is the
/// `(dir, max_mb)` to enable `BeginSnapshot` with, `None` to leave it off.
/// Returns the address and the table's directory.
fn start(staging: Option<(&Path, u64)>) -> (SocketAddr, PathBuf) {
    let dir = unique_dir("snapshot_source");
    std::fs::create_dir_all(&dir).unwrap();
    let path = dir.join("memories.mmap");
    let stack = create_memory_production_stack(vec![memory(1), memory(2)], &[], &path).unwrap();
    // A pattern that is not periodic in the chunk size, so a chunk read from
    // the wrong offset cannot hash the same.
    let pad: Vec<u8> = (0..PAD_BYTES)
        .map(|i| (i as u32).wrapping_mul(2_654_435_761).to_le_bytes()[2])
        .collect();
    std::fs::write(dir.join("memories.mmap.pad"), pad).unwrap();
    let store = Arc::new(
        MemoryConnectionStore::new(GenericProductionStore::new(stack)).with_backup_source(path),
    );
    let mut options =
        ServeOptions::new(None, Some("rw".into())).with_replication_token("repl".to_string());
    if let Some((staging_dir, max_mb)) = staging {
        options = options
            .with_snapshot_staging(staging_dir.to_path_buf(), max_mb)
            .unwrap();
    }
    let listener = TcpListener::bind("127.0.0.1:0").unwrap();
    let addr = listener.local_addr().unwrap();
    thread::spawn(move || serve(listener, store, options));
    (addr, dir)
}

type Raw = (BufReader<TcpStream>, BufWriter<TcpStream>);

/// A raw connection negotiated at `version`, authenticated with `token`.
fn raw(addr: SocketAddr, version: u32, token: &str) -> Raw {
    let stream = TcpStream::connect(addr).unwrap();
    stream
        .set_read_timeout(Some(Duration::from_secs(10)))
        .unwrap();
    let mut conn = (
        BufReader::new(stream.try_clone().unwrap()),
        BufWriter::new(stream),
    );
    let hello = Request::Hello {
        protocol_version: version,
    };
    assert!(matches!(call(&mut conn, &hello), Response::Hello { .. }));
    let auth = Request::Authenticate {
        token: token.into(),
    };
    assert_eq!(call(&mut conn, &auth), Response::Ok);
    conn
}

fn call(conn: &mut Raw, request: &Request) -> Response {
    write_message(&mut conn.1, request).unwrap();
    conn.1.flush().unwrap();
    read_message::<_, Response>(&mut conn.0).unwrap()
}

fn code(response: Response) -> ErrorCode {
    match response {
        Response::Err { code, .. } => code,
        other => panic!("expected an error, got {other:?}"),
    }
}

/// `BeginSnapshot` on `conn`, returning the handle.
fn begin(conn: &mut Raw) -> u64 {
    match call(conn, &Request::BeginSnapshot) {
        Response::SnapshotManifest { snapshot, .. } => snapshot,
        other => panic!("expected a manifest, got {other:?}"),
    }
}

fn wait_until(what: &str, mut done: impl FnMut() -> bool) {
    let deadline = Instant::now() + Duration::from_secs(5);
    while !done() {
        assert!(Instant::now() < deadline, "timed out waiting for {what}");
        thread::sleep(Duration::from_millis(20));
    }
}

#[test]
fn a_table_over_the_legacy_ceiling_streams_to_an_identical_reopenable_directory() {
    let staging = unique_dir("snapshot_staging");
    let (addr, source) = start(Some((&staging, 64)));
    let mut client = SchemaDrivenClient::connect_authenticated(addr, "repl").unwrap();
    assert!(matches!(
        client.fetch_snapshot(),
        Err(ClientError::Server(ErrorCode::TooLarge, _))
    ));

    let target = unique_dir("snapshot_target");
    std::fs::create_dir_all(&target).unwrap();
    let done = client.fetch_snapshot_chunked(&target).unwrap();
    assert_eq!(done.position, None, "this table has no change log");
    assert!(done.bytes > MAX_SNAPSHOT_BYTES);

    let mut names: Vec<_> = std::fs::read_dir(&source)
        .unwrap()
        .map(|e| e.unwrap().file_name())
        .collect();
    names.sort();
    assert_eq!(done.files as usize, names.len());
    for name in &names {
        let copied = std::fs::read(target.join(name)).unwrap();
        assert_eq!(
            copied,
            std::fs::read(source.join(name)).unwrap(),
            "{name:?}"
        );
    }
    let reopened = open_memory_production_stack_portable(&target.join("memories.mmap")).unwrap();
    assert_eq!(reopened.all_ids().len(), 2);
    wait_until("the staged copy to be freed by EndSnapshot", || {
        !staging.join("snap-0").exists()
    });
}

#[test]
fn begin_snapshot_is_refused_without_the_replication_token_staging_or_room() {
    let staging = unique_dir("snapshot_staging");
    let (addr, _) = start(Some((&staging, 64)));
    let mut writer = raw(addr, PROTOCOL_VERSION, "rw");
    for request in [
        Request::BeginSnapshot,
        Request::FetchChunk {
            snapshot: 1,
            file: 0,
            offset: 0,
            len: 1,
        },
        Request::EndSnapshot { snapshot: 1 },
    ] {
        assert_eq!(code(call(&mut writer, &request)), ErrorCode::Unauthorized);
    }

    let (addr, _) = start(None);
    let mut conn = raw(addr, PROTOCOL_VERSION, "repl");
    assert_eq!(
        code(call(&mut conn, &Request::BeginSnapshot)),
        ErrorCode::Unsupported,
        "no SERVER_SNAPSHOT_DIR"
    );

    let staging = unique_dir("snapshot_staging");
    let (addr, _) = start(Some((&staging, 1)));
    let mut conn = raw(addr, PROTOCOL_VERSION, "repl");
    assert_eq!(
        code(call(&mut conn, &Request::BeginSnapshot)),
        ErrorCode::TooLarge,
        "9 MiB against a 1 MiB ceiling"
    );
    assert!(!staging.join("snap-0").exists(), "nothing was copied");
}

#[test]
fn a_handle_belongs_to_its_connection_and_the_table_has_one_slot() {
    let staging = unique_dir("snapshot_staging");
    let (addr, _) = start(Some((&staging, 64)));
    let mut a = raw(addr, PROTOCOL_VERSION, "repl");
    let mut b = raw(addr, PROTOCOL_VERSION, "repl");
    let handle = begin(&mut a);

    let chunk = |snapshot, len| Request::FetchChunk {
        snapshot,
        file: 0,
        offset: 0,
        len,
    };
    assert_eq!(code(call(&mut b, &chunk(handle, 1))), ErrorCode::NoSnapshot);
    assert_eq!(
        code(call(&mut b, &Request::EndSnapshot { snapshot: handle })),
        ErrorCode::NoSnapshot
    );
    assert_eq!(code(call(&mut b, &Request::BeginSnapshot)), ErrorCode::Busy);
    assert_eq!(
        code(call(&mut a, &Request::BeginSnapshot)),
        ErrorCode::Busy,
        "one staged snapshot per connection"
    );

    assert_eq!(
        code(call(&mut a, &chunk(handle ^ 1, 1))),
        ErrorCode::NoSnapshot
    );
    assert_eq!(
        code(call(&mut a, &chunk(handle, MAX_CHUNK_BYTES + 1))),
        ErrorCode::Malformed
    );
    let past = Request::FetchChunk {
        snapshot: handle,
        file: 99,
        offset: 0,
        len: 1,
    };
    assert_eq!(code(call(&mut a, &past)), ErrorCode::Malformed);
    match call(&mut a, &chunk(handle, 16)) {
        Response::Chunk { bytes } => assert_eq!(bytes.len(), 16),
        other => panic!("{other:?}"),
    }

    // The connection ending frees the slot and the directory.
    drop(a);
    wait_until("the closed connection's copy to be freed", || {
        !staging.join("snap-0").exists()
    });
    let again = begin(&mut b);
    assert_eq!(
        call(&mut b, &Request::EndSnapshot { snapshot: again }),
        Response::Ok
    );
    assert_eq!(
        code(call(&mut b, &chunk(again, 1))),
        ErrorCode::NoSnapshot,
        "ended"
    );
}

#[test]
fn a_connection_negotiated_below_36_cannot_ask() {
    let staging = unique_dir("snapshot_staging");
    let (addr, _) = start(Some((&staging, 64)));
    let mut conn = raw(addr, 35, "repl");
    assert_eq!(
        code(call(&mut conn, &Request::BeginSnapshot)),
        ErrorCode::Malformed
    );
    assert!(!staging.join("snap-0").exists());
}

/// A scripted server: answers `files` as the manifest, every chunk as the
/// bytes `chunk`, and reports each request it sees (after the handshake) on
/// the returned channel, so a test can tell what the client did and did not ask.
fn fake_snapshot_server(
    files: Vec<(String, u64, [u8; 32])>,
    chunk: Vec<u8>,
) -> (SocketAddr, mpsc::Receiver<Request>) {
    let listener = TcpListener::bind("127.0.0.1:0").unwrap();
    let addr = listener.local_addr().unwrap();
    let (seen, requests) = mpsc::channel();
    thread::spawn(move || {
        let (stream, _) = listener.accept().unwrap();
        let mut conn = (
            BufReader::new(stream.try_clone().unwrap()),
            BufWriter::new(stream),
        );
        while let Ok(request) = read_message::<_, Request>(&mut conn.0) {
            let response = match &request {
                Request::Hello { .. } => Response::Hello {
                    protocol_version: PROTOCOL_VERSION,
                },
                Request::DescribeSchema => Response::Schema(DomainSchema {
                    fields: vec![],
                    relations: RelationCapabilities {
                        parent_children: false,
                        neighbors: false,
                    },
                }),
                Request::DescribeRelations => Response::Relations { relations: vec![] },
                Request::BeginSnapshot => Response::SnapshotManifest {
                    snapshot: 1,
                    position: None,
                    files: files.clone(),
                },
                Request::FetchChunk { .. } => Response::Chunk {
                    bytes: chunk.clone(),
                },
                _ => Response::Ok,
            };
            let _ = seen.send(request);
            write_message(&mut conn.1, &response).unwrap();
            conn.1.flush().unwrap();
        }
    });
    (addr, requests)
}

fn digest(bytes: &[u8]) -> [u8; 32] {
    <[u8; 32]>::from(Sha256::digest(bytes))
}

/// A server that answers a manifest whose digest is wrong: the client must
/// refuse the download rather than install bytes it cannot vouch for.
#[test]
fn the_client_rejects_a_chunk_stream_that_does_not_match_the_manifest() {
    let (addr, _) = fake_snapshot_server(vec![("t".into(), 3, digest(b"abc"))], b"abd".to_vec());
    let target = unique_dir("snapshot_target");
    std::fs::create_dir_all(&target).unwrap();
    let mut client = SchemaDrivenClient::connect(addr).unwrap();
    match client.fetch_snapshot_chunked(&target) {
        Err(ClientError::Snapshot(message)) => assert!(message.contains("SHA-256"), "{message}"),
        other => panic!("expected a Snapshot error, got {other:?}"),
    }
}

/// The manifest is the server's word, so a name that is not one plain file name
/// on every platform — a drive-relative `C:escape`, an absolute or UNC path, a
/// separator, a dot entry — is refused before any file is created or truncated
/// (even a good name listed first), no chunk is fetched, and the staged copy
/// is still released.
#[test]
fn a_hostile_manifest_name_is_refused_before_anything_is_written() {
    for bad in [
        "C:escape",
        "c:\\windows\\x",
        "\\\\server\\share\\x",
        "/etc/passwd",
        "..\\x",
        "sub/x",
        "..",
        "x:stream",
        "NUL",
        "con.txt",
    ] {
        let good = b"abc";
        let files = vec![
            ("good".to_string(), 3, digest(good)),
            (bad.to_string(), 3, digest(good)),
        ];
        let (addr, requests) = fake_snapshot_server(files, good.to_vec());
        let target = unique_dir("snapshot_target");
        std::fs::create_dir_all(&target).unwrap();
        let mut client = SchemaDrivenClient::connect(addr).unwrap();
        match client.fetch_snapshot_chunked(&target) {
            Err(ClientError::Snapshot(message)) => {
                assert!(
                    message.contains("not a plain file name"),
                    "{bad}: {message}"
                )
            }
            other => panic!("{bad}: expected a Snapshot error, got {other:?}"),
        }
        assert_eq!(std::fs::read_dir(&target).unwrap().count(), 0, "{bad}");
        drop(client);
        let asked: Vec<_> = requests.try_iter().collect();
        assert!(
            asked
                .iter()
                .all(|r| !matches!(r, Request::FetchChunk { .. })),
            "{bad}: no chunk is fetched for a refused manifest"
        );
        assert!(
            asked
                .iter()
                .any(|r| matches!(r, Request::EndSnapshot { snapshot: 1 })),
            "{bad}: the staged copy is still released"
        );
    }
}

/// The same server, an ordinary manifest: the download works and lands every
/// file under the target directory.
#[test]
fn ordinary_snapshot_names_still_download() {
    let names = [
        "memories.mmap",
        "memories.mmap.records",
        "memories.mmap.knows.edges",
        "a b.c",
    ];
    let files = names
        .iter()
        .map(|n| (n.to_string(), 3, digest(b"abc")))
        .collect();
    let (addr, _) = fake_snapshot_server(files, b"abc".to_vec());
    let target = unique_dir("snapshot_target");
    std::fs::create_dir_all(&target).unwrap();
    let mut client = SchemaDrivenClient::connect(addr).unwrap();
    let done = client.fetch_snapshot_chunked(&target).unwrap();
    assert_eq!((done.files, done.bytes), (4, 12));
    for name in names {
        assert_eq!(std::fs::read(target.join(name)).unwrap(), b"abc", "{name}");
    }
}
