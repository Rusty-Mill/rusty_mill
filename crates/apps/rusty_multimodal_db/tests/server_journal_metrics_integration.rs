//! `JSM-FR-001`/`JSM-FR-002` (ADR-0115): `Metrics` carries three gauge
//! families per journaled table — the journal's size in bytes, the
//! entries appended since the last checkpoint, and the writers parked
//! for their commit turn — read live from the table; nothing for a
//! table without a journal.

use rusty_multimodal_db::generic::memory::{create_memory_production_stack, Memory};
use rusty_multimodal_db::generic::production::GenericProductionStore;
use rusty_multimodal_db::server::client::SchemaDrivenClient;
use rusty_multimodal_db::server::memory::MemoryConnectionStore;
use rusty_multimodal_db::server::protocol::ScanValue;
use rusty_multimodal_db::server::{serve_tables, ServeOptions};
use std::net::{SocketAddr, TcpListener};
use std::sync::Arc;
use std::thread;
use uuid::Uuid;

fn unique_dir(label: &str) -> std::path::PathBuf {
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

/// Two tables: `journaled` with a transaction journal, `plain` without.
fn start_server() -> SocketAddr {
    let dir = unique_dir("server_journal_metrics");
    std::fs::create_dir_all(&dir).unwrap();
    let journaled_path = dir.join("journaled.mmap");
    let journal = dir.join("journaled.journal");
    let plain_path = dir.join("plain.mmap");
    let journaled = create_memory_production_stack(vec![memory(1)], &[], &journaled_path).unwrap();
    let plain = create_memory_production_stack(vec![memory(1)], &[], &plain_path).unwrap();
    let journaled = Arc::new(
        MemoryConnectionStore::with_journal(GenericProductionStore::new(journaled), &journal)
            .unwrap(),
    );
    let plain = Arc::new(MemoryConnectionStore::new(GenericProductionStore::new(
        plain,
    )));
    let listener = TcpListener::bind("127.0.0.1:0").unwrap();
    let addr = listener.local_addr().unwrap();
    thread::spawn(move || {
        serve_tables(
            listener,
            vec![("journaled".into(), journaled), ("plain".into(), plain)],
            0,
            ServeOptions::default(),
        )
    });
    addr
}

fn gauge(text: &str, name: &str, table: &str) -> Option<u64> {
    let prefix = format!("{name}{{table=\"{table}\"}} ");
    text.lines()
        .find_map(|line| line.strip_prefix(&prefix))
        .map(|value| value.parse().unwrap())
}

#[test]
fn metrics_carry_the_journal_gauges_for_journaled_tables_only() {
    let addr = start_server();
    let mut client = SchemaDrivenClient::connect(addr).unwrap();

    let text = client.metrics().unwrap();
    let header_len = 12;
    assert_eq!(
        gauge(&text, "dogserver_journal_bytes", "journaled"),
        Some(header_len),
        "a fresh journal is its header alone: {text}"
    );
    assert_eq!(
        gauge(
            &text,
            "dogserver_journal_entries_since_checkpoint",
            "journaled"
        ),
        Some(0)
    );
    assert_eq!(
        gauge(&text, "dogserver_journal_waiting_writers", "journaled"),
        Some(0)
    );
    for name in [
        "dogserver_journal_bytes",
        "dogserver_journal_entries_since_checkpoint",
        "dogserver_journal_waiting_writers",
    ] {
        assert_eq!(
            gauge(&text, name, "plain"),
            None,
            "a table without a journal has no {name} sample: {text}"
        );
        assert!(
            text.contains(&format!("# TYPE {name} gauge\n")),
            "{name} header: {text}"
        );
    }

    let mut session = client.begin().unwrap();
    session
        .update(Uuid::from_u128(1), "access_count", ScanValue::I64(1))
        .unwrap();
    session.commit().unwrap();
    let text = client.metrics().unwrap();
    assert!(
        gauge(&text, "dogserver_journal_bytes", "journaled").unwrap() > header_len,
        "the batch grew the journal: {text}"
    );
    assert_eq!(
        gauge(
            &text,
            "dogserver_journal_entries_since_checkpoint",
            "journaled"
        ),
        Some(1),
        "one entry since the last checkpoint: {text}"
    );
}
