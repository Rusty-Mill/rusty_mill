//! [`SessionStore`] over the Rusty-Mill `rusty_multimodal_db_engine`: an
//! embedded, mmap-backed record store with an fsync'd insert log and a directory
//! lock. Built only with the `mmdb` feature.
//!
//! Layout: `<root>/<namespace>/sessions.mmap` (plus the engine's companion
//! blob and insert log), guarded by `<root>/<namespace>/LOCK`.
//!
//! Each engine record is an envelope — `(id, saved_at, payload)` — whose payload
//! is the [`SessionRecord`] as JSON. The engine encodes records with bincode,
//! which is not self-describing, so storing the session natively would make every
//! new field (like `platform_id`) a breaking on-disk change. JSON inside a stable
//! envelope keeps sessions evolvable with `#[serde(default)]`; the engine still
//! provides durability, the insert log and the process lock.
//!
//! Engine constraints that shape this adapter:
//! - it keeps every record in RAM, fine for per-account histories of matches;
//! - one process per directory — a second server on the same data dir is refused
//!   with a clear error instead of corrupting it;
//! - ids are `Uuid`/`u32`/`i64`, so the 96-bit session key is packed into a `Uuid`.

use std::collections::HashMap;
use std::path::PathBuf;
use std::sync::{Arc, Mutex, MutexGuard};

use rusty_multimodal_db_engine::dir_lock::DirLock;
use rusty_multimodal_db_engine::generic::query::{AllIds, GetById};
use rusty_multimodal_db_engine::generic::traits::{
    IndexedField, Record, ScannableField, SchemaTag,
};
use rusty_multimodal_db_engine::generic::{GenericMmapStore, InsertError};
use serde::{Deserialize, Serialize};
use uuid::Uuid;

use crate::history::SessionRecord;
use crate::store::{AccountId, SaveOutcome, SessionStore, StoreError};

/// Length of a key produced by [`crate::store::session_key`]: 16 + 8 hex digits.
const KEY_HEX_LEN: usize = 24;
const STORE_FILE: &str = "sessions.mmap";
const LOCK_FILE: &str = "LOCK";

/// The engine record: a stable envelope around the session's JSON.
#[derive(Clone, Serialize, Deserialize)]
struct Blob {
    id: Uuid,
    saved_at: i64,
    payload: Vec<u8>,
}

impl Record for Blob {
    type Id = Uuid;
    fn id(&self) -> Uuid {
        self.id
    }
}

// Part of the on-disk format: changing it invalidates every stored blob.
impl SchemaTag for Blob {
    const SCHEMA_TAG: &'static str = "rleval::SessionBlob@1";
}

/// Marker for the one indexed + scannable field the engine's core requires.
/// Sessions are read whole and sorted in memory, so neither is queried.
struct Saved;

impl IndexedField<Saved> for Blob {
    type IndexValue = i64;
    fn indexed_value(&self) -> &i64 {
        &self.saved_at
    }
}

impl ScannableField<Saved> for Blob {
    type ScanValue = i64;
    fn scannable_value(&self) -> i64 {
        self.saved_at
    }
    fn set_scannable_value(&mut self, value: i64) {
        self.saved_at = value;
    }
}

type Core = GenericMmapStore<Blob, Saved, Saved>;

/// One opened namespace. Field order matters: the store is dropped (and its
/// mapping closed) before the lock is released.
struct Namespace {
    store: Core,
    _lock: DirLock,
}

pub struct MmdbSessionStore {
    root: PathBuf,
    open: Mutex<HashMap<String, Arc<Mutex<Namespace>>>>,
}

impl MmdbSessionStore {
    pub fn new(root: impl Into<PathBuf>) -> Self {
        Self {
            root: root.into(),
            open: Mutex::new(HashMap::new()),
        }
    }

    /// The namespace's open store, opening (or creating) it on first use.
    fn namespace(&self, ns: &AccountId) -> Result<Arc<Mutex<Namespace>>, StoreError> {
        let mut open = lock(&self.open)?;
        if let Some(existing) = open.get(ns.as_str()) {
            return Ok(Arc::clone(existing));
        }
        let dir = self.root.join(ns.as_str());
        let lock = DirLock::acquire(&dir, LOCK_FILE).map_err(backend)?;
        let path = dir.join(STORE_FILE);
        let store = if path.exists() {
            Core::open_portable(&path)
        } else {
            Core::create(Vec::new(), &path)
        }
        .map_err(backend)?;
        let opened = Arc::new(Mutex::new(Namespace { store, _lock: lock }));
        open.insert(ns.as_str().to_string(), Arc::clone(&opened));
        Ok(opened)
    }
}

impl SessionStore for MmdbSessionStore {
    fn save(&self, ns: &AccountId, record: &SessionRecord) -> Result<SaveOutcome, StoreError> {
        let id = key_to_uuid(&record.key)?;
        let payload = serde_json::to_vec(record).map_err(StoreError::Encode)?;
        let opened = self.namespace(ns)?;
        let mut guard = lock(&opened)?;
        if guard.store.get(id).is_some() {
            return Ok(SaveOutcome::AlreadyStored);
        }
        let blob = Blob {
            id,
            saved_at: i64::try_from(record.saved_at).unwrap_or(i64::MAX),
            payload,
        };
        match guard.store.insert(blob) {
            Ok(()) => Ok(SaveOutcome::Saved),
            Err(InsertError::Duplicate(_)) => Ok(SaveOutcome::AlreadyStored),
            Err(InsertError::Durability(e)) => Err(backend(e)),
        }
    }

    fn list(&self, ns: &AccountId) -> Result<Vec<SessionRecord>, StoreError> {
        // Listing must not create a store (and its directory) for a namespace
        // that has never saved anything.
        if !self.root.join(ns.as_str()).join(STORE_FILE).exists() {
            return Ok(Vec::new());
        }
        let opened = self.namespace(ns)?;
        let guard = lock(&opened)?;
        let mut records: Vec<SessionRecord> = guard
            .store
            .all_ids()
            .into_iter()
            .filter_map(|id| guard.store.get(id))
            .filter_map(|blob| match serde_json::from_slice(&blob.payload) {
                Ok(r) => Some(r),
                Err(e) => {
                    eprintln!("warning: skipping unreadable session {}: {e}", blob.id);
                    None
                }
            })
            .collect();
        records.sort_by(|a, b| (a.saved_at, &a.key).cmp(&(b.saved_at, &b.key)));
        Ok(records)
    }
}

/// Pack a session key (24 hex digits) into the engine's `Uuid` id.
fn key_to_uuid(key: &str) -> Result<Uuid, StoreError> {
    let valid = key.len() == KEY_HEX_LEN && key.bytes().all(|b| b.is_ascii_hexdigit());
    if !valid {
        return Err(StoreError::InvalidKey(key.to_string()));
    }
    u128::from_str_radix(key, 16)
        .map(Uuid::from_u128)
        .map_err(|_| StoreError::InvalidKey(key.to_string()))
}

fn backend(e: impl std::fmt::Display) -> StoreError {
    StoreError::Backend(e.to_string())
}

fn lock<T>(m: &Mutex<T>) -> Result<MutexGuard<'_, T>, StoreError> {
    m.lock()
        .map_err(|_| StoreError::Backend("a previous writer panicked; restart the server".into()))
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::store::{copy_all, session_key, CopyReport, FsSessionStore};
    use std::fs;
    use std::path::Path;

    fn temp_root(tag: &str) -> PathBuf {
        let dir = std::env::temp_dir().join(format!("rleval-mmdb-{tag}-{}", std::process::id()));
        let _ = fs::remove_dir_all(&dir);
        dir
    }

    fn rec(seed: &str, saved_at: u64) -> SessionRecord {
        SessionRecord {
            key: session_key(seed.as_bytes()),
            label: seed.into(),
            saved_at,
            players: vec![],
            ..Default::default()
        }
    }

    fn ns(n: &str) -> AccountId {
        AccountId::new(n).unwrap()
    }

    fn cleanup(root: &Path) {
        let _ = fs::remove_dir_all(root);
    }

    #[test]
    fn save_is_idempotent_and_list_is_oldest_first() {
        let root = temp_root("idem");
        let store = MmdbSessionStore::new(&root);
        let me = ns("me");
        assert_eq!(store.save(&me, &rec("b", 20)).unwrap(), SaveOutcome::Saved);
        assert_eq!(store.save(&me, &rec("a", 10)).unwrap(), SaveOutcome::Saved);
        assert_eq!(
            store.save(&me, &rec("a", 99)).unwrap(),
            SaveOutcome::AlreadyStored
        );
        let labels: Vec<_> = store
            .list(&me)
            .unwrap()
            .into_iter()
            .map(|r| r.label)
            .collect();
        assert_eq!(labels, ["a", "b"]);
        drop(store);
        cleanup(&root);
    }

    #[test]
    fn sessions_survive_reopening_the_store() {
        let root = temp_root("reopen");
        let first = MmdbSessionStore::new(&root);
        first.save(&ns("me"), &rec("kept", 5)).unwrap();
        drop(first); // releases the directory lock
        let second = MmdbSessionStore::new(&root);
        let got = second.list(&ns("me")).unwrap();
        assert_eq!(got, vec![rec("kept", 5)]);
        drop(second);
        cleanup(&root);
    }

    #[test]
    fn namespaces_are_isolated_and_listing_does_not_create_one() {
        let root = temp_root("iso");
        let store = MmdbSessionStore::new(&root);
        store.save(&ns("a"), &rec("x", 1)).unwrap();
        assert!(store.list(&ns("b")).unwrap().is_empty());
        assert!(
            !root.join("b").exists(),
            "a read must not create a namespace"
        );
        drop(store);
        cleanup(&root);
    }

    #[test]
    fn a_second_store_on_the_same_directory_is_refused() {
        let root = temp_root("lock");
        let first = MmdbSessionStore::new(&root);
        first.save(&ns("me"), &rec("x", 1)).unwrap();
        let second = MmdbSessionStore::new(&root);
        let err = second.save(&ns("me"), &rec("y", 2)).unwrap_err();
        assert!(matches!(err, StoreError::Backend(_)), "{err}");
        drop((first, second));
        cleanup(&root);
    }

    #[test]
    fn keys_that_are_not_session_keys_are_rejected() {
        let root = temp_root("badkey");
        let store = MmdbSessionStore::new(&root);
        for bad in ["", "short", "zzzzzzzzzzzzzzzzzzzzzzzz", &"a".repeat(25)] {
            let r = SessionRecord {
                key: bad.into(),
                ..rec("x", 1)
            };
            assert!(
                matches!(store.save(&ns("me"), &r), Err(StoreError::InvalidKey(_))),
                "{bad:?}"
            );
        }
        drop(store);
        cleanup(&root);
    }

    #[test]
    fn json_sessions_import_into_the_engine_store_and_reimport_is_a_noop() {
        let (fs_root, db_root) = (temp_root("imp-fs"), temp_root("imp-db"));
        let json = FsSessionStore::new(&fs_root);
        json.save(&ns("a"), &rec("one", 1)).unwrap();
        json.save(&ns("a"), &rec("two", 2)).unwrap();
        json.save(&ns("b"), &rec("three", 3)).unwrap();

        let db = MmdbSessionStore::new(&db_root);
        let names = json.namespaces().unwrap();
        assert_eq!(
            copy_all(&json, &db, &names).unwrap(),
            CopyReport {
                copied: 3,
                already_present: 0
            }
        );
        assert_eq!(db.list(&ns("a")).unwrap(), json.list(&ns("a")).unwrap());
        assert_eq!(
            copy_all(&json, &db, &names).unwrap(),
            CopyReport {
                copied: 0,
                already_present: 3
            }
        );
        drop(db);
        cleanup(&fs_root);
        cleanup(&db_root);
    }
}
