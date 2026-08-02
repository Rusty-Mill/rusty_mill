//! A single append-only segment file: `[header][record][record]...`.
//!
//! Storage I/O goes directly through `rusty_tokio`'s `OpDriver`/`UringFile`
//! (ADR-0002 D3/D4) — the real, io_uring-backed driver in production, an
//! in-memory `SimDriver` with seeded fault injection in tests (see this
//! module's own tests below, and ADR-0002 D4's three minimal DST scenarios).
//! No separate hand-rolled `Storage`/`Clock` trait: that would just
//! duplicate a seam `rusty_tokio` already provides.
//!
//! Recovery (`Segment::open_on`) is where the correctness-first thesis from
//! `docs/phase1-scope.md` §4 actually gets exercised: a segment that was
//! last written to mid-append (a real crash, or `SimDriver::inject_torn_write`
//! in a test) must come back with its tail truncated to the last valid
//! record boundary, never serving a partial or corrupt one.

use std::sync::Arc;

use rusty_tokio::io::{OpDriver, UringFile};

use crate::offset::{CommittedOffset, DurableOffset, Epoch, Offset};
use crate::record::{self, DecodeError};

const MAGIC: &[u8; 4] = b"RSSG"; // "RustyStream SeGment"
const HEADER_LEN: u64 = 4 + 8 + 8; // magic + epoch + base_offset

/// One append-only segment file, plus the in-memory index of where each
/// record starts within it. A real on-disk *sparse* index (per
/// `docs/phase1-scope.md` §2's "append-only segment log + sparse offset
/// index") is intentionally not built yet — this dense in-memory index is
/// enough to make the segment itself correct and testable; the on-disk
/// sparse index is a Phase 1 follow-up, not part of this scaffold.
pub struct Segment {
    file: UringFile,
    epoch: Epoch,
    base_offset: Offset,
    /// Byte offset each record starts at, indexed by
    /// `record_offset - base_offset`. `index[0]` is always [`HEADER_LEN`].
    index: Vec<u64>,
    /// Byte length of the file as of the last successful append —
    /// `write_at`'s position for the next one.
    write_pos: u64,
    /// Byte length as of the last [`Segment::sync`] — what recovery after a
    /// real crash could actually see, per ADR-0002 D2's durable/committed
    /// split.
    durable_len: u64,
}

impl Segment {
    /// Creates a brand-new, empty segment file starting at `base_offset`,
    /// tagged with `epoch` (see ADR-0002 D2 — `Epoch::INITIAL` until Phase 2
    /// consensus exists).
    pub async fn create_on(
        driver: Arc<dyn OpDriver>,
        path: impl AsRef<std::path::Path>,
        base_offset: Offset,
        epoch: Epoch,
    ) -> std::io::Result<Segment> {
        let file = UringFile::create_on(driver, path).await?;
        let mut header = Vec::with_capacity(HEADER_LEN as usize);
        header.extend_from_slice(MAGIC);
        header.extend_from_slice(&epoch.0.to_le_bytes());
        header.extend_from_slice(&base_offset.0.to_le_bytes());
        let (result, _buf) = file.write_at(header, 0).await;
        result?;
        Ok(Segment {
            file,
            epoch,
            base_offset,
            index: vec![HEADER_LEN],
            write_pos: HEADER_LEN,
            durable_len: 0,
        })
    }

    /// Opens an existing segment file and recovers it: reads the header,
    /// then scans records from the end of the header to the end of the
    /// file. A record that fails to decode — [`DecodeError::HeaderTruncated`]
    /// or [`DecodeError::PayloadTruncated`] (a torn write cut it off) or
    /// [`DecodeError::ChecksumMismatch`] (corruption) — ends the scan there;
    /// the file is truncated (`set_len`) to the last valid record boundary,
    /// exactly ADR-0002 D4's minimal DST test 2.
    pub async fn open_on(
        driver: Arc<dyn OpDriver>,
        path: impl AsRef<std::path::Path>,
    ) -> std::io::Result<Segment> {
        let file = UringFile::open_on(driver, path).await?;

        let header_buf = vec![0u8; HEADER_LEN as usize];
        let (result, header_buf) = file.read_at(header_buf, 0).await;
        let n = result?;
        if n < HEADER_LEN as usize || &header_buf[0..4] != MAGIC {
            return Err(std::io::Error::new(
                std::io::ErrorKind::InvalidData,
                "not a rusty_stream segment file (bad magic or truncated header)",
            ));
        }
        let epoch = Epoch(u64::from_le_bytes(header_buf[4..12].try_into().unwrap()));
        let base_offset = Offset(u64::from_le_bytes(header_buf[12..20].try_into().unwrap()));

        // Read the rest of the file and replay records to rebuild the
        // index, stopping at (and truncating away) the first bad one.
        let mut pos = HEADER_LEN;
        let mut index = vec![HEADER_LEN];
        loop {
            // A generously-sized read per record attempt is simpler than a
            // second syscall to stat the file first; real record sizes in a
            // segment log are small relative to this.
            let chunk = vec![0u8; 64 * 1024];
            let (result, chunk) = file.read_at(chunk, pos).await;
            let read = result?;
            if read == 0 {
                break; // clean end of file, nothing more to recover
            }
            match record::decode(&chunk[..read]) {
                Ok((_payload, len)) => {
                    pos += len as u64;
                    index.push(pos);
                }
                Err(DecodeError::HeaderTruncated) | Err(DecodeError::PayloadTruncated { .. }) => {
                    // A torn write at the tail -- truncate it away and stop.
                    file.set_len(pos).await?;
                    break;
                }
                Err(DecodeError::ChecksumMismatch) => {
                    // Corruption, not necessarily at the true tail -- still
                    // the correct recovery action is the same: don't serve
                    // anything from this point on. See this module's docs
                    // for why a scaffold doesn't try to distinguish the two
                    // cases further.
                    file.set_len(pos).await?;
                    break;
                }
            }
        }
        // The last entry in `index` is one-past-the-last-valid-record, i.e.
        // exactly the current write position and the current durable
        // length (recovery only ever sees what was actually on disk).
        let write_pos = *index.last().unwrap();
        Ok(Segment {
            file,
            epoch,
            base_offset,
            index,
            write_pos,
            durable_len: write_pos,
        })
    }

    /// This segment's epoch (ADR-0002 D2).
    pub fn epoch(&self) -> Epoch {
        self.epoch
    }

    /// The offset of this segment's first record.
    pub fn base_offset(&self) -> Offset {
        self.base_offset
    }

    /// How many records are in this segment.
    pub fn len(&self) -> u64 {
        self.index.len() as u64 - 1
    }

    /// Whether this segment has no records yet.
    pub fn is_empty(&self) -> bool {
        self.len() == 0
    }

    /// Appends `payload` as a new record. Does **not** fsync — call
    /// [`sync`](Self::sync) explicitly, per whatever fsync policy the
    /// caller wants (`docs/phase1-scope.md` §2: "WAL-durable, fsync policy
    /// configurable" — the policy itself is a Phase 1 follow-up, not part
    /// of this scaffold). Returns the offset the new record landed at.
    pub async fn append(&mut self, payload: &[u8]) -> std::io::Result<Offset> {
        let encoded = record::encode(payload);
        let pos = self.write_pos;
        let (result, _buf) = self.file.write_at(encoded, pos).await;
        let written = result?;
        self.write_pos = pos + written as u64;
        self.index.push(self.write_pos);
        Ok(Offset(self.base_offset.0 + self.len() - 1))
    }

    /// Reads the record at `offset` back out.
    pub async fn read(&self, offset: Offset) -> std::io::Result<Vec<u8>> {
        let idx = offset.0.checked_sub(self.base_offset.0).ok_or_else(|| {
            std::io::Error::new(
                std::io::ErrorKind::InvalidInput,
                "offset predates this segment's base offset",
            )
        })? as usize;
        if idx + 1 >= self.index.len() {
            return Err(std::io::Error::new(
                std::io::ErrorKind::NotFound,
                "offset is beyond this segment's last record",
            ));
        }
        let start = self.index[idx];
        let end = self.index[idx + 1];
        let buf = vec![0u8; (end - start) as usize];
        let (result, buf) = self.file.read_at(buf, start).await;
        let n = result?;
        let (payload, _len) = record::decode(&buf[..n])
            .map_err(|e| std::io::Error::new(std::io::ErrorKind::InvalidData, format!("{e:?}")))?;
        Ok(payload.to_vec())
    }

    /// Flushes every append since the last `sync` to disk and returns the
    /// new durable offset (ADR-0002 D2's `DurableOffset`).
    pub async fn sync(&mut self) -> std::io::Result<DurableOffset> {
        self.file.fsync().await?;
        self.durable_len = self.write_pos;
        Ok(DurableOffset(Offset(self.base_offset.0 + self.len().saturating_sub(1))))
    }

    /// The high-watermark offset visible to consumers, or `None` if no
    /// record in this segment has been durably synced yet. Phase 1 has no
    /// replication, so this only ever equals the last synced offset — see
    /// ADR-0002 D2 for why the type stays distinct from
    /// [`DurableOffset`]/[`Segment::sync`] anyway.
    pub fn committed_offset(&self) -> Option<CommittedOffset> {
        // `index[0]` is always `HEADER_LEN`, the position *before* any
        // record -- skip it so this counts completed records (each
        // identified by its *end* position), not header presence.
        let durable_records = self.index[1..]
            .iter()
            .filter(|&&end_pos| end_pos <= self.durable_len)
            .count() as u64;
        durable_records.checked_sub(1).map(|last| {
            CommittedOffset(Offset(self.base_offset.0 + last))
        })
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use rusty_tokio::io::SimDriver;

    #[rusty_tokio::test]
    async fn append_then_read_round_trips() {
        let driver = SimDriver::new();
        let mut seg = Segment::create_on(driver, "/segments/000.log", Offset(0), Epoch::INITIAL)
            .await
            .unwrap();
        let a = seg.append(b"first record").await.unwrap();
        let b = seg.append(b"second record").await.unwrap();
        assert_eq!(a, Offset(0));
        assert_eq!(b, Offset(1));
        assert_eq!(seg.read(a).await.unwrap(), b"first record");
        assert_eq!(seg.read(b).await.unwrap(), b"second record");
    }

    #[rusty_tokio::test]
    async fn reading_past_the_last_record_is_not_found() {
        let driver = SimDriver::new();
        let mut seg = Segment::create_on(driver, "/segments/000.log", Offset(0), Epoch::INITIAL)
            .await
            .unwrap();
        seg.append(b"only record").await.unwrap();
        let err = seg.read(Offset(5)).await.unwrap_err();
        assert_eq!(err.kind(), std::io::ErrorKind::NotFound);
    }

    /// ADR-0002 D4 minimal DST test 1 (crash during segment roll, adapted to
    /// "crash mid-append"): a fault mid-write, followed by a crash, must
    /// never lose a record that was actually synced beforehand, and
    /// recovery must land on a single consistent, readable segment.
    #[rusty_tokio::test]
    async fn repeated_crash_recovery_cycles_keep_every_synced_record() {
        let mut driver = SimDriver::new();
        let path = "/segments/000.log";
        let mut seg = Segment::create_on(driver.clone(), path, Offset(0), Epoch::INITIAL)
            .await
            .unwrap();

        seg.append(b"synced before crash").await.unwrap();
        seg.sync().await.unwrap();
        seg.append(b"also synced before crash").await.unwrap();
        seg.sync().await.unwrap();

        driver = driver.crash_and_reopen();
        seg = Segment::open_on(driver.clone(), path).await.unwrap();

        assert_eq!(seg.len(), 2);
        assert_eq!(seg.read(Offset(0)).await.unwrap(), b"synced before crash");
        assert_eq!(
            seg.read(Offset(1)).await.unwrap(),
            b"also synced before crash"
        );

        // A second cycle: append more, sync, crash again -- every
        // synced record across both cycles must still be there.
        seg.append(b"synced in second cycle").await.unwrap();
        seg.sync().await.unwrap();
        driver = driver.crash_and_reopen();
        let seg = Segment::open_on(driver, path).await.unwrap();
        assert_eq!(seg.len(), 3);
        assert_eq!(
            seg.read(Offset(2)).await.unwrap(),
            b"synced in second cycle"
        );
    }

    /// ADR-0002 D4 minimal DST test 2: a torn write on the last record —
    /// recovery must detect it via length/checksum and truncate to the last
    /// valid boundary, not crash and not serve a partial/corrupt record.
    #[rusty_tokio::test]
    async fn torn_write_is_truncated_away_and_the_segment_stays_usable() {
        let driver = SimDriver::new();
        let path = "/segments/000.log";
        let mut seg = Segment::create_on(driver.clone(), path, Offset(0), Epoch::INITIAL)
            .await
            .unwrap();
        seg.append(b"whole record").await.unwrap();
        seg.sync().await.unwrap();

        driver.inject_torn_write(0.3); // next write only 30% lands
        seg.append(b"this record gets torn").await.unwrap();
        // No sync -- a torn write is exactly what "crash before fsync
        // finished" looks like, so recovery must clean it up.

        let recovered = driver.crash_and_reopen();
        let seg = Segment::open_on(recovered, path).await.unwrap();

        // Only the whole, previously-synced record survives.
        assert_eq!(seg.len(), 1);
        assert_eq!(seg.read(Offset(0)).await.unwrap(), b"whole record");

        // The segment is still writable after recovery -- truncation
        // didn't leave it in a broken state.
        let mut seg = seg;
        let next = seg.append(b"appended after recovery").await.unwrap();
        assert_eq!(next, Offset(1));
    }

    /// ADR-0002 D4 minimal DST test 3: fsync fault — an fsync that reports
    /// success without actually persisting must not let recovery believe
    /// the record it covered survived a crash.
    #[rusty_tokio::test]
    async fn a_lying_fsync_loses_only_the_record_it_lied_about() {
        let driver = SimDriver::new();
        let path = "/segments/000.log";
        let mut seg = Segment::create_on(driver.clone(), path, Offset(0), Epoch::INITIAL)
            .await
            .unwrap();

        seg.append(b"really synced").await.unwrap();
        seg.sync().await.unwrap();

        driver.set_fsync_lies(true);
        seg.append(b"fsync lies about this one").await.unwrap();
        seg.sync().await.unwrap(); // reports Ok, but nothing new is durable

        let recovered = driver.crash_and_reopen();
        let seg = Segment::open_on(recovered, path).await.unwrap();

        assert_eq!(seg.len(), 1);
        assert_eq!(seg.read(Offset(0)).await.unwrap(), b"really synced");
    }

    #[rusty_tokio::test]
    async fn committed_offset_tracks_only_synced_records() {
        let driver = SimDriver::new();
        let mut seg = Segment::create_on(driver, "/segments/000.log", Offset(0), Epoch::INITIAL)
            .await
            .unwrap();

        assert_eq!(seg.committed_offset(), None);

        seg.append(b"first").await.unwrap();
        assert_eq!(seg.committed_offset(), None); // appended, not synced yet

        seg.sync().await.unwrap();
        assert_eq!(seg.committed_offset(), Some(CommittedOffset(Offset(0))));

        seg.append(b"second").await.unwrap();
        assert_eq!(seg.committed_offset(), Some(CommittedOffset(Offset(0)))); // unchanged

        seg.sync().await.unwrap();
        assert_eq!(seg.committed_offset(), Some(CommittedOffset(Offset(1))));
    }
}
