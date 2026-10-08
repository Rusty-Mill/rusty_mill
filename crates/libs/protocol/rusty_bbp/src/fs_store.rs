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
//! revision is stale. One writer per directory is the caller's responsibility.

use crate::event::Event;
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
}

impl std::fmt::Display for FsError {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        match self {
            FsError::Io(e) => write!(f, "io: {e}"),
            FsError::Corrupt { path, line, detail } => {
                write!(f, "{}: line {line}: {detail}", path.display())
            }
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
        StoreError::Backend(e.to_string())
    }
}

fn safe_name(task: &TaskId) -> String {
    task.0
        .chars()
        .map(|c| {
            if c.is_ascii_alphanumeric() || c == '-' || c == '_' {
                c
            } else {
                '_'
            }
        })
        .collect()
}

impl FsStore {
    /// Open or create a data directory. Loads every task log it finds,
    /// truncating a trailing partial line left by a crash.
    pub fn open(root: &Path) -> Result<FsStore, FsError> {
        fs::create_dir_all(root.join("tasks"))?;
        fs::create_dir_all(root.join("blobs"))?;
        let mut store = FsStore {
            root: root.to_path_buf(),
            logs: HashMap::new(),
        };
        for entry in fs::read_dir(root.join("tasks"))? {
            let path = entry?.path();
            let Some(stem) = path.file_stem().and_then(|s| s.to_str()) else {
                continue;
            };
            if path.extension().and_then(|e| e.to_str()) != Some("log") {
                continue;
            }
            let events = load_log(&path)?;
            store.logs.insert(TaskId(stem.to_owned()), events);
        }
        Ok(store)
    }

    pub fn root(&self) -> &Path {
        &self.root
    }

    fn log_path(&self, task: &TaskId) -> PathBuf {
        self.root
            .join("tasks")
            .join(format!("{}.log", safe_name(task)))
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
        let events = if path.exists() {
            load_log(&path)?
        } else {
            Vec::new()
        };
        self.logs.insert(task.clone(), events);
        Ok(())
    }
}

/// Read all complete lines; truncate the file after the last newline if a
/// partial line trails it. Each line is a JSON array of events.
fn load_log(path: &Path) -> Result<Vec<Event>, FsError> {
    let mut file = OpenOptions::new().read(true).write(true).open(path)?;
    let mut bytes = Vec::new();
    file.read_to_end(&mut bytes)?;
    let complete = bytes
        .iter()
        .rposition(|b| *b == b'\n')
        .map(|i| i + 1)
        .unwrap_or(0);
    if complete < bytes.len() {
        file.set_len(complete as u64)?;
        file.sync_all()?;
        bytes.truncate(complete);
    }
    let text = String::from_utf8(bytes).map_err(|e| FsError::Corrupt {
        path: path.to_path_buf(),
        line: 0,
        detail: e.to_string(),
    })?;
    let mut events = Vec::new();
    for (i, line) in text.lines().enumerate() {
        if line.is_empty() {
            continue;
        }
        let batch: Vec<Event> =
            rusty_serde::json::from_str(line).map_err(|e| FsError::Corrupt {
                path: path.to_path_buf(),
                line: i + 1,
                detail: e.to_string(),
            })?;
        events.extend(batch);
    }
    Ok(events)
}

impl Store for FsStore {
    fn append(
        &mut self,
        task: &TaskId,
        expected_rev: Rev,
        events: &[Event],
    ) -> Result<(), StoreError> {
        // Check against disk, not only memory: another handle may have written.
        let path = self.log_path(task);
        let on_disk = if path.exists() {
            load_log(&path).map_err(StoreError::from)?
        } else {
            Vec::new()
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
        let line = rusty_serde::json::to_string(&events.to_vec())
            .map_err(|e| StoreError::Backend(e.to_string()))?;
        let mut file = OpenOptions::new()
            .create(true)
            .append(true)
            .open(&path)
            .map_err(FsError::from)?;
        file.write_all(line.as_bytes()).map_err(FsError::from)?;
        file.write_all(b"\n").map_err(FsError::from)?;
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

/// Truncate a task log to `bytes` bytes. Test support: simulates a crash mid-write.
#[doc(hidden)]
pub fn truncate_log_for_test(root: &Path, task: &TaskId, bytes: u64) -> io::Result<()> {
    let path = root.join("tasks").join(format!("{}.log", safe_name(task)));
    let file = File::options().write(true).open(path)?;
    file.set_len(bytes)?;
    file.sync_all()
}
