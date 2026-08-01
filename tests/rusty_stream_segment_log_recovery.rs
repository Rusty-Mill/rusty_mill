//! A `rusty_stream`-shaped demonstration of the `OpDriver`/`SimDriver`
//! seam (handoff item 4): [`SegmentLog`] below is real, working
//! recovery code -- length-prefixed, CRC-checked records, replayed from
//! offset 0 and truncated at the first sign of trouble on open, exactly
//! the shape a Kafka-style `.log` segment's own crash recovery uses.
//! It's built entirely on [`UringFile`]/[`OpDriver`], with no idea
//! whether it's talking to a real disk or [`SimDriver`] -- that's the
//! whole point. Every test below drives its *real* `SegmentLog::recover`
//! against a fault [`SimDriver`] injected, and asserts on what the real
//! code actually does, not on the fault injection machinery itself.
//!
//! Record format on disk (deliberately simple -- this is a
//! demonstration, not `rusty_stream`'s actual wire format):
//! `[len: u32 LE][crc32(payload): u32 LE][payload; len bytes]`.

use rusty_tokio::io::{OpDriver, SimDriver, UringFile};
use std::sync::Arc;

// ---------------------------------------------------------------------
// SegmentLog: the "rusty_stream"-shaped piece under test
// ---------------------------------------------------------------------

/// A single append-only segment file, backed by whatever [`OpDriver`]
/// it's given -- real io_uring in production, [`SimDriver`] in these
/// tests. `tail` is always exactly "how many bytes of this file are
/// known-good, replayable records" -- [`SegmentLog::recover`] is the
/// only thing that ever determines it from scratch; [`SegmentLog::append`]
/// only ever advances it after a successful write.
struct SegmentLog {
    file: UringFile,
    tail: u64,
}

impl SegmentLog {
    /// Opens (creating if necessary) the segment at `path` and replays
    /// it from offset 0: reads each record's 8-byte header, then its
    /// payload, verifying the CRC -- the instant any of that comes up
    /// short (an incomplete header, an incomplete payload, or a CRC
    /// mismatch), stops and truncates the file to exactly the offset the
    /// last *good* record ended at, discarding everything after. This is
    /// the real recovery algorithm: a torn write, a crash mid-append, or
    /// a write that "landed" without ever being fsynced are all
    /// indistinguishable from each other by the time this runs, and all
    /// three are handled the exact same way -- by trusting nothing past
    /// the last verifiably-intact record.
    async fn recover(driver: Arc<dyn OpDriver>, path: &str) -> std::io::Result<SegmentLog> {
        let file = UringFile::options()
            .read(true)
            .write(true)
            .create(true)
            .open_on(driver, path)
            .await?;

        let mut offset = 0u64;
        loop {
            let header = vec![0u8; 8];
            let result = file.read_at(header, offset).await;
            let n = result.0?;
            let header = result.1;
            if n < 8 {
                break; // no complete header here -- stop, this is the tail
            }
            let len = u32::from_le_bytes(header[0..4].try_into().unwrap()) as usize;
            let expected_crc = u32::from_le_bytes(header[4..8].try_into().unwrap());

            let payload = vec![0u8; len];
            let result = file.read_at(payload, offset + 8).await;
            let n = result.0?;
            let payload = result.1;
            if n < len {
                break; // torn write: header landed, payload didn't fully
            }
            if crc32(&payload) != expected_crc {
                break; // corrupt payload -- same handling either way
            }

            offset += 8 + len as u64;
        }

        // Discard anything past the last verified-good record -- a
        // subsequent `append` starts writing exactly where recovery
        // decided the log was actually trustworthy up to.
        file.set_len(offset).await?;
        Ok(SegmentLog { file, tail: offset })
    }

    /// Appends `payload` as one length-prefixed, CRC-checked record.
    /// `self.tail` only advances on success -- a failed write (e.g.
    /// `ENOSPC`) leaves the log's own bookkeeping exactly where it was,
    /// so a subsequent `recover` sees precisely what's really on disk,
    /// nothing more.
    async fn append(&mut self, payload: &[u8]) -> std::io::Result<()> {
        let mut record = Vec::with_capacity(8 + payload.len());
        record.extend_from_slice(&(payload.len() as u32).to_le_bytes());
        record.extend_from_slice(&crc32(payload).to_le_bytes());
        record.extend_from_slice(payload);

        let start = self.tail;
        let result = self.file.write_at(record, start).await;
        let n = result.0?;
        self.tail += n as u64;
        Ok(())
    }

    async fn fsync(&self) -> std::io::Result<()> {
        self.file.fsync().await
    }

    /// Re-reads every record up to `self.tail` -- test-verification
    /// convenience, not part of the recovery algorithm itself (recovery
    /// already trusts everything up to `tail` by construction).
    async fn read_all(&self) -> std::io::Result<Vec<Vec<u8>>> {
        let mut records = Vec::new();
        let mut offset = 0u64;
        while offset < self.tail {
            let header = vec![0u8; 8];
            let result = self.file.read_at(header, offset).await;
            let header = result.1;
            let len = u32::from_le_bytes(header[0..4].try_into().unwrap()) as usize;
            let payload = vec![0u8; len];
            let result = self.file.read_at(payload, offset + 8).await;
            records.push(result.1);
            offset += 8 + len as u64;
        }
        Ok(records)
    }
}

/// A plain, hand-rolled IEEE CRC32 -- deliberately not a crate
/// dependency; this is demonstration code for `SegmentLog`'s own record
/// integrity check, not part of `rusty_tokio`'s public surface.
fn crc32(data: &[u8]) -> u32 {
    let mut crc: u32 = 0xFFFF_FFFF;
    for &byte in data {
        crc ^= byte as u32;
        for _ in 0..8 {
            let mask = (crc & 1).wrapping_neg();
            crc = (crc >> 1) ^ (0xEDB8_8320 & mask);
        }
    }
    !crc
}

// ---------------------------------------------------------------------
// End-to-end recovery tests, driven entirely through SimDriver faults
// ---------------------------------------------------------------------

fn rt() -> rusty_tokio::Runtime {
    rusty_tokio::Runtime::new().unwrap()
}

/// A torn write (the kernel/disk claims success, but only part of the
/// data actually landed) corrupts the tail record -- real recovery code
/// catches it via the CRC and rolls back to the last good record, then
/// the log is fully usable again (append continues where recovery left
/// off, overwriting the torn bytes).
#[test]
fn torn_write_is_caught_by_recovery_and_the_log_stays_usable() {
    let rt = rt();
    let sim = SimDriver::new();

    rt.block_on(async {
        let driver: Arc<dyn OpDriver> = sim.clone();
        let mut log = SegmentLog::recover(driver.clone(), "/virtual/segment-0.log")
            .await
            .unwrap();

        log.append(b"record-a").await.unwrap();
        log.append(b"record-b").await.unwrap();
        log.fsync().await.unwrap();

        // Simulate a crash mid-append of a third record: the write
        // reports success (the caller has no way to know anything went
        // wrong), but only half of it actually lands, and that's what
        // ends up "durable" by the time of the crash.
        sim.inject_torn_write(0.5);
        log.append(b"record-c-this-one-gets-torn").await.unwrap();
        log.fsync().await.unwrap();

        let post_crash_driver = sim.crash_and_reopen();

        // The real recovery path -- exactly what a `rusty_stream`
        // segment would run on startup after an unclean shutdown.
        let mut recovered = SegmentLog::recover(post_crash_driver, "/virtual/segment-0.log")
            .await
            .unwrap();

        let records = recovered.read_all().await.unwrap();
        assert_eq!(records, vec![b"record-a".to_vec(), b"record-b".to_vec()]);

        // The log is still fully writable -- appending after recovery
        // lands right after the last good record, not after the torn
        // one.
        recovered.append(b"record-d").await.unwrap();
        let records = recovered.read_all().await.unwrap();
        assert_eq!(
            records,
            vec![
                b"record-a".to_vec(),
                b"record-b".to_vec(),
                b"record-d".to_vec(),
            ]
        );
    });
}

/// An `fsync` that lies about persistence loses exactly the record it
/// lied about -- and nothing before it -- once a crash is simulated.
/// Confirms `SegmentLog` never trusts anything the caller didn't
/// genuinely confirm durable.
#[test]
fn a_lying_fsync_loses_only_the_record_it_lied_about() {
    let rt = rt();
    let sim = SimDriver::new();

    rt.block_on(async {
        let driver: Arc<dyn OpDriver> = sim.clone();
        let mut log = SegmentLog::recover(driver.clone(), "/virtual/segment-1.log")
            .await
            .unwrap();

        log.append(b"committed-before-the-lie").await.unwrap();
        log.fsync().await.unwrap(); // genuinely durable

        sim.set_fsync_lies(true);
        log.append(b"only-looks-committed").await.unwrap();
        log.fsync().await.unwrap(); // reports Ok -- the lie

        let post_crash_driver = sim.crash_and_reopen();
        let mut recovered = SegmentLog::recover(post_crash_driver, "/virtual/segment-1.log")
            .await
            .unwrap();

        let records = recovered.read_all().await.unwrap();
        assert_eq!(records, vec![b"committed-before-the-lie".to_vec()]);

        // Still fully usable -- the recovered tail is exactly after the
        // one record that really was durable.
        recovered.append(b"first-record-after-recovery").await.unwrap();
        assert_eq!(recovered.read_all().await.unwrap().len(), 2);
    });
}

/// Running out of space fails the offending `append` cleanly -- the
/// log's own `tail` bookkeeping never advances past what's genuinely on
/// disk, so a subsequent recovery (no crash even needed here) sees
/// exactly the same, uncorrupted state `read_all` already showed before
/// the failed append.
#[test]
fn disk_full_fails_the_append_without_corrupting_the_log() {
    let rt = rt();
    let sim = SimDriver::new();
    // Small enough that the second record won't fit alongside the
    // first.
    sim.set_disk_full_at(24);

    rt.block_on(async {
        let driver: Arc<dyn OpDriver> = sim.clone();
        let mut log = SegmentLog::recover(driver.clone(), "/virtual/segment-2.log")
            .await
            .unwrap();

        log.append(b"fits").await.unwrap(); // 8 (header) + 4 = 12 bytes
        log.fsync().await.unwrap();

        let err = log.append(b"this-one-does-not-fit").await.unwrap_err();
        assert_eq!(err.kind(), std::io::ErrorKind::StorageFull);

        // The failed append didn't touch `tail` or corrupt anything on
        // disk -- both the live handle and a fresh recovery agree.
        assert_eq!(log.read_all().await.unwrap(), vec![b"fits".to_vec()]);

        let recovered = SegmentLog::recover(driver, "/virtual/segment-2.log")
            .await
            .unwrap();
        assert_eq!(recovered.read_all().await.unwrap(), vec![b"fits".to_vec()]);
    });
}

/// Three independent crash/recover cycles compose correctly -- each
/// generation's own `fsync` genuinely lands before its crash, so
/// nothing is ever lost across the whole sequence, same as a real
/// system restarted more than once with clean shutdowns in between.
#[test]
fn repeated_crash_recovery_cycles_each_keep_every_durable_record() {
    let rt = rt();
    let sim = SimDriver::new();

    rt.block_on(async {
        let mut log = SegmentLog::recover(sim.clone(), "/virtual/segment-3.log")
            .await
            .unwrap();
        log.append(b"gen-0").await.unwrap();
        log.fsync().await.unwrap();

        // Crash 1: reopen against a fresh post-crash driver, append the
        // next generation, fsync it durable.
        let after_crash_1 = sim.crash_and_reopen();
        let mut log = SegmentLog::recover(after_crash_1.clone(), "/virtual/segment-3.log")
            .await
            .unwrap();
        log.append(b"gen-1").await.unwrap();
        log.fsync().await.unwrap();

        // Crash 2: same shape again.
        let after_crash_2 = after_crash_1.crash_and_reopen();
        let mut log = SegmentLog::recover(after_crash_2.clone(), "/virtual/segment-3.log")
            .await
            .unwrap();
        log.append(b"gen-2").await.unwrap();
        log.fsync().await.unwrap();

        // Crash 3, then a final recovery that only reads.
        let after_crash_3 = after_crash_2.crash_and_reopen();
        let log = SegmentLog::recover(after_crash_3, "/virtual/segment-3.log")
            .await
            .unwrap();

        assert_eq!(
            log.read_all().await.unwrap(),
            vec![b"gen-0".to_vec(), b"gen-1".to_vec(), b"gen-2".to_vec()]
        );
    });
}
