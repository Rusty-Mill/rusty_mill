//! One server process per data directory (`ADR-0092`, `DDL-FR-001`
//! through `DDL-FR-003`).
//!
//! Every mmap-backed store in this crate documents a single-process
//! exclusive-access assumption under its `unsafe` mapping: nothing else
//! truncates, rewrites, or compacts the file out from under the mapping.
//! Inside one process the store's `RwLock` upholds it. Across processes
//! nothing did — two `memory_server`s pointed at one `SERVER_DATA_DIR`
//! both opened, both mapped, and the second's `Compact` could rewrite a
//! file the first still had mapped. This module is the interlock that
//! was missing: an advisory exclusive lock on one file in the
//! directory, taken before any store is opened and held for the life
//! of the process.
//!
//! # Mechanism
//!
//! [`std::fs::File::try_lock`] (an `flock(LOCK_EX | LOCK_NB)` on Unix,
//! `LockFileEx` on Windows), on `<dir>/.rusty_multimodal_db.lock`. The
//! lock lives on the open file description, so it is released by the
//! kernel when the process exits *however* it exits — a `SIGKILL`ed
//! server leaves no stale lock to clear by hand, which is why a
//! pid-in-a-file scheme was not used. Advisory: a process that ignores
//! the file is not stopped, but every binary in this crate that honours
//! `SERVER_DATA_DIR` takes it first.
//!
//! # What it does not do
//!
//! Nothing about NFS (`flock` there is the mount's own story, as
//! `O_APPEND`'s atomicity already was — `docs/FUTURE-GROWTH.md`), and
//! nothing for the library: `GenericProductionStore` is unchanged, so
//! the two-process diagnosis harness (`src/bin/multiprocess_harness.rs`)
//! still races two writers on one file exactly as it always has.

use std::fs::{File, OpenOptions, TryLockError};
use std::path::{Path, PathBuf};
use thiserror::Error;

/// The lock file's name inside the data directory.
pub const LOCK_FILE_NAME: &str = ".rusty_multimodal_db.lock";

/// Why a data directory could not be claimed.
#[derive(Debug, Error)]
pub enum DataDirLockError {
    /// Another process holds the lock file — a second server on this
    /// directory. Names the file so an operator can find the holder.
    #[error("data directory is held by another process (lock file {path:?})")]
    Held { path: PathBuf },
    /// The directory could not be created or the lock file opened/locked.
    #[error("locking data directory (lock file {path:?}): {source}")]
    Io {
        path: PathBuf,
        #[source]
        source: std::io::Error,
    },
}

/// An exclusive claim on one data directory, released when dropped or
/// when the holding process ends.
#[derive(Debug)]
pub struct DataDirLock {
    file: File,
    path: PathBuf,
}

impl DataDirLock {
    /// Claim `dir` for this process: create it if needed, open
    /// [`LOCK_FILE_NAME`] inside it, and take the exclusive advisory
    /// lock without blocking.
    ///
    /// # Errors
    ///
    /// [`DataDirLockError::Held`] when another process (or another
    /// `DataDirLock` in this one) already holds it;
    /// [`DataDirLockError::Io`] for any other failure.
    pub fn acquire(dir: &Path) -> Result<Self, DataDirLockError> {
        let path = dir.join(LOCK_FILE_NAME);
        let io = |source| DataDirLockError::Io {
            path: path.clone(),
            source,
        };
        std::fs::create_dir_all(dir).map_err(io)?;
        let file = OpenOptions::new()
            .read(true)
            .write(true)
            .create(true)
            .truncate(false)
            .open(&path)
            .map_err(io)?;
        match file.try_lock() {
            Ok(()) => Ok(Self { file, path }),
            Err(TryLockError::WouldBlock) => Err(DataDirLockError::Held { path }),
            Err(TryLockError::Error(source)) => Err(io(source)),
        }
    }

    /// The lock file this claim holds.
    pub fn path(&self) -> &Path {
        &self.path
    }
}

impl Drop for DataDirLock {
    fn drop(&mut self) {
        // Explicit for the reader; closing the descriptor releases the
        // lock regardless, and there is nothing useful to do on failure.
        let _ = self.file.unlock();
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn fresh_dir(tag: &str) -> PathBuf {
        static NEXT: std::sync::atomic::AtomicU64 = std::sync::atomic::AtomicU64::new(0);
        let n = NEXT.fetch_add(1, std::sync::atomic::Ordering::Relaxed);
        let dir =
            std::env::temp_dir().join(format!("rmdb_data_lock_{tag}_{}_{n}", std::process::id()));
        std::fs::create_dir_all(&dir).unwrap();
        dir
    }

    #[test]
    fn a_second_claim_on_the_same_directory_is_held_until_the_first_drops() {
        let dir = fresh_dir("twice");
        let first = DataDirLock::acquire(&dir).unwrap();
        assert!(first.path().ends_with(LOCK_FILE_NAME));

        match DataDirLock::acquire(&dir) {
            Err(DataDirLockError::Held { path }) => assert_eq!(path, dir.join(LOCK_FILE_NAME)),
            other => panic!("expected Held, got {other:?}"),
        }

        drop(first);
        let again = DataDirLock::acquire(&dir).unwrap();
        drop(again);
        std::fs::remove_dir_all(&dir).unwrap();
    }

    #[test]
    fn two_directories_are_independent_and_a_missing_directory_is_created() {
        let a = fresh_dir("a");
        let b = fresh_dir("b").join("nested").join("deeper");
        assert!(!b.exists());
        let lock_a = DataDirLock::acquire(&a).unwrap();
        let lock_b = DataDirLock::acquire(&b).unwrap();
        assert!(b.join(LOCK_FILE_NAME).exists());
        drop(lock_a);
        drop(lock_b);
        std::fs::remove_dir_all(&a).unwrap();
        std::fs::remove_dir_all(b.parent().unwrap().parent().unwrap()).unwrap();
    }

    #[test]
    fn a_path_that_is_a_file_not_a_directory_is_an_io_error() {
        let dir = fresh_dir("file");
        let not_a_dir = dir.join("plain");
        std::fs::write(&not_a_dir, b"x").unwrap();
        match DataDirLock::acquire(&not_a_dir) {
            Err(DataDirLockError::Io { path, .. }) => {
                assert_eq!(path, not_a_dir.join(LOCK_FILE_NAME))
            }
            other => panic!("expected Io, got {other:?}"),
        }
        std::fs::remove_dir_all(&dir).unwrap();
    }
}
