//! Durable record of which segments exist in a [`crate::retention::Log`]'s
//! directory — the gap `retention.rs` previously documented as "real design
//! work this scaffold doesn't attempt yet": `rusty_tokio::io::OpDriver` has
//! no directory-listing operation, so [`crate::retention::Log::open`]
//! cannot discover which segments exist by scanning the directory. It reads
//! this instead.
//!
//! [`Manifest`] is backed by an append-only [`crate::Segment`] of `Opened`/
//! `Deleted` events, replayed on [`Manifest::open_on`] to reconstruct which
//! segments are currently live and when each was created — the same
//! last-write-wins-by-replay pattern [`crate::ConsumerOffsets`] uses,
//! reusing [`crate::Segment`]'s own torn-write/checksum recovery rather
//! than a second hand-rolled one. Recovering a segment's real creation time
//! this way also fixes the second gap `retention.rs` used to document:
//! time-based retention is now accurate across a restart, not just within
//! one process's uptime.
//!
//! Every event is synced immediately, unlike [`crate::Segment::append`]'s
//! caller-controlled fsync policy: segment lifecycle events (a roll, a
//! deletion) are rare structural changes, not the hot per-record path that
//! policy exists to make configurable, so there's no real cost to making
//! each one durable right away. [`Manifest::record_deleted`] is written
//! *before* the underlying segment file is actually removed — a crash in
//! between leaves an orphaned segment file the manifest no longer
//! references (wasted disk, harmless), never the reverse (a manifest entry
//! pointing at a file that's already gone, which would fail
//! [`crate::Segment::open_on`] on the next recovery). The same ordering
//! applies to a newly created segment: [`crate::retention::Log`] creates
//! the segment file first, then calls [`Manifest::record_opened`] — a crash
//! in between again leaves only a harmless orphan file, never a phantom
//! manifest entry.

use std::path::Path;
use std::sync::Arc;

use rusty_tokio::io::OpDriver;

use crate::offset::{Epoch, Offset};
use crate::segment::Segment;

const TAG_OPENED: u8 = 1;
const TAG_DELETED: u8 = 2;

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
enum Event {
    Opened {
        base_offset: Offset,
        created_at_millis: u64,
    },
    Deleted {
        base_offset: Offset,
    },
}

fn encode_event(event: Event) -> Vec<u8> {
    match event {
        Event::Opened {
            base_offset,
            created_at_millis,
        } => {
            let mut buf = Vec::with_capacity(17);
            buf.push(TAG_OPENED);
            buf.extend_from_slice(&base_offset.0.to_le_bytes());
            buf.extend_from_slice(&created_at_millis.to_le_bytes());
            buf
        }
        Event::Deleted { base_offset } => {
            let mut buf = Vec::with_capacity(9);
            buf.push(TAG_DELETED);
            buf.extend_from_slice(&base_offset.0.to_le_bytes());
            buf
        }
    }
}

fn decode_event(buf: &[u8]) -> std::io::Result<Event> {
    let bad = || std::io::Error::new(std::io::ErrorKind::InvalidData, "malformed manifest event");
    match buf.first() {
        Some(&TAG_OPENED) => {
            if buf.len() != 17 {
                return Err(bad());
            }
            let base_offset = Offset(u64::from_le_bytes(buf[1..9].try_into().unwrap()));
            let created_at_millis = u64::from_le_bytes(buf[9..17].try_into().unwrap());
            Ok(Event::Opened {
                base_offset,
                created_at_millis,
            })
        }
        Some(&TAG_DELETED) => {
            if buf.len() != 9 {
                return Err(bad());
            }
            let base_offset = Offset(u64::from_le_bytes(buf[1..9].try_into().unwrap()));
            Ok(Event::Deleted { base_offset })
        }
        _ => Err(bad()),
    }
}

/// One live segment as the manifest currently knows it: its base offset and
/// when it was created, in the order it was created (ascending base
/// offset, since offsets only ever increase).
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct LiveSegment {
    pub base_offset: Offset,
    pub created_at_millis: u64,
}

/// See this module's top-level docs.
pub struct Manifest {
    segment: Segment,
}

impl Manifest {
    /// Starts a brand-new, empty manifest at `path`. Pairs with a fresh
    /// [`crate::retention::Log::create`] — the caller records the initial
    /// segment via [`Manifest::record_opened`] right after.
    pub async fn create_on(
        driver: Arc<dyn OpDriver>,
        path: impl AsRef<Path>,
    ) -> std::io::Result<Manifest> {
        let segment = Segment::create_on(driver, path, Offset(0), Epoch::INITIAL).await?;
        Ok(Manifest { segment })
    }

    /// Recovers the manifest at `path`, replaying every event to
    /// reconstruct which segments are currently live and their real
    /// creation times. A torn event (crash mid-append) is truncated away
    /// exactly like a torn record in any other segment.
    pub async fn open_on(
        driver: Arc<dyn OpDriver>,
        path: impl AsRef<Path>,
    ) -> std::io::Result<(Manifest, Vec<LiveSegment>)> {
        let segment = Segment::open_on(driver, path).await?;
        let mut live: Vec<LiveSegment> = Vec::new();
        for i in 0..segment.len() {
            let payload = segment.read(Offset(i)).await?;
            match decode_event(&payload)? {
                Event::Opened {
                    base_offset,
                    created_at_millis,
                } => live.push(LiveSegment {
                    base_offset,
                    created_at_millis,
                }),
                Event::Deleted { base_offset } => live.retain(|s| s.base_offset != base_offset),
            }
        }
        Ok((Manifest { segment }, live))
    }

    /// Records that a new segment starting at `base_offset` now exists.
    /// Synced before returning — see this module's top-level docs for why.
    pub async fn record_opened(
        &mut self,
        base_offset: Offset,
        created_at_millis: u64,
    ) -> std::io::Result<()> {
        let record = encode_event(Event::Opened {
            base_offset,
            created_at_millis,
        });
        self.segment.append(&record).await?;
        self.segment.sync().await?;
        Ok(())
    }

    /// Records that the segment starting at `base_offset` has been
    /// deleted. Synced before returning, and before the caller actually
    /// removes the underlying segment file — see this module's top-level
    /// docs for why that ordering matters.
    pub async fn record_deleted(&mut self, base_offset: Offset) -> std::io::Result<()> {
        let record = encode_event(Event::Deleted { base_offset });
        self.segment.append(&record).await?;
        self.segment.sync().await?;
        Ok(())
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use rusty_tokio::io::SimDriver;

    #[rusty_tokio::test]
    async fn a_fresh_manifest_has_no_live_segments_until_told() {
        let driver = SimDriver::new();
        let manifest = Manifest::create_on(driver.clone(), "/log/manifest.log")
            .await
            .unwrap();
        drop(manifest);

        let (_manifest, live) = Manifest::open_on(driver, "/log/manifest.log")
            .await
            .unwrap();
        assert!(live.is_empty());
    }

    #[rusty_tokio::test]
    async fn recovery_replays_opened_events_in_order() {
        let driver = SimDriver::new();
        let mut manifest = Manifest::create_on(driver.clone(), "/log/manifest.log")
            .await
            .unwrap();
        manifest.record_opened(Offset(0), 100).await.unwrap();
        manifest.record_opened(Offset(5), 200).await.unwrap();
        drop(manifest);

        let (_manifest, live) = Manifest::open_on(driver, "/log/manifest.log")
            .await
            .unwrap();
        assert_eq!(
            live,
            vec![
                LiveSegment {
                    base_offset: Offset(0),
                    created_at_millis: 100
                },
                LiveSegment {
                    base_offset: Offset(5),
                    created_at_millis: 200
                },
            ]
        );
    }

    #[rusty_tokio::test]
    async fn a_deleted_segment_drops_out_of_the_live_list() {
        let driver = SimDriver::new();
        let mut manifest = Manifest::create_on(driver.clone(), "/log/manifest.log")
            .await
            .unwrap();
        manifest.record_opened(Offset(0), 100).await.unwrap();
        manifest.record_opened(Offset(5), 200).await.unwrap();
        manifest.record_deleted(Offset(0)).await.unwrap();
        drop(manifest);

        let (_manifest, live) = Manifest::open_on(driver, "/log/manifest.log")
            .await
            .unwrap();
        assert_eq!(
            live,
            vec![LiveSegment {
                base_offset: Offset(5),
                created_at_millis: 200
            }]
        );
    }

    /// A torn `Opened` event (crash mid-append, never synced this specific
    /// call -- simulated by injecting a torn write before the append) is
    /// truncated away, not served as a phantom live segment.
    #[rusty_tokio::test]
    async fn a_torn_event_is_dropped_not_served() {
        let driver = SimDriver::new();
        let mut manifest = Manifest::create_on(driver.clone(), "/log/manifest.log")
            .await
            .unwrap();
        manifest.record_opened(Offset(0), 100).await.unwrap();

        driver.inject_torn_write(0.3); // next write only 30% lands
                                       // `record_opened` still reports success -- a torn write reports a
                                       // full-length write just like the real hazard it simulates (see
                                       // `SimDriver::inject_torn_write`'s own docs) -- so its own internal
                                       // sync doesn't surface anything wrong here. What matters is what
                                       // recovery sees afterward.
        manifest.record_opened(Offset(5), 200).await.unwrap();

        let recovered = driver.crash_and_reopen();
        let (_manifest, live) = Manifest::open_on(recovered, "/log/manifest.log")
            .await
            .unwrap();
        assert_eq!(
            live,
            vec![LiveSegment {
                base_offset: Offset(0),
                created_at_millis: 100
            }]
        );
    }
}
