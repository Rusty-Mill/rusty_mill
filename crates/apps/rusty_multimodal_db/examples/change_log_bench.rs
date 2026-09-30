//! What the change log costs under concurrent writers (`ADR-0131`): the log's
//! lock is held across the wrapped store's whole apply, so writers that the
//! crash journal would group-commit into one `fsync` are serialised instead.
//! This measures it: `Memory` writes through `write_batch(atomic)` from N
//! threads, journaled or not, with and without `ChangeLogged`. `insert`
//! (the default) appends to the insert log under the store's lock, so it
//! already pays one `fsync` per write inside the apply; `update` sets a field
//! in place, so with a journal the journal's `fsync` is the only one and group
//! commit is what the log's lock can defeat.
//!
//! Run: `cargo run --release -p rusty_multimodal_db --features server
//! --example change_log_bench [-- <secs per cell> [<dir> [insert|update]]]`. Use a directory on
//! the disk you care about: the numbers are fsync-bound.
use rusty_multimodal_db::generic::memory::{create_memory_production_stack, Memory};
use rusty_multimodal_db::generic::production::GenericProductionStore;
use rusty_multimodal_db::server::changelog::{ChangeLog, DEFAULT_RETAIN_BYTES};
use rusty_multimodal_db::server::changelogged::ChangeLogged;
use rusty_multimodal_db::server::memory::MemoryConnectionStore;
use rusty_multimodal_db::server::protocol::{FieldRef, ScanValue, WriteOp};

const FIELD_ACCESS_COUNT: FieldRef = 10;
use rusty_multimodal_db::server::ConnectionStore;
use std::path::{Path, PathBuf};
use std::sync::atomic::{AtomicU64, Ordering};
use std::sync::Arc;
use std::time::{Duration, Instant};
use uuid::Uuid;

fn seed() -> Memory {
    Memory {
        id: Uuid::from_u128(1),
        content: "seed".into(),
        category: "general".into(),
        tags: vec![],
        source: "bench".into(),
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

/// A fresh table in `dir`: journaled or not, logged or not.
fn build(dir: &Path, journaled: bool, logged: bool) -> Arc<dyn ConnectionStore> {
    std::fs::create_dir_all(dir).expect("bench dir");
    let stack = create_memory_production_stack(vec![seed()], &[], &dir.join("memories.mmap"))
        .expect("create stack");
    let store = GenericProductionStore::new(stack);
    let adapter = if journaled {
        MemoryConnectionStore::with_journal(store, &dir.join("table.journal")).expect("journal")
    } else {
        MemoryConnectionStore::new(store)
    };
    let adapter: Arc<dyn ConnectionStore> = Arc::new(adapter);
    if !logged {
        return adapter;
    }
    let log = ChangeLog::open(&dir.join("table.changes"), DEFAULT_RETAIN_BYTES).expect("log");
    Arc::new(ChangeLogged::new(adapter, Arc::new(log)))
}

struct Cell {
    ops_per_sec: f64,
    p50_us: f64,
    p99_us: f64,
}

fn run(store: Arc<dyn ConnectionStore>, threads: usize, secs: u64, update: bool) -> Cell {
    let template: Vec<(FieldRef, ScanValue)> = store.get(Uuid::from_u128(1)).expect("seed record");
    let next = Arc::new(AtomicU64::new(10));
    let deadline = Instant::now() + Duration::from_secs(secs);
    let started = Instant::now();
    let handles: Vec<_> = (0..threads)
        .map(|_| {
            let (store, template, next) = (store.clone(), template.clone(), next.clone());
            std::thread::spawn(move || {
                let mut latencies = Vec::new();
                while Instant::now() < deadline {
                    let n = next.fetch_add(1, Ordering::Relaxed);
                    let op = if update {
                        WriteOp::UpdateField {
                            id: Uuid::from_u128(1),
                            field: FIELD_ACCESS_COUNT,
                            value: ScanValue::I64(n as i64),
                        }
                    } else {
                        WriteOp::Insert {
                            id: Uuid::from_u128(n as u128),
                            fields: template.clone(),
                        }
                    };
                    let t = Instant::now();
                    store.write_batch(&[op], true).expect("the insert applies");
                    latencies.push(t.elapsed().as_nanos() as u64);
                }
                latencies
            })
        })
        .collect();
    let mut all: Vec<u64> = handles
        .into_iter()
        .flat_map(|h| h.join().expect("worker"))
        .collect();
    let elapsed = started.elapsed().as_secs_f64();
    all.sort_unstable();
    let at = |q: f64| all[((all.len() - 1) as f64 * q) as usize] as f64 / 1000.0;
    Cell {
        ops_per_sec: all.len() as f64 / elapsed,
        p50_us: at(0.50),
        p99_us: at(0.99),
    }
}

fn main() {
    let mut args = std::env::args().skip(1);
    let secs: u64 = args.next().and_then(|s| s.parse().ok()).unwrap_or(2);
    let root: PathBuf = args.next().map(PathBuf::from).unwrap_or_else(|| {
        std::env::temp_dir().join(format!("change_log_bench_{}", std::process::id()))
    });
    let update = args.next().as_deref() == Some("update");
    println!(
        "dir {}  ({secs}s per cell, Memory {} via write_batch(atomic))\n",
        root.display(),
        if update { "update" } else { "insert" }
    );
    println!(
        "{:<12} {:>7} {:>12} {:>10} {:>10}",
        "config", "threads", "ops/s", "p50 µs", "p99 µs"
    );
    for (journaled, logged, name) in [
        (false, false, "plain"),
        (false, true, "logged"),
        (true, false, "journal"),
        (true, true, "journal+log"),
    ] {
        for threads in [1usize, 2, 4, 8, 16] {
            let dir = root.join(format!("{name}_{threads}"));
            let cell = run(build(&dir, journaled, logged), threads, secs, update);
            println!(
                "{name:<12} {threads:>7} {:>12.0} {:>10.0} {:>10.0}",
                cell.ops_per_sec, cell.p50_us, cell.p99_us
            );
            let _ = std::fs::remove_dir_all(&dir);
        }
    }
    let _ = std::fs::remove_dir_all(&root);
}
