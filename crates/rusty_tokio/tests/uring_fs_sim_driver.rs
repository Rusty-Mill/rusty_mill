#![cfg(target_os = "linux")]
// io-uring-fs is a Cargo feature, not a target predicate -- required-features
// can't express "only on Linux", so a plain `--features io-uring-fs` build on
// another OS still tries to compile this file against items `src/io/mod.rs`
// only re-exports under `cfg(target_os = "linux")`. This file-level cfg is what
// actually keeps it Linux-only.
//! Item 4 (recommended DST seam): [`SimDriver`] -- a fully in-memory,
//! deterministic [`OpDriver`] implementation with the three
//! fault-injection knobs a storage engine's own crash-recovery tests
//! actually need. Every test here runs the exact same [`UringFile`] API
//! a caller would use against the real io_uring driver -- only the
//! driver passed to [`UringFile::create_on`]/[`UringFile::open_on`]
//! differs -- so this is also a live demonstration of the seam itself:
//! swap `SimDriver::new()` for `global_driver()` and a storage engine's
//! real recovery code runs unmodified against simulated faults.

use rusty_tokio::io::{OpDriver, SimDriver, UringFile};
use std::sync::Arc;

fn rt() -> rusty_tokio::Runtime {
    rusty_tokio::Runtime::new().unwrap()
}

#[test]
fn basic_read_write_round_trips_through_sim_driver() {
    let rt = rt();
    let driver: Arc<dyn OpDriver> = SimDriver::new();

    rt.block_on(async {
        let file = UringFile::create_on(driver.clone(), "/virtual/basic.dat")
            .await
            .unwrap();
        let payload = b"hello sim driver".to_vec();
        let result = file.write_at(payload.clone(), 0).await;
        assert_eq!(result.0.unwrap(), payload.len());

        let buf = vec![0u8; payload.len()];
        let result = file.read_at(buf, 0).await;
        assert_eq!(result.0.unwrap(), payload.len());
        assert_eq!(result.1, payload);
    });
}

#[test]
fn sim_driver_open_semantics_match_real_posix_flags() {
    let rt = rt();
    let driver: Arc<dyn OpDriver> = SimDriver::new();

    rt.block_on(async {
        // Opening without `create` on a nonexistent path fails.
        let err = UringFile::open_on(driver.clone(), "/virtual/missing.dat")
            .await
            .unwrap_err();
        assert_eq!(err.kind(), std::io::ErrorKind::NotFound);

        // `create_new` on an already-existing path fails.
        UringFile::create_on(driver.clone(), "/virtual/exists.dat")
            .await
            .unwrap();
        let err = UringFile::options()
            .write(true)
            .create_new(true)
            .open_on(driver.clone(), "/virtual/exists.dat")
            .await
            .unwrap_err();
        assert_eq!(err.kind(), std::io::ErrorKind::AlreadyExists);
    });
}

/// The torn-write fault: a write that *reports* full success while only
/// partially landing -- exactly the hazard a real crash mid-write
/// produces, and exactly what a storage engine's own checksum/CRC
/// recovery logic has to detect.
#[test]
fn injected_torn_write_reports_success_but_only_partially_lands() {
    let rt = rt();
    let sim = SimDriver::new();
    let driver: Arc<dyn OpDriver> = sim.clone();

    rt.block_on(async {
        let file = UringFile::create_on(driver, "/virtual/torn.dat")
            .await
            .unwrap();

        sim.inject_torn_write(0.5);
        let payload = vec![0xAAu8; 100];
        let result = file.write_at(payload.clone(), 0).await;
        // Reports the full length as written -- the fault is silent.
        assert_eq!(result.0.unwrap(), 100);

        let buf = vec![0u8; 100];
        let result = file.read_at(buf, 0).await;
        assert_eq!(result.0.unwrap(), 100);
        // Only the first half actually landed; the rest is whatever was
        // there before (zeros, for a freshly grown file) -- a real
        // storage engine's checksum over this record would catch the
        // mismatch between "claimed 100 bytes of 0xAA" and reality.
        assert!(result.1[..50].iter().all(|&b| b == 0xAA));
        assert!(result.1[50..].iter().all(|&b| b != 0xAA));

        // One-shot: the *next* write is unaffected.
        let result = file.write_at(vec![0xBBu8; 20], 0).await;
        assert_eq!(result.0.unwrap(), 20);
        let buf = vec![0u8; 20];
        let result = file.read_at(buf, 0).await;
        assert!(result.1.iter().all(|&b| b == 0xBB));
    });
}

/// The lying-fsync fault: `fsync` reports success without actually
/// advancing what's durable, so a simulated crash rolls back past it --
/// exposing recovery code that incorrectly trusted the fsync.
#[test]
fn fsync_lies_then_crash_and_reopen_rolls_back_past_it() {
    let rt = rt();
    let sim = SimDriver::new();
    let driver: Arc<dyn OpDriver> = sim.clone();

    rt.block_on(async {
        let file = UringFile::create_on(driver.clone(), "/virtual/wal.dat")
            .await
            .unwrap();

        // Genuinely durable: a real fsync before the fault is enabled.
        let result = file.write_at(b"durable-record".to_vec(), 0).await;
        result.0.unwrap();
        file.fsync().await.unwrap();

        // Now fsync starts lying.
        sim.set_fsync_lies(true);
        let result = file.write_at(b"looks-durable-but-isnt".to_vec(), 14).await;
        result.0.unwrap();
        file.fsync().await.unwrap(); // reports Ok -- the lie
        file.close().await.unwrap();

        // Simulate a crash: the second write, "confirmed" only by a
        // lying fsync, doesn't survive.
        let post_crash_driver = sim.crash_and_reopen();
        let recovered = UringFile::open_on(post_crash_driver, "/virtual/wal.dat")
            .await
            .unwrap();
        let buf = vec![0u8; 64];
        let result = recovered.read_at(buf, 0).await;
        let n = result.0.unwrap();
        assert_eq!(&result.1[..n], b"durable-record");
    });
}

/// A real, non-lying fsync survives `crash_and_reopen` -- the control
/// case confirming the previous test's rollback is about the *lie*, not
/// about `crash_and_reopen` discarding everything indiscriminately.
#[test]
fn a_real_fsync_survives_crash_and_reopen() {
    let rt = rt();
    let sim = SimDriver::new();
    let driver: Arc<dyn OpDriver> = sim.clone();

    rt.block_on(async {
        let file = UringFile::create_on(driver, "/virtual/committed.dat")
            .await
            .unwrap();
        let result = file.write_at(b"this really is durable".to_vec(), 0).await;
        result.0.unwrap();
        file.fsync().await.unwrap();
        file.close().await.unwrap();

        let post_crash_driver = sim.crash_and_reopen();
        let recovered = UringFile::open_on(post_crash_driver, "/virtual/committed.dat")
            .await
            .unwrap();
        let buf = vec![0u8; 64];
        let result = recovered.read_at(buf, 0).await;
        let n = result.0.unwrap();
        assert_eq!(&result.1[..n], b"this really is durable");
    });
}

/// The disk-full fault: `write_at`/`fallocate` fail with `ENOSPC` once a
/// configured capacity is reached -- exactly the retention-pressure
/// scenario a segment-log engine's own space-management logic needs to
/// handle without corrupting anything.
#[test]
fn disk_full_fault_rejects_writes_and_fallocate_past_capacity() {
    let rt = rt();
    let sim = SimDriver::new();
    let driver: Arc<dyn OpDriver> = sim.clone();
    sim.set_disk_full_at(100);

    rt.block_on(async {
        let file = UringFile::create_on(driver, "/virtual/segment.log")
            .await
            .unwrap();

        // Fits within capacity.
        let result = file.write_at(vec![1u8; 80], 0).await;
        assert_eq!(result.0.unwrap(), 80);

        // Would grow the file past capacity -- rejected.
        let result = file.write_at(vec![2u8; 50], 80).await;
        let err = result.0.unwrap_err();
        assert_eq!(err.kind(), std::io::ErrorKind::StorageFull);
        // The buffer is still handed back even on failure.
        assert_eq!(result.1.len(), 50);

        // `fallocate` past capacity is rejected the same way.
        let err = file.fallocate(0, 1000).await.unwrap_err();
        assert_eq!(err.kind(), std::io::ErrorKind::StorageFull);

        // Shrinking never trips the disk-full check.
        file.set_len(10).await.unwrap();
    });
}

/// The seam itself: the same driver, with no fault injected, behaves
/// indistinguishably from a plain in-memory filesystem across the
/// segment-roll shape (`rename`/`remove_file`) -- confirming
/// `rename_on`/`remove_file_on` are wired through `SimDriver` too, not
/// just `UringFile`'s own methods.
#[test]
fn rename_on_and_remove_file_on_work_against_sim_driver() {
    let rt = rt();
    let driver: Arc<dyn OpDriver> = SimDriver::new();

    rt.block_on(async {
        let tmp = "/virtual/roll.log.tmp";
        let final_path = "/virtual/roll.log";
        UringFile::create_on(driver.clone(), tmp)
            .await
            .unwrap()
            .close()
            .await
            .unwrap();

        rusty_tokio::io::uring_rename_on(driver.clone(), tmp, final_path)
            .await
            .unwrap();

        // The old path is gone; the new one opens fine.
        let err = UringFile::open_on(driver.clone(), tmp).await.unwrap_err();
        assert_eq!(err.kind(), std::io::ErrorKind::NotFound);
        UringFile::open_on(driver.clone(), final_path)
            .await
            .unwrap()
            .close()
            .await
            .unwrap();

        rusty_tokio::io::uring_remove_file_on(driver.clone(), final_path)
            .await
            .unwrap();
        let err = UringFile::open_on(driver, final_path).await.unwrap_err();
        assert_eq!(err.kind(), std::io::ErrorKind::NotFound);
    });
}
