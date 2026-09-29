//! A bounded pool of open per-user stores (Rusty-Mill/rusty_mill#382, gap 6).
//!
//! One store directory per user keeps users isolated, and an open store
//! costs about 260 bytes of memory and 1.5 µs per record (see
//! `SPIKE-FINDINGS.md`), so the pool keeps only the most recently used
//! `capacity` open and closes the rest. Closing loses nothing: every write
//! was durable when it returned. Each directory is locked while open, since
//! the store does not lock by itself and two handles on one directory would
//! overwrite each other.
//!
//! Nothing calls this yet: the HTTP API has one token and no user identity.
//! It is here so per-user tokens can land on a tested pool.

use crate::store::{TaskStore, TickError};
use rusty_multimodal_db_engine::dir_lock::{DirLock, DirLockError};
use std::collections::VecDeque;
use std::num::NonZeroUsize;
use std::path::{Path, PathBuf};

/// The longest user key, in bytes.
pub const MAX_USER_KEY_BYTES: usize = 64;

#[derive(Debug, thiserror::Error)]
pub enum PoolError {
    #[error("invalid user key {0:?}: 1 to {MAX_USER_KEY_BYTES} of A-Z a-z 0-9 _ -")]
    InvalidUser(String),
    #[error(transparent)]
    Lock(#[from] DirLockError),
    #[error(transparent)]
    Store(#[from] TickError),
}

/// A user's name as a directory name. Only a plain, short, ASCII key passes,
/// so nothing (`..`, a separator, a NUL) can leave the pool's root. Case is
/// kept, so on a case-insensitive filesystem `Alice` and `alice` name one
/// directory; the lock then refuses the second, and callers should
/// normalize keys.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct UserKey(String);

impl UserKey {
    pub fn parse(key: &str) -> Result<Self, PoolError> {
        let plain = |c: char| c.is_ascii_alphanumeric() || c == '_' || c == '-';
        if key.is_empty() || key.len() > MAX_USER_KEY_BYTES || !key.chars().all(plain) {
            return Err(PoolError::InvalidUser(key.to_string()));
        }
        Ok(Self(key.to_string()))
    }

    pub fn as_str(&self) -> &str {
        &self.0
    }
}

/// An open store and the lock on its directory. Fields drop in order, so the
/// store closes before the lock is released.
struct Slot {
    store: TaskStore,
    _lock: DirLock,
}

/// At most `capacity` stores open at once, least recently used closed first.
pub struct StorePool {
    root: PathBuf,
    capacity: NonZeroUsize,
    /// Least recently used first.
    open: VecDeque<(UserKey, Slot)>,
}

impl StorePool {
    /// A pool over `root/<user key>/`, holding at most `capacity` stores.
    pub fn new(root: &Path, capacity: NonZeroUsize) -> Self {
        Self {
            root: root.to_path_buf(),
            capacity,
            open: VecDeque::new(),
        }
    }

    /// How many stores are open now.
    pub fn open_count(&self) -> usize {
        self.open.len()
    }

    /// Run `f` on `user`'s store, opening it (and closing the least recently
    /// used one first, when the pool is full) if it is not open.
    ///
    /// # Errors
    ///
    /// [`PoolError::Lock`] if another process holds the user's directory;
    /// [`PoolError::Store`] if it cannot be opened.
    pub fn with<R>(
        &mut self,
        user: &UserKey,
        f: impl FnOnce(&mut TaskStore) -> R,
    ) -> Result<R, PoolError> {
        let at = match self.open.iter().position(|(key, _)| key == user) {
            Some(at) => at,
            None => self.admit(user)?,
        };
        // Most recently used goes last.
        let entry = self.open.remove(at).expect("the index was just found");
        self.open.push_back(entry);
        let (_, slot) = self.open.back_mut().expect("just pushed");
        Ok(f(&mut slot.store))
    }

    /// Open `user`'s store at the back of the queue, closing the least
    /// recently used one first so the bound holds even while opening. A
    /// failed open leaves the pool one store smaller, never larger.
    fn admit(&mut self, user: &UserKey) -> Result<usize, PoolError> {
        while self.open.len() >= self.capacity.get() {
            self.open.pop_front();
        }
        let dir = self.root.join(user.as_str());
        let lock = DirLock::acquire(&dir, "store.lock")?;
        let store = TaskStore::open(&dir)?;
        self.open
            .push_back((user.clone(), Slot { store, _lock: lock }));
        Ok(self.open.len() - 1)
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::task::{Priority, Status, Task};
    use uuid::Uuid;

    fn user(name: &str) -> UserKey {
        UserKey::parse(name).unwrap()
    }

    fn pool(root: &Path, capacity: usize) -> StorePool {
        StorePool::new(root, NonZeroUsize::new(capacity).unwrap())
    }

    fn task(list: Uuid) -> Task {
        Task {
            id: Uuid::now_v7(),
            list_id: list,
            parent_id: None,
            title: "t".into(),
            notes: String::new(),
            status: Status::Open,
            priority: Priority::None,
            due_ms: None,
            sort_order: 0,
            tags: vec![],
            updated_ms: 0,
        }
    }

    #[test]
    fn user_keys_are_plain_short_ascii() {
        for good in ["alice", "Bob_2", "a-b", &"x".repeat(MAX_USER_KEY_BYTES)] {
            assert!(UserKey::parse(good).is_ok(), "{good}");
        }
        for bad in ["", "..", "../x", "a/b", "a\\b", "a.b", "a b", "é", "a\0"] {
            assert!(UserKey::parse(bad).is_err(), "{bad:?}");
        }
        assert!(UserKey::parse(&"x".repeat(MAX_USER_KEY_BYTES + 1)).is_err());
    }

    #[test]
    fn the_pool_never_holds_more_than_its_capacity() {
        let root = tempfile::tempdir().unwrap();
        let mut pool = pool(root.path(), 2);
        for name in ["a", "b", "c", "d", "a"] {
            pool.with(&user(name), |_| ()).unwrap();
            assert!(pool.open_count() <= 2);
        }
        assert_eq!(pool.open_count(), 2);
    }

    #[test]
    fn the_least_recently_used_store_is_the_one_closed() {
        let root = tempfile::tempdir().unwrap();
        let mut pool = pool(root.path(), 2);
        pool.with(&user("a"), |_| ()).unwrap();
        pool.with(&user("b"), |_| ()).unwrap();
        pool.with(&user("a"), |_| ()).unwrap(); // a is now newer than b
        pool.with(&user("c"), |_| ()).unwrap(); // closes b
        let open: Vec<&str> = pool.open.iter().map(|(k, _)| k.as_str()).collect();
        assert_eq!(open, ["a", "c"]);
        // b's directory lock was released with its store.
        DirLock::acquire(&root.path().join("b"), "store.lock").unwrap();
    }

    #[test]
    fn a_closed_store_reopens_with_everything_written() {
        let root = tempfile::tempdir().unwrap();
        let mut pool = pool(root.path(), 1);
        let list = Uuid::now_v7();
        let written = task(list);
        let id = written.id;
        pool.with(&user("alice"), |s| s.insert(written.clone()))
            .unwrap()
            .unwrap();
        pool.with(&user("bob"), |_| ()).unwrap(); // evicts alice
        assert_eq!(pool.open_count(), 1);
        let back = pool.with(&user("alice"), |s| s.get(id)).unwrap();
        assert_eq!(back, Some(written));
    }

    #[test]
    fn users_do_not_see_each_others_tasks() {
        let root = tempfile::tempdir().unwrap();
        let mut pool = pool(root.path(), 4);
        let list = Uuid::now_v7();
        pool.with(&user("alice"), |s| s.insert(task(list)))
            .unwrap()
            .unwrap();
        let bobs = pool.with(&user("bob"), |s| s.in_list(list)).unwrap();
        assert!(bobs.is_empty());
    }

    #[test]
    fn a_directory_held_elsewhere_is_refused_and_leaves_the_pool_usable() {
        let root = tempfile::tempdir().unwrap();
        let mut pool = pool(root.path(), 2);
        pool.with(&user("a"), |_| ()).unwrap();
        let _held = DirLock::acquire(&root.path().join("b"), "store.lock").unwrap();
        let err = pool.with(&user("b"), |_| ()).unwrap_err();
        assert!(
            matches!(err, PoolError::Lock(DirLockError::Held(_))),
            "{err}"
        );
        pool.with(&user("a"), |_| ()).unwrap();
        assert!(pool.open_count() <= 2);
    }
}
