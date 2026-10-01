//! An exclusive lock on a data directory, so a second process refuses to
//! open a store another one is serving.
//!
//! Every record lives in the owning process's memory, and each process
//! appends to the same insert logs, so two processes on one directory would
//! each overwrite the other's writes. `rusty_multimodal_db`'s server takes
//! its own lock (its ADR-0092); an embedder has no server, so it takes this
//! one (`rusty_remind_me`'s hub, and its node daemon: ADR-0021, ADR-0023).
//!
//! The lock is an OS file lock on a named file in the directory. The OS
//! releases it when the process exits, however it exits, so a crashed owner
//! never blocks the next.

use std::fmt;
use std::fs::{File, TryLockError};
use std::io;
use std::path::{Path, PathBuf};

/// Why a directory could not be locked.
#[derive(Debug)]
pub enum DirLockError {
    /// Another process holds it.
    Held(PathBuf),
    Io(PathBuf, io::Error),
}

impl fmt::Display for DirLockError {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        match self {
            DirLockError::Held(dir) => {
                write!(f, "{} is in use by another process", dir.display())
            }
            DirLockError::Io(path, e) => write!(f, "could not lock {}: {e}", path.display()),
        }
    }
}

impl std::error::Error for DirLockError {}

/// Held for as long as the value lives.
#[derive(Debug)]
pub struct DirLock {
    // Never read: the lock lasts exactly as long as this file is open.
    _file: File,
}

impl DirLock {
    /// Lock `dir` through the file `name` inside it, creating both if
    /// needed.
    ///
    /// # Errors
    ///
    /// [`DirLockError::Held`] when another process holds the lock, or
    /// [`DirLockError::Io`] if the directory or file cannot be made.
    pub fn acquire(dir: &Path, name: &str) -> Result<Self, DirLockError> {
        std::fs::create_dir_all(dir).map_err(|e| DirLockError::Io(dir.to_path_buf(), e))?;
        let path = dir.join(name);
        let file = File::options()
            .create(true)
            .truncate(false)
            .write(true)
            .open(&path)
            .map_err(|e| DirLockError::Io(path.clone(), e))?;
        match file.try_lock() {
            Ok(()) => Ok(Self { _file: file }),
            Err(TryLockError::WouldBlock) => Err(DirLockError::Held(dir.to_path_buf())),
            Err(TryLockError::Error(e)) => Err(DirLockError::Io(path, e)),
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::test_support::fresh_temp_dir;

    #[test]
    fn a_second_holder_is_refused_until_the_first_lets_go() {
        let dir = fresh_temp_dir("dir_lock").unwrap();
        let first = DirLock::acquire(&dir, "store.lock").unwrap();
        let err = DirLock::acquire(&dir, "store.lock").unwrap_err();
        assert!(matches!(err, DirLockError::Held(_)), "{err}");
        assert!(err.to_string().contains("in use by another process"));
        drop(first);
        DirLock::acquire(&dir, "store.lock").unwrap();
    }

    #[test]
    fn a_missing_directory_is_created() {
        let dir = fresh_temp_dir("dir_lock_new").unwrap().join("nested");
        DirLock::acquire(&dir, "store.lock").unwrap();
        assert!(dir.join("store.lock").exists());
    }
}
