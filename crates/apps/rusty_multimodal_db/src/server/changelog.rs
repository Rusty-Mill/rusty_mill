//! The change log behind continuous replication — `CHL-FR-001`..`CHL-FR-006`,
//! `ADR-0131`, protocol 34: an append-only, `fsync`ed record of the writes a
//! table committed, in commit order, that a standby tails instead of copying
//! the whole table again.
//!
//! # What an entry is, and why the crash contract is a resync
//!
//! An entry is the *effective* writes of one commit unit — the ops of one
//! single-shot write, transaction, batch or session commit that changed
//! something, as [`WriteOp`](crate::server::protocol::WriteOp)s (a `ReplaceIf` that applied is a `Replace`, a
//! transaction's updates are `UpdateField`s; an op that wrote nothing is not
//! in it). A standby applies an entry as one atomic `write_batch` and, having
//! applied every earlier entry, reaches the primary's state.
//!
//! The entry is appended after the store applied it and before the client is
//! answered, all under the log's lock, so the log's order is the apply order
//! and only committed writes are in it (a transaction refused for a
//! read-set conflict never is). A crash between the apply and the append
//! leaves a write the log does not hold. The log does not try to repair that;
//! it makes it visible: the header carries a clean-shutdown flag, cleared
//! (durably) whenever the log is opened and set only by an orderly
//! [`ChangeLog::close_clean`](crate::server::changelog::ChangeLog::close_clean); a log opened with the flag clear was not shut
//! down cleanly, so it is a new *epoch*, and a standby holding an older
//! epoch is told [`ErrorCode::Gone`](crate::server::protocol::ErrorCode::Gone) and re-bootstraps from a snapshot.
//! Correctness by construction, at the price of a full resync after a crash.
//!
//! # Format
//!
//! `RMDBCHLG`, a `u32` LE format version, the epoch (`u64` LE), the sequence
//! number of the file's first entry (`u64` LE), the clean-shutdown flag (one
//! byte); then records of `[u32 LE len][kind u8][payload]`, kind `0` an entry
//! (`crate::codec(Vec<WriteOp>)`). An entry's sequence number is the base plus
//! its index, so it is not stored. A torn tail is dropped, and the open is a
//! new epoch, like any other unclean end.
//!
//! # Retention
//!
//! When the file passes its byte bound, the oldest half of the entries is
//! dropped by rewriting the file (temp file, `fsync`, rename). A standby
//! behind the retained range is told `Gone` and resyncs.

use super::protocol::{ErrorCode, WriteOp};
use std::fs::{File, OpenOptions};
use std::io::{self, Read, Seek, SeekFrom, Write};
use std::path::{Path, PathBuf};
use std::sync::{Condvar, Mutex, MutexGuard};

/// The change log file's magic.
pub const CHANGELOG_MAGIC: &[u8; 8] = b"RMDBCHLG";
/// The format this build writes and reads; a strict match, as the journal's.
pub const CHANGELOG_FORMAT_VERSION: u32 = 1;
/// The default byte bound before the oldest half is dropped (64 MiB).
pub const DEFAULT_RETAIN_BYTES: u64 = 64 << 20;
/// The most entries one `FetchSince` answers, whatever `limit` asks.
pub const MAX_FETCH_ENTRIES: usize = 1_000;
/// An entry larger than this is refused rather than framed.
const MAX_ENTRY_BYTES: usize = 64 << 20;

const HEADER_LEN: u64 = 8 + 4 + 8 + 8 + 1;
const CLEAN_FLAG_AT: u64 = 28;
const KIND_ENTRY: u8 = 0;

/// What a change log can fail at.
#[derive(Debug)]
pub enum ChangeLogError {
    Io(io::Error),
    /// Not a change log, an unknown format version, or a record that is
    /// complete but does not decode.
    Format(String),
}

impl std::fmt::Display for ChangeLogError {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        match self {
            Self::Io(e) => write!(f, "change log I/O: {e}"),
            Self::Format(m) => write!(f, "change log format: {m}"),
        }
    }
}

impl std::error::Error for ChangeLogError {}

impl From<io::Error> for ChangeLogError {
    fn from(e: io::Error) -> Self {
        Self::Io(e)
    }
}

/// Where the log stands: which epoch, the sequence number of the last entry
/// (`0` before any), and the first sequence number still held.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct LogPosition {
    pub epoch: u64,
    pub head: u64,
    pub first: u64,
}

/// A change log for one table. Thread-safe: [`Self::commit`] serializes
/// writers, which is what makes the log's order the apply order.
pub struct ChangeLog {
    inner: Mutex<Inner>,
    /// `ADR-0134` (spike): the group-sync state behind [`Self::sync_through`].
    synced: Mutex<SyncState>,
    /// Wakes the callers waiting on a leader's `fsync`.
    synced_cv: Condvar,
}

/// The group-sync state: the highest durable head and whether a caller is
/// leading an `fsync` right now.
struct SyncState {
    durable: u64,
    syncing: bool,
}

struct Inner {
    path: PathBuf,
    file: File,
    epoch: u64,
    /// The sequence number of the first entry in the file.
    base: u64,
    /// The byte offset of every entry record, in order.
    offsets: Vec<u64>,
    len: u64,
    retain_bytes: u64,
    /// Set when an append failed after its write was applied: the log no
    /// longer holds the table's whole history, and the file may be torn
    /// mid-record. Writes, fetches and a clean close are refused from then
    /// on, so the next open is a new epoch and every standby resyncs.
    poisoned: bool,
    /// Tests only: the next append fails, as a full disk would.
    #[cfg(test)]
    fail_next_append: bool,
    /// Tests only: the next group `fsync` fails.
    #[cfg(test)]
    fail_next_sync: bool,
}

/// A header for a log that is open (the clean flag clear).
fn header(epoch: u64, base: u64) -> [u8; HEADER_LEN as usize] {
    let mut h = [0u8; HEADER_LEN as usize];
    h[..8].copy_from_slice(CHANGELOG_MAGIC);
    h[8..12].copy_from_slice(&CHANGELOG_FORMAT_VERSION.to_le_bytes());
    h[12..20].copy_from_slice(&epoch.to_le_bytes());
    h[20..28].copy_from_slice(&base.to_le_bytes());
    h
}

fn sync_parent(path: &Path) {
    if let Some(dir) = path.parent() {
        if let Ok(d) = File::open(dir) {
            let _ = d.sync_all();
        }
    }
}

impl ChangeLog {
    /// Open (creating if absent) the log at `path`. A file that does not end
    /// in a clean-shutdown marker — the process was killed, or the tail is
    /// torn — is a new epoch, durably, before this returns.
    pub fn open(path: &Path, retain_bytes: u64) -> Result<Self, ChangeLogError> {
        let existed = path.exists();
        let mut file = OpenOptions::new()
            .read(true)
            .write(true)
            .create(true)
            .truncate(false)
            .open(path)?;
        if !existed || file.metadata()?.len() == 0 {
            file.write_all(&header(1, 1))?;
            file.sync_all()?;
            sync_parent(path);
            return Ok(Self::from_parts(
                path,
                file,
                1,
                1,
                Vec::new(),
                HEADER_LEN,
                retain_bytes,
            ));
        }
        let mut bytes = Vec::new();
        file.seek(SeekFrom::Start(0))?;
        file.read_to_end(&mut bytes)?;
        if bytes.len() < HEADER_LEN as usize || &bytes[..8] != CHANGELOG_MAGIC {
            return Err(ChangeLogError::Format(format!(
                "{} is not a change log",
                path.display()
            )));
        }
        let version = u32::from_le_bytes(bytes[8..12].try_into().unwrap_or([0; 4]));
        if version != CHANGELOG_FORMAT_VERSION {
            return Err(ChangeLogError::Format(format!(
                "format version {version}, this build reads {CHANGELOG_FORMAT_VERSION}"
            )));
        }
        let mut epoch = u64::from_le_bytes(bytes[12..20].try_into().unwrap_or([0; 8]));
        let base = u64::from_le_bytes(bytes[20..28].try_into().unwrap_or([0; 8]));
        let said_goodbye = bytes[CLEAN_FLAG_AT as usize] == 1;
        let mut offsets = Vec::new();
        let mut pos = HEADER_LEN as usize;
        while pos + 4 <= bytes.len() {
            let len = u32::from_le_bytes(bytes[pos..pos + 4].try_into().unwrap_or([0; 4])) as usize;
            if len == 0 || pos + 4 + len > bytes.len() {
                break; // a torn tail
            }
            if bytes[pos + 4] != KIND_ENTRY {
                return Err(ChangeLogError::Format(format!(
                    "unknown record kind {}",
                    bytes[pos + 4]
                )));
            }
            crate::codec::decode::<Vec<WriteOp>>(&bytes[pos + 5..pos + 4 + len])
                .map_err(|e| ChangeLogError::Format(format!("entry {}: {e}", offsets.len())))?;
            offsets.push(pos as u64);
            pos += 4 + len;
        }
        let torn = pos < bytes.len();
        if torn {
            file.set_len(pos as u64)?;
        }
        // Whatever the last run's end, this one is open: clear the flag
        // durably before any write. A log that did not say goodbye (or whose
        // tail was torn) is a new epoch.
        if !said_goodbye || torn {
            epoch += 1;
            file.seek(SeekFrom::Start(12))?;
            file.write_all(&epoch.to_le_bytes())?;
        }
        file.seek(SeekFrom::Start(CLEAN_FLAG_AT))?;
        file.write_all(&[0])?;
        file.sync_all()?;
        let end = pos as u64;
        file.seek(SeekFrom::Start(end))?;
        Ok(Self::from_parts(
            path,
            file,
            epoch,
            base,
            offsets,
            end,
            retain_bytes,
        ))
    }

    fn from_parts(
        path: &Path,
        file: File,
        epoch: u64,
        base: u64,
        offsets: Vec<u64>,
        len: u64,
        retain_bytes: u64,
    ) -> Self {
        Self {
            inner: Mutex::new(Inner {
                path: path.to_path_buf(),
                file,
                epoch,
                base,
                offsets,
                len,
                retain_bytes,
                poisoned: false,
                #[cfg(test)]
                fail_next_append: false,
                #[cfg(test)]
                fail_next_sync: false,
            }),
            synced: Mutex::new(SyncState {
                durable: 0,
                syncing: false,
            }),
            synced_cv: Condvar::new(),
        }
    }

    fn lock(&self) -> MutexGuard<'_, Inner> {
        self.inner.lock().unwrap_or_else(|p| p.into_inner())
    }

    /// Where the log stands now.
    pub fn position(&self) -> LogPosition {
        self.lock().position()
    }

    /// Run `apply`, and if it reports effective writes, append them as one
    /// entry (`fsync`ed) before returning — all under the lock, so the log's
    /// order is the apply order. `apply` returns its own result and the ops
    /// that took effect (`None`/empty: nothing to record). An append failure
    /// is `Err(Storage)`: the write is applied and not logged. It poisons
    /// the log: every later commit is `Storage` without running `apply`,
    /// fetches are `Storage`, and [`Self::close_clean`] refuses, so the next
    /// open is a new epoch and every standby resyncs. Continuing would
    /// present a history with a hole in it as whole.
    pub fn commit<R>(
        &self,
        apply: impl FnOnce() -> (R, Option<Vec<WriteOp>>),
    ) -> Result<R, ErrorCode> {
        let mut inner = self.lock();
        inner.ensure_writable()?;
        let (result, ops) = apply();
        if let Some(ops) = ops.filter(|ops| !ops.is_empty()) {
            inner.append(&ops, true).map_err(|_| ErrorCode::Storage)?;
        }
        Ok(result)
    }

    /// `Err(Storage)` once the log is poisoned. A writer that logs from
    /// inside its own ordered section, rather than through [`Self::commit`],
    /// must call this **before applying** each write it will log: after a
    /// failed append or sync the table already holds a write the log lacks,
    /// and no later write may be applied as if the history were whole.
    pub fn ensure_writable(&self) -> Result<(), ErrorCode> {
        self.lock().ensure_writable()
    }

    /// Poison the log for a failure it did not see itself: an `fsync` of
    /// already-appended entries, made outside the append, that failed. The
    /// writes those entries record are applied; their durability is not.
    pub fn poison(&self) {
        self.lock().poisoned = true;
    }

    /// `ADR-0134` (spike): append one entry **without** `fsync`, returning the
    /// head it made — the ticket to hand [`Self::sync_through`]. Called from
    /// inside the wrapped store's own ordered section, so the log's order is
    /// the apply order without this lock being held across the apply.
    pub fn append_deferred(&self, ops: &[WriteOp]) -> Result<u64, ErrorCode> {
        let mut inner = self.lock();
        inner.append(ops, false).map_err(|_| ErrorCode::Storage)?;
        Ok(inner.position().head)
    }

    /// `ADR-0134` (spike): make every entry through `ticket` durable. One
    /// caller does the `fsync` for all the entries appended so far; callers
    /// that arrive while it runs find their entry covered and return.
    pub fn sync_through(&self, ticket: u64) -> Result<(), ErrorCode> {
        let mut state = self.synced.lock().unwrap_or_else(|p| p.into_inner());
        loop {
            if state.durable >= ticket {
                return Ok(());
            }
            // A failed sync poisoned the log: every writer it covered fails
            // too, rather than retrying an `fsync` whose failure may already
            // have dropped the dirty pages (a later "success" proves nothing).
            if self.lock().poisoned {
                return Err(ErrorCode::Storage);
            }
            if state.syncing {
                state = self
                    .synced_cv
                    .wait(state)
                    .unwrap_or_else(|p| p.into_inner());
                continue;
            }
            state.syncing = true;
            drop(state);
            // Read how far to sync as late as possible: everything appended
            // before the `fsync` starts is covered by it.
            let (head, handle) = {
                #[allow(unused_mut)]
                let mut inner = self.lock();
                #[cfg(test)]
                if std::mem::take(&mut inner.fail_next_sync) {
                    drop(inner);
                    let injected = io::Error::other("injected");
                    let head = self.position().head;
                    (head, Err(injected))
                } else {
                    (inner.position().head, inner.file.try_clone())
                }
                #[cfg(not(test))]
                (inner.position().head, inner.file.try_clone())
            };
            let synced = handle.and_then(|f| f.sync_data());
            state = self.synced.lock().unwrap_or_else(|p| p.into_inner());
            state.syncing = false;
            if synced.is_ok() {
                state.durable = state.durable.max(head);
            }
            if synced.is_err() {
                self.poison();
            }
            self.synced_cv.notify_all();
            if synced.is_err() {
                return Err(ErrorCode::Storage);
            }
        }
    }

    /// `f` under the lock, with the position — for a read that must agree
    /// with the log, a snapshot of the table's files.
    pub fn with_position<R>(&self, f: impl FnOnce(LogPosition) -> R) -> R {
        let inner = self.lock();
        f(inner.position())
    }

    /// The entries after `after` in `epoch`, oldest first, at most `limit`
    /// (and [`MAX_FETCH_ENTRIES`]), with the log's position. [`ErrorCode::Gone`]
    /// for another epoch or an `after` older than the log still holds;
    /// `Malformed` for an `after` past the head.
    pub fn since(
        &self,
        epoch: u64,
        after: u64,
        limit: usize,
    ) -> Result<(LogPosition, Vec<Vec<WriteOp>>), ErrorCode> {
        let mut inner = self.lock();
        if inner.poisoned {
            return Err(ErrorCode::Storage);
        }
        let position = inner.position();
        if epoch != position.epoch || after + 1 < position.first {
            return Err(ErrorCode::Gone);
        }
        if after > position.head {
            return Err(ErrorCode::Malformed);
        }
        let first = (after + 1 - inner.base) as usize;
        let take = limit.clamp(1, MAX_FETCH_ENTRIES);
        let mut entries = Vec::new();
        for i in first..inner.offsets.len().min(first + take) {
            entries.push(inner.read_entry(i).map_err(|_| ErrorCode::Storage)?);
        }
        Ok((position, entries))
    }

    /// Say goodbye: a clean-shutdown marker, so the next open continues this
    /// epoch instead of starting a new one.
    /// Refused on a poisoned log (see [`Self::commit`]).
    pub fn close_clean(&self) -> Result<(), ChangeLogError> {
        let mut inner = self.lock();
        if inner.poisoned {
            return Err(ChangeLogError::Format(
                "an append failed: the log is not whole, so it does not close clean".into(),
            ));
        }
        let end = inner.file.stream_position()?;
        inner.file.seek(SeekFrom::Start(CLEAN_FLAG_AT))?;
        inner.file.write_all(&[1])?;
        inner.file.sync_all()?;
        inner.file.seek(SeekFrom::Start(end))?;
        Ok(())
    }
}

impl Inner {
    fn position(&self) -> LogPosition {
        LogPosition {
            epoch: self.epoch,
            head: self.base - 1 + self.offsets.len() as u64,
            first: self.base,
        }
    }

    fn write_record(&mut self, kind: u8, payload: &[u8]) -> io::Result<()> {
        let len = u32::try_from(payload.len() + 1)
            .map_err(|_| io::Error::new(io::ErrorKind::InvalidInput, "record too large"))?;
        let mut record = Vec::with_capacity(4 + len as usize);
        record.extend_from_slice(&len.to_le_bytes());
        record.push(kind);
        record.extend_from_slice(payload);
        self.file.write_all(&record)?;
        self.len += record.len() as u64;
        Ok(())
    }

    fn ensure_writable(&self) -> Result<(), ErrorCode> {
        if self.poisoned {
            return Err(ErrorCode::Storage);
        }
        Ok(())
    }

    /// Every append goes through here — `commit`'s synced one and
    /// `append_deferred`'s unsynced one alike — so any failure poisons the
    /// log (see [`ChangeLog::commit`]); a poisoned log appends nothing more.
    fn append(&mut self, ops: &[WriteOp], sync: bool) -> Result<(), ChangeLogError> {
        if self.poisoned {
            return Err(ChangeLogError::Format("the log is poisoned".into()));
        }
        let appended = self.append_record(ops, sync);
        if appended.is_err() {
            self.poisoned = true;
        }
        appended
    }

    fn append_record(&mut self, ops: &[WriteOp], sync: bool) -> Result<(), ChangeLogError> {
        #[cfg(test)]
        if std::mem::take(&mut self.fail_next_append) {
            return Err(ChangeLogError::Io(io::Error::other("injected")));
        }
        let payload = crate::codec::encode(&ops.to_vec())
            .map_err(|e| ChangeLogError::Format(format!("encoding an entry: {e}")))?;
        if payload.len() > MAX_ENTRY_BYTES {
            return Err(ChangeLogError::Format("entry too large".into()));
        }
        let at = self.len;
        self.write_record(KIND_ENTRY, &payload)?;
        if sync {
            self.file.sync_data()?;
        }
        self.offsets.push(at);
        if self.len > self.retain_bytes && self.offsets.len() > 1 {
            self.compact()?;
        }
        Ok(())
    }

    fn read_entry(&mut self, index: usize) -> Result<Vec<WriteOp>, ChangeLogError> {
        let at = self.offsets[index];
        let end = self.file.stream_position()?;
        self.file.seek(SeekFrom::Start(at))?;
        let mut len = [0u8; 4];
        self.file.read_exact(&mut len)?;
        let mut record = vec![0u8; u32::from_le_bytes(len) as usize];
        self.file.read_exact(&mut record)?;
        self.file.seek(SeekFrom::Start(end))?;
        crate::codec::decode(&record[1..])
            .map_err(|e| ChangeLogError::Format(format!("entry {index}: {e}")))
    }

    /// Drop the oldest half of the entries: rewrite the tail into a temp
    /// file with the new base, `fsync`, rename over the log.
    fn compact(&mut self) -> Result<(), ChangeLogError> {
        let drop_n = self.offsets.len() / 2;
        let keep_from = self.offsets[drop_n];
        let mut tail = vec![0u8; (self.len - keep_from) as usize];
        self.file.seek(SeekFrom::Start(keep_from))?;
        self.file.read_exact(&mut tail)?;
        let new_base = self.base + drop_n as u64;
        let tmp = self.path.with_extension("changes.tmp");
        {
            let mut out = File::create(&tmp)?;
            out.write_all(&header(self.epoch, new_base))?;
            out.write_all(&tail)?;
            out.sync_all()?;
        }
        std::fs::rename(&tmp, &self.path)?;
        sync_parent(&self.path);
        let shift = keep_from - HEADER_LEN;
        self.offsets = self.offsets[drop_n..].iter().map(|o| o - shift).collect();
        self.base = new_base;
        self.file = OpenOptions::new().read(true).write(true).open(&self.path)?;
        self.len = HEADER_LEN + tail.len() as u64;
        self.file.seek(SeekFrom::Start(self.len))?;
        Ok(())
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::server::protocol::RecordId;

    fn dir(label: &str) -> PathBuf {
        use std::sync::atomic::{AtomicU64, Ordering};
        static COUNTER: AtomicU64 = AtomicU64::new(0);
        let d = std::env::temp_dir().join(format!(
            "changelog_{label}_{}_{}",
            std::process::id(),
            COUNTER.fetch_add(1, Ordering::Relaxed)
        ));
        std::fs::create_dir_all(&d).unwrap();
        d
    }

    fn delete(n: u128) -> Vec<WriteOp> {
        vec![WriteOp::Delete {
            id: RecordId::from_u128(n),
        }]
    }

    fn append(log: &ChangeLog, n: u128) {
        log.commit(|| ((), Some(delete(n)))).unwrap();
    }

    /// `CHL-FR-002`: entries are numbered from 1 in commit order; a commit
    /// that took no effect records nothing; `since` returns what follows.
    #[test]
    fn entries_are_numbered_in_commit_order_and_no_effect_records_nothing() {
        let log = ChangeLog::open(&dir("order").join("t.changes"), DEFAULT_RETAIN_BYTES).unwrap();
        assert_eq!(
            log.position(),
            LogPosition {
                epoch: 1,
                head: 0,
                first: 1
            }
        );
        append(&log, 1);
        log.commit(|| ((), None)).unwrap();
        log.commit(|| ((), Some(Vec::new()))).unwrap();
        append(&log, 2);
        append(&log, 3);
        assert_eq!(log.position().head, 3);
        let (at, entries) = log.since(1, 0, 10).unwrap();
        assert_eq!(
            (at.head, entries),
            (3, vec![delete(1), delete(2), delete(3)])
        );
        let (_, entries) = log.since(1, 2, 10).unwrap();
        assert_eq!(entries, vec![delete(3)]);
        let (_, entries) = log.since(1, 3, 10).unwrap();
        assert!(entries.is_empty(), "caught up");
        assert_eq!(log.since(1, 1, 1).unwrap().1, vec![delete(2)], "limit");
        assert_eq!(log.since(1, 4, 10).unwrap_err(), ErrorCode::Malformed);
    }

    /// `CHL-FR-003`: a clean close continues the epoch; a file that just
    /// ends starts a new one, keeps its entries, and turns the old epoch away.
    #[test]
    fn a_clean_close_continues_the_epoch_and_anything_else_starts_a_new_one() {
        let path = dir("epoch").join("t.changes");
        let log = ChangeLog::open(&path, DEFAULT_RETAIN_BYTES).unwrap();
        append(&log, 1);
        log.close_clean().unwrap();
        drop(log);

        let log = ChangeLog::open(&path, DEFAULT_RETAIN_BYTES).unwrap();
        assert_eq!(log.position().epoch, 1, "said goodbye");
        append(&log, 2);
        drop(log); // never said goodbye

        let log = ChangeLog::open(&path, DEFAULT_RETAIN_BYTES).unwrap();
        assert_eq!(log.position().epoch, 2, "a new epoch");
        assert_eq!(log.position().head, 2, "the entries are kept");
        assert_eq!(log.since(1, 0, 10).unwrap_err(), ErrorCode::Gone);
        assert_eq!(log.since(2, 0, 10).unwrap().1, vec![delete(1), delete(2)]);
        log.close_clean().unwrap();
        drop(log);
        let log = ChangeLog::open(&path, DEFAULT_RETAIN_BYTES).unwrap();
        assert_eq!(log.position().epoch, 2, "and it continues");
        append(&log, 3);
        assert_eq!(
            log.since(2, 2, 10).unwrap().1,
            vec![delete(3)],
            "seq continues"
        );
    }

    /// `CHL-FR-003`: opening clears the flag before any write, so a crash
    /// after a clean start, with nothing yet appended, is still a new epoch
    /// (a write may have applied and not been logged).
    #[test]
    fn a_crash_after_a_clean_start_is_still_a_new_epoch() {
        let path = dir("reopen").join("t.changes");
        let log = ChangeLog::open(&path, DEFAULT_RETAIN_BYTES).unwrap();
        log.close_clean().unwrap();
        drop(log);
        let log = ChangeLog::open(&path, DEFAULT_RETAIN_BYTES).unwrap();
        assert_eq!(log.position().epoch, 1, "a clean start continues");
        drop(log); // killed: nothing appended, the flag was cleared at open
        let log = ChangeLog::open(&path, DEFAULT_RETAIN_BYTES).unwrap();
        assert_eq!(log.position().epoch, 2);
    }

    /// `CHL-FR-003`: a torn tail (a crash mid-append) is dropped, the intact
    /// entries survive, and it is a new epoch.
    #[test]
    fn a_torn_tail_is_dropped_and_is_a_new_epoch() {
        let path = dir("torn").join("t.changes");
        let log = ChangeLog::open(&path, DEFAULT_RETAIN_BYTES).unwrap();
        append(&log, 1);
        append(&log, 2);
        log.close_clean().unwrap();
        drop(log);
        let len = std::fs::metadata(&path).unwrap().len();
        let f = OpenOptions::new().write(true).open(&path).unwrap();
        // Cut into entry 2's payload.
        f.set_len(len - 3).unwrap();
        drop(f);
        let log = ChangeLog::open(&path, DEFAULT_RETAIN_BYTES).unwrap();
        assert_eq!(log.position().epoch, 2);
        assert_eq!(log.position().head, 1, "only the intact entry");
        assert_eq!(log.since(2, 0, 10).unwrap().1, vec![delete(1)]);
        append(&log, 9);
        assert_eq!(
            log.since(2, 1, 10).unwrap().1,
            vec![delete(9)],
            "appends after the cut"
        );
    }

    /// `CHL-FR-006`: past its byte bound the oldest half goes; a position
    /// older than what is left is `Gone`; numbering and reopening survive it.
    #[test]
    fn retention_drops_the_oldest_half_and_numbering_survives() {
        let path = dir("retain").join("t.changes");
        let log = ChangeLog::open(&path, 120).unwrap();
        for n in 1..=20 {
            append(&log, n);
        }
        let at = log.position();
        assert_eq!(at.head, 20);
        assert!(at.first > 1, "the oldest entries were dropped: {at:?}");
        assert_eq!(log.since(1, 0, 100).unwrap_err(), ErrorCode::Gone);
        let (_, entries) = log.since(1, at.first - 1, 100).unwrap();
        assert_eq!(entries.len() as u64, 20 - at.first + 1);
        assert_eq!(entries.last().unwrap(), &delete(20));
        assert_eq!(entries.first().unwrap(), &delete(at.first as u128));
        log.close_clean().unwrap();
        drop(log);
        let log = ChangeLog::open(&path, 120).unwrap();
        assert_eq!(log.position(), at, "reopened at the same position");
        append(&log, 21);
        assert_eq!(log.position().head, 21);
    }

    /// `CHL-FR-002`: `commit` runs `apply` under the lock, so concurrent
    /// commits are numbered in the order they applied.
    #[test]
    fn concurrent_commits_are_numbered_in_apply_order() {
        let log = std::sync::Arc::new(
            ChangeLog::open(&dir("conc").join("t.changes"), DEFAULT_RETAIN_BYTES).unwrap(),
        );
        let applied = std::sync::Arc::new(Mutex::new(Vec::new()));
        let handles: Vec<_> = (0..8u128)
            .map(|t| {
                let (log, applied) = (log.clone(), applied.clone());
                std::thread::spawn(move || {
                    for i in 0..10u128 {
                        let n = t * 100 + i;
                        log.commit(|| {
                            applied.lock().unwrap().push(n);
                            ((), Some(delete(n)))
                        })
                        .unwrap();
                    }
                })
            })
            .collect();
        for h in handles {
            h.join().unwrap();
        }
        let (_, entries) = log.since(1, 0, MAX_FETCH_ENTRIES).unwrap();
        let logged: Vec<Vec<WriteOp>> =
            applied.lock().unwrap().iter().map(|n| delete(*n)).collect();
        assert_eq!(entries, logged);
    }

    #[test]
    fn a_file_that_is_not_a_change_log_is_refused() {
        let path = dir("bad").join("t.changes");
        std::fs::write(&path, b"this is not a change log at all, not even close").unwrap();
        assert!(matches!(
            ChangeLog::open(&path, DEFAULT_RETAIN_BYTES),
            Err(ChangeLogError::Format(_))
        ));
    }

    /// Review 2.3 (R3): an append that fails after its write applied
    /// poisons the log: later writes are refused without applying, fetches
    /// fail, a clean close is refused, and the next open is a new epoch.
    #[test]
    fn a_failed_append_poisons_the_log_until_a_new_epoch() {
        let d = dir("poison");
        let path = d.join("t.changes");
        let log = ChangeLog::open(&path, DEFAULT_RETAIN_BYTES).unwrap();
        log.commit(|| ((), Some(delete(1)))).unwrap();
        let epoch = log.position().epoch;

        log.lock().fail_next_append = true;
        assert_eq!(
            log.commit(|| ((), Some(delete(2)))),
            Err(ErrorCode::Storage)
        );
        let mut applied = false;
        assert_eq!(
            log.commit(|| {
                applied = true;
                ((), Some(delete(3)))
            }),
            Err(ErrorCode::Storage)
        );
        assert!(!applied, "a poisoned log runs no further writes");
        assert!(matches!(log.since(epoch, 0, 10), Err(ErrorCode::Storage)));
        assert!(log.close_clean().is_err());
        drop(log);

        let reopened = ChangeLog::open(&path, DEFAULT_RETAIN_BYTES).unwrap();
        assert!(reopened.position().epoch > epoch, "standbys must resync");
    }

    /// The user-flagged interaction with a deferred/grouped writer: a
    /// failure the log only hears about afterwards (a failed group `fsync`)
    /// poisons it too, and `ensure_writable` is the pre-apply check such a
    /// writer uses.
    #[test]
    fn a_poisoned_log_refuses_before_apply_for_any_writer() {
        let d = dir("poison_external");
        let log = ChangeLog::open(&d.join("t.changes"), DEFAULT_RETAIN_BYTES).unwrap();
        assert_eq!(log.ensure_writable(), Ok(()));
        log.poison();
        assert_eq!(log.ensure_writable(), Err(ErrorCode::Storage));
        assert!(
            log.lock().append(&delete(1), true).is_err(),
            "no append after poison"
        );
        assert!(log.close_clean().is_err());
    }

    /// Review 2.3 × ADR-0134: a failed group `fsync` poisons the log, so
    /// the writer it covered fails, later deferred appends are refused, the
    /// pre-apply check refuses, and the log does not close clean.
    #[test]
    fn a_failed_group_sync_poisons_the_log() {
        let d = dir("poison_group_sync");
        let log = ChangeLog::open(&d.join("t.changes"), DEFAULT_RETAIN_BYTES).unwrap();
        let ticket = log.append_deferred(&delete(1)).unwrap();
        log.lock().fail_next_sync = true;
        assert_eq!(log.sync_through(ticket), Err(ErrorCode::Storage));
        assert_eq!(
            log.sync_through(ticket),
            Err(ErrorCode::Storage),
            "no retry succeeds"
        );
        assert_eq!(log.ensure_writable(), Err(ErrorCode::Storage));
        assert_eq!(log.append_deferred(&delete(2)), Err(ErrorCode::Storage));
        assert!(log.close_clean().is_err());
    }
}
