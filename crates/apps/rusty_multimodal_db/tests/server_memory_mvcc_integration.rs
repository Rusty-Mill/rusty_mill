//! Real end-to-end coverage of `SESSION_MVCC_ISOLATION` (`ADR-0072`,
//! `docs/design/MVCC-PRODUCTION-DESIGN.md`) against the `Memory` domain —
//! a real `TcpListener`, real client `TcpStream`s, real `bincode` framing
//! over the wire, matching `tests/server_transaction_integration.rs`'s own
//! raw-request style (not `SchemaDrivenClient`, since this bit has no
//! high-level client support yet — `MVCC2-FR-011`, deferred). Mirrors
//! `ADR-0033`'s own flagship pattern (`snapshot_isolation_detects_a_
//! conflicting_commit_from_another_connection`) but proves the strictly
//! stronger guarantee real MVCC adds over read-set replay validation:
//! a snapshot survives a concurrent *ordinary* (non-session) write, not
//! only a conflicting session commit (`MVCC2-FR-008`, acceptance
//! criteria 3-6, 13).
//!
//! Scope of this round: the non-journaled, non-`Compact`ed path only —
//! `MemoryConnectionStore::new(..).with_mvcc(path)`, no journal. The
//! journal-checkpoint/`Compact`/restart-recovery pieces (`MVCC2-FR-010`,
//! acceptance criteria 7-10, 14, 16) are explicitly not yet wired
//! (`memory.rs`'s own doc comments name the gap); this file tests only
//! what's actually implemented so far, not the full design's acceptance
//! list.

use rusty_multimodal_db::generic::memory::{create_memory_production_stack, Memory};
use rusty_multimodal_db::generic::production::GenericProductionStore;
use rusty_multimodal_db::server::framing::{read_message, write_message};
use rusty_multimodal_db::server::memory::{
    MemoryConnectionStore, FIELD_ACCESS_COUNT, FIELD_CATEGORY, FIELD_CONTENT, FIELD_CREATED_AT,
    FIELD_DELETED_AT, FIELD_MEMORY_TYPE, FIELD_METADATA_JSON, FIELD_NODE_ID, FIELD_SENSITIVE,
    FIELD_SOURCE, FIELD_STATUS, FIELD_TAGS, FIELD_UPDATED_AT,
};
use rusty_multimodal_db::server::protocol::{
    ErrorCode, Request, Response, ScanValue, PROTOCOL_VERSION, SESSION_MVCC_ISOLATION,
};
use rusty_multimodal_db::server::{serve, ServeOptions};
use std::net::{SocketAddr, TcpListener, TcpStream};
use std::sync::Arc;
use std::thread;
use uuid::Uuid;

fn unique_dir(label: &str) -> std::path::PathBuf {
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
        tags: vec!["sample".into()],
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

/// The wire shape of one memory, in the exact tag order
/// `MemoryConnectionStore::fields_of` uses — built by hand since that
/// method is a private implementation detail, not part of the public
/// server API.
fn full_fields(n: u128, content: &str) -> Vec<(u16, ScanValue)> {
    let m = memory(n, content);
    vec![
        (FIELD_CONTENT, ScanValue::Str(m.content)),
        (FIELD_CATEGORY, ScanValue::Str(m.category)),
        (FIELD_TAGS, ScanValue::StrList(m.tags)),
        (FIELD_SOURCE, ScanValue::Str(m.source)),
        (FIELD_METADATA_JSON, ScanValue::Str(m.metadata_json)),
        (FIELD_CREATED_AT, ScanValue::I64(m.created_at_unix_ms)),
        (FIELD_UPDATED_AT, ScanValue::I64(m.updated_at_unix_ms)),
        (FIELD_MEMORY_TYPE, ScanValue::Str(m.memory_type)),
        (FIELD_STATUS, ScanValue::Str(m.status)),
        (FIELD_SENSITIVE, ScanValue::Bool(m.sensitive)),
        (FIELD_ACCESS_COUNT, ScanValue::I64(m.access_count)),
        (FIELD_DELETED_AT, ScanValue::I64(m.deleted_at_unix_ms)),
        (FIELD_NODE_ID, ScanValue::Str(m.node_id)),
    ]
}

fn start_server() -> SocketAddr {
    let dir = unique_dir("memory_mvcc_integration");
    std::fs::create_dir_all(&dir).unwrap();
    let path = dir.join("memories.mmap");
    let stack =
        create_memory_production_stack(vec![memory(1, "one"), memory(2, "two")], &[], &path)
            .unwrap();
    let connection_store = Arc::new(
        MemoryConnectionStore::new(GenericProductionStore::new(stack))
            .with_mvcc(&path)
            .unwrap(),
    );
    let listener = TcpListener::bind("127.0.0.1:0").unwrap();
    let addr = listener.local_addr().unwrap();
    thread::spawn(move || serve(listener, connection_store, ServeOptions::default()));
    addr
}

fn connect(addr: SocketAddr) -> TcpStream {
    let stream = TcpStream::connect(addr).unwrap();
    stream.set_nodelay(true).unwrap();
    stream
}

/// A client negotiated at the current [`PROTOCOL_VERSION`] (27+, so
/// `SESSION_MVCC_ISOLATION` is known — `MVCC2-FR-004`).
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

fn assert_err(resp: Response, code: ErrorCode) {
    match resp {
        Response::Err { code: got, .. } if got == code => {}
        other => panic!("expected Err {{ {code:?} }}, got {other:?}"),
    }
}

fn assert_transaction_failed(resp: Response, index: usize, code: ErrorCode) {
    match resp {
        Response::TransactionFailed {
            index: got_index,
            code: got_code,
            ..
        } if got_index == index && got_code == code => {}
        other => {
            panic!("expected TransactionFailed {{ index: {index}, code: {code:?} }}, got {other:?}")
        }
    }
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

/// `MVCC2-FR-004`: the bit is unknown (`Malformed`) below protocol 27.
#[test]
fn the_mvcc_bit_is_unknown_below_protocol_27() {
    let addr = start_server();
    let mut c = connect(addr); // negotiated at version 1 (no Hello)
    assert_err(begin_mvcc(&mut c), ErrorCode::Malformed);
}

/// Acceptance criterion 13 (activation baseline): a pre-existing record
/// is visible under a freshly begun MVCC session, with its real value —
/// not absent, and not blocked by any conflict.
#[test]
fn mvcc_session_sees_pre_existing_records_via_the_activation_baseline() {
    let addr = start_server();
    let mut a = connect_negotiated(addr);
    assert_eq!(begin_mvcc(&mut a), Response::Ok);
    assert_eq!(content_of(&mut a, Uuid::from_u128(1)), "one");
    assert_eq!(roundtrip(&mut a, Request::Rollback), Response::Ok);
}

/// `MVCC2-FR-006`/`008`, acceptance criteria 4 and 6: a real-MVCC
/// snapshot survives a *concurrent ordinary* (non-session) `Replace` from
/// another connection — the exact gap read-set replay validation cannot
/// close, since an ordinary write is never a session commit. A later
/// snapshot on a fresh connection sees the new value; no false conflict
/// when nothing concurrent happens either.
#[test]
fn mvcc_snapshot_survives_a_concurrent_ordinary_write_from_another_connection() {
    let addr = start_server();
    let mut a = connect_negotiated(addr);
    let mut b = connect_negotiated(addr);
    let id = Uuid::from_u128(1);

    assert_eq!(begin_mvcc(&mut a), Response::Ok);
    assert_eq!(content_of(&mut a, id), "one", "a's own snapshot's read");

    // `b` is a plain connection with no session at all — an ordinary
    // `Replace`, not another session's `Commit`.
    assert_eq!(
        roundtrip(
            &mut b,
            Request::Replace {
                id,
                fields: full_fields(1, "one, edited by b"),
            }
        ),
        Response::Ok
    );

    assert_eq!(
        content_of(&mut a, id),
        "one",
        "a's snapshot still sees the pre-replace value"
    );
    assert_eq!(roundtrip(&mut a, Request::Rollback), Response::Ok);

    let mut fresh = connect_negotiated(addr);
    assert_eq!(begin_mvcc(&mut fresh), Response::Ok);
    assert_eq!(
        content_of(&mut fresh, id),
        "one, edited by b",
        "a fresh snapshot sees the ordinary write"
    );
    assert_eq!(roundtrip(&mut fresh, Request::Rollback), Response::Ok);
}

/// Acceptance criterion 5: a key created by an ordinary (non-session)
/// `Insert` after a snapshot began is invisible to that snapshot, even
/// though the insert never happened inside any session.
#[test]
fn mvcc_snapshot_does_not_see_a_key_created_after_it_began_over_the_wire() {
    let addr = start_server();
    let mut a = connect_negotiated(addr);
    let mut b = connect_negotiated(addr);
    let new_id = Uuid::from_u128(42);

    assert_eq!(begin_mvcc(&mut a), Response::Ok);
    assert_eq!(
        roundtrip(
            &mut b,
            Request::Insert {
                id: new_id,
                fields: full_fields(42, "created after a's snapshot"),
            }
        ),
        Response::Ok
    );
    assert_eq!(
        roundtrip(&mut a, Request::GetById { id: new_id }),
        Response::NotFound
    );
    assert_eq!(roundtrip(&mut a, Request::Rollback), Response::Ok);

    let mut fresh = connect_negotiated(addr);
    assert_eq!(begin_mvcc(&mut fresh), Response::Ok);
    assert_eq!(content_of(&mut fresh, new_id), "created after a's snapshot");
    assert_eq!(roundtrip(&mut fresh, Request::Rollback), Response::Ok);
}

/// `MVCC2-FR-007`, acceptance criterion 3 (adapted): a session's staged
/// `UpdateField` batch, once a concurrent *ordinary* write has touched
/// the same field, is refused whole with `Conflict` — nothing applied.
#[test]
fn mvcc_session_commit_conflicts_with_a_concurrent_ordinary_write() {
    let addr = start_server();
    let mut a = connect_negotiated(addr);
    let mut b = connect_negotiated(addr);
    let id = Uuid::from_u128(1);

    assert_eq!(begin_mvcc(&mut a), Response::Ok);
    assert_eq!(content_of(&mut a, id), "one");
    assert_eq!(
        roundtrip(
            &mut a,
            Request::UpdateField {
                id,
                field: FIELD_ACCESS_COUNT,
                value: ScanValue::I64(7),
            }
        ),
        Response::Staged { index: 0 }
    );

    // `b`, with no session, replaces the same record — touching
    // `access_count` among every other field.
    assert_eq!(
        roundtrip(
            &mut b,
            Request::Replace {
                id,
                fields: full_fields(1, "replaced by b"),
            }
        ),
        Response::Ok
    );

    assert_transaction_failed(roundtrip(&mut a, Request::Commit), 0, ErrorCode::Conflict);

    let mut check = connect(addr);
    assert_eq!(
        content_of(&mut check, id),
        "replaced by b",
        "a's refused commit applied nothing over b's write"
    );
}

/// The same sequence with no intervening conflicting write: the session
/// commits normally — no false conflict.
#[test]
fn mvcc_session_commit_succeeds_with_no_conflicting_write() {
    let addr = start_server();
    let mut a = connect_negotiated(addr);
    let id = Uuid::from_u128(1);

    assert_eq!(begin_mvcc(&mut a), Response::Ok);
    assert_eq!(content_of(&mut a, id), "one");
    assert_eq!(
        roundtrip(
            &mut a,
            Request::UpdateField {
                id,
                field: FIELD_ACCESS_COUNT,
                value: ScanValue::I64(3),
            }
        ),
        Response::Staged { index: 0 }
    );
    assert_eq!(roundtrip(&mut a, Request::Commit), Response::Ok);

    let mut check = connect(addr);
    match roundtrip(&mut check, Request::GetById { id }) {
        Response::Record { fields, .. } => {
            assert!(fields.contains(&(FIELD_ACCESS_COUNT, ScanValue::I64(3))));
        }
        other => panic!("expected Response::Record, got {other:?}"),
    }
}
