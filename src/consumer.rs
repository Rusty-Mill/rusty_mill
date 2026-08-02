//! Per-consumer offset tracking, single-node only — no consumer-group
//! rebalancing protocol yet (`docs/phase1-scope.md` §2: that's a Phase 2+
//! problem once there's a second real consumer needing it).
//!
//! [`ConsumerOffsets`] answers the open question the previous version of
//! this module's docs left open ("where does a consumer's last-read offset
//! get persisted?"): directly on [`crate::Segment`], not a new storage
//! primitive. Each commit is itself just a record — `[consumer_id][offset]`
//! — appended to a small dedicated segment. Recovery replays every commit
//! record and keeps only the last one per consumer (last-write-wins),
//! reusing exactly the torn-write/checksum recovery [`crate::Segment`]
//! already has, rather than a second hand-rolled recovery path.

use std::collections::HashMap;
use std::sync::Arc;

use rusty_tokio::io::OpDriver;

use crate::offset::{DurableOffset, Epoch, Offset};
use crate::segment::Segment;

/// Tracks the last offset each consumer has committed, backed by an
/// append-only [`Segment`] of commit records. Not tied to any particular
/// [`crate::retention::Log`] — a caller with multiple logs (topics) keeps
/// one `ConsumerOffsets` per log it wants independently trackable consumers
/// for.
pub struct ConsumerOffsets {
    segment: Segment,
    committed: HashMap<String, Offset>,
}

fn encode_commit(consumer_id: &str, offset: Offset) -> Vec<u8> {
    let id = consumer_id.as_bytes();
    let mut buf = Vec::with_capacity(2 + id.len() + 8);
    buf.extend_from_slice(&(id.len() as u16).to_le_bytes());
    buf.extend_from_slice(id);
    buf.extend_from_slice(&offset.0.to_le_bytes());
    buf
}

fn decode_commit(buf: &[u8]) -> std::io::Result<(String, Offset)> {
    let bad = || {
        std::io::Error::new(
            std::io::ErrorKind::InvalidData,
            "malformed consumer-offset commit record",
        )
    };
    if buf.len() < 2 {
        return Err(bad());
    }
    let id_len = u16::from_le_bytes(buf[0..2].try_into().unwrap()) as usize;
    if buf.len() != 2 + id_len + 8 {
        return Err(bad());
    }
    let consumer_id = String::from_utf8(buf[2..2 + id_len].to_vec()).map_err(|_| bad())?;
    let offset = Offset(u64::from_le_bytes(
        buf[2 + id_len..2 + id_len + 8].try_into().unwrap(),
    ));
    Ok((consumer_id, offset))
}

impl ConsumerOffsets {
    /// Starts a brand-new, empty commit log at `path`.
    pub async fn create_on(
        driver: Arc<dyn OpDriver>,
        path: impl AsRef<std::path::Path>,
    ) -> std::io::Result<ConsumerOffsets> {
        let segment = Segment::create_on(driver, path, Offset(0), Epoch::INITIAL).await?;
        Ok(ConsumerOffsets {
            segment,
            committed: HashMap::new(),
        })
    }

    /// Recovers an existing commit log at `path`, replaying every commit
    /// record to rebuild each consumer's last-committed offset. A torn
    /// write on the last commit is handled exactly like a torn write on any
    /// other segment (see [`Segment::open_on`]) — truncated away, not
    /// served.
    pub async fn open_on(
        driver: Arc<dyn OpDriver>,
        path: impl AsRef<std::path::Path>,
    ) -> std::io::Result<ConsumerOffsets> {
        let segment = Segment::open_on(driver, path).await?;
        let mut committed = HashMap::new();
        for i in 0..segment.len() {
            let payload = segment.read(Offset(i)).await?;
            let (consumer_id, offset) = decode_commit(&payload)?;
            committed.insert(consumer_id, offset);
        }
        Ok(ConsumerOffsets { segment, committed })
    }

    /// Records that `consumer_id` has processed up to and including
    /// `offset`. Does not sync — call [`sync`](Self::sync) explicitly, same
    /// fsync-policy-is-the-caller's-call stance as [`Segment::append`].
    pub async fn commit(&mut self, consumer_id: &str, offset: Offset) -> std::io::Result<()> {
        let record = encode_commit(consumer_id, offset);
        self.segment.append(&record).await?;
        self.committed.insert(consumer_id.to_string(), offset);
        Ok(())
    }

    /// The last offset `consumer_id` has committed, or `None` if it's never
    /// committed anything.
    pub fn last_committed(&self, consumer_id: &str) -> Option<Offset> {
        self.committed.get(consumer_id).copied()
    }

    /// Flushes every commit since the last sync to disk.
    pub async fn sync(&mut self) -> std::io::Result<DurableOffset> {
        self.segment.sync().await
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use rusty_tokio::io::SimDriver;

    #[rusty_tokio::test]
    async fn unknown_consumer_has_no_committed_offset() {
        let driver = SimDriver::new();
        let offsets = ConsumerOffsets::create_on(driver, "/offsets/consumers.log")
            .await
            .unwrap();
        assert_eq!(offsets.last_committed("nobody"), None);
    }

    #[rusty_tokio::test]
    async fn commit_updates_the_in_memory_view_immediately() {
        let driver = SimDriver::new();
        let mut offsets = ConsumerOffsets::create_on(driver, "/offsets/consumers.log")
            .await
            .unwrap();

        offsets.commit("reader-a", Offset(5)).await.unwrap();
        assert_eq!(offsets.last_committed("reader-a"), Some(Offset(5)));

        offsets.commit("reader-a", Offset(9)).await.unwrap();
        assert_eq!(offsets.last_committed("reader-a"), Some(Offset(9)));
    }

    #[rusty_tokio::test]
    async fn different_consumers_are_tracked_independently() {
        let driver = SimDriver::new();
        let mut offsets = ConsumerOffsets::create_on(driver, "/offsets/consumers.log")
            .await
            .unwrap();

        offsets.commit("reader-a", Offset(3)).await.unwrap();
        offsets.commit("reader-b", Offset(100)).await.unwrap();
        offsets.commit("reader-a", Offset(4)).await.unwrap();

        assert_eq!(offsets.last_committed("reader-a"), Some(Offset(4)));
        assert_eq!(offsets.last_committed("reader-b"), Some(Offset(100)));
    }

    #[rusty_tokio::test]
    async fn recovery_replays_commits_and_keeps_only_the_latest_per_consumer() {
        let driver = SimDriver::new();
        let path = "/offsets/consumers.log";
        let mut offsets = ConsumerOffsets::create_on(driver.clone(), path)
            .await
            .unwrap();

        offsets.commit("reader-a", Offset(1)).await.unwrap();
        offsets.commit("reader-b", Offset(50)).await.unwrap();
        offsets.commit("reader-a", Offset(2)).await.unwrap();
        offsets.commit("reader-a", Offset(3)).await.unwrap();
        offsets.sync().await.unwrap();

        let recovered = driver.crash_and_reopen();
        let offsets = ConsumerOffsets::open_on(recovered, path).await.unwrap();

        assert_eq!(offsets.last_committed("reader-a"), Some(Offset(3)));
        assert_eq!(offsets.last_committed("reader-b"), Some(Offset(50)));
        assert_eq!(offsets.last_committed("nobody"), None);
    }

    /// A torn commit (crash mid-append, never synced) must not surface a
    /// bogus offset -- recovery truncates it away exactly like a torn
    /// record in any other segment (`segment.rs`'s own tests), and the
    /// consumer just falls back to its last real commit.
    #[rusty_tokio::test]
    async fn a_torn_commit_is_dropped_not_served() {
        let driver = SimDriver::new();
        let path = "/offsets/consumers.log";
        let mut offsets = ConsumerOffsets::create_on(driver.clone(), path)
            .await
            .unwrap();

        offsets.commit("reader-a", Offset(1)).await.unwrap();
        offsets.sync().await.unwrap();

        driver.inject_torn_write(0.3);
        offsets.commit("reader-a", Offset(2)).await.unwrap();
        // No sync -- this commit never becomes durable.

        let recovered = driver.crash_and_reopen();
        let offsets = ConsumerOffsets::open_on(recovered, path).await.unwrap();

        assert_eq!(offsets.last_committed("reader-a"), Some(Offset(1)));
    }
}
