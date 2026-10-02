//! What a chunked snapshot costs the table's writers (`ADR-0136`): the staging
//! copy runs under the write lock, so the slowest write during a
//! `BeginSnapshot` waits about as long as the copy. One writer updates a field
//! in a loop while a standby fetches the whole table in chunks; per table size
//! it prints the standby's total time and the writer's worst and median wait.
//!
//! Run: `cargo run --release -p rusty_multimodal_db --features server
//! --example snapshot_stall_bench [-- <dir> [<MiB> ...]]` (default 128 512 1024).
//! The table, its staged copy and the standby's copy coexist: allow 3x the size.
use rusty_multimodal_db::generic::memory::{create_memory_production_stack, Memory};
use rusty_multimodal_db::generic::production::GenericProductionStore;
use rusty_multimodal_db::server::client::SchemaDrivenClient;
use rusty_multimodal_db::server::memory::MemoryConnectionStore;
use rusty_multimodal_db::server::protocol::ScanValue;
use rusty_multimodal_db::server::{serve, ServeOptions};
use std::net::TcpListener;
use std::path::PathBuf;
use std::sync::atomic::{AtomicBool, Ordering};
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

fn main() {
    let mut args = std::env::args().skip(1);
    let base = args
        .next()
        .map_or_else(std::env::temp_dir, PathBuf::from)
        .join(format!("snapshot_stall_{}", std::process::id()));
    let mut sizes: Vec<usize> = args.filter_map(|a| a.parse().ok()).collect();
    if sizes.is_empty() {
        sizes = vec![128, 512, 1024];
    }
    println!(
        "{:>8} {:>10} {:>12} {:>12} {:>8}",
        "MiB", "fetch s", "worst ms", "median ms", "writes"
    );
    for mib in sizes {
        let dir = base.join(mib.to_string());
        std::fs::create_dir_all(&dir).expect("bench dir");
        let path = dir.join("memories.mmap");
        let stack = create_memory_production_stack(vec![seed()], &[], &path).expect("table");
        let chunk: Vec<u8> = (0..1 << 20)
            .map(|i: u32| (i.wrapping_mul(2_654_435_761) >> 13) as u8)
            .collect();
        let pad = std::fs::File::create(dir.join("memories.mmap.pad")).expect("pad");
        {
            use std::io::Write;
            let mut pad = std::io::BufWriter::new(pad);
            for _ in 0..mib {
                pad.write_all(&chunk).expect("pad");
            }
            pad.flush().expect("pad");
        }
        let store = Arc::new(
            MemoryConnectionStore::new(GenericProductionStore::new(stack)).with_backup_source(path),
        );
        let options = ServeOptions::new(None, Some("rw".into()))
            .with_replication_token("repl".to_string())
            .with_snapshot_staging(dir.join("staging"), u64::MAX / (1 << 20))
            .expect("staging");
        let listener = TcpListener::bind("127.0.0.1:0").expect("bind");
        let addr = listener.local_addr().expect("addr");
        std::thread::spawn(move || serve(listener, store, options));

        let stop = Arc::new(AtomicBool::new(false));
        let writer_stop = Arc::clone(&stop);
        let writer = std::thread::spawn(move || {
            let mut client = SchemaDrivenClient::connect_authenticated(addr, "rw").expect("writer");
            let mut waits = Vec::new();
            let mut n = 0u32;
            while !writer_stop.load(Ordering::Relaxed) {
                n += 1;
                let started = Instant::now();
                client
                    .update(
                        Uuid::from_u128(1),
                        "access_count",
                        ScanValue::I64(i64::from(n)),
                    )
                    .expect("update");
                waits.push(started.elapsed());
            }
            waits
        });
        std::thread::sleep(Duration::from_millis(300));
        let target = dir.join("standby");
        std::fs::create_dir_all(&target).expect("standby dir");
        let mut standby = SchemaDrivenClient::connect_authenticated(addr, "repl").expect("standby");
        let started = Instant::now();
        standby.fetch_snapshot_chunked(&target).expect("snapshot");
        let fetch = started.elapsed();
        stop.store(true, Ordering::Relaxed);
        let mut waits = writer.join().expect("writer");
        waits.sort();
        println!(
            "{mib:>8} {:>10.2} {:>12.1} {:>12.3} {:>8}",
            fetch.as_secs_f64(),
            waits.last().map_or(0.0, |d| d.as_secs_f64() * 1e3),
            waits[waits.len() / 2].as_secs_f64() * 1e3,
            waits.len()
        );
        let _ = std::fs::remove_dir_all(&dir);
    }
    let _ = std::fs::remove_dir_all(&base);
}
