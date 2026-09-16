//! Segment rolling and retention — size/time-based, no compaction yet
//! (`docs/phase1-scope.md` §2).
//!
//! `Log` owns a sequence of [`Segment`]s in one directory: the active
//! (writable) one plus zero or more closed ones. `append` rolls to a new
//! segment once the active one would cross [`RetentionPolicy::max_segment_bytes`];
//! [`Log::enforce_retention`] then deletes closed segments once they cross
//! [`RetentionPolicy::max_total_bytes`] (size-based) or
//! [`RetentionPolicy::max_segment_age_millis`] (time-based, via
//! `crate::clock::Clock` — real wall-clock in production,
//! `crate::clock::SimClock` in tests, the same seam pairing
//! `rusty_tokio::io::SimDriver` uses for disk faults).
//!
//! ## Recovery
//!
//! `rusty_tokio::io::OpDriver` has no directory-listing operation
//! (`SimDriver` has no concept of "list files in a directory" at all — it
//! only knows about paths it's been explicitly told about), so `Log::open`
//! cannot discover which segments exist by scanning the directory the way a
//! naive implementation might. It reads `crate::manifest::Manifest`
//! instead — a durable, replayable record of which segments exist and when
//! each was created, kept alongside the segments themselves. See that
//! module's own docs for the format and the crash-safety ordering between
//! a manifest write and the segment file it describes.
//!
//! Recovering each segment's real creation time from the manifest also
//! means time-based retention is accurate across a restart, not just
//! within one process's uptime, the way it was before the manifest existed.

use std::path::{Path, PathBuf};
use std::sync::Arc;

use rusty_tokio::io::{uring_remove_file_on, OpDriver};

use crate::clock::Clock;
use crate::manifest::Manifest;
use crate::offset::{Epoch, Offset};
use crate::record;
use crate::segment::Segment;

/// When to roll to a new segment, and when to delete old ones. `None` on
/// either retention field means that axis never deletes anything — Phase 1
/// allows an unbounded log if the caller genuinely wants one.
pub struct RetentionPolicy {
    /// Roll to a new segment once appending would push the active segment
    /// past this many bytes. Never splits a single record across segments —
    /// the record that would cross the threshold starts a new segment
    /// instead of being the one that crosses it.
    pub max_segment_bytes: u64,
    /// Size-based retention: once the total size of *closed* segments (the
    /// active segment is never deleted) exceeds this, delete the oldest
    /// closed segments until it doesn't. `None` disables size-based
    /// retention.
    pub max_total_bytes: Option<u64>,
    /// Time-based retention: delete any closed segment older than this many
    /// milliseconds, regardless of size. `None` disables time-based
    /// retention. See this module's docs for the recovered-segment age
    /// caveat.
    pub max_segment_age_millis: Option<u64>,
}

struct ClosedSegment {
    segment: Segment,
    path: PathBuf,
    created_at_millis: u64,
}

/// A durable log: one active segment plus zero or more closed ones, in one
/// directory, with size/time-based rolling and retention. See this module's
/// top-level docs and `crate::manifest`'s for how recovery discovers which
/// segments exist.
pub struct Log {
    dir: PathBuf,
    driver: Arc<dyn OpDriver>,
    clock: Arc<dyn Clock>,
    policy: RetentionPolicy,
    closed: Vec<ClosedSegment>,
    active: Segment,
    active_created_at_millis: u64,
    manifest: Manifest,
}

impl Log {
    fn segment_path(dir: &Path, base_offset: Offset) -> PathBuf {
        dir.join(format!("{:020}.log", base_offset.0))
    }

    fn manifest_path(dir: &Path) -> PathBuf {
        dir.join("manifest.log")
    }

    /// Starts a brand-new, empty log in `dir` with a single active segment
    /// at offset 0, and a fresh manifest recording it.
    pub async fn create(
        driver: Arc<dyn OpDriver>,
        clock: Arc<dyn Clock>,
        dir: impl Into<PathBuf>,
        policy: RetentionPolicy,
    ) -> std::io::Result<Log> {
        let dir = dir.into();
        let base = Offset(0);
        let path = Self::segment_path(&dir, base);
        let active = Segment::create_on(driver.clone(), &path, base, Epoch::INITIAL).await?;
        let active_created_at_millis = clock.now_millis();

        let mut manifest = Manifest::create_on(driver.clone(), Self::manifest_path(&dir)).await?;
        manifest
            .record_opened(base, active_created_at_millis)
            .await?;

        Ok(Log {
            dir,
            driver,
            clock,
            policy,
            closed: Vec::new(),
            active,
            active_created_at_millis,
            manifest,
        })
    }

    /// Recovers a log from `dir` by reading its manifest — no caller-supplied
    /// segment list needed. See `crate::manifest`'s docs for how that
    /// recovery works and what it guarantees across a crash.
    pub async fn open(
        driver: Arc<dyn OpDriver>,
        clock: Arc<dyn Clock>,
        dir: impl Into<PathBuf>,
        policy: RetentionPolicy,
    ) -> std::io::Result<Log> {
        let dir = dir.into();
        let (manifest, live) = Manifest::open_on(driver.clone(), Self::manifest_path(&dir)).await?;
        let (active_entry, closed_entries) = live.split_last().ok_or_else(|| {
            std::io::Error::new(
                std::io::ErrorKind::InvalidInput,
                "Log::open found an empty manifest -- use Log::create for a fresh log",
            )
        })?;

        let mut closed = Vec::with_capacity(closed_entries.len());
        for entry in closed_entries {
            let path = Self::segment_path(&dir, entry.base_offset);
            let segment = Segment::open_on(driver.clone(), &path).await?;
            closed.push(ClosedSegment {
                segment,
                path,
                created_at_millis: entry.created_at_millis,
            });
        }

        let active_path = Self::segment_path(&dir, active_entry.base_offset);
        let active = Segment::open_on(driver.clone(), &active_path).await?;

        Ok(Log {
            dir,
            driver,
            clock,
            policy,
            closed,
            active,
            active_created_at_millis: active_entry.created_at_millis,
            manifest,
        })
    }

    /// Appends `payload`, rolling to a new segment first if it wouldn't fit
    /// within [`RetentionPolicy::max_segment_bytes`], then enforces
    /// retention. Does not sync — same fsync-policy-is-the-caller's-call
    /// stance as [`Segment::append`].
    pub async fn append(&mut self, payload: &[u8]) -> std::io::Result<Offset> {
        let projected = self.active.byte_len() + record::HEADER_LEN as u64 + payload.len() as u64;
        if !self.active.is_empty() && projected > self.policy.max_segment_bytes {
            self.roll().await?;
        }
        let offset = self.active.append(payload).await?;
        self.enforce_retention().await?;
        Ok(offset)
    }

    /// Reads the record at `offset` back out, from whichever segment
    /// (active or closed) actually holds it.
    pub async fn read(&self, offset: Offset) -> std::io::Result<Vec<u8>> {
        if offset.0 >= self.active.base_offset().0 {
            return self.active.read(offset).await;
        }
        for closed in self.closed.iter().rev() {
            if offset.0 >= closed.segment.base_offset().0 {
                return closed.segment.read(offset).await;
            }
        }
        Err(std::io::Error::new(
            std::io::ErrorKind::NotFound,
            "offset predates every segment in this log",
        ))
    }

    /// The directory this log's segments live in.
    pub fn dir(&self) -> &Path {
        &self.dir
    }

    /// How many closed (rolled-off, not-yet-deleted) segments this log has.
    pub fn closed_segment_count(&self) -> usize {
        self.closed.len()
    }

    /// Total bytes across every closed segment — what size-based retention
    /// actually measures against [`RetentionPolicy::max_total_bytes`].
    pub fn closed_bytes(&self) -> u64 {
        self.closed.iter().map(|c| c.segment.byte_len()).sum()
    }

    async fn roll(&mut self) -> std::io::Result<()> {
        let next_base = Offset(self.active.base_offset().0 + self.active.len());
        let path = Self::segment_path(&self.dir, next_base);
        let epoch = self.active.epoch(); // Phase 1 has no consensus yet -- always Epoch::INITIAL
        let new_active = Segment::create_on(self.driver.clone(), &path, next_base, epoch).await?;
        let new_created_at = self.clock.now_millis();
        // The segment file exists before the manifest is told about it --
        // see `crate::manifest`'s docs for why that ordering, not the
        // reverse, is the crash-safe one.
        self.manifest
            .record_opened(next_base, new_created_at)
            .await?;

        let mut retired = std::mem::replace(&mut self.active, new_active);
        // A closed segment is never appended to again -- sync it now so a
        // crash right after a roll can't lose records this process already
        // considered safely rolled off, not just whatever the caller
        // happened to sync explicitly beforehand.
        retired.sync().await?;
        let retired_path = Self::segment_path(&self.dir, retired.base_offset());
        self.closed.push(ClosedSegment {
            segment: retired,
            path: retired_path,
            created_at_millis: self.active_created_at_millis,
        });
        self.active_created_at_millis = new_created_at;
        Ok(())
    }

    /// Deletes closed segments that have aged out
    /// ([`RetentionPolicy::max_segment_age_millis`]) or that size-based
    /// retention ([`RetentionPolicy::max_total_bytes`]) says must go, oldest
    /// first. The active segment is never deleted by either policy. Called
    /// automatically after every [`Log::append`]; also callable directly
    /// (e.g. on a timer, for a log that isn't actively being appended to).
    pub async fn enforce_retention(&mut self) -> std::io::Result<()> {
        if let Some(max_age) = self.policy.max_segment_age_millis {
            let now = self.clock.now_millis();
            while let Some(oldest) = self.closed.first() {
                if now.saturating_sub(oldest.created_at_millis) > max_age {
                    self.delete_oldest_closed().await?;
                } else {
                    break;
                }
            }
        }
        if let Some(max_total) = self.policy.max_total_bytes {
            while self.closed_bytes() > max_total {
                if self.closed.is_empty() {
                    break;
                }
                self.delete_oldest_closed().await?;
            }
        }
        Ok(())
    }

    async fn delete_oldest_closed(&mut self) -> std::io::Result<()> {
        let oldest = self.closed.remove(0);
        // Manifest first, file second -- see `crate::manifest`'s docs for
        // why a crash between the two must leave an orphan file rather
        // than a phantom manifest entry.
        self.manifest
            .record_deleted(oldest.segment.base_offset())
            .await?;
        uring_remove_file_on(self.driver.clone(), &oldest.path).await
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::clock::SimClock;
    use rusty_tokio::io::SimDriver;

    fn no_retention(max_segment_bytes: u64) -> RetentionPolicy {
        RetentionPolicy {
            max_segment_bytes,
            max_total_bytes: None,
            max_segment_age_millis: None,
        }
    }

    #[rusty_tokio::test]
    async fn appends_within_one_segment_dont_roll() {
        let driver = SimDriver::new();
        let clock = Arc::new(SimClock::new());
        let mut log = Log::create(driver, clock, "/log", no_retention(1_000_000))
            .await
            .unwrap();

        log.append(b"a").await.unwrap();
        log.append(b"b").await.unwrap();
        assert_eq!(log.closed_segment_count(), 0);
    }

    #[rusty_tokio::test]
    async fn crossing_max_segment_bytes_rolls_to_a_new_segment() {
        let driver = SimDriver::new();
        let clock = Arc::new(SimClock::new());
        // Small enough that a couple of short records force a roll.
        let mut log = Log::create(driver, clock, "/log", no_retention(40))
            .await
            .unwrap();

        let a = log.append(b"first record").await.unwrap();
        let b = log.append(b"second record").await.unwrap();
        // Header (20) + framing (8) + "first record" (12) = 40, already at
        // the limit -- the second record must roll.
        assert_eq!(a, Offset(0));
        assert_eq!(b, Offset(1));
        assert_eq!(log.closed_segment_count(), 1);

        assert_eq!(log.read(a).await.unwrap(), b"first record");
        assert_eq!(log.read(b).await.unwrap(), b"second record");
    }

    #[rusty_tokio::test]
    async fn size_based_retention_deletes_oldest_closed_segments() {
        let driver = SimDriver::new();
        let clock = Arc::new(SimClock::new());
        let policy = RetentionPolicy {
            max_segment_bytes: 40,
            max_total_bytes: Some(1), // effectively "keep no closed segments"
            max_segment_age_millis: None,
        };
        let mut log = Log::create(driver, clock, "/log", policy).await.unwrap();

        log.append(b"first record").await.unwrap(); // segment 0, still active
        log.append(b"second record").await.unwrap(); // rolls; segment 0 closes, then gets deleted
        assert_eq!(log.closed_segment_count(), 0);
        assert_eq!(log.closed_bytes(), 0);

        // The still-active segment's own record is untouched by retention.
        assert_eq!(log.read(Offset(1)).await.unwrap(), b"second record");
        // The deleted segment's record is genuinely gone.
        assert!(log.read(Offset(0)).await.is_err());
    }

    #[rusty_tokio::test]
    async fn time_based_retention_deletes_only_once_aged_out() {
        let driver = SimDriver::new();
        let clock = Arc::new(SimClock::new());
        let policy = RetentionPolicy {
            max_segment_bytes: 40,
            max_total_bytes: None,
            max_segment_age_millis: Some(1_000),
        };
        let mut log = Log::create(driver, clock.clone(), "/log", policy)
            .await
            .unwrap();

        log.append(b"first record").await.unwrap();
        log.append(b"second record").await.unwrap(); // rolls -- segment 0 closes

        // Not aged out yet.
        assert_eq!(log.closed_segment_count(), 1);

        clock.advance(500);
        log.enforce_retention().await.unwrap();
        assert_eq!(log.closed_segment_count(), 1); // still within the window

        clock.advance(600); // total 1100ms since segment 0 closed
        log.enforce_retention().await.unwrap();
        assert_eq!(log.closed_segment_count(), 0); // now aged out
    }

    #[rusty_tokio::test]
    async fn open_recovers_closed_and_active_segments_from_a_manifest() {
        let driver = SimDriver::new();
        let clock = Arc::new(SimClock::new());
        let mut log = Log::create(driver.clone(), clock.clone(), "/log", no_retention(40))
            .await
            .unwrap();

        let a = log.append(b"first record").await.unwrap();
        let b = log.append(b"second record").await.unwrap(); // rolls
        log.active.sync().await.unwrap();

        drop(log);
        let driver = driver.crash_and_reopen();
        let log = Log::open(driver, clock, "/log", no_retention(40))
            .await
            .unwrap();

        assert_eq!(log.closed_segment_count(), 1);
        assert_eq!(log.read(a).await.unwrap(), b"first record");
        assert_eq!(log.read(b).await.unwrap(), b"second record");
    }

    /// A segment deleted by retention before a restart must not reappear
    /// after one -- the manifest's `Deleted` event is what makes that
    /// durable, not just an in-memory fact this process forgets on exit.
    #[rusty_tokio::test]
    async fn a_deleted_segment_stays_deleted_across_a_restart() {
        let driver = SimDriver::new();
        let clock = Arc::new(SimClock::new());
        let policy = RetentionPolicy {
            max_segment_bytes: 40,
            max_total_bytes: Some(1), // effectively "keep no closed segments"
            max_segment_age_millis: None,
        };
        let mut log = Log::create(driver.clone(), clock.clone(), "/log", policy)
            .await
            .unwrap();

        log.append(b"first record").await.unwrap(); // segment 0, still active
        log.append(b"second record").await.unwrap(); // rolls; segment 0 closes, then gets deleted
        log.active.sync().await.unwrap();
        assert_eq!(log.closed_segment_count(), 0);

        drop(log);
        let driver = driver.crash_and_reopen();
        let policy = RetentionPolicy {
            max_segment_bytes: 40,
            max_total_bytes: Some(1),
            max_segment_age_millis: None,
        };
        let log = Log::open(driver, clock, "/log", policy).await.unwrap();

        assert_eq!(log.closed_segment_count(), 0);
        assert_eq!(log.read(Offset(1)).await.unwrap(), b"second record");
        assert!(log.read(Offset(0)).await.is_err());
    }

    /// The manifest persists each segment's real creation time, so
    /// time-based retention recovered from a restart doesn't restart the
    /// clock at the moment of recovery -- a segment already old enough to
    /// age out before the crash is still old enough to age out right after
    /// `Log::open`, without needing a fresh `enforce_retention` window to
    /// elapse first.
    #[rusty_tokio::test]
    async fn recovered_segment_ages_are_real_not_reset_at_open() {
        let driver = SimDriver::new();
        let clock = Arc::new(SimClock::new());
        let policy = RetentionPolicy {
            max_segment_bytes: 40,
            max_total_bytes: None,
            max_segment_age_millis: Some(1_000),
        };
        let mut log = Log::create(driver.clone(), clock.clone(), "/log", policy)
            .await
            .unwrap();

        log.append(b"first record").await.unwrap();
        log.append(b"second record").await.unwrap(); // rolls -- segment 0 closes
        log.active.sync().await.unwrap();
        assert_eq!(log.closed_segment_count(), 1);

        clock.advance(1_100); // segment 0 is already past max_segment_age_millis

        drop(log);
        let driver = driver.crash_and_reopen();
        let policy = RetentionPolicy {
            max_segment_bytes: 40,
            max_total_bytes: None,
            max_segment_age_millis: Some(1_000),
        };
        let mut log = Log::open(driver, clock, "/log", policy).await.unwrap();

        // No further clock advance -- if the recovered segment's age were
        // (wrongly) reset to zero at open, this would find nothing to
        // delete yet.
        log.enforce_retention().await.unwrap();
        assert_eq!(log.closed_segment_count(), 0);
    }
}
