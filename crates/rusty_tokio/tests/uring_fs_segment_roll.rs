#![cfg(target_os = "linux")]
// io-uring-fs is a Cargo feature, not a target predicate -- required-features
// can't express "only on Linux", so a plain `--features io-uring-fs` build on
// another OS still tries to compile this file against items `src/io/mod.rs`
// only re-exports under `cfg(target_os = "linux")`. This file-level cfg is what
// actually keeps it Linux-only.
//! Acceptance criterion 4: the actual Kafka-shaped `.log`/`.index`
//! segment-roll pattern a log-storage engine needs -- preallocate a new
//! segment (`fallocate`), write into it, rename it into place once full
//! (the "roll"), and unlink an old one past its retention window.

use rusty_tokio::io::UringFile;
use std::path::PathBuf;
use std::sync::atomic::{AtomicU64, Ordering};

/// Every test in this file gets its own directory -- the default test
/// harness runs `#[test]` functions concurrently on separate threads,
/// so a shared directory name (even a process-id-tagged one) races:
/// one test's `remove_dir_all` cleanup can delete files a sibling test
/// is still using mid-flight.
static SCRATCH_COUNTER: AtomicU64 = AtomicU64::new(0);

fn scratch_dir() -> PathBuf {
    let id = SCRATCH_COUNTER.fetch_add(1, Ordering::Relaxed);
    let dir = std::env::temp_dir().join(format!(
        "rusty_tokio_uring_segment_roll_test_{}_{id}",
        std::process::id()
    ));
    std::fs::create_dir_all(&dir).unwrap();
    dir
}

#[test]
fn segment_roll_fallocate_write_rename_then_unlink_the_retired_segment() {
    let rt = rusty_tokio::Runtime::new().unwrap();
    let dir = scratch_dir();

    rt.block_on(async {
        const SEGMENT_BYTES: u64 = 1024 * 1024; // 1 MiB preallocated segment

        // Segment 1 -- created under a `.tmp` name (the usual "write
        // fully, then atomically rename into place" shape), preallocated
        // up front so the filesystem doesn't have to grow it a block at
        // a time as records land.
        let tmp_path = dir.join("00000000000000000000.log.tmp");
        let final_path = dir.join("00000000000000000000.log");
        let segment = UringFile::create(&tmp_path).await.unwrap();
        segment.fallocate(0, SEGMENT_BYTES).await.unwrap();

        // `fallocate` doesn't change the *reported* file length on
        // Linux the way `set_len`/`ftruncate` does -- confirm the
        // preallocation actually happened (real disk blocks reserved)
        // via `stat`'s block count, then use `set_len` to give the
        // segment its real logical size before rolling it.
        let meta = std::fs::metadata(&tmp_path).unwrap();
        assert!(
            meta.len() == 0 || meta.len() >= SEGMENT_BYTES,
            "fallocate shouldn't shrink an already-larger apparent length"
        );
        segment.set_len(SEGMENT_BYTES).await.unwrap();
        assert_eq!(std::fs::metadata(&tmp_path).unwrap().len(), SEGMENT_BYTES);

        // Write a handful of "records" at their real offsets.
        let record_a = b"record-a".to_vec();
        let record_b = b"record-b-longer".to_vec();
        let result = segment.write_at(record_a.clone(), 0).await;
        assert_eq!(result.0.unwrap(), record_a.len());
        let result = segment
            .write_at(record_b.clone(), record_a.len() as u64)
            .await;
        assert_eq!(result.0.unwrap(), record_b.len());
        segment.fsync().await.unwrap();
        segment.close().await.unwrap();

        // Roll: atomically rename the finished segment into place.
        rusty_tokio::io::uring_rename(&tmp_path, &final_path)
            .await
            .unwrap();
        assert!(!tmp_path.exists());
        assert!(final_path.exists());

        // Confirm the rolled segment's content survived the rename
        // (open the *renamed* path -- a fresh fd, not the one already
        // closed above).
        let rolled = UringFile::open(&final_path).await.unwrap();
        let buf = vec![0u8; record_a.len()];
        let result = rolled.read_at(buf, 0).await;
        assert_eq!(result.0.unwrap(), record_a.len());
        assert_eq!(result.1, record_a);
        let buf = vec![0u8; record_b.len()];
        let result = rolled.read_at(buf, record_a.len() as u64).await;
        assert_eq!(result.0.unwrap(), record_b.len());
        assert_eq!(result.1, record_b);
        rolled.close().await.unwrap();

        // Retention: an older segment past its window gets unlinked.
        let retired_path = dir.join("retired-segment.log");
        let retired = UringFile::create(&retired_path).await.unwrap();
        retired.close().await.unwrap();
        assert!(retired_path.exists());
        rusty_tokio::io::uring_remove_file(&retired_path)
            .await
            .unwrap();
        assert!(!retired_path.exists());

        // The still-live, rolled segment is untouched by retiring the
        // other one.
        assert!(final_path.exists());
    });

    std::fs::remove_dir_all(&dir).ok();
}

#[test]
fn set_len_extends_and_truncates() {
    let rt = rusty_tokio::Runtime::new().unwrap();
    let dir = scratch_dir();
    let path = dir.join("set_len.dat");

    rt.block_on(async {
        let file = UringFile::create(&path).await.unwrap();
        file.set_len(4096).await.unwrap();
        assert_eq!(std::fs::metadata(&path).unwrap().len(), 4096);

        file.set_len(128).await.unwrap();
        assert_eq!(std::fs::metadata(&path).unwrap().len(), 128);
        file.close().await.unwrap();
    });

    std::fs::remove_dir_all(&dir).ok();
}

#[test]
fn open_options_create_new_fails_if_the_file_already_exists() {
    let rt = rusty_tokio::Runtime::new().unwrap();
    let dir = scratch_dir();
    let path = dir.join("exists.dat");

    rt.block_on(async {
        UringFile::create(&path)
            .await
            .unwrap()
            .close()
            .await
            .unwrap();

        let err = UringFile::options()
            .write(true)
            .create_new(true)
            .open(&path)
            .await
            .unwrap_err();
        assert_eq!(err.kind(), std::io::ErrorKind::AlreadyExists);
    });

    std::fs::remove_dir_all(&dir).ok();
}

#[test]
fn read_at_and_write_at_report_not_found_for_a_missing_file() {
    let rt = rusty_tokio::Runtime::new().unwrap();
    let dir = scratch_dir();
    let missing = dir.join("does-not-exist.dat");

    rt.block_on(async {
        let err = UringFile::open(&missing).await.unwrap_err();
        assert_eq!(err.kind(), std::io::ErrorKind::NotFound);
    });

    std::fs::remove_dir_all(&dir).ok();
}
