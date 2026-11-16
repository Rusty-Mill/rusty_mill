//! The trusted origins on disk (C63, gui-design D72): one small file per
//! origin in a directory beside `config.toml`.
//!
//! Trusting is one `create_new` of a file named from the origin's hash: no
//! read-merge-write, so two windows or processes never lose each other's
//! additions; no lock, so there is nothing to go stale or to wait on; no
//! temp file to share. Reading lists the directory and keeps only entries
//! that are exactly a normalised http(s) origin. Removing a site is deleting
//! its file.

use super::link_policy::{Policy, valid_origin};
use std::io::{Error, ErrorKind, Result, Write};
use std::path::{Path, PathBuf};

/// The file name's extension, and how many same-hash origins one hash may
/// hold (a 64-bit collision, in practice never).
const EXT: &str = "origin";
const SLOTS: u32 = 8;
/// An origin file longer than this is not one of ours.
const MAX_FILE: u64 = 4096;

/// The directory: beside `config.toml`, so a run-time write never touches
/// the user's hand-edited config.
pub(crate) fn default_dir() -> Option<PathBuf> {
    Some(
        rusty_dirs::config_root()?
            .join("rusty_term")
            .join("trusted_origins"),
    )
}

/// The saved origins. A missing directory is an empty list; anything else
/// that cannot be read is an error. Entries that are not a valid origin
/// (damaged, hand-made, half-written) are skipped, so the directory can only
/// ever trust fewer things than it holds.
pub(crate) fn read(dir: &Path) -> Result<Policy> {
    let entries = match std::fs::read_dir(dir) {
        Ok(e) => e,
        Err(e) if e.kind() == ErrorKind::NotFound => return Ok(Policy::default()),
        Err(e) => return Err(e),
    };
    let mut origins = Vec::new();
    for entry in entries {
        let path = entry?.path();
        if path.extension().is_none_or(|e| e != EXT) {
            continue;
        }
        let Ok(meta) = std::fs::metadata(&path) else {
            continue;
        };
        if !meta.is_file() || meta.len() > MAX_FILE {
            continue;
        }
        if let Some(o) = std::fs::read_to_string(&path)
            .ok()
            .as_deref()
            .and_then(valid_origin)
        {
            origins.push(o.to_owned());
        }
    }
    Ok(Policy::new(origins))
}

/// Trust `origin` (already checked with [`valid_origin`]). Idempotent and
/// safe to run at once from any number of windows and processes.
pub(crate) fn add(dir: &Path, origin: &str) -> Result<()> {
    if valid_origin(origin).is_none() {
        return Err(Error::new(ErrorKind::InvalidInput, "not a web origin"));
    }
    std::fs::create_dir_all(dir)?;
    let hash = fnv1a(origin.as_bytes());
    for slot in 0..SLOTS {
        let name = match slot {
            0 => format!("{hash:016x}.{EXT}"),
            n => format!("{hash:016x}-{n}.{EXT}"),
        };
        let path = dir.join(name);
        match std::fs::OpenOptions::new()
            .write(true)
            .create_new(true)
            .open(&path)
        {
            Ok(mut f) => {
                return f
                    .write_all(origin.as_bytes())
                    .inspect_err(|_| drop(std::fs::remove_file(&path)));
            }
            Err(e) if e.kind() == ErrorKind::AlreadyExists => {
                // Ours already (or a hash collision: try the next slot).
                if std::fs::read_to_string(&path).is_ok_and(|t| t == origin) {
                    return Ok(());
                }
            }
            Err(e) => return Err(e),
        }
    }
    Err(Error::other("too many origins share a hash"))
}

/// FNV-1a, 64 bit: a stable file name, not a security boundary (the file's
/// content is what is trusted, and it is checked on load).
fn fnv1a(bytes: &[u8]) -> u64 {
    bytes.iter().fold(0xcbf2_9ce4_8422_2325, |h, &b| {
        (h ^ u64::from(b)).wrapping_mul(0x0100_0000_01b3)
    })
}

#[cfg(test)]
mod tests {
    use super::*;

    fn dir(name: &str) -> PathBuf {
        let d = std::env::temp_dir().join(format!("rt_trust_{name}_{}", std::process::id()));
        drop(std::fs::remove_dir_all(&d));
        d
    }

    fn origins(p: &Policy) -> Vec<&str> {
        p.origins().collect()
    }

    #[test]
    fn a_missing_directory_is_empty_and_adding_creates_it() {
        let d = dir("new").join("sub");
        assert_eq!(read(&d).unwrap(), Policy::default());
        add(&d, "https://a.com").unwrap();
        add(&d, "http://b.com:8080").unwrap();
        assert_eq!(
            origins(&read(&d).unwrap()),
            ["http://b.com:8080", "https://a.com"]
        );
    }

    #[test]
    fn adding_twice_makes_one_entry() {
        let d = dir("twice");
        add(&d, "https://a.com").unwrap();
        add(&d, "https://a.com").unwrap();
        assert_eq!(std::fs::read_dir(&d).unwrap().count(), 1);
    }

    #[test]
    fn overlapping_adds_from_many_threads_keep_every_origin() {
        let d = dir("race");
        let handles: Vec<_> = (0..48)
            .map(|i| {
                let d = d.clone();
                // Two threads per origin: the same one added at once.
                std::thread::spawn(move || add(&d, &format!("https://site{}.example", i / 2)))
            })
            .collect();
        for h in handles {
            h.join().expect("no panic").expect("added");
        }
        let got = read(&d).unwrap();
        assert_eq!(got.origins().count(), 24);
        for i in 0..24 {
            assert!(
                got.origins()
                    .any(|g| g == format!("https://site{i}.example"))
            );
        }
    }

    #[test]
    fn a_same_hash_origin_takes_the_next_slot() {
        let d = dir("collide");
        std::fs::create_dir_all(&d).unwrap();
        let hash = fnv1a(b"https://a.com");
        std::fs::write(d.join(format!("{hash:016x}.{EXT}")), "https://other.com").unwrap();
        add(&d, "https://a.com").unwrap();
        assert_eq!(
            origins(&read(&d).unwrap()),
            ["https://a.com", "https://other.com"]
        );
    }

    #[test]
    fn damaged_and_foreign_entries_are_skipped() {
        let d = dir("damaged");
        add(&d, "https://ok.com").unwrap();
        for (name, body) in [
            ("a.origin", "https://x.com/path"),
            ("b.origin", "javascript:alert(1)"),
            ("c.origin", ""),
            ("d.origin", "https://Up.com"),
            ("e.txt", "https://txt.com"),
        ] {
            std::fs::write(d.join(name), body).unwrap();
        }
        std::fs::write(d.join("big.origin"), "x".repeat(10_000)).unwrap();
        std::fs::create_dir(d.join("dir.origin")).unwrap();
        assert_eq!(origins(&read(&d).unwrap()), ["https://ok.com"]);
    }

    #[test]
    fn only_a_valid_origin_is_added_and_a_bad_directory_errors() {
        let d = dir("bad");
        for o in ["https://a.com/path", "mailto:a@b.c", "", "file:///x"] {
            assert!(add(&d, o).is_err(), "{o}");
        }
        assert!(!d.exists(), "nothing was created");
        // A file where the directory should be: neither read nor add works.
        std::fs::create_dir_all(d.parent().unwrap()).unwrap();
        std::fs::write(&d, "").unwrap();
        assert!(read(&d).is_err());
        assert!(add(&d, "https://a.com").is_err());
        drop(std::fs::remove_file(&d));
    }
}
