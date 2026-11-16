//! The trusted-origins file (C63, gui-design D72): read it, and save to it
//! safely when several windows or processes may save at once.
//!
//! A save is one transaction: take a lock file, read what is on disk, merge,
//! write a temp file of its own, rename it over the file, drop the lock. So
//! two saves never lose each other's additions and never share a temp file.

use super::link_policy::Policy;
use std::io::{Error, ErrorKind, Result};
use std::path::{Path, PathBuf};
use std::sync::atomic::{AtomicU64, Ordering};
use std::time::{Duration, Instant};

/// How long a save waits for the lock, and how old a lock must be to be a
/// crashed writer's leftover.
const WAIT: Duration = Duration::from_secs(5);
const STALE: Duration = Duration::from_secs(30);
const POLL: Duration = Duration::from_millis(10);

/// Where the file lives: beside `config.toml`, so a run-time write never
/// rewrites the user's hand-edited config.
pub(crate) fn default_path() -> Option<PathBuf> {
    Some(
        rusty_dirs::config_root()?
            .join("rusty_term")
            .join("trusted_origins.toml"),
    )
}

/// The saved origins. A missing file is an empty list; any other read error
/// is an error, so a save never overwrites what it could not read.
pub(crate) fn read(path: &Path) -> Result<Policy> {
    match std::fs::read_to_string(path) {
        Ok(text) => Ok(Policy::from_toml(&text)),
        Err(e) if e.kind() == ErrorKind::NotFound => Ok(Policy::default()),
        Err(e) => Err(e),
    }
}

/// Save `policy` merged with the file's current content. Returns what was
/// saved.
pub(crate) fn save(path: &Path, policy: &Policy) -> Result<Policy> {
    if let Some(dir) = path.parent() {
        std::fs::create_dir_all(dir)?;
    }
    let _lock = Lock::take(&path.with_extension("toml.lock"))?;
    let merged = read(path)?.union(policy);
    static N: AtomicU64 = AtomicU64::new(0);
    let tmp = path.with_extension(format!(
        "toml.{}.{}.tmp",
        std::process::id(),
        N.fetch_add(1, Ordering::Relaxed)
    ));
    std::fs::write(&tmp, merged.to_toml())?;
    std::fs::rename(&tmp, path).inspect_err(|_| drop(std::fs::remove_file(&tmp)))?;
    Ok(merged)
}

/// A lock file made with `create_new` (atomic on every platform); removed
/// on drop.
struct Lock(PathBuf);

impl Lock {
    fn take(path: &Path) -> Result<Self> {
        let start = Instant::now();
        loop {
            match std::fs::OpenOptions::new()
                .write(true)
                .create_new(true)
                .open(path)
            {
                Ok(_) => return Ok(Self(path.to_owned())),
                Err(e) if e.kind() == ErrorKind::AlreadyExists => {}
                Err(e) => return Err(e),
            }
            if is_stale(path) {
                // Another waiter may remove it first; either way, retry.
                drop(std::fs::remove_file(path));
                continue;
            }
            if start.elapsed() > WAIT {
                return Err(Error::new(
                    ErrorKind::TimedOut,
                    "the trusted sites file is locked",
                ));
            }
            std::thread::sleep(POLL);
        }
    }
}

impl Drop for Lock {
    fn drop(&mut self) {
        drop(std::fs::remove_file(&self.0));
    }
}

fn is_stale(path: &Path) -> bool {
    std::fs::metadata(path)
        .and_then(|m| m.modified())
        .ok()
        .and_then(|t| t.elapsed().ok())
        .is_some_and(|age| age > STALE)
}

#[cfg(test)]
mod tests {
    use super::*;

    fn dir(name: &str) -> PathBuf {
        let d = std::env::temp_dir().join(format!("rt_trust_{name}_{}", std::process::id()));
        drop(std::fs::remove_dir_all(&d));
        d
    }

    fn one(origin: &str) -> Policy {
        Policy::new([origin.to_owned()])
    }

    #[test]
    fn a_missing_file_is_empty_and_a_save_creates_it() {
        let path = dir("new").join("sub").join("trusted_origins.toml");
        assert_eq!(read(&path).unwrap(), Policy::default());
        let saved = save(&path, &one("https://a.com")).unwrap();
        assert_eq!(read(&path).unwrap(), saved);
        assert!(
            !path.with_extension("toml.lock").exists(),
            "the lock is gone"
        );
    }

    #[test]
    fn overlapping_saves_keep_every_addition() {
        let path = dir("race").join("trusted_origins.toml");
        let origins: Vec<String> = (0..24)
            .map(|i| format!("https://site{i}.example"))
            .collect();
        let handles: Vec<_> = origins
            .iter()
            .map(|o| {
                let (path, o) = (path.clone(), o.clone());
                std::thread::spawn(move || save(&path, &one(&o)).expect("saved"))
            })
            .collect();
        for h in handles {
            h.join().expect("no panic");
        }
        let got = read(&path).unwrap();
        for o in &origins {
            assert!(got.origins().any(|g| g == o), "{o} survived");
        }
        let leftovers = std::fs::read_dir(path.parent().unwrap())
            .unwrap()
            .filter(|e| e.as_ref().unwrap().file_name() != "trusted_origins.toml")
            .count();
        assert_eq!(leftovers, 0, "no temp or lock file left behind");
    }

    #[test]
    fn an_unreadable_file_is_never_overwritten() {
        let path = dir("unreadable").join("trusted_origins.toml");
        // A directory where the file should be: reading it fails, not NotFound.
        std::fs::create_dir_all(&path).unwrap();
        assert!(read(&path).is_err());
        assert!(save(&path, &one("https://a.com")).is_err());
        assert!(path.is_dir(), "left as it was");
    }

    #[test]
    fn a_crashed_writers_old_lock_is_cleared_but_a_fresh_one_waits() {
        let path = dir("stale").join("trusted_origins.toml");
        std::fs::create_dir_all(path.parent().unwrap()).unwrap();
        let lock = path.with_extension("toml.lock");
        std::fs::write(&lock, b"").unwrap();
        let old = std::time::SystemTime::now() - Duration::from_secs(120);
        std::fs::File::options()
            .write(true)
            .open(&lock)
            .unwrap()
            .set_modified(old)
            .unwrap();
        assert!(
            save(&path, &one("https://a.com")).is_ok(),
            "stale lock cleared"
        );
        // While one holder has the lock, nobody else can create it.
        let held = Lock::take(&lock).unwrap();
        assert!(
            std::fs::OpenOptions::new()
                .write(true)
                .create_new(true)
                .open(&lock)
                .is_err()
        );
        drop(held);
        assert!(!lock.exists());
    }
}
