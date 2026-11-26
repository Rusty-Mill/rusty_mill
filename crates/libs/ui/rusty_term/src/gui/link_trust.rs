//! The trusted origins on disk (C63, gui-design D72): one small file per
//! origin in a directory beside `config.toml`.
//!
//! A file's name is the hash of the origin it holds, and it is published
//! whole: written under a dot-name no reader looks at, then renamed into
//! place. So a reader never sees a half-written record, a crash leaves only
//! an ignored temp file, and two windows or processes adding the same origin
//! write identical bytes to the same name (one file, however many race).
//! There is no read-merge-write, no lock and nothing to wait on. Reading
//! lists the directory and keeps only entries that are exactly a normalised
//! http(s) origin *and* match their file name's hash, so a truncated or
//! hand-made file can never trust another origin. Removing a site is
//! deleting its file.
//!
//! Forwarded ports (C81, D341) are kept the same way in a second directory,
//! one `<host>:<port>` entry per file, checked by their own validator, so a
//! forward entry can never trust a web origin nor the other way round.

use super::link_policy::{Policy, valid_origin};
use std::io::{Error, ErrorKind, Result};
use std::path::{Path, PathBuf};
use std::sync::atomic::{AtomicU64, Ordering};

/// What one directory holds: the file name's extension and the check an
/// entry must pass, on load and before it is added.
struct Kind {
    ext: &'static str,
    valid: fn(&str) -> Option<&str>,
}

const ORIGINS: Kind = Kind {
    ext: EXT,
    valid: valid_origin,
};
const FORWARDS: Kind = Kind {
    ext: "forward",
    valid: valid_forward,
};

/// The origin files' extension, and how many same-hash entries one hash may
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

/// The directory of trusted forwarded ports, beside the origins'.
pub(crate) fn forwards_dir() -> Option<PathBuf> {
    Some(
        rusty_dirs::config_root()?
            .join("rusty_term")
            .join("trusted_forwards"),
    )
}

/// `host` and `port` as one forward entry: `host` is the source's opaque
/// name for the pane's host, so it must be a single line.
pub(crate) fn forward_entry(host: &str, port: u16) -> Option<String> {
    let entry = format!("{host}:{port}");
    valid_forward(&entry).is_some().then_some(entry)
}

/// `s` if it is `<host>:<port>` as [`forward_entry`] writes it.
fn valid_forward(s: &str) -> Option<&str> {
    let (host, port) = s.rsplit_once(':')?;
    let port: u16 = port.parse().ok().filter(|p| *p != 0)?;
    let plain = !host.is_empty() && host.len() <= 1024 && !host.chars().any(char::is_control);
    (plain && s.ends_with(&format!(":{port}"))).then_some(s)
}

/// The saved forwards, as entries; see [`read`].
pub(crate) fn read_forwards(dir: &Path) -> Result<Vec<String>> {
    read_kind(dir, &FORWARDS)
}

/// Trust forward `entry` (from [`forward_entry`]); see [`add`].
pub(crate) fn add_forward(dir: &Path, entry: &str) -> Result<()> {
    add_by(dir, &FORWARDS, entry, name_hash)
}

/// The saved origins. A missing directory is an empty list; anything else
/// that cannot be read is an error. Entries that are not a valid origin
/// (damaged, hand-made, half-written) are skipped, so the directory can only
/// ever trust fewer things than it holds.
pub(crate) fn read(dir: &Path) -> Result<Policy> {
    read_kind(dir, &ORIGINS).map(Policy::new)
}

fn read_kind(dir: &Path, kind: &Kind) -> Result<Vec<String>> {
    let entries = match std::fs::read_dir(dir) {
        Ok(e) => e,
        Err(e) if e.kind() == ErrorKind::NotFound => return Ok(Vec::new()),
        Err(e) => return Err(e),
    };
    let mut origins = Vec::new();
    for entry in entries {
        let path = entry?.path();
        if path.extension().is_none_or(|e| e != kind.ext) {
            continue;
        }
        let Ok(meta) = std::fs::metadata(&path) else {
            continue;
        };
        if !meta.is_file() || meta.len() > MAX_FILE {
            continue;
        }
        let stem = path.file_stem().and_then(|s| s.to_str()).unwrap_or("");
        let named = stem.split('-').next().unwrap_or("");
        if let Some(o) = std::fs::read_to_string(&path)
            .ok()
            .as_deref()
            .and_then(kind.valid)
            .filter(|o| name_hash(o) == named)
        {
            origins.push(o.to_owned());
        }
    }
    Ok(origins)
}

/// Trust `origin` (already checked with [`valid_origin`]). Idempotent and
/// safe to run at once from any number of windows and processes.
pub(crate) fn add(dir: &Path, origin: &str) -> Result<()> {
    add_by(dir, &ORIGINS, origin, name_hash)
}

/// [`add`] with the naming hash injectable, so tests can force collisions.
fn add_by(dir: &Path, kind: &Kind, origin: &str, hash: fn(&str) -> String) -> Result<()> {
    if (kind.valid)(origin).is_none() {
        return Err(Error::new(ErrorKind::InvalidInput, "not a valid entry"));
    }
    std::fs::create_dir_all(dir)?;
    let hash = hash(origin);
    for slot in 0..SLOTS {
        let name = match slot {
            0 => format!("{hash}.{}", kind.ext),
            n => format!("{hash}-{n}.{}", kind.ext),
        };
        let path = dir.join(name);
        match std::fs::read_to_string(&path) {
            Ok(held) if held == origin => return Ok(()),
            // Another origin with the same hash (or a damaged file): next slot.
            Ok(_) => continue,
            Err(e) if e.kind() != ErrorKind::NotFound => {
                // Damaged (not UTF-8) or not a file: leave it, use the next slot.
                if slot + 1 == SLOTS {
                    return Err(e);
                }
            }
            Err(_) => {
                if publish(dir, &hash, &path, origin)? {
                    return Ok(());
                }
                // Lost the slot: fine if to the same origin, else try the next.
                if std::fs::read_to_string(&path).is_ok_and(|held| held == origin) {
                    return Ok(());
                }
            }
        }
    }
    Err(Error::other("too many origins share a hash"))
}

/// Write `origin` whole to `path` without replacing anything: to a temp
/// name first, then a hard link (which fails if `path` exists). `Ok(false)`
/// means `path` was taken. Where links are unsupported, falls back to rename,
/// which can replace an entry only if two origins with one 64-bit hash race.
fn publish(dir: &Path, hash: &str, path: &Path, origin: &str) -> Result<bool> {
    static N: AtomicU64 = AtomicU64::new(0);
    let tmp = dir.join(format!(
        ".{hash}.{}.{}.tmp",
        std::process::id(),
        N.fetch_add(1, Ordering::Relaxed)
    ));
    std::fs::write(&tmp, origin).inspect_err(|_| drop(std::fs::remove_file(&tmp)))?;
    let done = match std::fs::hard_link(&tmp, path) {
        Ok(()) => Ok(true),
        Err(e) if e.kind() == ErrorKind::AlreadyExists => Ok(false),
        // Links unsupported here: rename, unless something has appeared.
        Err(_) if path.exists() => Ok(false),
        Err(_) => std::fs::rename(&tmp, path).map(|()| true),
    };
    drop(std::fs::remove_file(&tmp));
    done
}

/// The hash `origin`'s file is named by, in hex.
fn name_hash(origin: &str) -> String {
    format!("{:016x}", fnv1a(origin.as_bytes()))
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
        static N: AtomicU64 = AtomicU64::new(0);
        let n = N.fetch_add(1, Ordering::Relaxed);
        let d = std::env::temp_dir().join(format!("rt_trust_{name}_{}_{n}", std::process::id()));
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
    fn a_race_to_add_one_origin_leaves_one_file_that_revoking_clears() {
        let d = dir("race");
        std::fs::create_dir_all(&d).unwrap();
        let start = std::sync::Arc::new(std::sync::Barrier::new(16));
        let handles: Vec<_> = (0..16)
            .map(|_| {
                let (d, start) = (d.clone(), start.clone());
                std::thread::spawn(move || {
                    start.wait();
                    add(&d, "https://same.example")
                })
            })
            .collect();
        for h in handles {
            h.join().expect("no panic").expect("added");
        }
        let files: Vec<_> = std::fs::read_dir(&d)
            .unwrap()
            .map(|e| e.unwrap().path())
            .collect();
        assert_eq!(
            files.len(),
            1,
            "one file, no duplicates and no temp left: {files:?}"
        );
        assert_eq!(origins(&read(&d).unwrap()), ["https://same.example"]);
        std::fs::remove_file(&files[0]).unwrap();
        assert_eq!(read(&d).unwrap(), Policy::default(), "revoked for good");
    }

    #[test]
    fn overlapping_adds_of_different_origins_keep_every_one() {
        let d = dir("many");
        let handles: Vec<_> = (0..24)
            .map(|i| {
                let d = d.clone();
                std::thread::spawn(move || add(&d, &format!("https://site{i}.example")))
            })
            .collect();
        for h in handles {
            h.join().expect("no panic").expect("added");
        }
        assert_eq!(read(&d).unwrap().origins().count(), 24);
        assert_eq!(std::fs::read_dir(&d).unwrap().count(), 24);
    }

    #[test]
    fn a_truncated_record_never_trusts_another_origin() {
        let d = dir("prefix");
        std::fs::create_dir_all(&d).unwrap();
        // A write cut short: `https://example.co` is itself a valid origin.
        let name = format!("{}.{EXT}", name_hash("https://example.com"));
        std::fs::write(d.join(&name), "https://example.co").unwrap();
        assert_eq!(
            read(&d).unwrap(),
            Policy::default(),
            "the prefix is not trusted"
        );
        // Adding the real origin works around the bad file, not through it.
        add(&d, "https://example.com").unwrap();
        assert_eq!(origins(&read(&d).unwrap()), ["https://example.com"]);
    }

    #[test]
    fn an_unpublished_temp_file_is_ignored() {
        let d = dir("temp");
        std::fs::create_dir_all(&d).unwrap();
        std::fs::write(d.join(".abc.1.0.tmp"), "https://tmp.example").unwrap();
        std::fs::write(
            d.join(format!("{}.{EXT}.tmp", name_hash("https://t.example"))),
            "https://t.example",
        )
        .unwrap();
        assert_eq!(read(&d).unwrap(), Policy::default());
    }

    #[test]
    fn racing_origins_with_one_hash_all_survive() {
        let d = dir("collide");
        let origins: Vec<String> = (0..6).map(|i| format!("https://h{i}.test")).collect();
        let gate = std::sync::Barrier::new(origins.len());
        std::thread::scope(|s| {
            for o in &origins {
                let (d, gate) = (&d, &gate);
                s.spawn(move || {
                    gate.wait();
                    add_by(d, &ORIGINS, o, |_| "0".repeat(16)).unwrap();
                });
            }
        });
        // The injected hash is not the real one, so `read` would skip them: count files.
        let files = std::fs::read_dir(&d)
            .unwrap()
            .filter_map(|e| e.ok())
            .filter(|e| !e.file_name().to_string_lossy().starts_with('.'))
            .count();
        assert_eq!(files, origins.len());
    }

    #[test]
    fn a_damaged_or_directory_slot_is_skipped() {
        let d = dir("damaged_slot");
        std::fs::create_dir_all(&d).unwrap();
        let h = name_hash("https://a.test");
        std::fs::write(d.join(format!("{h}.origin")), [0xff, 0xfe]).unwrap();
        std::fs::create_dir(d.join(format!("{h}-1.origin"))).unwrap();
        add(&d, "https://a.test").unwrap();
        assert!(d.join(format!("{h}-2.origin")).is_file());
    }

    #[test]
    fn a_same_hash_origin_takes_the_next_slot() {
        let d = dir("collide");
        std::fs::create_dir_all(&d).unwrap();
        let hash = name_hash("https://a.com");
        std::fs::write(d.join(format!("{hash}.{EXT}")), "https://other.com").unwrap();
        add(&d, "https://a.com").unwrap();
        // The planted file is bound to another hash, so only ours is trusted.
        assert_eq!(origins(&read(&d).unwrap()), ["https://a.com"]);
        assert!(d.join(format!("{hash}-1.{EXT}")).exists());
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
            ("f.origin", "https://valid-but-unbound.com"),
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

    #[test]
    fn forwards_are_kept_apart_from_origins_and_checked_on_load() {
        let d = dir("forwards");
        let entry = forward_entry("/run/user/1000/mux.sock", 5173).unwrap();
        add_forward(&d, &entry).unwrap();
        add_forward(&d, &entry).unwrap();
        assert_eq!(std::fs::read_dir(&d).unwrap().count(), 1);
        assert_eq!(read_forwards(&d).unwrap(), std::slice::from_ref(&entry));
        // Neither kind reads the other's files, even in one directory.
        assert_eq!(read(&d).unwrap(), Policy::default());
        add(&d, "https://a.com").unwrap();
        assert_eq!(read_forwards(&d).unwrap(), std::slice::from_ref(&entry));
        // Unbound, damaged and hand-made files trust nothing.
        std::fs::write(d.join("x.forward"), &entry).unwrap();
        std::fs::write(d.join(format!("{}.forward", name_hash("box:80"))), "box:81").unwrap();
        assert_eq!(read_forwards(&d).unwrap(), [entry]);
    }

    #[test]
    fn a_forward_entry_is_one_line_host_and_a_real_port() {
        assert_eq!(forward_entry("box", 80).as_deref(), Some("box:80"));
        assert_eq!(forward_entry("a:b", 80).as_deref(), Some("a:b:80"));
        assert_eq!(forward_entry("", 80), None);
        assert_eq!(forward_entry("box", 0), None);
        assert_eq!(forward_entry("bo\nx", 80), None);
        for bad in [
            "box",
            "box:",
            "box:0",
            "box:070",
            "box:65536",
            "box:+80",
            ":80",
        ] {
            assert_eq!(valid_forward(bad), None, "{bad}");
        }
        assert!(add_forward(&dir("badfwd"), "box:0").is_err());
    }
}
