//! `ADR-0130` (`TXS-FR-001`..`005`) over a real socket on `Memory`: a
//! transaction session stages record writes beside field updates (protocol
//! 33) and commits the whole ordered list as one atomic `WriteBatch`.

use rusty_multimodal_db::generic::memory::{create_memory_production_stack, Memory};
use rusty_multimodal_db::generic::production::GenericProductionStore;
use rusty_multimodal_db::server::client::{
    BatchOp, ClientError, ConnectOptions, SchemaDrivenClient, SessionOptions,
};
use rusty_multimodal_db::server::memory::MemoryConnectionStore;
use rusty_multimodal_db::server::protocol::{ErrorCode, ScanValue, WriteResult};
use rusty_multimodal_db::server::{serve, ServeOptions};
use std::net::{SocketAddr, TcpListener};
use std::sync::Arc;
use std::thread;
use uuid::Uuid;

fn memory(n: u128) -> Memory {
    Memory {
        id: Uuid::from_u128(n),
        content: format!("memory {n}"),
        category: "general".into(),
        tags: vec![],
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

fn start() -> SocketAddr {
    use std::sync::atomic::{AtomicU64, Ordering};
    static COUNTER: AtomicU64 = AtomicU64::new(0);
    let dir = std::env::temp_dir().join(format!(
        "session_writes_{}_{}",
        std::process::id(),
        COUNTER.fetch_add(1, Ordering::Relaxed)
    ));
    std::fs::create_dir_all(&dir).unwrap();
    let stack = create_memory_production_stack(
        vec![memory(1), memory(2), memory(3)],
        &[],
        &dir.join("memories.mmap"),
    )
    .unwrap();
    let store = Arc::new(MemoryConnectionStore::new(GenericProductionStore::new(
        stack,
    )));
    let listener = TcpListener::bind("127.0.0.1:0").unwrap();
    let addr = listener.local_addr().unwrap();
    thread::spawn(move || serve(listener, store, ServeOptions::default()));
    addr
}

fn id(n: u128) -> Uuid {
    Uuid::from_u128(n)
}

fn count_of(client: &mut SchemaDrivenClient, n: u128) -> Option<ScanValue> {
    client.get(id(n)).unwrap().map(|fields| {
        fields
            .into_iter()
            .find(|(name, _)| name == "access_count")
            .unwrap()
            .1
    })
}

/// The named fields of a record, ready to insert under another id.
fn fields_like(
    client: &mut SchemaDrivenClient,
    n: u128,
    content: &str,
) -> Vec<(String, ScanValue)> {
    client
        .get(id(n))
        .unwrap()
        .unwrap()
        .into_iter()
        .map(|(name, value)| {
            if name == "content" {
                (name, ScanValue::Str(content.into()))
            } else {
                (name, value)
            }
        })
        .collect()
}

fn borrowed(fields: &[(String, ScanValue)]) -> Vec<(&str, ScanValue)> {
    fields
        .iter()
        .map(|(n, v)| (n.as_str(), v.clone()))
        .collect()
}

/// `TXS-FR-003`/`004`: record writes and updates stage in order, are invisible
/// to every connection until `commit`, and land together with one result per op.
#[test]
fn a_session_stages_record_writes_beside_updates_and_commits_them_together() {
    let addr = start();
    let mut client = SchemaDrivenClient::connect(addr).unwrap();
    let mut other = SchemaDrivenClient::connect(addr).unwrap();
    let new_fields = fields_like(&mut client, 1, "staged insert");

    let mut session = client.begin().unwrap();
    assert_eq!(
        session
            .update(id(1), "access_count", ScanValue::I64(3))
            .unwrap(),
        0
    );
    assert_eq!(session.insert(id(9), &borrowed(&new_fields)).unwrap(), 1);
    // An update of the record this very session inserts: staged in order.
    assert_eq!(
        session
            .update(id(9), "access_count", ScanValue::I64(5))
            .unwrap(),
        2
    );
    assert_eq!(session.delete(id(3)).unwrap(), 3);
    let replaced = fields_like(&mut other, 2, "replaced in session");
    assert_eq!(session.replace(id(2), &borrowed(&replaced)).unwrap(), 4);

    // Nothing is visible to anyone before commit.
    assert_eq!(count_of(&mut other, 1), Some(ScanValue::I64(0)));
    assert_eq!(count_of(&mut other, 9), None);
    assert!(other.get(id(3)).unwrap().is_some());

    let results = session.commit_results().unwrap();
    assert_eq!(
        results,
        vec![
            WriteResult::Updated,
            WriteResult::Inserted,
            WriteResult::Updated,
            WriteResult::Deleted,
            WriteResult::Replaced,
        ]
    );
    assert_eq!(count_of(&mut other, 1), Some(ScanValue::I64(3)));
    assert_eq!(count_of(&mut other, 9), Some(ScanValue::I64(5)));
    assert!(other.get(id(3)).unwrap().is_none());
}

/// `TXS-FR-004`: a rollback discards every staged write, record writes too;
/// a session that staged only updates still commits through the old path.
#[test]
fn rollback_discards_record_writes_and_an_update_only_session_is_unchanged() {
    let addr = start();
    let mut client = SchemaDrivenClient::connect(addr).unwrap();
    let new_fields = fields_like(&mut client, 1, "never written");
    let mut session = client.begin().unwrap();
    session.insert(id(9), &borrowed(&new_fields)).unwrap();
    session.rollback().unwrap();
    assert!(client.get(id(9)).unwrap().is_none());

    let mut session = client.begin().unwrap();
    session
        .update(id(1), "access_count", ScanValue::I64(2))
        .unwrap();
    assert_eq!(
        session.commit_results().unwrap(),
        Vec::new(),
        "the classic answer is `Ok`"
    );
    assert_eq!(count_of(&mut client, 1), Some(ScanValue::I64(2)));
}

/// `TXS-FR-004`: a soft outcome (here a duplicate insert) is a result, not an
/// error — the batch is `WriteBatch`'s atomic mode, and the ops beside it applied.
#[test]
fn a_soft_outcome_is_a_result_and_the_rest_of_the_batch_still_applies() {
    let addr = start();
    let mut client = SchemaDrivenClient::connect(addr).unwrap();
    let dup = fields_like(&mut client, 1, "duplicate");
    let mut session = client.begin().unwrap();
    session.insert(id(1), &borrowed(&dup)).unwrap();
    session
        .update(id(2), "access_count", ScanValue::I64(8))
        .unwrap();
    assert_eq!(
        session.commit_results().unwrap(),
        vec![WriteResult::Duplicate, WriteResult::Updated]
    );
    assert_eq!(count_of(&mut client, 2), Some(ScanValue::I64(8)));
}

/// `TXS-FR-003`: a bad staged write aborts the whole commit before anything
/// is applied, naming the op; read-your-writes / snapshot sessions (and MVCC ones, same guard)
/// refuse a record write (`Unsupported`); below 33 nothing is staged.
#[test]
fn a_bad_write_aborts_the_commit_and_the_richer_sessions_refuse_record_writes() {
    let addr = start();
    let mut client = SchemaDrivenClient::connect(addr).unwrap();
    let new_fields = fields_like(&mut client, 1, "not written");
    let mut session = client.begin().unwrap();
    session.insert(id(9), &borrowed(&new_fields)).unwrap();
    session
        .insert(
            id(10),
            &[("content", ScanValue::Str("too few fields".into()))],
        )
        .unwrap();
    match session.commit() {
        Err(ClientError::TransactionFailed { index: 1, code, .. }) => {
            assert_eq!(code, ErrorCode::Malformed)
        }
        other => panic!("expected TransactionFailed at op 1, got {other:?}"),
    }
    assert!(client.get(id(9)).unwrap().is_none(), "nothing applied");

    for options in [
        SessionOptions::new().read_your_writes(),
        SessionOptions::new().snapshot_isolation(),
    ] {
        let mut session = client.begin_with(options).unwrap();
        match session.insert(id(9), &borrowed(&new_fields)) {
            Err(ClientError::Server(ErrorCode::Unsupported, _)) => {}
            other => panic!("expected Unsupported, got {other:?}"),
        }
        session.rollback().unwrap();
    }

    let mut old =
        SchemaDrivenClient::connect_with(addr, ConnectOptions::new().max_protocol_version(32))
            .unwrap();
    let mut session = old.begin().unwrap();
    assert!(matches!(
        session.insert(id(9), &borrowed(&new_fields)),
        Err(ClientError::Unsupported("session record writes"))
    ));
    session.rollback().unwrap();
}

/// `TXS-FR-001`/`002`: `UpdateField` is a `WriteBatch` op at 33, applied and
/// reported `Updated`/`NotFound`; below 33 it is refused locally by the client.
#[test]
fn update_field_is_a_batch_op_at_33_and_not_below() {
    let addr = start();
    let mut client = SchemaDrivenClient::connect(addr).unwrap();
    let results = client
        .write_batch(
            &[
                BatchOp::UpdateField {
                    id: id(1),
                    field: "access_count",
                    value: ScanValue::I64(6),
                },
                BatchOp::UpdateField {
                    id: id(99),
                    field: "access_count",
                    value: ScanValue::I64(6),
                },
            ],
            true,
        )
        .unwrap();
    assert_eq!(results, vec![WriteResult::Updated, WriteResult::NotFound]);
    assert_eq!(count_of(&mut client, 1), Some(ScanValue::I64(6)));
    // A read-only field is refused as `update` refuses it, before any frame.
    assert!(matches!(
        client.write_batch(
            &[BatchOp::UpdateField {
                id: id(1),
                field: "content",
                value: ScanValue::Str("x".into()),
            }],
            true
        ),
        Err(ClientError::Unsupported("update on this field"))
    ));

    let mut old =
        SchemaDrivenClient::connect_with(addr, ConnectOptions::new().max_protocol_version(32))
            .unwrap();
    assert!(matches!(
        old.write_batch(
            &[BatchOp::UpdateField {
                id: id(1),
                field: "access_count",
                value: ScanValue::I64(1),
            }],
            true
        ),
        Err(ClientError::Unsupported("write_batch update"))
    ));
}

/// `STC-FR-001`..`004` (ADR-0133, protocol 35): a strict session is all or
/// nothing including soft outcomes, reads the record as its staged list would
/// leave it, and is refused beside other options and below 35.
#[test]
fn a_strict_session_commits_all_or_nothing_and_reads_what_it_staged() {
    let addr = start();
    let mut client = SchemaDrivenClient::connect(addr).unwrap();
    let mut other = SchemaDrivenClient::connect(addr).unwrap();
    let new_fields = fields_like(&mut client, 1, "strict insert");

    // A clean list: whole-record read-your-writes, then one commit.
    let mut session = client
        .begin_with(SessionOptions::new().strict_commit())
        .unwrap();
    session.insert(id(9), &borrowed(&new_fields)).unwrap();
    session
        .update(id(9), "access_count", ScanValue::I64(5))
        .unwrap();
    session.delete(id(3)).unwrap();
    let staged = session
        .get(id(9))
        .unwrap()
        .expect("the staged insert is read back");
    assert!(staged.contains(&("access_count".to_string(), ScanValue::I64(5))));
    assert!(
        session.get(id(3)).unwrap().is_none(),
        "the staged delete is read back"
    );
    assert!(
        other.get(id(9)).unwrap().is_none(),
        "invisible before commit"
    );
    assert_eq!(
        session.commit_results().unwrap(),
        vec![
            WriteResult::Inserted,
            WriteResult::Updated,
            WriteResult::Deleted
        ]
    );
    assert_eq!(count_of(&mut other, 9), Some(ScanValue::I64(5)));
    assert!(other.get(id(3)).unwrap().is_none());

    // One soft op (a duplicate insert) fails the commit; nothing applies.
    let mut session = client
        .begin_with(SessionOptions::new().strict_commit())
        .unwrap();
    session
        .update(id(1), "access_count", ScanValue::I64(7))
        .unwrap();
    session.insert(id(2), &borrowed(&new_fields)).unwrap();
    match session.commit_results() {
        Err(ClientError::TransactionFailed { index, code, .. }) => {
            assert_eq!((index, code), (1, ErrorCode::Duplicate));
        }
        other => panic!("expected TransactionFailed, got {other:?}"),
    }
    assert_eq!(
        count_of(&mut other, 1),
        Some(ScanValue::I64(0)),
        "the update beside the duplicate did not apply"
    );

    // Stands alone, and below 35 the bit is unknown.
    assert!(matches!(
        client.begin_with(SessionOptions::new().strict_commit().read_your_writes()),
        Err(ClientError::Server(ErrorCode::Unsupported, _))
    ));
    let mut old =
        SchemaDrivenClient::connect_with(addr, ConnectOptions::new().max_protocol_version(34))
            .unwrap();
    assert!(matches!(
        old.begin_with(SessionOptions::new().strict_commit()),
        Err(ClientError::Unsupported("session options"))
    ));
}
