//! Keeping the index live as you work.
//!
//! "First pass takes seconds. After that it stays live as you work, reading
//! each file once."
//!
//! # Polling, deliberately
//!
//! Source stores are stat-ed on an interval rather than watched through
//! filesystem events. That is a choice, not a shortcut: it needs no extra
//! dependency, it behaves identically on macOS, Linux and Windows, and a
//! stat of a few hundred paths every few seconds is far cheaper than the
//! indexing it gates. An event-based watcher would also be defensible — it
//! would also be a new dependency and a new class of platform-specific
//! failure, in a crate whose whole pitch is that it needs nothing installed.
//!
//! # The debounce is the subtle part
//!
//! A file whose `(mtime, size)` has *just* changed is **deferred** until a
//! later tick observes the same signature. That is what stops the indexer
//! reading a transcript while the agent is still appending to it.
//!
//! Without it, an active Claude Code session would re-trigger indexing on
//! every single message — the exact opposite of "reading each file once".
//! Parsers here already skip a truncated trailing line, so a mid-write read
//! costs correctness nothing; it costs *work*, repeatedly, on the machine of
//! someone who is in the middle of using their editor.
//!
//! # But a file modified before the grace window settles immediately
//!
//! Otherwise every launch would sit through a full interval before touching
//! the backlog already on disk. Old files are, by definition, not being
//! written to right now.

use crate::model::SourceId;
use crate::sources;
use std::collections::{BTreeSet, HashMap, HashSet};
use std::path::{Path, PathBuf};
use std::time::Duration;

/// How often to stat the source stores.
pub const DEFAULT_INTERVAL: Duration = Duration::from_secs(5);
/// A file untouched for at least this long is not being written to.
pub const DEFAULT_GRACE_SECS: i64 = 4;

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
struct Signature {
    mtime: i64,
    size: i64,
}

fn signature(path: &Path) -> Option<Signature> {
    let meta = std::fs::metadata(path).ok()?;
    Some(Signature {
        mtime: meta
            .modified()
            .ok()
            .and_then(|t| t.duration_since(std::time::UNIX_EPOCH).ok())
            .map(|d| d.as_secs() as i64)
            .unwrap_or(0),
        size: meta.len() as i64,
    })
}

/// What one tick observed.
#[derive(Debug, Clone, Default)]
pub struct WatchTick {
    /// Sources with at least one settled change — index these.
    pub changed_sources: BTreeSet<SourceId>,
    /// Files whose contents have settled since the last tick.
    pub settled: Vec<PathBuf>,
    /// Files still being written, held back for a later tick.
    pub deferred: Vec<PathBuf>,
    /// Files that have disappeared. Reported, but never a reason to index:
    /// a deleted transcript does not delete what was indexed from it.
    pub vanished: Vec<PathBuf>,
}

impl WatchTick {
    /// Is there anything worth re-indexing?
    pub fn needs_index(&self) -> bool {
        !self.changed_sources.is_empty()
    }
}

/// Tracks file signatures across ticks so changes can be debounced.
pub struct Watcher {
    grace_secs: i64,
    /// Last signature accepted as settled.
    settled: HashMap<PathBuf, Signature>,
    /// Changed, awaiting a confirming observation.
    pending: HashMap<PathBuf, Signature>,
}

impl Default for Watcher {
    fn default() -> Self {
        Watcher::new(DEFAULT_GRACE_SECS)
    }
}

impl Watcher {
    pub fn new(grace_secs: i64) -> Self {
        Watcher {
            grace_secs: grace_secs.max(0),
            settled: HashMap::new(),
            pending: HashMap::new(),
        }
    }

    /// Every file the installed sources read, tagged with its source.
    pub fn enumerate() -> Vec<(SourceId, PathBuf)> {
        let mut out = Vec::new();
        for source in sources::all() {
            if !source.is_installed() {
                continue;
            }
            let id = source.id();
            out.extend(source.files().into_iter().map(|p| (id, p)));
        }
        out
    }

    /// Record the current state without reporting it.
    ///
    /// Call this straight after the initial index, so the first real tick
    /// reports what changed *since* that index rather than re-reporting the
    /// entire backlog.
    pub fn prime(&mut self) {
        self.prime_at(crate::model::now_unix());
    }

    pub fn prime_at(&mut self, now: i64) {
        self.observe(now, &Watcher::enumerate(), false);
    }

    /// Stat every source file and report what has settled.
    pub fn poll(&mut self) -> WatchTick {
        self.poll_at(crate::model::now_unix())
    }

    pub fn poll_at(&mut self, now: i64) -> WatchTick {
        self.observe(now, &Watcher::enumerate(), true)
    }

    /// Poll an explicit file list rather than the installed sources. Lets a
    /// caller watch a subset, and lets the tests run without a fixture
    /// machine installed behind an environment variable.
    pub fn poll_paths_at(&mut self, now: i64, entries: &[(SourceId, PathBuf)]) -> WatchTick {
        self.observe(now, entries, true)
    }

    fn observe(&mut self, now: i64, entries: &[(SourceId, PathBuf)], report: bool) -> WatchTick {
        let mut tick = WatchTick::default();
        let mut present: HashSet<&Path> = HashSet::with_capacity(entries.len());

        for (source, path) in entries {
            // Presence means "stat succeeded", not "the enumeration mentioned
            // it". A path that cannot be stat-ed is gone as far as the watcher
            // is concerned, whether the source's file list has caught up or
            // not.
            let Some(sig) = signature(path) else {
                continue;
            };
            present.insert(path.as_path());

            if self.settled.get(path) == Some(&sig) {
                // Back to a state we have already indexed — including the case
                // of a file that changed and changed back within one interval.
                self.pending.remove(path);
                continue;
            }

            if !report {
                self.settled.insert(path.clone(), sig);
                self.pending.remove(path);
                continue;
            }

            // Settle if the file has been quiet long enough to be finished
            // with, or if this tick confirms what the last one saw.
            let quiet = now.saturating_sub(sig.mtime) >= self.grace_secs;
            let confirmed = self.pending.get(path) == Some(&sig);

            if quiet || confirmed {
                self.settled.insert(path.clone(), sig);
                self.pending.remove(path);
                tick.settled.push(path.clone());
                tick.changed_sources.insert(*source);
            } else {
                self.pending.insert(path.clone(), sig);
                tick.deferred.push(path.clone());
            }
        }

        // Forget files that are gone, so a path that returns later is treated
        // as new rather than compared against a stale signature.
        let gone: Vec<PathBuf> = self
            .settled
            .keys()
            .chain(self.pending.keys())
            .filter(|p| !present.contains(p.as_path()))
            .cloned()
            .collect();
        for path in gone {
            self.settled.remove(&path);
            self.pending.remove(&path);
            if report {
                tick.vanished.push(path);
            }
        }

        tick
    }

    /// Files currently held back as still-being-written.
    pub fn deferred_count(&self) -> usize {
        self.pending.len()
    }

    pub fn tracked_count(&self) -> usize {
        self.settled.len()
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    struct Fixture {
        dir: tempfile::TempDir,
    }

    impl Fixture {
        fn new() -> Self {
            Fixture {
                dir: tempfile::tempdir().unwrap(),
            }
        }

        fn write(&self, name: &str, body: &str) -> PathBuf {
            let p = self.dir.path().join(name);
            std::fs::write(&p, body).unwrap();
            p
        }

        fn entries(&self, paths: &[&PathBuf]) -> Vec<(SourceId, PathBuf)> {
            paths
                .iter()
                .map(|p| (SourceId::ClaudeCode, (*p).clone()))
                .collect()
        }

        fn mtime(&self, path: &Path) -> i64 {
            signature(path).unwrap().mtime
        }
    }

    /// The headline behaviour: a file being written right now is held back
    /// until a later tick sees it unchanged.
    #[test]
    fn a_file_being_written_is_deferred_until_it_settles() {
        let fx = Fixture::new();
        let f = fx.write("session.jsonl", "{}");
        let entries = fx.entries(&[&f]);
        let now = fx.mtime(&f);
        let mut w = Watcher::new(4);

        // Modified this instant: not safe to read yet.
        let tick = w.poll_paths_at(now, &entries);
        assert!(tick.settled.is_empty(), "read a file mid-write");
        assert_eq!(tick.deferred, vec![f.clone()]);
        assert!(!tick.needs_index());

        // One interval later, unchanged — now it is safe.
        let tick = w.poll_paths_at(now + 1, &entries);
        assert_eq!(tick.settled, vec![f.clone()]);
        assert!(tick.deferred.is_empty());
        assert!(tick.needs_index());
        assert!(tick.changed_sources.contains(&SourceId::ClaudeCode));

        // And it is not reported again.
        let tick = w.poll_paths_at(now + 2, &entries);
        assert!(tick.settled.is_empty());
        assert!(!tick.needs_index());
    }

    /// The startup backlog must not wait an interval before being indexed.
    #[test]
    fn a_file_older_than_the_grace_window_settles_immediately() {
        let fx = Fixture::new();
        let f = fx.write("old.jsonl", "{}");
        let entries = fx.entries(&[&f]);
        let mut w = Watcher::new(4);

        let tick = w.poll_paths_at(fx.mtime(&f) + 60, &entries);
        assert_eq!(tick.settled, vec![f], "backlog should not be deferred");
        assert!(tick.needs_index());
    }

    /// An agent appending message after message must not settle in between.
    #[test]
    fn a_growing_file_stays_deferred() {
        let fx = Fixture::new();
        let f = fx.write("live.jsonl", "{}");
        let entries = fx.entries(&[&f]);
        let base = fx.mtime(&f);
        let mut w = Watcher::new(4);

        for i in 0..4 {
            // Each tick finds it larger than the last.
            std::fs::write(&f, "{}".repeat(i + 2)).unwrap();
            let now = fx.mtime(&f);
            let tick = w.poll_paths_at(now, &entries);
            assert!(
                tick.settled.is_empty(),
                "settled while still growing on tick {i}"
            );
            assert_eq!(tick.deferred.len(), 1);
        }
        assert_eq!(w.deferred_count(), 1);

        // The agent stops; the next tick confirms the same signature.
        let tick = w.poll_paths_at(fx.mtime(&f), &entries);
        assert_eq!(tick.settled.len(), 1);
        assert_eq!(w.deferred_count(), 0);
        assert!(base <= fx.mtime(&f));
    }

    #[test]
    fn priming_suppresses_the_initial_backlog() {
        let fx = Fixture::new();
        let f = fx.write("a.jsonl", "{}");
        let entries = fx.entries(&[&f]);
        let now = fx.mtime(&f) + 60;
        let mut w = Watcher::new(4);

        w.observe(now, &entries, false);
        assert_eq!(w.tracked_count(), 1);

        let tick = w.poll_paths_at(now + 1, &entries);
        assert!(!tick.needs_index(), "primed file was reported as a change");
    }

    #[test]
    fn a_vanished_file_is_forgotten_and_never_triggers_indexing() {
        let fx = Fixture::new();
        let f = fx.write("gone.jsonl", "{}");
        let entries = fx.entries(&[&f]);
        let now = fx.mtime(&f) + 60;
        let mut w = Watcher::new(4);

        assert!(w.poll_paths_at(now, &entries).needs_index());
        std::fs::remove_file(&f).unwrap();

        let tick = w.poll_paths_at(now + 1, &entries);
        assert_eq!(tick.vanished, vec![f]);
        assert!(
            !tick.needs_index(),
            "a deleted transcript must not trigger a re-index"
        );
        assert_eq!(w.tracked_count(), 0);
    }

    /// A file that changes and reverts within one interval is not a change.
    #[test]
    fn a_reverted_file_is_not_reported() {
        let fx = Fixture::new();
        let f = fx.write("r.jsonl", "hello");
        let entries = fx.entries(&[&f]);
        let mut w = Watcher::new(4);
        assert!(w.poll_paths_at(fx.mtime(&f) + 60, &entries).needs_index());

        let before = signature(&f).unwrap();
        std::fs::write(&f, "changed!!").unwrap();
        let tick = w.poll_paths_at(fx.mtime(&f), &entries);
        assert_eq!(tick.deferred.len(), 1);

        // Restore the original bytes and mtime.
        std::fs::write(&f, "hello").unwrap();
        let restored = std::fs::File::open(&f).unwrap();
        restored
            .set_times(
                std::fs::FileTimes::new()
                    .set_modified(std::time::UNIX_EPOCH + Duration::from_secs(before.mtime as u64)),
            )
            .unwrap();

        let tick = w.poll_paths_at(fx.mtime(&f) + 60, &entries);
        assert!(tick.settled.is_empty(), "revert reported as a change");
        assert_eq!(w.deferred_count(), 0, "pending entry not cleared on revert");
    }

    #[test]
    fn changes_are_attributed_to_the_right_source() {
        let fx = Fixture::new();
        let a = fx.write("a.jsonl", "{}");
        let b = fx.write("b.vscdb", "{}");
        let entries = vec![
            (SourceId::ClaudeCode, a.clone()),
            (SourceId::Cursor, b.clone()),
        ];
        let mut w = Watcher::new(4);

        let tick = w.poll_paths_at(fx.mtime(&a) + 60, &entries);
        assert_eq!(
            tick.changed_sources,
            [SourceId::ClaudeCode, SourceId::Cursor]
                .into_iter()
                .collect::<BTreeSet<_>>()
        );

        // Touch only Cursor's store.
        std::fs::write(&b, "{}{}").unwrap();
        let tick = w.poll_paths_at(fx.mtime(&b) + 60, &entries);
        assert_eq!(
            tick.changed_sources,
            [SourceId::Cursor].into_iter().collect::<BTreeSet<_>>()
        );
    }

    #[test]
    fn every_installed_source_can_enumerate_its_files() {
        // No fixture home here, so this asserts the call is total rather than
        // that it finds anything.
        let entries = Watcher::enumerate();
        assert!(entries.iter().all(|(_, p)| p.is_absolute() || p.exists()));
    }
}
