//! Per-namespace session storage: the [`SessionStore`] port and its file-backed
//! adapter, [`FsSessionStore`].
//!
//! A *namespace* is an account, or a team's shared pool. Saves are idempotent by
//! content key, so re-uploading the same replay never duplicates history.
//!
//! [`FsSessionStore`] lays sessions out as `<root>/<namespace>/<key>.json`, one
//! [`SessionRecord`] per file, written atomically (temp file, rename). A second
//! adapter over the Rusty-Mill engine lives in `store_mmdb` behind the `mmdb`
//! feature; [`copy_all`] moves sessions between any two.

use std::fmt;
use std::fs;
use std::io;
use std::path::{Path, PathBuf};

use crate::history::SessionRecord;

/// A validated storage namespace — an account name, or a team name for a team's
/// shared pool: `[A-Za-z0-9_-]{1,32}`, not starting with `_` (reserved for the
/// app's own directories). Because it is validated at construction it is safe to
/// use as a directory name — no separators, no `..`.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct AccountId(String);

impl AccountId {
    pub fn new(name: &str) -> Result<Self, StoreError> {
        let ok = !name.is_empty()
            && !name.starts_with('_')
            && name.len() <= 32
            && name
                .bytes()
                .all(|b| b.is_ascii_alphanumeric() || b == b'_' || b == b'-');
        if ok {
            Ok(Self(name.to_string()))
        } else {
            Err(StoreError::InvalidAccount(name.to_string()))
        }
    }

    pub fn as_str(&self) -> &str {
        &self.0
    }
}

impl fmt::Display for AccountId {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        f.write_str(&self.0)
    }
}

#[derive(Debug)]
pub enum StoreError {
    InvalidAccount(String),
    Io(io::Error),
    Encode(serde_json::Error),
    /// A session key this backend cannot represent.
    InvalidKey(String),
    /// A failure reported by a non-filesystem backend.
    Backend(String),
}

impl fmt::Display for StoreError {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        match self {
            Self::InvalidAccount(n) => {
                write!(
                    f,
                    "invalid account name {n:?} (use 1-32 of A-Z a-z 0-9 _ -)"
                )
            }
            Self::Io(e) => write!(f, "session store I/O error: {e}"),
            Self::Encode(e) => write!(f, "session encode error: {e}"),
            Self::InvalidKey(k) => write!(f, "session key {k:?} is not valid for this store"),
            Self::Backend(m) => write!(f, "session store error: {m}"),
        }
    }
}

impl std::error::Error for StoreError {}

impl From<io::Error> for StoreError {
    fn from(e: io::Error) -> Self {
        Self::Io(e)
    }
}

/// Whether [`SessionStore::save`] wrote a new session or found it already stored.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum SaveOutcome {
    Saved,
    AlreadyStored,
}

/// Stable content key for a replay: FNV-1a 64 over the bytes plus the length,
/// as hex. This is a dedupe key, not a security boundary — an accidental
/// collision needs two different replays of equal length and equal FNV hash.
pub fn session_key(bytes: &[u8]) -> String {
    let mut hash: u64 = 0xcbf2_9ce4_8422_2325;
    for &b in bytes {
        hash ^= u64::from(b);
        hash = hash.wrapping_mul(0x0000_0100_0000_01b3);
    }
    format!("{hash:016x}{:08x}", bytes.len() as u32)
}

/// Where sessions live. Implementations must be safe to share across the
/// server's connection threads.
pub trait SessionStore: Send + Sync {
    /// Persist `record` in `ns`; a no-op if that key is already stored.
    fn save(&self, ns: &AccountId, record: &SessionRecord) -> Result<SaveOutcome, StoreError>;

    /// Every stored session in `ns`, oldest first. An unknown namespace is empty.
    fn list(&self, ns: &AccountId) -> Result<Vec<SessionRecord>, StoreError>;
}

/// What [`copy_all`] did.
#[derive(Debug, Clone, Copy, Default, PartialEq, Eq)]
pub struct CopyReport {
    pub copied: usize,
    pub already_present: usize,
}

/// Copy every session of every namespace in `namespaces` from `from` to `to`.
/// Idempotent: re-running reports the sessions as already present.
pub fn copy_all(
    from: &dyn SessionStore,
    to: &dyn SessionStore,
    namespaces: &[AccountId],
) -> Result<CopyReport, StoreError> {
    let mut report = CopyReport::default();
    for ns in namespaces {
        for record in from.list(ns)? {
            match to.save(ns, &record)? {
                SaveOutcome::Saved => report.copied += 1,
                SaveOutcome::AlreadyStored => report.already_present += 1,
            }
        }
    }
    Ok(report)
}

#[derive(Debug, Clone)]
pub struct FsSessionStore {
    root: PathBuf,
}

impl FsSessionStore {
    pub fn new(root: impl Into<PathBuf>) -> Self {
        Self { root: root.into() }
    }

    fn account_dir(&self, account: &AccountId) -> PathBuf {
        self.root.join(account.as_str())
    }

    /// Every valid namespace directory directly under the root. Directories that
    /// are not valid namespace names (such as the reserved `_teams`) are skipped.
    pub fn namespaces(&self) -> Result<Vec<AccountId>, StoreError> {
        let entries = match fs::read_dir(&self.root) {
            Ok(e) => e,
            Err(e) if e.kind() == io::ErrorKind::NotFound => return Ok(Vec::new()),
            Err(e) => return Err(e.into()),
        };
        let mut out: Vec<AccountId> = entries
            .filter_map(Result::ok)
            .filter(|e| e.path().is_dir())
            .filter_map(|e| AccountId::new(e.file_name().to_str()?).ok())
            .collect();
        out.sort_by(|a, b| a.as_str().cmp(b.as_str()));
        Ok(out)
    }
}

impl SessionStore for FsSessionStore {
    fn save(&self, account: &AccountId, record: &SessionRecord) -> Result<SaveOutcome, StoreError> {
        let dir = self.account_dir(account);
        fs::create_dir_all(&dir)?;
        let dest = dir.join(format!("{}.json", record.key));
        if dest.exists() {
            return Ok(SaveOutcome::AlreadyStored);
        }
        let json = serde_json::to_vec(record).map_err(StoreError::Encode)?;
        let tmp = dir.join(format!("{}.json.tmp", record.key));
        fs::write(&tmp, json)?;
        fs::rename(&tmp, &dest)?;
        Ok(SaveOutcome::Saved)
    }

    /// A file that cannot be read or parsed is skipped with a stderr warning
    /// rather than hiding the rest of the history.
    fn list(&self, account: &AccountId) -> Result<Vec<SessionRecord>, StoreError> {
        let dir = self.account_dir(account);
        let entries = match fs::read_dir(&dir) {
            Ok(e) => e,
            Err(e) if e.kind() == io::ErrorKind::NotFound => return Ok(Vec::new()),
            Err(e) => return Err(e.into()),
        };
        let mut records: Vec<SessionRecord> = entries
            .filter_map(|e| e.ok().map(|e| e.path()))
            .filter(|p| p.extension().is_some_and(|x| x == "json"))
            .filter_map(|p| read_record(&p))
            .collect();
        records.sort_by(|a, b| (a.saved_at, &a.key).cmp(&(b.saved_at, &b.key)));
        Ok(records)
    }
}

fn read_record(path: &Path) -> Option<SessionRecord> {
    let parsed = fs::read(path)
        .map_err(|e| e.to_string())
        .and_then(|b| serde_json::from_slice(&b).map_err(|e| e.to_string()));
    match parsed {
        Ok(r) => Some(r),
        Err(e) => {
            eprintln!(
                "warning: skipping unreadable session {}: {e}",
                path.display()
            );
            None
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::history::SessionRecord;

    fn temp_root(tag: &str) -> PathBuf {
        let dir = std::env::temp_dir().join(format!("rleval-store-{tag}-{}", std::process::id()));
        let _ = fs::remove_dir_all(&dir);
        dir
    }

    fn rec(key: &str, saved_at: u64) -> SessionRecord {
        SessionRecord {
            key: key.into(),
            label: key.into(),
            saved_at,
            players: vec![],
        }
    }

    #[test]
    fn account_ids_reject_path_tricks() {
        for bad in [
            "",
            "..",
            "a/b",
            "a\\b",
            "a b",
            &"x".repeat(33),
            "é",
            "_teams",
        ] {
            assert!(AccountId::new(bad).is_err(), "{bad:?} should be rejected");
        }
        assert!(AccountId::new("bailey_rd-1").is_ok());
    }

    #[test]
    fn session_key_is_stable_and_content_sensitive() {
        assert_eq!(session_key(b"abc"), session_key(b"abc"));
        assert_ne!(session_key(b"abc"), session_key(b"abd"));
        assert_ne!(session_key(b"abc"), session_key(b"abcd"));
    }

    #[test]
    fn save_is_idempotent_and_list_is_oldest_first() {
        let root = temp_root("idem");
        let store = FsSessionStore::new(&root);
        let me = AccountId::new("me").unwrap();
        assert_eq!(store.save(&me, &rec("b", 20)).unwrap(), SaveOutcome::Saved);
        assert_eq!(store.save(&me, &rec("a", 10)).unwrap(), SaveOutcome::Saved);
        assert_eq!(
            store.save(&me, &rec("a", 99)).unwrap(),
            SaveOutcome::AlreadyStored
        );
        let keys: Vec<_> = store
            .list(&me)
            .unwrap()
            .into_iter()
            .map(|r| r.key)
            .collect();
        assert_eq!(keys, ["a", "b"]);
        let _ = fs::remove_dir_all(root);
    }

    #[test]
    fn accounts_are_isolated_and_unknown_account_is_empty() {
        let root = temp_root("iso");
        let store = FsSessionStore::new(&root);
        let (a, b) = (AccountId::new("a").unwrap(), AccountId::new("b").unwrap());
        store.save(&a, &rec("k", 1)).unwrap();
        assert_eq!(store.list(&a).unwrap().len(), 1);
        assert!(store.list(&b).unwrap().is_empty());
        let _ = fs::remove_dir_all(root);
    }

    #[test]
    fn corrupt_files_are_skipped_not_fatal() {
        let root = temp_root("corrupt");
        let store = FsSessionStore::new(&root);
        let me = AccountId::new("me").unwrap();
        store.save(&me, &rec("good", 1)).unwrap();
        fs::write(root.join("me").join("bad.json"), b"{not json").unwrap();
        let recs = store.list(&me).unwrap();
        assert_eq!(recs.len(), 1);
        assert_eq!(recs[0].key, "good");
        let _ = fs::remove_dir_all(root);
    }
}
