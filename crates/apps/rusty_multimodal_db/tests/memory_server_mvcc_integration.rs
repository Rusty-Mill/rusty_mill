//! Real end-to-end coverage of `memory_server`'s `SERVER_MVCC_ISOLATION`
//! deployment wiring (`ADR-0072`) — the actual compiled binary, spawned
//! as a subprocess (`env!("CARGO_BIN_EXE_memory_server")`, mirroring
//! `tests/server_metrics_http_integration.rs`'s own subprocess pattern),
//! not an in-process `serve()` call. `tests/server_memory_mvcc_
//! integration.rs` already proves `MemoryConnectionStore::with_mvcc`
//! itself is correct in-process; this file proves the separate `main()`
//! wiring in `src/bin/memory_server.rs` — the `SERVER_MVCC_ISOLATION`
//! env var, and its file-existence check choosing `with_mvcc` (fresh)
//! vs. `open_with_mvcc` (reopen) — is actually reachable and correct
//! from a real deployment, including across a real process restart.
//!
//! Scope: `Memory` only, matching that file's own scope note; `Entity`/
//! `Relation` get the identical wiring in `memory_server.rs` but are not
//! separately covered here.

use rusty_multimodal_db::server::framing::{read_message, write_message};
use rusty_multimodal_db::server::memory::{
    FIELD_ACCESS_COUNT, FIELD_CATEGORY, FIELD_CONTENT, FIELD_CREATED_AT, FIELD_DELETED_AT,
    FIELD_MEMORY_TYPE, FIELD_METADATA_JSON, FIELD_NODE_ID, FIELD_SENSITIVE, FIELD_SOURCE,
    FIELD_STATUS, FIELD_TAGS, FIELD_UPDATED_AT,
};
use rusty_multimodal_db::server::protocol::{
    ErrorCode, Request, Response, ScanValue, WriteOp, WriteResult, PROTOCOL_VERSION,
    SESSION_MVCC_ISOLATION,
};
use std::net::{SocketAddr, TcpStream};
use std::path::{Path, PathBuf};
use std::process::{Child, Command, Stdio};
use std::time::Duration;
use uuid::Uuid;

fn unique_dir(label: &str) -> PathBuf {
    use std::sync::atomic::{AtomicU64, Ordering};
    static COUNTER: AtomicU64 = AtomicU64::new(0);
    let n = COUNTER.fetch_add(1, Ordering::Relaxed);
    std::env::temp_dir().join(format!("{label}_{}_{n}", std::process::id()))
}

/// A free loopback port, picked by binding to port 0 and releasing it
/// immediately — `memory_server` itself has no way to report back which
/// ephemeral port it bound (its own ready banner prints the CLI argument
/// string, not `listener.local_addr()`), so the caller must pick a real
/// port up front instead of asking for one.
/// `RGT-FR-004` (ADR-0123): a server spawned on port 0, its bound address
/// read from the listening banner — no port is picked ahead of the bind,
/// so two tests can never race for one. `before_banner` is every stderr
/// line the server wrote before it; the pipe stays open for its lifetime.
struct Server {
    child: Child,
    addr: SocketAddr,
    // The same helper in every binary test; each reads the fields it needs.
    #[allow(dead_code)]
    before_banner: String,
    /// The listening banner itself, the server's own account of its settings.
    #[allow(dead_code)]
    banner: String,
    _stderr: std::io::Lines<std::io::BufReader<std::process::ChildStderr>>,
}

impl Drop for Server {
    fn drop(&mut self) {
        let _ = self.child.kill();
        let _ = self.child.wait();
    }
}

fn spawn_listening(mut command: Command) -> Server {
    use std::io::BufRead;
    let mut child = command.stderr(Stdio::piped()).spawn().unwrap();
    let stderr = child.stderr.take().expect("stderr is piped");
    let mut lines = std::io::BufReader::new(stderr).lines();
    let mut before_banner = String::new();
    let (addr, banner) = loop {
        let line = lines
            .next()
            .unwrap_or_else(|| panic!("stderr closed before the listening banner: {before_banner}"))
            .unwrap();
        if let Some(rest) = line.strip_prefix("memory_server listening on ") {
            let addr = rest
                .split(' ')
                .next()
                .unwrap()
                .parse::<SocketAddr>()
                .unwrap();
            break (addr, line);
        }
        before_banner.push_str(&line);
        before_banner.push('\n');
    };
    Server {
        child,
        addr,
        before_banner,
        banner,
        _stderr: lines,
    }
}

fn spawn_memory_server(data_dir: &Path, mvcc: bool) -> Server {
    spawn_memory_server_with(data_dir, mvcc, None)
}

/// `JMC-FR-003` (ADR-0114): the same, with `SERVER_TXN_JOURNAL_PATH` too.
fn spawn_memory_server_with(data_dir: &Path, mvcc: bool, journal: Option<&Path>) -> Server {
    let mut command = Command::new(env!("CARGO_BIN_EXE_memory_server"));
    for (name, _) in std::env::vars_os() {
        if name.to_string_lossy().starts_with("SERVER_") {
            command.env_remove(name);
        }
    }
    command
        .arg("127.0.0.1:0")
        .env("SERVER_DATA_DIR", data_dir)
        .stdout(Stdio::null());
    if mvcc {
        command.env("SERVER_MVCC_ISOLATION", "1");
    }
    if let Some(journal) = journal {
        command.env("SERVER_TXN_JOURNAL_PATH", journal);
    }
    spawn_listening(command)
}

fn connect(addr: SocketAddr) -> TcpStream {
    let stream = TcpStream::connect(addr).unwrap();
    stream.set_nodelay(true).unwrap();
    stream
        .set_read_timeout(Some(Duration::from_secs(5)))
        .unwrap();
    stream
}

fn connect_negotiated(addr: SocketAddr) -> TcpStream {
    let mut stream = connect(addr);
    assert_eq!(
        roundtrip(
            &mut stream,
            Request::Hello {
                protocol_version: PROTOCOL_VERSION
            }
        ),
        Response::Hello {
            protocol_version: PROTOCOL_VERSION
        }
    );
    stream
}

fn roundtrip(stream: &mut TcpStream, req: Request) -> Response {
    write_message(stream, &req).unwrap();
    read_message(stream).unwrap()
}

fn begin_mvcc(stream: &mut TcpStream) -> Response {
    roundtrip(
        stream,
        Request::BeginWith {
            flags: SESSION_MVCC_ISOLATION,
        },
    )
}

fn content_of(stream: &mut TcpStream, id: Uuid) -> String {
    match roundtrip(stream, Request::GetById { id }) {
        Response::Record { fields, .. } => fields
            .into_iter()
            .find_map(|(field, value)| match (field, value) {
                (FIELD_CONTENT, ScanValue::Str(content)) => Some(content),
                _ => None,
            })
            .expect("Memory always has a content field"),
        other => panic!("expected Response::Record, got {other:?}"),
    }
}

/// The full field list `Insert`/`Replace` need, in `MemoryConnectionStore::
/// fields_of`'s tag order. `SERVER_DATA_DIR` (durable mode, required for
/// the restart test below) opens *empty* — `open_or_create_*_production_
/// stack`, no sample data — unlike the no-`SERVER_DATA_DIR` scratch mode
/// `memory_server.rs`'s own `sample_memories()` seeds; every record these
/// tests read, they first insert themselves over the wire.
fn full_fields_with_content(content: &str) -> Vec<(u16, ScanValue)> {
    vec![
        (FIELD_CONTENT, ScanValue::Str(content.into())),
        (FIELD_CATEGORY, ScanValue::Str("preference".into())),
        (FIELD_TAGS, ScanValue::StrList(vec!["sample".into()])),
        (FIELD_SOURCE, ScanValue::Str("manual".into())),
        (FIELD_METADATA_JSON, ScanValue::Str("{}".into())),
        (FIELD_CREATED_AT, ScanValue::I64(1_000)),
        (FIELD_UPDATED_AT, ScanValue::I64(1_000)),
        (FIELD_MEMORY_TYPE, ScanValue::Str("unclassified".into())),
        (FIELD_STATUS, ScanValue::Str("active".into())),
        (FIELD_SENSITIVE, ScanValue::Bool(false)),
        (FIELD_ACCESS_COUNT, ScanValue::I64(0)),
        (FIELD_DELETED_AT, ScanValue::I64(0)),
        (FIELD_NODE_ID, ScanValue::Str(String::new())),
    ]
}

/// Without `SERVER_MVCC_ISOLATION` set, the real binary's `mvcc_enabled`
/// gate never calls `with_mvcc`, so `store.mvcc_supported()` is `false`
/// and `BeginWith { flags: SESSION_MVCC_ISOLATION }` is refused
/// `Unsupported` — `MVCC2-FR-012`'s same precedent `Dog`/`Order`/
/// `Employee` get, reachable end-to-end through the deployed binary.
#[test]
fn real_binary_refuses_the_mvcc_bit_when_server_mvcc_isolation_is_unset() {
    let dir = unique_dir("memory_server_mvcc_disabled");
    let server = spawn_memory_server(&dir, false);
    let addr = server.addr;

    let mut c = connect_negotiated(addr);
    match begin_mvcc(&mut c) {
        Response::Err {
            code: ErrorCode::Unsupported,
            ..
        } => {}
        other => panic!("expected Err(Unsupported), got {other:?}"),
    }
}

/// With `SERVER_MVCC_ISOLATION` set, the fresh-create path (`with_mvcc`,
/// since `memories_existed` is false on a brand-new `SERVER_DATA_DIR`)
/// serves a real MVCC snapshot through the real binary: it survives a
/// concurrent *ordinary* (non-session) `Replace` from another
/// connection, and a fresh snapshot afterward sees the new value —
/// `tests/server_memory_mvcc_integration.rs`'s own flagship guarantee,
/// now proven through the deployed process rather than an in-process
/// `serve()` call.
#[test]
fn real_binary_serves_a_working_mvcc_session_and_survives_a_concurrent_ordinary_write() {
    let dir = unique_dir("memory_server_mvcc_enabled");
    let server = spawn_memory_server(&dir, true);
    let addr = server.addr;
    let id = Uuid::from_u128(1);

    let mut seed = connect_negotiated(addr);
    assert_eq!(
        roundtrip(
            &mut seed,
            Request::Insert {
                id,
                fields: full_fields_with_content("original"),
            }
        ),
        Response::Ok
    );

    let mut a = connect_negotiated(addr);
    let mut b = connect_negotiated(addr);

    assert_eq!(begin_mvcc(&mut a), Response::Ok);
    assert_eq!(content_of(&mut a, id), "original");

    assert_eq!(
        roundtrip(
            &mut b,
            Request::Replace {
                id,
                fields: full_fields_with_content("edited by b"),
            }
        ),
        Response::Ok
    );

    assert_eq!(
        content_of(&mut a, id),
        "original",
        "a's snapshot must not see b's concurrent ordinary write"
    );
    assert_eq!(roundtrip(&mut a, Request::Rollback), Response::Ok);

    let mut fresh = connect_negotiated(addr);
    assert_eq!(begin_mvcc(&mut fresh), Response::Ok);
    assert_eq!(content_of(&mut fresh, id), "edited by b");
    assert_eq!(roundtrip(&mut fresh, Request::Rollback), Response::Ok);
}

/// The reopen path this round's `memory_server.rs` change actually
/// added: a second real process, started against the same
/// `SERVER_DATA_DIR` with `SERVER_MVCC_ISOLATION` still set, takes
/// `open_with_mvcc` (`memories_existed` is now `true`) instead of
/// `with_mvcc` — and must start up at all, rather than panicking or
/// losing the underlying record data, against a directory an earlier
/// MVCC-active process wrote to. The write itself is checked through a
/// plain (non-session) connection, the durability guarantee this
/// binary's underlying store always had regardless of MVCC — proving
/// `open_with_mvcc` doesn't corrupt or roll back live data on reopen.
///
/// Also asserts the round-ten fix (`self.mvcc_flush_now(0)` in
/// `apply_transaction_mvcc`'s non-journaled branch, which `Commit`'s own
/// MVCC-session path always runs through): a **fresh MVCC session**
/// opened on the *second* process now sees this write too. Before that
/// fix, an ordinary session `Commit` updated the live record store
/// directly but was never flushed into the persisted `<mmap_path>.mvcc`
/// history, so reopening restored `active: true` from the stale
/// pre-commit baseline. This is distinct from the separate, still-open
/// `MVCC-OPEN-HOOK-PROPOSAL.md`/`open_with_mvcc` gap (insert-log
/// entries/existence only), which this test does not exercise.
#[test]
fn real_binary_reopening_with_mvcc_preserves_a_committed_write_across_restart() {
    let dir = unique_dir("memory_server_mvcc_restart");
    let id = Uuid::from_u128(1);
    let pending = Uuid::from_u128(2);

    {
        let server = spawn_memory_server(&dir, true);
        let addr = server.addr;
        let mut seed = connect_negotiated(addr);
        assert_eq!(
            roundtrip(
                &mut seed,
                Request::Insert {
                    id,
                    fields: full_fields_with_content("original"),
                }
            ),
            Response::Ok
        );

        let mut c = connect_negotiated(addr);
        assert_eq!(begin_mvcc(&mut c), Response::Ok);
        assert_eq!(
            roundtrip(
                &mut c,
                Request::UpdateField {
                    id,
                    field: FIELD_ACCESS_COUNT,
                    value: ScanValue::I64(9),
                }
            ),
            Response::Staged { index: 0 }
        );
        assert_eq!(roundtrip(&mut c, Request::Commit), Response::Ok);
        // An ordinary insert after the index activated: the insert
        // itself flushes the history (round ten), so a restart keeps it
        // whether or not the reopen reads the pending log. Pinned here
        // as the contrast to `real_binary_composes_the_journal_with_
        // mvcc_across_a_restart`, whose journaled insert flushes nothing
        // and needs that read (`JMC-FR-003`, ADR-0114).
        assert_eq!(
            roundtrip(
                &mut seed,
                Request::Insert {
                    id: pending,
                    fields: full_fields_with_content("pending in the insert log"),
                }
            ),
            Response::Ok
        );
    }

    {
        // Must start up and serve at all — not panic, not lose or roll
        // back the underlying record — reopening via `open_with_mvcc`
        // against a directory an MVCC-active process already wrote to.
        let server = spawn_memory_server(&dir, true);
        let addr = server.addr;
        let mut c = connect(addr);
        match roundtrip(&mut c, Request::GetById { id }) {
            Response::Record { fields, .. } => {
                assert!(fields.contains(&(FIELD_ACCESS_COUNT, ScanValue::I64(9))));
            }
            other => panic!("expected Response::Record, got {other:?}"),
        }

        // The round-ten assertion: a fresh MVCC snapshot on the reopened
        // process sees the pre-restart commit too, not just a plain read.
        let mut fresh = connect_negotiated(addr);
        assert_eq!(begin_mvcc(&mut fresh), Response::Ok);
        match roundtrip(&mut fresh, Request::GetById { id }) {
            Response::Record { fields, .. } => {
                assert!(
                    fields.contains(&(FIELD_ACCESS_COUNT, ScanValue::I64(9))),
                    "a fresh MVCC snapshot after reopen must see the commit \
                     flushed before restart, not a stale pre-commit baseline"
                );
            }
            other => panic!("expected Response::Record, got {other:?}"),
        }
        assert_eq!(
            content_of(&mut fresh, pending),
            "pending in the insert log",
            "a fresh snapshot after reopen sees the insert that was pending in the log"
        );
        assert_eq!(roundtrip(&mut fresh, Request::Rollback), Response::Ok);
    }
}

/// `JMC-FR-003` (ADR-0114): `SERVER_TXN_JOURNAL_PATH` and
/// `SERVER_MVCC_ISOLATION` together, through the real binary — refused
/// at startup until this ADR. The first process commits an MVCC session's
/// `UpdateField` (a journaled `Transaction` batch: in the journal, not
/// yet in `.mvcc`) and an atomic `WriteBatch` insert; the second process,
/// started against the same directory with both variables still set,
/// must start at all, and a fresh MVCC snapshot on it must see both —
/// the journal replayed into the index, not only into the store.
#[test]
fn real_binary_composes_the_journal_with_mvcc_across_a_restart() {
    let dir = unique_dir("memory_server_journal_mvcc_restart");
    let journal = dir.join("memories.journal");
    let (updated, inserted) = (Uuid::from_u128(1), Uuid::from_u128(2));

    {
        let server = spawn_memory_server_with(&dir, true, Some(&journal));
        let addr = server.addr;
        let mut seed = connect_negotiated(addr);
        assert_eq!(
            roundtrip(
                &mut seed,
                Request::Insert {
                    id: updated,
                    fields: full_fields_with_content("original"),
                }
            ),
            Response::Ok
        );
        // Activate the index (its baseline flush) before the journaled
        // batches, so the restart has a history file to reconstruct from.
        let mut c = connect_negotiated(addr);
        assert_eq!(begin_mvcc(&mut c), Response::Ok);
        assert_eq!(
            roundtrip(
                &mut c,
                Request::UpdateField {
                    id: updated,
                    field: FIELD_ACCESS_COUNT,
                    value: ScanValue::I64(9),
                }
            ),
            Response::Staged { index: 0 }
        );
        assert_eq!(roundtrip(&mut c, Request::Commit), Response::Ok);
        match roundtrip(
            &mut seed,
            Request::WriteBatch {
                ops: vec![WriteOp::Insert {
                    id: inserted,
                    fields: full_fields_with_content("batched in"),
                }],
                atomic: true,
            },
        ) {
            Response::BatchResults { results } => assert_eq!(results, vec![WriteResult::Inserted]),
            other => panic!("expected BatchResults, got {other:?}"),
        }
    }

    {
        let server = spawn_memory_server_with(&dir, true, Some(&journal));
        let addr = server.addr;
        let mut fresh = connect_negotiated(addr);
        assert_eq!(begin_mvcc(&mut fresh), Response::Ok);
        match roundtrip(&mut fresh, Request::GetById { id: updated }) {
            Response::Record { fields, .. } => assert!(
                fields.contains(&(FIELD_ACCESS_COUNT, ScanValue::I64(9))),
                "a fresh snapshot after the restart sees the journaled transaction: {fields:?}"
            ),
            other => panic!("expected Response::Record, got {other:?}"),
        }
        assert_eq!(
            content_of(&mut fresh, inserted),
            "batched in",
            "a fresh snapshot after the restart sees the journaled insert"
        );
        assert_eq!(roundtrip(&mut fresh, Request::Rollback), Response::Ok);
    }
}
