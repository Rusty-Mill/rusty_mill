//! `CSN-FR-001`–`005` (`ADR-0136`, protocol 36): chunked snapshots.
//!
//! `BeginSnapshot` copies a table's files into a staging directory under the
//! table's write lock (the same lock `Backup` takes) and releases it; the
//! staged copy is then served in bounded chunks by handle, so the lock is held
//! for a local copy and never for a transfer, and neither end buffers more than
//! one chunk. One staged snapshot per table; it is freed by `EndSnapshot`, by
//! its connection closing, when it has sat idle past the TTL and another
//! `BeginSnapshot` needs the slot, and by the sweep at startup (a crash leaves
//! directories behind).

use super::protocol::{ErrorCode, MAX_CHUNK_BYTES};
use sha2::{Digest, Sha256};
use std::collections::hash_map::RandomState;
use std::collections::HashMap;
use std::hash::{BuildHasher, Hasher};
use std::io::{self, Read, Seek, SeekFrom};
use std::path::{Path, PathBuf};
use std::sync::Mutex;
use std::time::{Duration, Instant};

/// The copy a store makes under its own lock: into `dir`, refusing a table
/// over the byte ceiling, answering the change-log position it agrees with
/// (`ConnectionStore::stage_snapshot`).
pub(crate) type Stage<'a> = dyn Fn(&Path, u64) -> Result<Option<(u64, u64)>, ErrorCode> + 'a;

/// A staged file as the manifest names it: file name, length, SHA-256.
pub(crate) type ManifestFile = (String, u64, [u8; 32]);

/// A finished copy: the change-log position it agrees with and its files.
type Staged = (Option<(u64, u64)>, Vec<ManifestFile>);

/// What `BeginSnapshot` answers with.
pub(crate) struct Begun {
    pub(crate) handle: u64,
    pub(crate) position: Option<(u64, u64)>,
    pub(crate) files: Vec<ManifestFile>,
}

struct Held {
    handle: u64,
    touched: Instant,
    /// Name and length per staged file, in manifest order; empty while the
    /// copy is still being made (the handle has not been handed out yet).
    files: Vec<(String, u64)>,
}

/// The staging area: where copies go and which table holds one.
pub(crate) struct SnapshotStaging {
    root: PathBuf,
    max_bytes: u64,
    ttl: Duration,
    held: Mutex<HashMap<usize, Held>>,
}

/// Every staging directory name starts with this, so the startup sweep never
/// touches anything else the operator keeps in the directory.
const DIR_PREFIX: &str = "snap-";

impl SnapshotStaging {
    /// Create `root` if needed and remove the staging directories a crashed
    /// server left in it.
    pub(crate) fn new(root: PathBuf, max_bytes: u64, ttl: Duration) -> io::Result<Self> {
        std::fs::create_dir_all(&root)?;
        for entry in std::fs::read_dir(&root)? {
            let entry = entry?;
            if entry.file_name().to_string_lossy().starts_with(DIR_PREFIX) {
                std::fs::remove_dir_all(entry.path())?;
            }
        }
        Ok(Self {
            root,
            max_bytes,
            ttl,
            held: Mutex::new(HashMap::new()),
        })
    }

    fn dir(&self, table: usize) -> PathBuf {
        self.root.join(format!("{DIR_PREFIX}{table}"))
    }

    fn lock(&self) -> std::sync::MutexGuard<'_, HashMap<usize, Held>> {
        self.held.lock().unwrap_or_else(|e| e.into_inner())
    }

    /// Stage `table`'s files and describe them. `Busy` while the table's slot
    /// is held and not yet idle past the TTL.
    pub(crate) fn begin(&self, table: usize, stage: &Stage) -> Result<Begun, ErrorCode> {
        let dir = self.dir(table);
        let handle = random_handle();
        {
            let mut held = self.lock();
            if held
                .get(&table)
                .is_some_and(|h| h.touched.elapsed() < self.ttl)
            {
                return Err(ErrorCode::Busy);
            }
            held.insert(
                table,
                Held {
                    handle,
                    touched: Instant::now(),
                    files: Vec::new(),
                },
            );
        }
        let staged = self.stage(stage, &dir);
        match staged {
            Ok((position, files)) => {
                let listing = files.iter().map(|(n, len, _)| (n.clone(), *len)).collect();
                if let Some(h) = self.lock().get_mut(&table).filter(|h| h.handle == handle) {
                    h.files = listing;
                    h.touched = Instant::now();
                }
                Ok(Begun {
                    handle,
                    position,
                    files,
                })
            }
            Err(code) => {
                self.free(table, handle);
                Err(code)
            }
        }
    }

    /// The copy, under the table's lock, then the hashing, outside it.
    fn stage(&self, stage: &Stage, dir: &Path) -> Result<Staged, ErrorCode> {
        let _ = std::fs::remove_dir_all(dir);
        let position = stage(dir, self.max_bytes)?;
        let files = manifest(dir).map_err(|_| ErrorCode::Storage)?;
        Ok((position, files))
    }

    /// Up to `len` bytes of staged file `file` from `offset`: short at the
    /// end, empty past it. `NoSnapshot` for a handle that is not the table's
    /// live one; `Malformed` for a `file` out of range or a `len` over the cap.
    pub(crate) fn chunk(
        &self,
        table: usize,
        handle: u64,
        file: u32,
        offset: u64,
        len: u32,
    ) -> Result<Vec<u8>, ErrorCode> {
        if len > MAX_CHUNK_BYTES {
            return Err(ErrorCode::Malformed);
        }
        let (name, file_len) = {
            let mut held = self.lock();
            let live = self.live(&mut held, table, handle)?;
            live.touched = Instant::now();
            live.files
                .get(file as usize)
                .cloned()
                .ok_or(ErrorCode::Malformed)?
        };
        let take = u64::from(len).min(file_len.saturating_sub(offset));
        read_range(&self.dir(table).join(name), offset, take).map_err(|_| ErrorCode::Storage)
    }

    /// Free `table`'s staged copy. `NoSnapshot` unless `handle` is its live one.
    pub(crate) fn end(&self, table: usize, handle: u64) -> Result<(), ErrorCode> {
        self.live(&mut self.lock(), table, handle)?;
        self.free(table, handle);
        Ok(())
    }

    /// `table`'s entry if `handle` is its own and it has not idled out (an
    /// idled-out one is freed here).
    fn live<'a>(
        &self,
        held: &'a mut HashMap<usize, Held>,
        table: usize,
        handle: u64,
    ) -> Result<&'a mut Held, ErrorCode> {
        let expired = match held.get(&table) {
            Some(h) if h.handle == handle => h.touched.elapsed() >= self.ttl,
            _ => return Err(ErrorCode::NoSnapshot),
        };
        if expired {
            held.remove(&table);
            let _ = std::fs::remove_dir_all(self.dir(table));
            return Err(ErrorCode::NoSnapshot);
        }
        Ok(held.get_mut(&table).expect("checked above"))
    }

    /// Drop `table`'s entry if it is `handle`'s, and its directory. Also the
    /// connection-close path, so it never fails.
    pub(crate) fn free(&self, table: usize, handle: u64) {
        let mut held = self.lock();
        if held.get(&table).is_some_and(|h| h.handle == handle) {
            held.remove(&table);
            let _ = std::fs::remove_dir_all(self.dir(table));
        }
    }
}

/// A random 64-bit handle: unguessable enough for a token that is also bound
/// to one connection's memory.
fn random_handle() -> u64 {
    RandomState::new().build_hasher().finish()
}

/// The files of `dir` by name, each with its length and SHA-256.
fn manifest(dir: &Path) -> io::Result<Vec<ManifestFile>> {
    let mut names = Vec::new();
    for entry in std::fs::read_dir(dir)? {
        names.push(entry?.file_name().to_string_lossy().into_owned());
    }
    names.sort();
    names
        .into_iter()
        .map(|name| {
            let (len, digest) = sha256_file(&dir.join(&name))?;
            Ok((name, len, digest))
        })
        .collect()
}

/// A file's length and SHA-256, read in bounded pieces.
fn sha256_file(path: &Path) -> io::Result<(u64, [u8; 32])> {
    let mut file = std::fs::File::open(path)?;
    let mut hasher = Sha256::new();
    let mut buf = vec![0u8; 1 << 20];
    let mut len = 0u64;
    loop {
        let n = file.read(&mut buf)?;
        if n == 0 {
            return Ok((len, hasher.finalize().into()));
        }
        hasher.update(&buf[..n]);
        len += n as u64;
    }
}

fn read_range(path: &Path, offset: u64, len: u64) -> io::Result<Vec<u8>> {
    let mut file = std::fs::File::open(path)?;
    file.seek(SeekFrom::Start(offset))?;
    let mut bytes = Vec::new();
    file.take(len).read_to_end(&mut bytes)?;
    Ok(bytes)
}

#[cfg(test)]
mod tests {
    use super::*;

    /// A "copy" that writes the given files and answers a position.
    struct Fake {
        files: Vec<(&'static str, Vec<u8>)>,
        position: Option<(u64, u64)>,
        refuse: Option<ErrorCode>,
    }

    impl Fake {
        fn stage(&self, dir: &Path, _max_bytes: u64) -> Result<Option<(u64, u64)>, ErrorCode> {
            if let Some(code) = self.refuse {
                return Err(code);
            }
            std::fs::create_dir_all(dir).map_err(|_| ErrorCode::Storage)?;
            for (name, bytes) in &self.files {
                std::fs::write(dir.join(name), bytes).map_err(|_| ErrorCode::Storage)?;
            }
            Ok(self.position)
        }

        fn begin(&self, s: &SnapshotStaging, table: usize) -> Result<Begun, ErrorCode> {
            s.begin(table, &|dir, max| self.stage(dir, max))
        }
    }

    fn fake(position: Option<(u64, u64)>) -> Fake {
        Fake {
            files: vec![("t.b", b"world".to_vec()), ("t.a", b"hello".to_vec())],
            position,
            refuse: None,
        }
    }

    fn scratch() -> PathBuf {
        use std::sync::atomic::{AtomicU64, Ordering};
        static COUNTER: AtomicU64 = AtomicU64::new(0);
        let n = COUNTER.fetch_add(1, Ordering::Relaxed);
        std::env::temp_dir().join(format!("snapshot_unit_{}_{n}", std::process::id()))
    }

    /// A staging area under a fresh scratch directory (returned as its root).
    fn staging(ttl: Duration) -> (PathBuf, SnapshotStaging) {
        let tmp = scratch();
        let s = SnapshotStaging::new(tmp.join("stage"), 1 << 20, ttl).unwrap();
        (tmp, s)
    }

    #[test]
    fn a_staged_copy_is_listed_sorted_hashed_and_read_in_ranges() {
        let (_tmp, s) = staging(Duration::from_secs(60));
        let b = fake(Some((2, 9))).begin(&s, 0).unwrap();
        assert_eq!(b.position, Some((2, 9)));
        let names: Vec<_> = b.files.iter().map(|f| (f.0.as_str(), f.1)).collect();
        assert_eq!(names, [("t.a", 5), ("t.b", 5)]);
        assert_eq!(b.files[0].2, <[u8; 32]>::from(Sha256::digest(b"hello")));
        assert_eq!(s.chunk(0, b.handle, 0, 1, 3).unwrap(), b"ell");
        assert_eq!(
            s.chunk(0, b.handle, 1, 3, 100).unwrap(),
            b"ld",
            "short at the end"
        );
        assert!(
            s.chunk(0, b.handle, 1, 5, 4).unwrap().is_empty(),
            "empty at the end"
        );
        assert_eq!(s.chunk(0, b.handle, 2, 0, 1), Err(ErrorCode::Malformed));
        assert_eq!(
            s.chunk(0, b.handle, 0, 0, MAX_CHUNK_BYTES + 1),
            Err(ErrorCode::Malformed)
        );
    }

    #[test]
    fn a_wrong_or_ended_handle_is_no_snapshot_and_end_frees_the_copy() {
        let (tmp, s) = staging(Duration::from_secs(60));
        let b = fake(None).begin(&s, 0).unwrap();
        assert_eq!(b.position, None);
        assert_eq!(
            s.chunk(0, b.handle ^ 1, 0, 0, 1),
            Err(ErrorCode::NoSnapshot)
        );
        assert_eq!(s.chunk(1, b.handle, 0, 0, 1), Err(ErrorCode::NoSnapshot));
        assert!(tmp.join("stage/snap-0").exists());
        s.end(0, b.handle).unwrap();
        assert!(!tmp.join("stage/snap-0").exists());
        assert_eq!(s.end(0, b.handle), Err(ErrorCode::NoSnapshot));
        assert_eq!(s.chunk(0, b.handle, 0, 0, 1), Err(ErrorCode::NoSnapshot));
    }

    #[test]
    fn a_held_table_is_busy_until_the_ttl_then_the_old_handle_is_dead() {
        let (_tmp, s) = staging(Duration::from_millis(150));
        let first = fake(None).begin(&s, 0).unwrap();
        assert_eq!(fake(None).begin(&s, 0).err(), Some(ErrorCode::Busy));
        assert!(fake(None).begin(&s, 1).is_ok(), "another table is free");
        std::thread::sleep(Duration::from_millis(200));
        let second = fake(None).begin(&s, 0).unwrap();
        assert_eq!(
            s.chunk(0, first.handle, 0, 0, 1),
            Err(ErrorCode::NoSnapshot)
        );
        assert_eq!(s.chunk(0, second.handle, 0, 0, 1).unwrap(), b"h");
    }

    #[test]
    fn a_failed_copy_leaves_nothing_and_frees_the_slot() {
        let (tmp, s) = staging(Duration::from_secs(60));
        for code in [
            ErrorCode::TooLarge,
            ErrorCode::Unsupported,
            ErrorCode::Storage,
        ] {
            let mut f = fake(None);
            f.refuse = Some(code);
            assert_eq!(f.begin(&s, 0).err(), Some(code));
        }
        assert!(!tmp.join("stage/snap-0").exists());
        assert!(fake(None).begin(&s, 0).is_ok());
    }

    #[test]
    fn startup_sweeps_leftovers_and_only_ours() {
        let root = scratch().join("stage");
        std::fs::create_dir_all(root.join("snap-3")).unwrap();
        std::fs::write(root.join("snap-3/x"), b"x").unwrap();
        std::fs::write(root.join("keep.txt"), b"k").unwrap();
        SnapshotStaging::new(root.clone(), 1, Duration::from_secs(1)).unwrap();
        assert!(!root.join("snap-3").exists());
        assert!(root.join("keep.txt").exists());
    }
}
