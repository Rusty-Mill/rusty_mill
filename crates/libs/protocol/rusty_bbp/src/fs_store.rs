//! Durable store: one append-only log file per task plus content-addressed
//! blob files. Stage 2 of the implementation plan.
//!
//! Layout under the data directory:
//!
//! ```text
//! tasks/<task>.log      one JSON line per committed batch of events
//! blobs/<sha256 hex>    blob bytes, written crash-atomically
//! ```
//!
//! A batch is written as a single line and `fsync`ed, so a crash leaves at
//! most one partial line; `open` truncates it. The revision is the number of
//! events in all complete lines, and `append` rejects a caller whose expected
//! revision is stale.
//!
//! Hash chain (#722): a line is `{"prev":"<hex>","events":[...]}`, where
//! `prev` is the SHA-256 of the previous line's bytes (without its `\n`), or
//! of every byte before it for the first chained line of a log that began
//! unchained (zeros when there are none). Legacy lines are bare `[...]` arrays
//! and may only precede the chain. Every load checks the chain, so an edited,
//! removed, reordered or inserted line is `Corrupt`; [`verify_log`] reports
//! the head without touching the file.
//!
//! Cross-process exclusion: every read-repair, revision check and append
//! runs under an exclusive OS lock on `<root>/lock` (`std::fs::File::lock`,
//! `flock` on Unix, `LockFileEx` on Windows), held for the operation only.
//! Two processes on one directory therefore serialise: a repair never sees
//! another writer's half-written batch, and two writers cannot both pass the
//! revision check. The lock is never held across a caller's work.

use crate::event::{
    log_schema, schema_supported, Event, LOG_SCHEMA, LOG_SCHEMA_CONCURRENT, LOG_SCHEMA_EVENT_DRIVEN,
};
use crate::ids::*;
use crate::store::{Store, StoreError};
use std::collections::HashMap;
use std::fs::{self, File, OpenOptions};
use std::io::{self, Read, Write};
use std::path::{Path, PathBuf};

/// File-backed [`Store`].
pub struct FsStore {
    root: PathBuf,
    /// Events per task, loaded on open or on first touch.
    logs: HashMap<TaskId, Vec<Event>>,
}

#[derive(Debug)]
pub enum FsError {
    Io(io::Error),
    Corrupt {
        path: PathBuf,
        line: usize,
        detail: String,
    },
    BlobMismatch {
        expected: Sha256,
        actual: Sha256,
    },
    /// A log written under another schema. `None`: it predates the field.
    UnsupportedSchema {
        path: PathBuf,
        found: Option<u32>,
    },
}

impl std::fmt::Display for FsError {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        match self {
            FsError::Io(e) => write!(f, "io: {e}"),
            FsError::Corrupt { path, line, detail } => {
                write!(f, "{}: line {line}: {detail}", path.display())
            }
            FsError::UnsupportedSchema { path, found } => write!(
                f,
                "{}: log schema {} is not supported (this build reads schemas {LOG_SCHEMA}, {LOG_SCHEMA_CONCURRENT} and {LOG_SCHEMA_EVENT_DRIVEN}); archive it, it does not replay",
                path.display(),
                found.map_or("1 or older".to_string(), |n| n.to_string())
            ),
            FsError::BlobMismatch { expected, actual } => write!(
                f,
                "blob digest mismatch: expected {expected:?}, got {actual:?}"
            ),
        }
    }
}

impl std::error::Error for FsError {}

impl From<io::Error> for FsError {
    fn from(e: io::Error) -> FsError {
        FsError::Io(e)
    }
}

impl From<FsError> for StoreError {
    fn from(e: FsError) -> StoreError {
        match e {
            FsError::UnsupportedSchema { found, .. } => StoreError::UnsupportedSchema { found },
            other => StoreError::Backend(other.to_string()),
        }
    }
}

/// `<root>/tasks/<sha256(task id) hex>.log`. The digest is injective and
/// filesystem-neutral (no case folding, no reserved characters), so two
/// task ids never share a log. The id itself is read back from the log's
/// first event, never from the filename.
pub fn log_path_for(root: &Path, task: &TaskId) -> PathBuf {
    root.join("tasks")
        .join(format!("{}.log", Sha256::of(task.0.as_bytes()).hex()))
}

/// The task a log belongs to: the id its opening event recorded.
fn task_of(path: &Path, events: &[Event]) -> Result<Option<TaskId>, FsError> {
    match events.first() {
        None => Ok(None),
        Some(Event::TaskOpened { schema, .. }) if !schema_supported(*schema) => {
            Err(FsError::UnsupportedSchema {
                path: path.to_path_buf(),
                found: log_schema(events),
            })
        }
        Some(Event::TaskOpened { task, .. }) => Ok(Some(task.clone())),
        Some(other) => Err(FsError::Corrupt {
            path: path.to_path_buf(),
            line: 1,
            detail: format!("log does not start with TaskOpened: {other:?}"),
        }),
    }
}

/// Take the directory's exclusive lock. Released when the returned file drops.
fn lock(root: &Path) -> Result<File, FsError> {
    let file = OpenOptions::new()
        .read(true)
        .write(true)
        .create(true)
        .truncate(false)
        .open(root.join("lock"))?;
    file.lock()?;
    Ok(file)
}

impl FsStore {
    /// Open or create a data directory. Loads every task log it finds,
    /// truncating a trailing partial line left by a crash. Holds the
    /// directory lock while it reads and repairs.
    pub fn open(root: &Path) -> Result<FsStore, FsError> {
        fs::create_dir_all(root.join("tasks"))?;
        fs::create_dir_all(root.join("blobs"))?;
        let mut store = FsStore {
            root: root.to_path_buf(),
            logs: HashMap::new(),
        };
        let _lock = lock(root)?;
        for entry in fs::read_dir(root.join("tasks"))? {
            let path = entry?.path();
            if path.extension().and_then(|e| e.to_str()) != Some("log") {
                continue;
            }
            let (events, _) = load_log(&path)?;
            // An empty log (a crash before its first batch) names no task.
            let Some(task) = task_of(&path, &events)? else {
                continue;
            };
            // Every later read and append goes to the canonical path, so a
            // log anywhere else (a pre-digest name, a copy under another
            // task's digest) would be visible here and unreachable after:
            // refuse it rather than load history nothing can address. One
            // path per digest also means no two logs can claim one id.
            let canonical = log_path_for(root, &task);
            if path != canonical {
                return Err(FsError::Corrupt {
                    path: path.clone(),
                    line: 1,
                    detail: format!(
                        "log names task {} whose canonical path is {}; move or remove it",
                        task.0,
                        canonical.display()
                    ),
                });
            }
            store.logs.insert(task, events);
        }
        Ok(store)
    }

    /// Stable inventory of durable tasks loaded by this store. Refresh or reopen
    /// the store to observe tasks created by another process.
    pub fn task_ids(&self) -> Vec<TaskId> {
        let mut ids: Vec<_> = self.logs.keys().cloned().collect();
        ids.sort_by(|a, b| a.0.cmp(&b.0));
        ids
    }

    pub fn root(&self) -> &Path {
        &self.root
    }

    fn log_path(&self, task: &TaskId) -> PathBuf {
        log_path_for(&self.root, task)
    }

    fn blob_path(&self, sha: &Sha256) -> PathBuf {
        self.root.join("blobs").join(sha.hex())
    }

    /// Current revision of a task, from the loaded log.
    pub fn rev(&self, task: &TaskId) -> Rev {
        Rev(self.logs.get(task).map(|l| l.len() as u64).unwrap_or(0))
    }

    /// Re-read a task's log from disk, so a handle sees another writer's appends.
    pub fn refresh(&mut self, task: &TaskId) -> Result<(), FsError> {
        let path = self.log_path(task);
        let _lock = lock(&self.root)?;
        let events = if path.exists() {
            load_log(&path)?.0
        } else {
            Vec::new()
        };
        self.logs.insert(task.clone(), events);
        Ok(())
    }
}

/// Read all complete lines; truncate the file after the last newline if a
/// partial line trails it, then [`decode`]. Returns the events and the
/// `prev` the next line must carry.
fn load_log(path: &Path) -> Result<(Vec<Event>, Sha256), FsError> {
    let mut file = OpenOptions::new().read(true).write(true).open(path)?;
    let mut bytes = Vec::new();
    file.read_to_end(&mut bytes)?;
    let complete = complete_len(&bytes);
    if complete < bytes.len() {
        file.set_len(complete as u64)?;
        file.sync_all()?;
        bytes.truncate(complete);
    }
    let (events, chain) = decode(path, &bytes)?;
    Ok((events, chain.next))
}

/// Check the hash chain of a log's complete lines, then decode each line's
/// events. Shared by load and [`verify_log`], so the two refuse the same logs.
fn decode(path: &Path, bytes: &[u8]) -> Result<(Vec<Event>, Chain), FsError> {
    let corrupt = |line: usize, detail: String| FsError::Corrupt {
        path: path.to_path_buf(),
        line,
        detail,
    };
    let chain = check(bytes).map_err(|e| corrupt(e.line, e.detail))?;
    let mut events = Vec::new();
    for (i, line) in split_lines(bytes).enumerate() {
        let body = match parse_line(line) {
            // A blank line is tolerated only before the chain (`check`).
            Line::Legacy([]) => continue,
            Line::Legacy(body) | Line::Chained { events: body, .. } => body,
            // `check` has refused it already.
            Line::Malformed => return Err(corrupt(i + 1, "malformed chained line".into())),
        };
        let text = std::str::from_utf8(body).map_err(|e| corrupt(i + 1, e.to_string()))?;
        let batch: Vec<Event> = rusty_serde::json::from_str(text).map_err(|e| {
            // A schema-1 `task_opened` has no `schema` field and fails to decode.
            if i == 0 && text.contains("\"TaskOpened\"") && !text.contains("\"schema\"") {
                return FsError::UnsupportedSchema {
                    path: path.to_path_buf(),
                    found: None,
                };
            }
            corrupt(i + 1, e.to_string())
        })?;
        events.extend(batch);
    }
    Ok((events, chain))
}

/// What [`check`] found in a log's bytes.
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct Chain {
    /// SHA-256 of each complete line, without its `\n`, in order.
    pub hashes: Vec<Sha256>,
    /// Leading lines written before the log was chained.
    pub unchained: usize,
    /// The `prev` the next appended line must carry.
    pub next: Sha256,
    /// Bytes after the last `\n`: a crash mid-write, or a cut edit.
    pub partial: bool,
}

impl Chain {
    /// SHA-256 of the last complete line; zeros for an empty log.
    pub fn head(&self) -> Sha256 {
        self.hashes.last().copied().unwrap_or(GENESIS)
    }
}

/// Where in the bytes the chain broke. `line` is 1-based.
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct ChainError {
    pub line: usize,
    pub detail: String,
}

/// The `prev` of the first line of a log that has no bytes before it.
pub const GENESIS: Sha256 = Sha256([0; 32]);

const PREV_OPEN: &[u8] = b"{\"prev\":\"";
const PREV_CLOSE: &[u8] = b"\",\"events\":";

/// Check a log's hash chain. Pure: reads `bytes`, writes nothing.
pub fn check(bytes: &[u8]) -> Result<Chain, ChainError> {
    let complete = complete_len(bytes);
    let mut chain = Chain {
        hashes: Vec::new(),
        unchained: 0,
        next: GENESIS,
        partial: complete < bytes.len(),
    };
    let mut offset = 0;
    for (i, line) in split_lines(&bytes[..complete]).enumerate() {
        let fail = |detail: String| ChainError {
            line: i + 1,
            detail,
        };
        let chained = chain.hashes.len() > chain.unchained;
        match parse_line(line) {
            Line::Legacy(_) if chained => {
                return Err(fail("unchained line after the chain began".into()))
            }
            Line::Legacy(_) => chain.unchained += 1,
            Line::Malformed => return Err(fail("malformed chained line".into())),
            Line::Chained { prev, .. } => {
                let want = match (chained, offset) {
                    (true, _) => chain.head(),
                    (false, 0) => GENESIS,
                    (false, _) => Sha256::of(&bytes[..offset]),
                };
                if prev != want {
                    return Err(fail(format!("chain broken: prev should be {}", want.hex())));
                }
            }
        }
        chain.hashes.push(Sha256::of(line));
        offset += line.len() + 1;
    }
    chain.next = match (chain.hashes.len() > chain.unchained, complete) {
        (true, _) => chain.head(),
        (false, 0) => GENESIS,
        (false, _) => Sha256::of(&bytes[..complete]),
    };
    Ok(chain)
}

/// One log line, by its framing.
enum Line<'a> {
    /// A bare events array, written before #722.
    Legacy(&'a [u8]),
    /// `{"prev":"<hex>","events":<events>}`, exactly.
    Chained { prev: Sha256, events: &'a [u8] },
    /// Starts like a chained line but is not exactly one.
    Malformed,
}

/// Classify a line. A line that opens with `{"prev":"` must be the whole
/// envelope, closing brace included, or it is malformed.
fn parse_line(line: &[u8]) -> Line<'_> {
    let Some(rest) = line.strip_prefix(PREV_OPEN) else {
        return Line::Legacy(line);
    };
    let prev = rest
        .get(..64)
        .and_then(|h| std::str::from_utf8(h).ok())
        .and_then(Sha256::from_hex);
    let events = rest
        .get(64..)
        .and_then(|r| r.strip_prefix(PREV_CLOSE))
        .and_then(|r| r.strip_suffix(b"}"));
    match (prev, events) {
        (Some(prev), Some(events)) => Line::Chained { prev, events },
        _ => Line::Malformed,
    }
}

/// One chained line for `events`, with its `\n`.
fn chained_line(prev: Sha256, events: &[Event]) -> Result<String, StoreError> {
    let json = rusty_serde::json::to_string(&events.to_vec())
        .map_err(|e| StoreError::Backend(e.to_string()))?;
    Ok(format!(
        "{{\"prev\":\"{}\",\"events\":{json}}}\n",
        prev.hex()
    ))
}

/// Bytes up to and including the last `\n`.
fn complete_len(bytes: &[u8]) -> usize {
    bytes.iter().rposition(|b| *b == b'\n').map_or(0, |i| i + 1)
}

/// The `\n`-terminated lines of `bytes`, without their `\n`. A trailing
/// partial line is not yielded.
fn split_lines(bytes: &[u8]) -> impl Iterator<Item = &[u8]> {
    let complete = &bytes[..complete_len(bytes)];
    complete
        .split_inclusive(|b| *b == b'\n')
        .map(|l| &l[..l.len() - 1])
}

/// Check one task's log as it stands on disk, under the directory lock,
/// without repairing or otherwise changing it (`bbp verify`). It refuses
/// exactly what a load refuses, apart from a partial last line, which it
/// reports in [`Chain::partial`].
pub fn verify_log(root: &Path, task: &TaskId) -> Result<Chain, FsError> {
    let _lock = lock(root)?;
    let path = log_path_for(root, task);
    let bytes = fs::read(&path)?;
    decode(&path, &bytes).map(|(_, chain)| chain)
}

impl Store for FsStore {
    fn append(
        &mut self,
        task: &TaskId,
        expected_rev: Rev,
        events: &[Event],
    ) -> Result<(), StoreError> {
        // Check against disk, not only memory: another handle may have written.
        // The lock spans repair, check, write and fsync, so no other process
        // can slip a batch in between or repair ours away.
        let path = self.log_path(task);
        let _lock = lock(&self.root).map_err(StoreError::from)?;
        let (on_disk, prev) = if path.exists() {
            load_log(&path).map_err(StoreError::from)?
        } else {
            (Vec::new(), GENESIS)
        };
        let actual = Rev(on_disk.len() as u64);
        if actual != expected_rev {
            self.logs.insert(task.clone(), on_disk);
            return Err(StoreError::Conflict {
                expected: expected_rev,
                actual,
            });
        }
        if events.is_empty() {
            return Ok(());
        }
        let line = chained_line(prev, events)?;
        let mut file = OpenOptions::new()
            .create(true)
            .append(true)
            .open(&path)
            .map_err(FsError::from)?;
        file.write_all(line.as_bytes()).map_err(FsError::from)?;
        file.sync_all().map_err(FsError::from)?;
        let log = self.logs.entry(task.clone()).or_default();
        *log = on_disk;
        log.extend_from_slice(events);
        Ok(())
    }

    fn events(&self, task: &TaskId, after: usize) -> Result<Vec<Event>, StoreError> {
        let log = self.logs.get(task).ok_or(StoreError::UnknownTask)?;
        Ok(log.get(after..).unwrap_or(&[]).to_vec())
    }

    fn refresh(&mut self, task: &TaskId) -> Result<(), StoreError> {
        FsStore::refresh(self, task).map_err(StoreError::from)
    }

    fn blob_put(&mut self, bytes: &[u8]) -> BlobRef {
        let sha = Sha256::of(bytes);
        let path = self.blob_path(&sha);
        if !path.exists() {
            // A failed write leaves no target file; the caller's next put retries.
            let _ = rusty_atomic_file::write(&path, bytes);
        }
        BlobRef {
            sha,
            len: bytes.len() as u64,
        }
    }

    fn blob_get(&self, sha: &Sha256) -> Result<Vec<u8>, StoreError> {
        let bytes = fs::read(self.blob_path(sha)).map_err(|_| StoreError::UnknownBlob)?;
        let actual = Sha256::of(&bytes);
        if actual != *sha {
            return Err(StoreError::Backend(
                FsError::BlobMismatch {
                    expected: *sha,
                    actual,
                }
                .to_string(),
            ));
        }
        Ok(bytes)
    }
}

/// Hold the directory lock from outside. Test support: stands in for a
/// process that is mid-operation.
#[doc(hidden)]
pub fn lock_for_test(root: &Path) -> io::Result<File> {
    lock(root).map_err(|e| match e {
        FsError::Io(e) => e,
        other => io::Error::other(other.to_string()),
    })
}

/// Truncate a task log to `bytes` bytes. Test support: simulates a crash mid-write.
#[doc(hidden)]
pub fn truncate_log_for_test(root: &Path, task: &TaskId, bytes: u64) -> io::Result<()> {
    let file = File::options().write(true).open(log_path_for(root, task))?;
    file.set_len(bytes)?;
    file.sync_all()
}
