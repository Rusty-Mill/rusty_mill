//! Real end-to-end coverage of `SESSION_MVCC_ISOLATION` (`ADR-0072`,
//! `docs/design/MVCC-PRODUCTION-DESIGN.md`) against the `Relation` domain
//! — a real `TcpListener`, real client `TcpStream`s, real `bincode`
//! framing over the wire, matching `tests/server_memory_mvcc_integration.rs`'s
//! own raw-request style (not `SchemaDrivenClient`, since this bit has no
//! high-level client support yet — `MVCC2-FR-011`, deferred). Proves the
//! same guarantees against `Relation`'s own field shape (`subject`/
//! `relation`/`object`/timestamps) that `tests/server_memory_mvcc_integration.rs`
//! proves for `Memory`: a snapshot survives a concurrent *ordinary*
//! (non-session) write, not only a conflicting session commit
//! (`MVCC2-FR-008`, acceptance criteria 3-6, 13).
//!
//! Scope of this round: the non-journaled, non-`Compact`ed path only —
//! `RelationConnectionStore::new(..).with_mvcc(path)`, no journal —
//! matching the `Memory` integration test's own documented scope.

use rusty_multimodal_db::generic::production::GenericProductionStore;
use rusty_multimodal_db::generic::relation::{create_relation_production_stack, Relation};
use rusty_multimodal_db::server::framing::{read_message, write_message};
use rusty_multimodal_db::server::protocol::{
    ErrorCode, Request, Response, ScanValue, PROTOCOL_VERSION, SESSION_MVCC_ISOLATION,
};
use rusty_multimodal_db::server::relation::{
    RelationConnectionStore, FIELD_CREATED_AT, FIELD_DELETED_AT, FIELD_NODE_ID, FIELD_OBJECT,
    FIELD_RELATION, FIELD_SUBJECT, FIELD_UPDATED_AT,
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

fn relation(n: u128, subject: &str, label: &str, object: &str) -> Relation {
    Relation {
        id: Uuid::from_u128(n),
        subject: subject.into(),
        relation: label.into(),
        object: object.into(),
        created_at_unix_ms: 1_000 * n as i64,
        updated_at_unix_ms: 1_000 * n as i64,
        node_id: String::new(),
        deleted_at_unix_ms: 0,
    }
}

/// The wire shape of one relation, in the exact tag order
/// `RelationConnectionStore::fields_of` uses — built by hand since that
/// method is a private implementation detail, not part of the public
/// server API.
fn full_fields(n: u128, subject: &str, label: &str, object: &str) -> Vec<(u16, ScanValue)> {
    let r = relation(n, subject, label, object);
    vec![
        (FIELD_SUBJECT, ScanValue::Str(r.subject)),
        (FIELD_RELATION, ScanValue::Str(r.relation)),
        (FIELD_OBJECT, ScanValue::Str(r.object)),
        (FIELD_CREATED_AT, ScanValue::I64(r.created_at_unix_ms)),
        (FIELD_UPDATED_AT, ScanValue::I64(r.updated_at_unix_ms)),
        (FIELD_NODE_ID, ScanValue::Str(r.node_id)),
        (FIELD_DELETED_AT, ScanValue::I64(r.deleted_at_unix_ms)),
    ]
}

fn start_server() -> SocketAddr {
    let dir = unique_dir("relation_mvcc_integration");
    std::fs::create_dir_all(&dir).unwrap();
    let path = dir.join("relations.mmap");
    let relations = vec![
        relation(1, "aaa", "works_with", "bbb"),
        relation(2, "aaa", "located_in", "ccc"),
    ];
    let stack = create_relation_production_stack(relations, &path).unwrap();
    let connection_store = Arc::new(
        RelationConnectionStore::new(GenericProductionStore::new(stack))
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

fn label_of(stream: &mut TcpStream, id: Uuid) -> String {
    match roundtrip(stream, Request::GetById { id }) {
        Response::Record { fields, .. } => fields
            .into_iter()
            .find_map(|(field, value)| match (field, value) {
                (FIELD_RELATION, ScanValue::Str(label)) => Some(label),
                _ => None,
            })
            .expect("Relation always has a relation-label field"),
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
    assert_eq!(label_of(&mut a, Uuid::from_u128(1)), "works_with");
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
    assert_eq!(
        label_of(&mut a, id),
        "works_with",
        "a's own snapshot's read"
    );

    // `b` is a plain connection with no session at all — an ordinary
    // `Replace`, not another session's `Commit`.
    assert_eq!(
        roundtrip(
            &mut b,
            Request::Replace {
                id,
                fields: full_fields(1, "aaa", "edited_by_b", "bbb"),
            }
        ),
        Response::Ok
    );

    assert_eq!(
        label_of(&mut a, id),
        "works_with",
        "a's snapshot still sees the pre-replace value"
    );
    assert_eq!(roundtrip(&mut a, Request::Rollback), Response::Ok);

    let mut fresh = connect_negotiated(addr);
    assert_eq!(begin_mvcc(&mut fresh), Response::Ok);
    assert_eq!(
        label_of(&mut fresh, id),
        "edited_by_b",
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
    let new_id = Uuid::from_u128(77);

    assert_eq!(begin_mvcc(&mut a), Response::Ok);
    assert_eq!(
        roundtrip(
            &mut b,
            Request::Insert {
                id: new_id,
                fields: full_fields(77, "ddd", "created_after_snapshot", "eee"),
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
    assert_eq!(label_of(&mut fresh, new_id), "created_after_snapshot");
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
    assert_eq!(label_of(&mut a, id), "works_with");
    assert_eq!(
        roundtrip(
            &mut a,
            Request::UpdateField {
                id,
                field: FIELD_UPDATED_AT,
                value: ScanValue::I64(9_000),
            }
        ),
        Response::Staged { index: 0 }
    );

    // `b`, with no session, replaces the same record — touching
    // `updated_at_unix_ms` among every other field.
    assert_eq!(
        roundtrip(
            &mut b,
            Request::Replace {
                id,
                fields: full_fields(1, "aaa", "edited_by_b", "bbb"),
            }
        ),
        Response::Ok
    );

    assert_transaction_failed(roundtrip(&mut a, Request::Commit), 0, ErrorCode::Conflict);

    let mut check = connect(addr);
    assert_eq!(
        label_of(&mut check, id),
        "edited_by_b",
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
    assert_eq!(label_of(&mut a, id), "works_with");
    assert_eq!(
        roundtrip(
            &mut a,
            Request::UpdateField {
                id,
                field: FIELD_UPDATED_AT,
                value: ScanValue::I64(9_000),
            }
        ),
        Response::Staged { index: 0 }
    );
    assert_eq!(roundtrip(&mut a, Request::Commit), Response::Ok);

    let mut check = connect(addr);
    match roundtrip(&mut check, Request::GetById { id }) {
        Response::Record { fields, .. } => {
            assert!(fields.contains(&(FIELD_UPDATED_AT, ScanValue::I64(9_000))));
        }
        other => panic!("expected Response::Record, got {other:?}"),
    }
}
