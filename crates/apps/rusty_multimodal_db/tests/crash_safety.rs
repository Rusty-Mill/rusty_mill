//! Crash safety as a regression gate (`ADR-0095`, `CSC-FR-001` through
//! `CSC-FR-004`): the four trials `src/bin/crash_safety_harness.rs`
//! runs by hand, each spawning the real `crash_writer` binary and
//! `SIGKILL`ing it mid-work, asserted rather than printed, so `cargo
//! test --all-features` (CI's `test` job) fails the moment a torn slot
//! is treated as committed or a flushed update goes missing.
//!
//! The harness's own scope note holds here exactly: `SIGKILL` leaves
//! the page cache intact, so these trials prove survival of a *process*
//! crash, never of power loss. What each asserts:
//!
//! 1. **Flushed then killed** — every update `Flush` returned for is
//!    visible after reopen (`CSC-FR-001`).
//! 2. **Torn slot write** — killed after the id and before the value
//!    or marker, the new slot reads as absent and the caller's own
//!    record is reseeded; the uninterrupted control run keeps the
//!    attempted value (`CSC-FR-002`).
//! 3. **Torn in-place update** — killed mid-burst, the value is exactly
//!    one of the two written patterns, never a mix (`CSC-FR-003`).
//! 4. **Unflushed, killed mid-stream** — every record reads as either
//!    its seed or its update, never a third value; the count that
//!    survives is reported, not asserted (`CSC-FR-004`).
//!
//! Fewer repeats than the harness (3, not 8): a regression is
//! deterministic here (the torn-write kill lands on a sync line), and
//! the gate runs on every push.

#![cfg(all(unix, feature = "research"))]

use rusty_multimodal_db::bench_support::fresh_temp_dir;
use rusty_multimodal_db::generic::mmap_store::GenericMmapStore;
use rusty_multimodal_db::generic::order_customer::{Amount, Order, OrderStatus, Status};
use rusty_multimodal_db::generic::query::GetById;
use std::io::{BufRead, BufReader};
use std::path::{Path, PathBuf};
use std::process::{Command, Stdio};
use std::time::Duration;
use uuid::Uuid;

const TRIALS: usize = 3;
const RECORD_COUNT: usize = 500;
const KILL_AFTER: usize = 250;
const TORN_UPDATE_ITERATIONS: u64 = 500_000_000;
const TORN_UPDATE_KILL_DELAY: Duration = Duration::from_millis(2);
const TORN_WRITE_NEW_ID: u128 = 9_999;
const TORN_WRITE_ATTEMPTED_VALUE: i64 = 424_242;
const TORN_WRITE_RESEED_VALUE: i64 = 999_999;
const TORN_UPDATE_ID: u128 = 55_555;

fn writer() -> &'static str {
    env!("CARGO_BIN_EXE_crash_writer")
}

fn make_orders(count: usize) -> Vec<Order> {
    (0..count)
        .map(|i| Order {
            id: Uuid::from_u128((i + 1) as u128),
            customer_id: Uuid::from_u128(1),
            amount_cents: i as i64,
            status: OrderStatus::Pending,
            created_at_unix_ms: 0,
            discount_cents: 0,
        })
        .collect()
}

struct TempDir(PathBuf);

impl Drop for TempDir {
    fn drop(&mut self) {
        let _ = std::fs::remove_dir_all(&self.0);
    }
}

fn temp_dir(label: &str) -> TempDir {
    TempDir(fresh_temp_dir(label).unwrap())
}

/// Spawn the writer, read its stdout until `target_line`, `SIGKILL` it.
fn spawn_and_kill_on(args: &[&str], target_line: &str) {
    let mut child = Command::new(writer())
        .args(args)
        .stdout(Stdio::piped())
        .spawn()
        .unwrap();
    let stdout = child.stdout.take().unwrap();
    for line in BufReader::new(stdout).lines() {
        if line.unwrap() == target_line {
            break;
        }
    }
    child.kill().unwrap();
    let status = child.wait().unwrap();
    assert!(
        !status.success(),
        "the writer finished before it could be killed: {status:?}"
    );
}

fn spawn_and_kill_after(args: &[&str], delay: Duration) {
    let mut child = Command::new(writer())
        .args(args)
        .stdout(Stdio::null())
        .spawn()
        .unwrap();
    std::thread::sleep(delay);
    child.kill().unwrap();
    let status = child.wait().unwrap();
    assert!(
        !status.success(),
        "the writer finished before it could be killed: {status:?}"
    );
}

fn amount_of(store: &GenericMmapStore<Order, Status, Amount>, id: u128) -> i64 {
    GetById::get(store, Uuid::from_u128(id))
        .expect("the id is in `records`")
        .amount_cents
}

/// `CSC-FR-001`: `Flush` returned, then the process died — every
/// update it flushed is there after reopen.
#[test]
fn every_update_flushed_before_the_kill_survives_reopen() {
    for trial in 0..TRIALS {
        let dir = temp_dir(&format!("crash_gate_flushed_{trial}"));
        let path = dir.0.join("orders.mmap");
        spawn_and_kill_on(
            &[
                "flushed-updates",
                path.to_str().unwrap(),
                &RECORD_COUNT.to_string(),
            ],
            "FLUSHED",
        );
        let store =
            GenericMmapStore::<Order, Status, Amount>::open(make_orders(RECORD_COUNT), &path)
                .unwrap();
        let survived = (0..RECORD_COUNT)
            .filter(|&i| amount_of(&store, (i + 1) as u128) == 1_000_000 + i as i64)
            .count();
        assert_eq!(
            survived, RECORD_COUNT,
            "trial {trial}: only {survived}/{RECORD_COUNT} flushed updates survived"
        );
    }
}

fn torn_write_baseline(label: &str) -> (TempDir, PathBuf, usize) {
    let dir = temp_dir(label);
    let path = dir.0.join("orders.mmap");
    let existing = make_orders(3);
    let count = existing.len();
    drop(GenericMmapStore::<Order, Status, Amount>::create(existing, &path).unwrap());
    (dir, path, count)
}

fn observe_after_torn_write(path: &Path) -> i64 {
    let mut records = make_orders(3);
    records.push(Order {
        id: Uuid::from_u128(TORN_WRITE_NEW_ID),
        customer_id: Uuid::from_u128(1),
        amount_cents: TORN_WRITE_RESEED_VALUE,
        status: OrderStatus::Pending,
        created_at_unix_ms: 0,
        discount_cents: 0,
    });
    let store = GenericMmapStore::<Order, Status, Amount>::open(records, path).unwrap();
    amount_of(&store, TORN_WRITE_NEW_ID)
}

fn torn_write_args<'a>(path: &'a str, existing: &'a str) -> [&'a str; 5] {
    [
        "torn-write",
        path,
        existing,
        "9999",   // TORN_WRITE_NEW_ID
        "424242", // TORN_WRITE_ATTEMPTED_VALUE
    ]
}

/// `CSC-FR-002`: the control run keeps the attempted value; a kill
/// after the id, or after the value but before the marker, leaves a
/// slot that reads as absent and is reseeded from the caller's record.
#[test]
fn a_torn_slot_write_is_excluded_on_reopen_and_the_control_run_is_kept() {
    let (_dir, path, existing) = torn_write_baseline("crash_gate_torn_control");
    let status = Command::new(writer())
        .args(torn_write_args(
            path.to_str().unwrap(),
            &existing.to_string(),
        ))
        .stdout(Stdio::null())
        .status()
        .unwrap();
    assert!(status.success(), "the control writer should exit cleanly");
    assert_eq!(
        observe_after_torn_write(&path),
        TORN_WRITE_ATTEMPTED_VALUE,
        "uninterrupted, the attempted value must survive — else the trial proves nothing"
    );

    for kill_on in ["ID_WRITTEN", "VALUE_WRITTEN"] {
        for trial in 0..TRIALS {
            let (_dir, path, existing) =
                torn_write_baseline(&format!("crash_gate_torn_{kill_on}_{trial}"));
            spawn_and_kill_on(
                &torn_write_args(path.to_str().unwrap(), &existing.to_string()),
                kill_on,
            );
            let observed = observe_after_torn_write(&path);
            assert_eq!(
                observed, TORN_WRITE_RESEED_VALUE,
                "killed after {kill_on}, trial {trial}: observed {observed} — a torn slot was \
                 treated as committed"
            );
        }
    }
}

/// `CSC-FR-003`: killed mid-burst, the in-place value is exactly one of
/// the two patterns the writer alternates, never a mix of both.
#[test]
fn a_torn_in_place_update_reads_as_exactly_one_written_pattern() {
    let pattern_a = i64::from_le_bytes([0x11u8; 8]);
    let pattern_b = i64::from_le_bytes([0x22u8; 8]);
    let id = Uuid::from_u128(TORN_UPDATE_ID);
    for trial in 0..TRIALS {
        let dir = temp_dir(&format!("crash_gate_torn_update_{trial}"));
        let path = dir.0.join("orders.mmap");
        let seed = Order {
            id,
            customer_id: Uuid::from_u128(1),
            amount_cents: pattern_a,
            status: OrderStatus::Pending,
            created_at_unix_ms: 0,
            discount_cents: 0,
        };
        drop(GenericMmapStore::<Order, Status, Amount>::create(vec![seed.clone()], &path).unwrap());
        spawn_and_kill_after(
            &[
                "torn-update",
                path.to_str().unwrap(),
                &TORN_UPDATE_ID.to_string(),
                &TORN_UPDATE_ITERATIONS.to_string(),
            ],
            TORN_UPDATE_KILL_DELAY,
        );
        let store = GenericMmapStore::<Order, Status, Amount>::open(vec![seed], &path).unwrap();
        let observed = amount_of(&store, TORN_UPDATE_ID);
        assert!(
            observed == pattern_a || observed == pattern_b,
            "trial {trial}: observed {observed:#x}, neither pattern — a torn in-place update"
        );
    }
}

/// `CSC-FR-004`: killed mid-stream with nothing flushed, every record
/// still reads as its seed or its update — never a third value. How
/// many survived is printed for the log, not asserted: a process crash
/// leaves the page cache intact, so it is expected to be all of them,
/// but that is the kernel's promise, not this crate's.
#[test]
fn an_unflushed_kill_leaves_no_record_with_a_value_it_was_never_given() {
    for trial in 0..TRIALS {
        let dir = temp_dir(&format!("crash_gate_unflushed_{trial}"));
        let path = dir.0.join("orders.mmap");
        spawn_and_kill_on(
            &[
                "unflushed-updates",
                path.to_str().unwrap(),
                &RECORD_COUNT.to_string(),
            ],
            &format!("WROTE {}", KILL_AFTER - 1),
        );
        let store =
            GenericMmapStore::<Order, Status, Amount>::open(make_orders(RECORD_COUNT), &path)
                .unwrap();
        let mut survived = 0;
        for i in 0..RECORD_COUNT {
            let observed = amount_of(&store, (i + 1) as u128);
            let seed = i as i64;
            let updated = 1_000_000 + seed;
            assert!(
                observed == seed || observed == updated,
                "trial {trial}, record {i}: observed {observed}, neither seed nor update"
            );
            if observed == updated && i < KILL_AFTER {
                survived += 1;
            }
        }
        println!("trial {trial}: {survived}/{KILL_AFTER} confirmed-before-kill updates survived");
    }
}
