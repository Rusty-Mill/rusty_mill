//! Real end-to-end coverage of `SESSION_MVCC_ISOLATION` (`ADR-0072`,
//! `docs/design/MVCC-PRODUCTION-DESIGN.md`) against the `Entity` domain —
//! a real `TcpListener`, real client `TcpStream`s, real `bincode` framing
//! over the wire, matching `tests/server_memory_mvcc_integration.rs`'s own
//! raw-request style (not `SchemaDrivenClient`, since this bit has no
//! high-level client support yet — `MVCC2-FR-011`, deferred). Proves the
//! same guarantees against `Entity`'s own field shape (`label`/`kind`/
//! `mention_count`/`aliases`) that `tests/server_memory_mvcc_integration.rs`
//! proves for `Memory`: a snapshot survives a concurrent *ordinary*
//! (non-session) write, not only a conflicting session commit
//! (`MVCC2-FR-008`, acceptance criteria 3-6, 13).
//!
//! Scope of this round: the non-journaled, non-`Compact`ed path only —
//! `EntityConnectionStore::new(..).with_mvcc(path)`, no journal — matching
//! the `Memory` integration test's own documented scope.

use rusty_multimodal_db::generic::entity::{create_entity_production_stack, Entity};
use rusty_multimodal_db::generic::production::GenericProductionStore;
use rusty_multimodal_db::server::entity::{
    EntityConnectionStore, FIELD_ALIASES, FIELD_KIND, FIELD_LABEL, FIELD_MENTION_COUNT,
};
use rusty_multimodal_db::server::framing::{read_message, write_message};
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

fn entity(n: u128, label: &str, kind: &str) -> Entity {
    Entity {
        id: Uuid::from_u128(n),
        label: label.into(),
        kind: kind.into(),
        mention_count: 0,
        aliases: vec![],
    }
}

/// The wire shape of one entity, in the exact tag order
/// `EntityConnectionStore::fields_of` uses — built by hand since that
/// method is a private implementation detail, not part of the public
/// server API.
fn full_fields(n: u128, label: &str, kind: &str) -> Vec<(u16, ScanValue)> {
    let e = entity(n, label, kind);
    vec![
        (FIELD_LABEL, ScanValue::Str(e.label)),
        (FIELD_KIND, ScanValue::Str(e.kind)),
        (FIELD_MENTION_COUNT, ScanValue::I64(e.mention_count)),
        (FIELD_ALIASES, ScanValue::StrList(e.aliases)),
    ]
}

fn start_server() -> SocketAddr {
    let dir = unique_dir("entity_mvcc_integration");
    std::fs::create_dir_all(&dir).unwrap();
    let path = dir.join("entities.mmap");
    let entities = vec![
        entity(1, "Ada Lovelace", "person"),
        entity(2, "Analytical Engine", "concept"),
    ];
    let stack = create_entity_production_stack(entities, &[], &[], &path).unwrap();
    let connection_store = Arc::new(
        EntityConnectionStore::new(GenericProductionStore::new(stack))
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
                (FIELD_LABEL, ScanValue::Str(label)) => Some(label),
                _ => None,
            })
            .expect("Entity always has a label field"),
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
    assert_eq!(label_of(&mut a, Uuid::from_u128(1)), "Ada Lovelace");
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
        "Ada Lovelace",
        "a's own snapshot's read"
    );

    // `b` is a plain connection with no session at all — an ordinary
    // `Replace`, not another session's `Commit`.
    assert_eq!(
        roundtrip(
            &mut b,
            Request::Replace {
                id,
                fields: full_fields(1, "Ada King", "person"),
            }
        ),
        Response::Ok
    );

    assert_eq!(
        label_of(&mut a, id),
        "Ada Lovelace",
        "a's snapshot still sees the pre-replace value"
    );
    assert_eq!(roundtrip(&mut a, Request::Rollback), Response::Ok);

    let mut fresh = connect_negotiated(addr);
    assert_eq!(begin_mvcc(&mut fresh), Response::Ok);
    assert_eq!(
        label_of(&mut fresh, id),
        "Ada King",
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
                fields: full_fields(77, "Grace Hopper", "person"),
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
    assert_eq!(label_of(&mut fresh, new_id), "Grace Hopper");
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
    assert_eq!(label_of(&mut a, id), "Ada Lovelace");
    assert_eq!(
        roundtrip(
            &mut a,
            Request::UpdateField {
                id,
                field: FIELD_MENTION_COUNT,
                value: ScanValue::I64(7),
            }
        ),
        Response::Staged { index: 0 }
    );

    // `b`, with no session, replaces the same record — touching
    // `mention_count` among every other field.
    assert_eq!(
        roundtrip(
            &mut b,
            Request::Replace {
                id,
                fields: full_fields(1, "Ada King", "person"),
            }
        ),
        Response::Ok
    );

    assert_transaction_failed(roundtrip(&mut a, Request::Commit), 0, ErrorCode::Conflict);

    let mut check = connect(addr);
    assert_eq!(
        label_of(&mut check, id),
        "Ada King",
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
    assert_eq!(label_of(&mut a, id), "Ada Lovelace");
    assert_eq!(
        roundtrip(
            &mut a,
            Request::UpdateField {
                id,
                field: FIELD_MENTION_COUNT,
                value: ScanValue::I64(3),
            }
        ),
        Response::Staged { index: 0 }
    );
    assert_eq!(roundtrip(&mut a, Request::Commit), Response::Ok);

    let mut check = connect(addr);
    match roundtrip(&mut check, Request::GetById { id }) {
        Response::Record { fields, .. } => {
            assert!(fields.contains(&(FIELD_MENTION_COUNT, ScanValue::I64(3))));
        }
        other => panic!("expected Response::Record, got {other:?}"),
    }
}
