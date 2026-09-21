//! Custom (non-Criterion) throughput/latency harness for the server/query
//! layer (`src/server/**`, `SERVER-001`) — the benchmark `SERVER-001`'s
//! own "Open questions" named as a real, unscoped follow-up since the
//! layer's initial implementation (v0.1.0's acceptance criteria were
//! correctness, not performance). The owner's "2" in "3 then 2": a third
//! validation domain (`Employee`, `SERVER-001` v0.3.0) first, then this.
//!
//! # Why not Criterion
//!
//! Same reasoning as `benches/concurrency.rs`: the real questions here —
//! "what does one request/response round trip cost over a real socket,"
//! and "how does aggregate throughput scale with the number of concurrent
//! client connections under the thread-per-connection model" — aren't
//! what Criterion's `b.iter` model measures (one closure, repeated, timed
//! on one thread). This reuses `benches/concurrency.rs`'s own custom-harness
//! shape instead: a `Barrier`-synchronized thread sweep, aggregate ops/sec
//! computed from the slowest thread's wall-clock time, not the average.
//!
//! # Real sockets, not `dispatch` in-process
//!
//! Every prior server-layer benchmark-shaped measurement is either
//! `dispatch`'s in-process unit tests (no I/O at all) or a real socket used
//! only for correctness (`tests/server_*_integration.rs`, pass/fail, not
//! timed). This is the first benchmark to put a real
//! `TcpListener`/`TcpStream` pair (loopback) in the timed path — the
//! questions above only exist at that layer.
//!
//! # `TCP_NODELAY`, non-negotiable here
//!
//! `SERVER-001-FR-006` already found and fixed a real ~40ms-per-round-trip
//! Nagle/delayed-ACK cost when `TCP_NODELAY` isn't set on both ends of a
//! synchronous request/response protocol. Every client connection this
//! benchmark opens sets it, same as every existing integration test —
//! omitting it here wouldn't just skew the numbers, it would make them
//! meaningless (measuring TCP's delayed-ACK timer, not this server).
//!
//! # Dataset: a small, fixed 20-id pool per domain, not `SIZES`
//!
//! Matches `tests/server_dog_integration.rs`'s own flagship concurrent-client
//! stress test precedent (`concurrent_clients_over_the_wire_match_a_sequential_replay`,
//! a 20-id contended pool) rather than this crate's usual 1K/100K/1M
//! `SIZES` sweep: those sizes measure in-process lookup cost, which the
//! existing `benches/workloads.rs`/`benches/generic_production.rs` suites
//! already cover per domain. What this benchmark adds is the network/dispatch
//! layer *on top* of that already-measured lookup cost, so a small, realistic
//! id pool is enough — a bigger dataset would mostly re-measure
//! `GetById`'s already-known in-process cost under a much larger, mostly
//! irrelevant fixed setup cost.
//!
//! # Thread counts: 1/4/32/64, `baileyai` itself
//!
//! The original container pass swept 1/4/8/16 unconditionally on a 4-core
//! container, so 8 and 16 were honest but oversubscribed data points, not
//! genuine added parallelism — `SERVER-001`'s own "Open questions" named
//! the thread-per-connection model's real connection-count ceiling as
//! unmeasured because of it. A second pass substituted the owner's Windows
//! dev machine (`Beast`, 24 logical / 12 physical cores) because `baileyai`
//! was unreachable over SSH from that session, and swept `[1, 4, 24, 48]`
//! to match. This third pass runs directly on `baileyai` itself — this
//! session executes on that machine (confirmed via `hostname`/`whoami`/
//! `nproc`), so no SSH substitution is needed. `baileyai` reports 32
//! logical processors via both `nproc` and
//! `std::thread::available_parallelism()` (16 physical cores, AMD Ryzen AI
//! MAX+ 395, SMT enabled) — the same machine, and the same
//! `[1, 4, 32, 64]` array, `benches/concurrency.rs`'s own third history
//! entry already established: `1` (serial baseline), `4` (kept, directly
//! comparable to both the container's and `Beast`'s own non-oversubscribed
//! 4-thread rows), `32` (this machine's actual core count — the first
//! genuinely non-oversubscribed high-thread-count data point this
//! environment can produce), and `64` (2x cores, deliberate
//! oversubscription, matching the "2x cores" pattern every prior pass in
//! this history already established). 8/16/24/48 are dropped since none is
//! at or past this machine's real headroom. See `RESULTS.md`'s
//! `## Server / query layer` section for the answer this pass found: does
//! the thread-per-connection model's throughput keep scaling past 4
//! threads on real, non-oversubscribed hardware, or does it plateau — and
//! if so, at what count relative to real core count.
//!
//! # One request kind (`GetById`), all three domains
//!
//! `GetById` is the one request kind every `ConnectionStore` adapter
//! implements as a real operation (`Dog`, `Order`/`Customer`, `Employee`
//! alike) — the only kind that lets the three domains' numbers sit in one
//! comparable table. Per-request-kind cost differences (a `filter_eq`
//! linear scan vs. an indexed `get`, `Employee`'s extra field) are already
//! covered by each domain's own in-process benchmark
//! (`benches/workloads.rs`, `benches/generic_production.rs`); this
//! benchmark's own question is what the network/dispatch layer adds on
//! top, which one representative request kind answers without tripling
//! this benchmark's own already-large thread-count/domain matrix.
//!
//! # `Request::Transaction`, added alongside `SERVER-001` v0.7.0
//!
//! `SERVER-001` shipped `Request::Transaction` (ADR-0013) without a
//! throughput/latency characterization the way `GetById` got here at
//! v0.4.0 — this is that follow-up, using the same latency-baseline-then-
//! throughput-sweep shape. Each domain's transaction touches two distinct
//! ids from the same `POOL_SIZE` pool, both set to the same new value —
//! matching `tests/server_transaction_integration.rs`'s own flagship
//! stress test's request shape, the smallest batch that's still a real,
//! representative multi-operation transaction rather than a single
//! `UpdateField` in disguise. Reported as its own `{domain}-txn` row set,
//! directly comparable to that domain's plain `GetById` rows above it
//! (same pool, same thread counts, same iteration counts) — the
//! difference between the two is exactly the cost `Request::Transaction`'s
//! validate-then-apply, longer-held-lock mechanism adds over a single
//! read.

use rusty_multimodal_db::bench_support::{fresh_temp_dir, RoundRobin};
use rusty_multimodal_db::generic::memory::{create_memory_production_stack, Memory};
use rusty_multimodal_db::generic::order_customer::{
    create_order_production_stack, Order, OrderStatus,
};
use rusty_multimodal_db::generic::production::GenericProductionStore;
use rusty_multimodal_db::generic_spike::employee_impl::{
    create_employee_production_stack, Department, Employee,
};
use rusty_multimodal_db::production::ProductionStore;
use rusty_multimodal_db::record::DogRecord;
use rusty_multimodal_db::server::dog::{DogConnectionStore, FIELD_AGE};
use rusty_multimodal_db::server::employee::{EmployeeConnectionStore, FIELD_SALARY};
use rusty_multimodal_db::server::framing::{read_message, write_message};
use rusty_multimodal_db::server::memory::{
    MemoryConnectionStore, FIELD_CATEGORY, FIELD_CREATED_AT, FIELD_SOURCE, FIELD_UPDATED_AT,
};
use rusty_multimodal_db::server::order::{OrderConnectionStore, FIELD_AMOUNT};
use rusty_multimodal_db::server::protocol::{
    AggregateFn, AggregateSpec, CompareOp, FieldRef, Predicate, Request, Response, ScanValue,
    Selection, TransactionOp, PROTOCOL_VERSION,
};
use rusty_multimodal_db::server::{serve, ServeOptions};
use std::net::{SocketAddr, TcpListener, TcpStream};
use std::sync::{Arc, Barrier};
use std::thread;
use std::time::{Duration, Instant};
use uuid::Uuid;

/// Matches `tests/server_dog_integration.rs`'s flagship stress test's own
/// contended-pool size — see this module's own doc comment.
const POOL_SIZE: usize = 20;
/// See this module's own doc comment — matches `benches/concurrency.rs`'s
/// own `baileyai`-specific sweep.
const THREAD_COUNTS: [usize; 4] = [1, 4, 32, 64];
/// Real-socket round trips are far more expensive per-op than the
/// in-process operations `benches/concurrency.rs` sweeps (`OPS_PER_THREAD
/// = 10_000` there), so this is lower to keep the full domain × thread-count
/// matrix's total wall-clock time tractable.
const OPS_PER_THREAD: usize = 2_000;
/// Iterations for the single-connection, zero-contention latency baseline
/// each domain reports before its throughput sweep.
const LATENCY_ITERATIONS: usize = 5_000;

fn connect(addr: SocketAddr) -> TcpStream {
    let stream = TcpStream::connect(addr).unwrap();
    stream.set_nodelay(true).unwrap();
    stream
}

fn roundtrip(stream: &mut TcpStream, req: &Request) -> Response {
    write_message(stream, req).unwrap();
    read_message(stream).unwrap()
}

fn start_dog_server() -> (SocketAddr, Vec<Uuid>) {
    let dir = fresh_temp_dir("server_bench_dog").expect("fresh temp dir for dog server bench");
    let path = dir.join("dogs.mmap");
    let ids: Vec<Uuid> = (0..POOL_SIZE as u128).map(Uuid::from_u128).collect();
    let records: Vec<DogRecord> = ids
        .iter()
        .map(|&id| DogRecord::new(id, "labrador", 3))
        .collect();
    let store = ProductionStore::create(records, Vec::new(), &path)
        .expect("create ProductionStore for dog server bench");
    let connection_store = Arc::new(DogConnectionStore::new(store));
    let listener = TcpListener::bind("127.0.0.1:0").unwrap();
    let addr = listener.local_addr().unwrap();
    thread::spawn(move || serve(listener, connection_store, ServeOptions::default()));
    (addr, ids)
}

/// `start_dog_server` with a batch journal (`SERVER-001` FR-025,
/// ADR-0025): every `Request::Transaction` is appended and `fsync`'d
/// before its first write, so the `dog-jrnl-txn` rows measure exactly
/// the cost of crash-atomicity — one `fsync` per batch — against the
/// unjournaled `dog-txn` rows above them.
fn start_dog_server_journaled() -> (SocketAddr, Vec<Uuid>) {
    let dir = fresh_temp_dir("server_bench_dog_journaled")
        .expect("fresh temp dir for journaled dog server bench");
    let path = dir.join("dogs.mmap");
    let journal = dir.join("txn.journal");
    let ids: Vec<Uuid> = (0..POOL_SIZE as u128).map(Uuid::from_u128).collect();
    let records: Vec<DogRecord> = ids
        .iter()
        .map(|&id| DogRecord::new(id, "labrador", 3))
        .collect();
    let store = ProductionStore::create(records, Vec::new(), &path)
        .expect("create ProductionStore for journaled dog server bench");
    let connection_store = Arc::new(
        DogConnectionStore::with_journal(store, &journal)
            .expect("open the batch journal for the dog server bench"),
    );
    let listener = TcpListener::bind("127.0.0.1:0").unwrap();
    let addr = listener.local_addr().unwrap();
    thread::spawn(move || serve(listener, connection_store, ServeOptions::default()));
    (addr, ids)
}

fn start_order_server() -> (SocketAddr, Vec<Uuid>) {
    let dir = fresh_temp_dir("server_bench_order").expect("fresh temp dir for order server bench");
    let path = dir.join("amount.mmap");
    let ids: Vec<Uuid> = (0..POOL_SIZE as u128)
        .map(|n| Uuid::from_u128(1_000 + n))
        .collect();
    let orders: Vec<Order> = ids
        .iter()
        .enumerate()
        .map(|(i, &id)| Order {
            id,
            customer_id: Uuid::from_u128(9_000 + (i as u128 % 4)),
            amount_cents: 1_000 + i as i64,
            status: OrderStatus::Shipped,
            created_at_unix_ms: 0,
            discount_cents: 0,
        })
        .collect();
    let stack = create_order_production_stack(orders, &path)
        .expect("create OrderProductionStack for order server bench");
    let connection_store = Arc::new(OrderConnectionStore::new(GenericProductionStore::new(
        stack,
    )));
    let listener = TcpListener::bind("127.0.0.1:0").unwrap();
    let addr = listener.local_addr().unwrap();
    thread::spawn(move || serve(listener, connection_store, ServeOptions::default()));
    (addr, ids)
}

fn start_employee_server() -> (SocketAddr, Vec<Uuid>) {
    let dir =
        fresh_temp_dir("server_bench_employee").expect("fresh temp dir for employee server bench");
    let path = dir.join("salary.mmap");
    let ids: Vec<Uuid> = (0..POOL_SIZE as u128)
        .map(|n| Uuid::from_u128(2_000 + n))
        .collect();
    let manager_id = ids[0];
    let employees: Vec<Employee> = ids
        .iter()
        .enumerate()
        .map(|(i, &id)| Employee {
            id,
            name: format!("employee-{i}"),
            department: Department::Engineering,
            salary_cents: 100_000 + i as i64,
            manager_id: if id == manager_id {
                None
            } else {
                Some(manager_id)
            },
        })
        .collect();
    // A small collaboration chain (i collaborates_with i+1) — enough for a
    // real, non-empty `SymmetricRelation`, not exercised by `GetById`
    // itself but built the same way every other `Employee` fixture is.
    let edges: Vec<(Uuid, Uuid)> = ids.windows(2).map(|pair| (pair[0], pair[1])).collect();
    let stack = create_employee_production_stack(employees, &edges, &path)
        .expect("create EmployeeProductionStack for employee server bench");
    let connection_store = Arc::new(EmployeeConnectionStore::new(GenericProductionStore::new(
        stack,
    )));
    let listener = TcpListener::bind("127.0.0.1:0").unwrap();
    let addr = listener.local_addr().unwrap();
    thread::spawn(move || serve(listener, connection_store, ServeOptions::default()));
    (addr, ids)
}

/// Single-connection, zero-contention `GetById` round-trip latency —
/// average over [`LATENCY_ITERATIONS`] sequential requests on one client
/// connection, no concurrency at all. The baseline every throughput-sweep
/// row below is scaling up from.
fn measure_latency(addr: SocketAddr, ids: &[Uuid]) -> Duration {
    let mut client = connect(addr);
    let mut cursor = RoundRobin::new(ids.len());
    let start = Instant::now();
    for _ in 0..LATENCY_ITERATIONS {
        let id = ids[cursor.advance()];
        let resp = roundtrip(&mut client, &Request::GetById { id });
        debug_assert!(matches!(resp, Response::Record { .. }));
    }
    start.elapsed() / LATENCY_ITERATIONS as u32
}

/// `threads` real client connections, each opened before the barrier so
/// connection setup itself isn't counted, each firing [`OPS_PER_THREAD`]
/// sequential `GetById` requests against a shared, contended id pool once
/// every thread is ready. Aggregate ops/sec from the *slowest* thread's
/// elapsed time — same rationale as `benches/concurrency.rs`'s own
/// `run_throughput`: a slow straggler should show up as lower throughput,
/// not be averaged away.
fn run_throughput(addr: SocketAddr, ids: &Arc<Vec<Uuid>>, threads: usize) -> f64 {
    let barrier = Arc::new(Barrier::new(threads));
    let mut handles = Vec::with_capacity(threads);
    for thread_index in 0..threads {
        let ids = Arc::clone(ids);
        let barrier = Arc::clone(&barrier);
        handles.push(thread::spawn(move || {
            let mut client = connect(addr);
            let mut cursor = RoundRobin::new(ids.len());
            // Offset each thread's starting position so threads don't all
            // hammer the same id on the same iteration — genuine, not
            // artificially synchronized, contention across the pool.
            for _ in 0..thread_index {
                cursor.advance();
            }
            barrier.wait();
            let start = Instant::now();
            for _ in 0..OPS_PER_THREAD {
                let id = ids[cursor.advance()];
                let resp = roundtrip(&mut client, &Request::GetById { id });
                debug_assert!(matches!(resp, Response::Record { .. }));
            }
            start.elapsed()
        }));
    }

    let mut slowest = Duration::ZERO;
    for handle in handles {
        if let Ok(elapsed) = handle.join() {
            slowest = slowest.max(elapsed);
        }
    }

    let total_ops = (threads * OPS_PER_THREAD) as f64;
    total_ops / slowest.as_secs_f64()
}

fn bench_domain(name: &str, addr: SocketAddr, ids: Vec<Uuid>) {
    let latency = measure_latency(addr, &ids);
    println!(
        "{name:<10} {:>10} {:>14.1}",
        "latency",
        latency.as_secs_f64() * 1_000_000.0
    );

    let ids = Arc::new(ids);
    for &threads in &THREAD_COUNTS {
        let ops_per_sec = run_throughput(addr, &ids, threads);
        println!("{name:<10} {threads:>10} {ops_per_sec:>14.0}");
    }
}

/// Single-connection, zero-contention `Request::Transaction` round-trip
/// latency — the transaction analogue of [`measure_latency`]: a
/// two-operation batch touching two distinct ids from the pool each round
/// trip, both set to `value_at(i)`. Reuses [`LATENCY_ITERATIONS`] for
/// direct comparability with the single-operation baseline.
fn measure_transaction_latency(
    addr: SocketAddr,
    ids: &[Uuid],
    field: FieldRef,
    value_at: impl Fn(usize) -> ScanValue,
) -> Duration {
    let mut client = connect(addr);
    let mut cursor = RoundRobin::new(ids.len());
    let start = Instant::now();
    for i in 0..LATENCY_ITERATIONS {
        let id_a = ids[cursor.advance()];
        let id_b = ids[cursor.advance()];
        let resp = roundtrip(
            &mut client,
            &Request::Transaction {
                updates: vec![
                    TransactionOp {
                        id: id_a,
                        field,
                        value: value_at(i),
                    },
                    TransactionOp {
                        id: id_b,
                        field,
                        value: value_at(i),
                    },
                ],
            },
        );
        debug_assert_eq!(resp, Response::Ok);
    }
    start.elapsed() / LATENCY_ITERATIONS as u32
}

/// `threads` real client connections, each firing [`OPS_PER_THREAD`]
/// sequential two-operation `Request::Transaction` batches against a
/// shared, contended id pool — the transaction analogue of
/// [`run_throughput`], same `Barrier`-synchronized, slowest-thread-wins
/// measurement.
fn run_transaction_throughput(
    addr: SocketAddr,
    ids: &Arc<Vec<Uuid>>,
    threads: usize,
    field: FieldRef,
    value_at: impl Fn(usize) -> ScanValue + Send + Sync + Copy + 'static,
) -> f64 {
    let barrier = Arc::new(Barrier::new(threads));
    let mut handles = Vec::with_capacity(threads);
    for thread_index in 0..threads {
        let ids = Arc::clone(ids);
        let barrier = Arc::clone(&barrier);
        handles.push(thread::spawn(move || {
            let mut client = connect(addr);
            let mut cursor = RoundRobin::new(ids.len());
            for _ in 0..thread_index {
                cursor.advance();
            }
            barrier.wait();
            let start = Instant::now();
            for i in 0..OPS_PER_THREAD {
                let id_a = ids[cursor.advance()];
                let id_b = ids[cursor.advance()];
                let resp = roundtrip(
                    &mut client,
                    &Request::Transaction {
                        updates: vec![
                            TransactionOp {
                                id: id_a,
                                field,
                                value: value_at(i),
                            },
                            TransactionOp {
                                id: id_b,
                                field,
                                value: value_at(i),
                            },
                        ],
                    },
                );
                debug_assert_eq!(resp, Response::Ok);
            }
            start.elapsed()
        }));
    }

    let mut slowest = Duration::ZERO;
    for handle in handles {
        if let Ok(elapsed) = handle.join() {
            slowest = slowest.max(elapsed);
        }
    }

    let total_ops = (threads * OPS_PER_THREAD) as f64;
    total_ops / slowest.as_secs_f64()
}

fn bench_transaction_domain(
    name: &str,
    addr: SocketAddr,
    ids: Vec<Uuid>,
    field: FieldRef,
    value_at: impl Fn(usize) -> ScanValue + Send + Sync + Copy + 'static,
) {
    let txn_name = format!("{name}-txn");
    let latency = measure_transaction_latency(addr, &ids, field, value_at);
    println!(
        "{txn_name:<10} {:>10} {:>14.1}",
        "latency",
        latency.as_secs_f64() * 1_000_000.0
    );

    let ids = Arc::new(ids);
    for &threads in &THREAD_COUNTS {
        let ops_per_sec = run_transaction_throughput(addr, &ids, threads, field, value_at);
        println!("{txn_name:<10} {threads:>10} {ops_per_sec:>14.0}");
    }
}

fn main() {
    let available_parallelism = std::thread::available_parallelism()
        .map(|n| n.get())
        .unwrap_or(0);
    println!("std::thread::available_parallelism() reports: {available_parallelism}");
    println!(
        "(thread counts below are swept at 1/4/32/64 — this machine's real core \
         count and a deliberate 2x-oversubscription point, matching \
         benches/concurrency.rs's own baileyai sweep; see this module's own doc \
         comment for the container-then-Beast-then-baileyai history)"
    );
    println!();
    println!(
        "first row per domain is the single-connection latency baseline (µs/op); \
         remaining rows are aggregate throughput (ops/sec) at that thread count"
    );
    println!();
    println!("{:<10} {:>10} {:>14}", "domain", "threads", "value");

    let (addr, ids) = start_dog_server();
    bench_domain("dog", addr, ids.clone());
    bench_transaction_domain("dog", addr, ids, FIELD_AGE, |i| {
        ScanValue::U32((i % 1_000) as u32)
    });

    // FR-025: the same two-op batches through a journaled adapter — the
    // `fsync`-per-batch cost, isolated (`dog-jrnl-txn` vs. `dog-txn`).
    let (addr, ids) = start_dog_server_journaled();
    bench_transaction_domain("dog-jrnl", addr, ids, FIELD_AGE, |i| {
        ScanValue::U32((i % 1_000) as u32)
    });

    let (addr, ids) = start_order_server();
    bench_domain("order", addr, ids.clone());
    bench_transaction_domain("order", addr, ids, FIELD_AMOUNT, |i| {
        ScanValue::I64(1_000 + i as i64)
    });

    let (addr, ids) = start_employee_server();
    bench_domain("employee", addr, ids.clone());
    bench_transaction_domain("employee", addr, ids, FIELD_SALARY, |i| {
        ScanValue::I64(100_000 + i as i64)
    });

    bench_query_planner();
}

/// `ADR-0073` acceptance criterion 5: how many `Memory` records the
/// planner benchmark serves, and over how many distinct `category`
/// values they are spread — so one equality bucket is 1% of the table.
const PLANNER_RECORDS: usize = 100_000;
const PLANNER_CATEGORIES: usize = 100;
const PLANNER_QUERY_ITERATIONS: usize = 20;

/// A `Memory` table sized for the planner measurement, with `category`
/// (`filter_eq: true` — the generic equality index) and `source`
/// (`filter_eq: false` — never indexed) holding the *same* value per
/// record, so `WHERE source = v` is an equal-selectivity full-scan
/// control for `WHERE category = v`, with no test-only hook to force a
/// plan.
fn start_memory_planner_server() -> SocketAddr {
    let dir = fresh_temp_dir("server_bench_memory_planner")
        .expect("fresh temp dir for memory planner bench");
    let path = dir.join("memories.mmap");
    let memories: Vec<Memory> = (0..PLANNER_RECORDS as u128)
        .map(|n| {
            let bucket = format!("c{}", n as usize % PLANNER_CATEGORIES);
            Memory {
                id: Uuid::from_u128(n + 1),
                content: format!("memory {n}"),
                category: bucket.clone(),
                tags: vec![],
                source: bucket,
                metadata_json: "{}".into(),
                created_at_unix_ms: n as i64,
                updated_at_unix_ms: n as i64,
                memory_type: "unclassified".into(),
                status: "active".into(),
                sensitive: false,
                access_count: 0,
                deleted_at_unix_ms: 0,
                node_id: String::new(),
            }
        })
        .collect();
    let stack = create_memory_production_stack(memories, &[], &path)
        .expect("create MemoryProductionStack for planner bench");
    let connection_store = Arc::new(MemoryConnectionStore::new(GenericProductionStore::new(
        stack,
    )));
    let listener = TcpListener::bind("127.0.0.1:0").unwrap();
    let addr = listener.local_addr().unwrap();
    thread::spawn(move || serve(listener, connection_store, ServeOptions::default()));
    addr
}

/// One request round trip, timed over [`PLANNER_QUERY_ITERATIONS`] on one
/// connection after one untimed warm-up (so the first request's page-in
/// lands in neither column); returns the mean wall-clock per request and
/// the row or group count, which the caller asserts identical for the
/// indexed and the control field — the planner changes what is read, not
/// what is returned.
fn measure_planner_request(addr: SocketAddr, request: &Request) -> (Duration, usize) {
    let mut client = connect(addr);
    // Negotiate the current protocol first: `FilteredPage` is gated at
    // 26 (`FPG-FR-007`) and an un-negotiated connection sits below it
    // (`Query`/`Aggregate`, gated at 8/9, pass either way) — the same
    // `Hello` every integration test sends before its first request.
    match roundtrip(
        &mut client,
        &Request::Hello {
            protocol_version: PROTOCOL_VERSION,
        },
    ) {
        Response::Hello { .. } => {}
        other => panic!("unexpected Hello response {other:?}"),
    }
    let count = match roundtrip(&mut client, request) {
        Response::Rows { rows } => rows.len(),
        Response::Groups { groups } => groups.len(),
        other => panic!("unexpected response {other:?}"),
    };
    let start = Instant::now();
    for _ in 0..PLANNER_QUERY_ITERATIONS {
        let resp = roundtrip(&mut client, request);
        debug_assert!(matches!(
            resp,
            Response::Rows { .. } | Response::Groups { .. }
        ));
    }
    (start.elapsed() / PLANNER_QUERY_ITERATIONS as u32, count)
}

fn c7_filter(field: FieldRef) -> Vec<Predicate> {
    vec![Predicate {
        field,
        op: CompareOp::Eq,
        value: ScanValue::Str("c7".into()),
    }]
}

/// `ADR-0075` acceptance criterion 7: the same 1% of the table as a
/// two-sided range — record *n* carries `created_at = updated_at = n`,
/// so `[50_000, 51_000)` names exactly 1,000 records on either field.
/// `updated_at_unix_ms` is the `Ordered` field (`QueryPlan::IndexRange`);
/// `created_at_unix_ms` is never range-indexed (the `FullScan` control).
const PLANNER_RANGE_LOWER: i64 = 50_000;
const PLANNER_RANGE_UPPER: i64 = 51_000;

fn range_filter(field: FieldRef) -> Vec<Predicate> {
    vec![
        Predicate {
            field,
            op: CompareOp::Ge,
            value: ScanValue::I64(PLANNER_RANGE_LOWER),
        },
        Predicate {
            field,
            op: CompareOp::Lt,
            value: ScanValue::I64(PLANNER_RANGE_UPPER),
        },
    ]
}

/// `ADR-0076` acceptance criterion: the `since`-shaped listing — one
/// lower bound admitting ~99% of the table (`field >= 1_000`), paged 50
/// at a time by the same field. Under `ADR-0075` alone the walk reads
/// every in-range record before `page_rows` cuts the page; the bounded
/// walk reads the page. `created_at_unix_ms` is the full-scan control.
const PLANNER_SINCE: i64 = 1_000;

fn since_filter(field: FieldRef) -> Vec<Predicate> {
    vec![Predicate {
        field,
        op: CompareOp::Ge,
        value: ScanValue::I64(PLANNER_SINCE),
    }]
}

/// The `since`-shaped page alone — `Query`/`Aggregate` over ~99,000 rows
/// would measure response shipping, not the planner. Paged by `field`
/// itself: the indexed request by `updated_at_unix_ms`, the control by
/// `created_at_unix_ms` (identical values, so the identical sequence —
/// and, since `ADR-0077` widened the walk to every page ordered by the
/// range field, the one way a control cannot walk).
fn since_page_request(field: FieldRef) -> [(&'static str, Request); 1] {
    [(
        "fpage-50",
        Request::FilteredPage {
            order_by: field,
            after: None,
            limit: 50,
            filter: since_filter(field),
        },
    )]
}

/// `ADR-0077` acceptance criterion: the *mixed* `since`-shaped page —
/// the same ~99% bound on `range_field` plus a 1%-selective equality on
/// `eq_field` (`= 'c7'`), paged 50 at a time by the range field. With
/// `eq_field` the never-indexed `source`, the walk-past-rejects round
/// reads ~50 × 100 records and stops, where `ADR-0076` alone read every
/// in-range record; with `eq_field` the declared `category` index the
/// planner's equality-first rule keeps the bucket (`FPM-FR-001`), so
/// that row measures what the rule costs, not what the walk saves.
fn since_mixed_page_request(
    range_field: FieldRef,
    eq_field: FieldRef,
) -> [(&'static str, Request); 1] {
    let mut filter = since_filter(range_field);
    filter.extend(c7_filter(eq_field));
    [(
        "fpage-50",
        Request::FilteredPage {
            order_by: range_field,
            after: None,
            limit: 50,
            filter,
        },
    )]
}

/// The three planner consumers this benchmark measures, each with the
/// identical 1%-selective equality on `field`: `Query` (`ADR-0073`),
/// and `Aggregate` / `FilteredPage` (`ADR-0074`). `Join` is not measured
/// — its left side is this same candidate fetch and its cost is
/// dominated by the per-left-row right-side lookups the planner does not
/// touch (`SERVER-QUERY-PLANNER-CONSUMERS-DESIGN.md`, acceptance
/// criterion 4). The page is ordered by `page_by`: `updated_at_unix_ms`
/// for the indexed request, `created_at_unix_ms` (identical values,
/// identical sequence) for the control — since `ADR-0077` a page
/// ordered by the range field walks whenever the filter does not plan
/// the equality index, so a control ordered by it would no longer be a
/// full scan.
fn planner_requests(filter: Vec<Predicate>, page_by: FieldRef) -> [(&'static str, Request); 3] {
    [
        (
            "query",
            Request::Query {
                select: Selection::Fields(vec![FIELD_CATEGORY]),
                filter: filter.clone(),
                limit: None,
            },
        ),
        (
            "count(*)",
            Request::Aggregate {
                group_by: vec![],
                filter: filter.clone(),
                aggregates: vec![AggregateSpec {
                    func: AggregateFn::Count,
                    field: None,
                }],
                limit: None,
            },
        ),
        (
            "fpage-50",
            Request::FilteredPage {
                order_by: page_by,
                after: None,
                limit: 50,
                filter,
            },
        ),
    ]
}

/// One indexed/control pair per consumer: the same request shape with
/// the planner's field and with its never-indexed twin holding identical
/// values, the counts asserted equal — the planner changes what is read,
/// not what is returned.
fn measure_planner_pair<const N: usize>(
    addr: SocketAddr,
    plan: &str,
    indexed_requests: [(&'static str, Request); N],
    indexed_clause: &str,
    control_requests: [(&'static str, Request); N],
    control_clause: &str,
) {
    for ((label, indexed_request), (_, control_request)) in
        indexed_requests.iter().zip(control_requests.iter())
    {
        let (indexed, indexed_count) = measure_planner_request(addr, indexed_request);
        let (scanned, scanned_count) = measure_planner_request(addr, control_request);
        assert_eq!(
            indexed_count, scanned_count,
            "{label}: the planner must not change the result"
        );
        println!(
            "{:<10} {:>11} {:>14.1}   {label} {indexed_clause} (indexed, {indexed_count} rows/groups of {PLANNER_RECORDS})",
            "memory-planner",
            plan,
            indexed.as_secs_f64() * 1e6
        );
        println!(
            "{:<10} {:>11} {:>14.1}   {label} {control_clause} (unindexed control, same {scanned_count})",
            "memory-planner",
            "full-scan",
            scanned.as_secs_f64() * 1e6
        );
    }
}

/// `ADR-0073` acceptance criterion 5 and `ADR-0074` acceptance criterion 4
/// (`docs/design/SERVER-QUERY-PLANNER{,-CONSUMERS}-DESIGN.md`): the same
/// 1%-selective equality over a 100K `Memory` table, once through the
/// declared `category` index (`QueryPlan::IndexEq`) and once through the
/// unindexed `source` field (`QueryPlan::FullScan`) holding identical
/// values — the difference is exactly what the planner saves — for each
/// consumer that plans. Reported beside the other domains' rows as
/// `memory-planner`, in µs per request.
fn bench_query_planner() {
    let addr = start_memory_planner_server();
    measure_planner_pair(
        addr,
        "index-eq",
        planner_requests(c7_filter(FIELD_CATEGORY), FIELD_UPDATED_AT),
        "WHERE category = 'c7'",
        planner_requests(c7_filter(FIELD_SOURCE), FIELD_CREATED_AT),
        "WHERE source = 'c7'",
    );
    // `ADR-0075` acceptance criterion 7: the `Ordered` walk vs. the scan.
    measure_planner_pair(
        addr,
        "index-range",
        planner_requests(range_filter(FIELD_UPDATED_AT), FIELD_UPDATED_AT),
        "WHERE 50000 <= updated_at_unix_ms < 51000",
        planner_requests(range_filter(FIELD_CREATED_AT), FIELD_CREATED_AT),
        "WHERE 50000 <= created_at_unix_ms < 51000",
    );
    // `ADR-0076`: the `since`-shaped page — a bound admitting ~99% of
    // the table, paged by the same field.
    measure_planner_pair(
        addr,
        "index-since",
        since_page_request(FIELD_UPDATED_AT),
        "WHERE updated_at_unix_ms >= 1000 ORDER BY updated_at_unix_ms",
        since_page_request(FIELD_CREATED_AT),
        "WHERE created_at_unix_ms >= 1000 ORDER BY created_at_unix_ms",
    );
    // `ADR-0077`: the mixed `since`-shaped page — the same bound plus an
    // unindexed 1%-selective equality the walk must pass rejects for.
    measure_planner_pair(
        addr,
        "since-mixed",
        since_mixed_page_request(FIELD_UPDATED_AT, FIELD_SOURCE),
        "WHERE updated_at_unix_ms >= 1000 AND source = 'c7' ORDER BY updated_at_unix_ms",
        since_mixed_page_request(FIELD_CREATED_AT, FIELD_SOURCE),
        "WHERE created_at_unix_ms >= 1000 AND source = 'c7' ORDER BY created_at_unix_ms",
    );
    // `ADR-0077`: the same shape with the equality on the *declared*
    // index — equality-first keeps the bucket, so this row is the rule's
    // cost, measured rather than assumed.
    measure_planner_pair(
        addr,
        "since-eq",
        since_mixed_page_request(FIELD_UPDATED_AT, FIELD_CATEGORY),
        "WHERE updated_at_unix_ms >= 1000 AND category = 'c7' ORDER BY updated_at_unix_ms",
        since_mixed_page_request(FIELD_CREATED_AT, FIELD_SOURCE),
        "WHERE created_at_unix_ms >= 1000 AND source = 'c7' ORDER BY created_at_unix_ms",
    );
}
