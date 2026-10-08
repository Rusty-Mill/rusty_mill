#![allow(clippy::expect_used, clippy::unwrap_used)]
//! Stage-2 exit criteria for the file store: durable append and reopen, a
//! crash mid-batch, a revision conflict between two handles, blob consistency.
mod common;
use common::*;
use rusty_bbp::fs_store::{lock_for_test, truncate_log_for_test};
use rusty_bbp::*;
use std::path::{Path, PathBuf};
use std::sync::atomic::{AtomicU64, Ordering};

static N: AtomicU64 = AtomicU64::new(0);

fn tempdir() -> PathBuf {
    let n = N.fetch_add(1, Ordering::Relaxed);
    let dir = std::env::temp_dir().join(format!("rusty_bbp_{}_{n}", std::process::id()));
    let _ = std::fs::remove_dir_all(&dir);
    dir
}

fn open(dir: &Path) -> FsStore {
    FsStore::open(dir).expect("open store")
}

fn reopened_state(dir: &Path, task: &TaskId) -> TaskState {
    let mut d = Driver::new(open(dir), task.clone());
    d.reload().expect("reload");
    d.state
}

#[test]
fn append_is_durable_across_reopen() {
    let dir = tempdir();
    let mut fx = Fx::with_store(open(&dir));
    let c = fx.reach_approved();
    ok(fx.human(HumanAction::MergeReceipt {
        candidate: c,
        revision: "abc123".into(),
    }));
    assert_eq!(fx.st().state, State::Closed);
    let task = fx.st().task.clone();
    let live = fx.st().clone();
    drop(fx);
    let back = reopened_state(&dir, &task);
    assert_eq!(back, live, "fold of the on-disk log equals the live state");
    assert_eq!(back.rev, live.rev);
    let _ = std::fs::remove_dir_all(&dir);
}

#[test]
fn crash_mid_batch_drops_only_the_partial_batch() {
    let dir = tempdir();
    let mut fx = Fx::with_store(open(&dir));
    fx.reach_build();
    let before = fx.rev();
    let task = fx.st().task.clone();
    // A whole batch lands, then "the machine dies" halfway through the next one:
    // simulate by appending a batch and cutting its line in half.
    let log = dir.join("tasks").join("T1.log");
    let len_before = std::fs::metadata(&log).expect("meta").len();
    posted(fx.post(Role::Coder, ask(Role::Tester, "Which 429 cases matter?")));
    let len_after = std::fs::metadata(&log).expect("meta").len();
    assert!(len_after > len_before);
    truncate_log_for_test(&dir, &task, len_before + (len_after - len_before) / 2)
        .expect("truncate");
    drop(fx);
    let back = reopened_state(&dir, &task);
    assert_eq!(
        back.rev, before,
        "partial batch discarded, earlier batches intact"
    );
    assert_eq!(
        std::fs::metadata(&log).expect("meta").len(),
        len_before,
        "file truncated to the last complete line"
    );
    // The store is usable again: the same command appends cleanly.
    let mut fx = Fx::with_store(open(&dir));
    fx.d.reload().expect("reload");
    assert_eq!(fx.st().state, State::Build);
    assert_eq!(fx.turn_role(), Some(Role::Coder));
    posted(fx.post(Role::Coder, ask(Role::Tester, "Which 429 cases matter?")));
    let _ = std::fs::remove_dir_all(&dir);
}

#[test]
fn two_handles_conflict_and_the_driver_recovers() {
    let dir = tempdir();
    let mut a = Fx::with_store(open(&dir));
    a.reach_build();
    let task = a.st().task.clone();
    // Handle B opens the same directory with the same view, then A moves on.
    let mut b = Driver::new(open(&dir), task.clone());
    b.reload().expect("reload b");
    assert_eq!(b.state.rev, a.rev());
    posted(a.post(Role::Coder, ask(Role::Tester, "first")));
    // B's view is stale. A raw append with the stale rev is refused with the real one.
    let stale = b.state.rev;
    let err = b
        .store
        .append(
            &task,
            stale,
            &[Event::BudgetExtended {
                field: BudgetField::Reads,
                limit: 999,
            }],
        )
        .err();
    assert!(
        matches!(err, Some(StoreError::Conflict { expected, actual }) if expected == stale && actual == a.rev())
    );
    // Through the driver, the conflict is absorbed: reload, recompute, append.
    let resp = b
        .dispatch(
            &Command::Human {
                op: OpId("b-1".into()),
                action: HumanAction::BudgetExtended {
                    field: BudgetField::Reads,
                    limit: 999,
                },
            },
            Time(2_000),
        )
        .expect("dispatch");
    assert_eq!(resp, Response::Ok);
    assert_eq!(b.state.budget.reads, 999);
    // The extension plus its op receipt: two events.
    assert_eq!(b.state.rev.0, a.rev().0 + 2);
    // A is now stale and recovers the same way.
    posted(a.post(Role::Coder, ask(Role::Tester, "second")));
    assert_eq!(
        a.st().budget.reads,
        999,
        "A reloaded B's extension before applying its own command"
    );
    let _ = std::fs::remove_dir_all(&dir);
}

#[test]
fn blobs_survive_reopen_and_are_verified() {
    let dir = tempdir();
    let mut fx = Fx::with_store(open(&dir));
    fx.reach_test();
    let diff = fx
        .st()
        .artifacts
        .values()
        .find(|a| a.kind == ArtifactKind::Diff)
        .expect("diff")
        .blob;
    drop(fx);
    let store = open(&dir);
    let bytes = store.blob_get(&diff.sha).expect("blob after reopen");
    assert_eq!(bytes.len() as u64, diff.len);
    assert_eq!(Sha256::of(&bytes), diff.sha);
    assert_eq!(
        store.blob_get(&Sha256([1; 32])).err(),
        Some(StoreError::UnknownBlob)
    );
    // A corrupted blob file is detected on read.
    std::fs::write(dir.join("blobs").join(diff.sha.hex()), b"tampered").expect("write");
    assert!(matches!(
        store.blob_get(&diff.sha),
        Err(StoreError::Backend(_))
    ));
    let _ = std::fs::remove_dir_all(&dir);
}

#[test]
fn every_referenced_blob_exists_after_reopen() {
    let dir = tempdir();
    let mut fx = Fx::with_store(open(&dir));
    fx.reach_merge_gate();
    let task = fx.st().task.clone();
    drop(fx);
    let store = open(&dir);
    let state = TaskState::fold(task.clone(), &store.events(&task, 0).expect("events"));
    assert!(state.artifacts.len() >= 6);
    for art in state.artifacts.values() {
        let bytes = store
            .blob_get(&art.blob.sha)
            .expect("referenced blob present");
        assert_eq!(bytes.len() as u64, art.blob.len);
    }
    let _ = std::fs::remove_dir_all(&dir);
}

/// A process mid-operation holds the directory lock; another process's open
/// (with its tail repair) waits for it instead of racing it.
#[test]
fn open_waits_for_a_process_that_holds_the_lock() {
    let dir = tempdir();
    let task = TaskId("T1".into());
    let mut fx = Fx::with_store(open(&dir));
    fx.reach_build();
    let before = fx.st().rev;
    // The "other process" is mid-write: it holds the lock while its batch is
    // not yet complete on disk.
    let held = lock_for_test(&dir).expect("lock");
    // The last line of the log is the batch that will be cut; everything
    // before it must survive exactly.
    let log_path = dir.join("tasks").join("T1.log");
    let text = std::fs::read_to_string(&log_path).expect("read log");
    let last_line = text.lines().last().expect("a batch");
    let cut: Vec<Event> = rusty_serde::json::from_str(last_line).expect("batch json");
    let all = fx.d.store.events(&task, 0).expect("events");
    let expected: Vec<Event> = all[..all.len() - cut.len()].to_vec();
    let size = text.len() as u64;
    truncate_log_for_test(&dir, &task, size - 5).expect("truncate");
    let (tx, rx) = std::sync::mpsc::channel();
    let dir2 = dir.clone();
    let opener = std::thread::spawn(move || {
        let store = open(&dir2);
        let events = store.events(&TaskId("T1".into()), 0).expect("events");
        tx.send(events).expect("send");
    });
    assert!(
        rx.recv_timeout(std::time::Duration::from_millis(300))
            .is_err(),
        "open must block while another process holds the lock"
    );
    drop(held);
    let seen = rx
        .recv_timeout(std::time::Duration::from_secs(10))
        .expect("open completes once the lock is released");
    opener.join().expect("join");
    // Repair ran after the lock was released and dropped exactly the cut
    // batch: every earlier batch is intact, in order.
    assert_eq!(seen.len() as u64, before.0 - cut.len() as u64);
    assert_eq!(seen, expected, "only the cut batch is gone");
    let _ = std::fs::remove_dir_all(&dir);
}

/// Two writers racing on one directory: per round exactly one append lands and
/// the other conflicts, and every acknowledged batch survives reopen.
#[test]
fn racing_writers_never_both_pass_the_revision_check() {
    let dir = tempdir();
    let task = TaskId("T1".into());
    let fx = Fx::with_store(open(&dir));
    let base = fx.st().rev;
    let payload = fx.d.store.events(&task, 0).expect("events")[0].clone();
    let rounds = 25;
    let barrier = std::sync::Arc::new(std::sync::Barrier::new(2));
    let writer = |dir: PathBuf, barrier: std::sync::Arc<std::sync::Barrier>, ev: Event| {
        std::thread::spawn(move || {
            let task = TaskId("T1".into());
            let mut store = open(&dir);
            let mut outcomes = Vec::with_capacity(rounds);
            for _ in 0..rounds {
                store.refresh(&task).expect("refresh");
                let rev = store.rev(&task);
                barrier.wait();
                let landed = match store.append(&task, rev, std::slice::from_ref(&ev)) {
                    Ok(()) => true,
                    Err(StoreError::Conflict { .. }) => false,
                    Err(e) => panic!("unexpected {e:?}"),
                };
                outcomes.push(landed);
                barrier.wait();
            }
            outcomes
        })
    };
    let a = writer(dir.clone(), barrier.clone(), payload.clone());
    let b = writer(dir.clone(), barrier, payload);
    let (a, b) = (a.join().expect("a"), b.join().expect("b"));
    for (round, (x, y)) in a.iter().zip(&b).enumerate() {
        assert!(
            x != y,
            "round {round}: exactly one writer lands and the other conflicts, got {x} and {y}"
        );
    }
    let reopened = open(&dir);
    assert_eq!(
        reopened.rev(&task).0,
        base.0 + rounds as u64,
        "one batch per round is on disk and nothing else"
    );
    let _ = std::fs::remove_dir_all(&dir);
}
