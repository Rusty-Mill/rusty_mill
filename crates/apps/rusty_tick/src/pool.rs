//! A bounded pool of open per-user services (Rusty-Mill/rusty_mill#382, gap 6;
//! ADR-0002).
//!
//! One directory per user keeps users isolated, and an open store costs
//! about 260 bytes of memory and 1.5 µs per record (see `SPIKE-FINDINGS.md`),
//! so the pool keeps only the most recently used `capacity` open and closes
//! the rest. Closing loses nothing: every write was durable when it returned.
//! Each directory is locked while open, since the stores do not lock by
//! themselves and two handles on one directory would overwrite each other.
//!
//! What a user has open is a [`Service`] (tasks and lists), which is what the
//! API needs. Nothing calls the pool yet: the HTTP API has one token and no
//! user identity, and ADR-0002 adds it.

use crate::service::{Clock, Service, ServiceError};
use crate::users::UserKey;
use rusty_multimodal_db_engine::dir_lock::{DirLock, DirLockError};
use std::collections::VecDeque;
use std::num::NonZeroUsize;
use std::path::{Path, PathBuf};

/// How many users may be open at once unless the caller says otherwise
/// (ADR-0002): a constant, not a flag, until someone needs more.
pub const DEFAULT_MAX_OPEN_USERS: NonZeroUsize = match NonZeroUsize::new(32) {
    Some(n) => n,
    None => unreachable!(),
};

/// Makes the clock each opened service reads the time from.
pub type ClockFactory = Box<dyn Fn() -> Clock + Send>;

#[derive(Debug, thiserror::Error)]
pub enum PoolError {
    #[error(transparent)]
    Lock(#[from] DirLockError),
    #[error(transparent)]
    Service(#[from] ServiceError),
}

/// An open service and the lock on its directory. Fields drop in order, so the
/// service closes before the lock is released.
struct Slot {
    service: Service,
    _lock: DirLock,
}

/// At most `capacity` users open at once, least recently used closed first.
pub struct ServicePool {
    root: PathBuf,
    capacity: NonZeroUsize,
    clock: ClockFactory,
    /// Least recently used first.
    open: VecDeque<(UserKey, Slot)>,
}

impl ServicePool {
    /// A pool over `root/<user key>/`, holding at most `capacity` users. Each
    /// service it opens reads the time from a clock `clock` makes.
    pub fn new(root: &Path, capacity: NonZeroUsize, clock: ClockFactory) -> Self {
        Self {
            root: root.to_path_buf(),
            capacity,
            clock,
            open: VecDeque::new(),
        }
    }

    /// How many users are open now.
    pub fn open_count(&self) -> usize {
        self.open.len()
    }

    /// Run `f` on `user`'s service, opening it (and closing the least
    /// recently used one first, when the pool is full) if it is not open.
    ///
    /// # Errors
    ///
    /// [`PoolError::Lock`] if another process holds the user's directory;
    /// [`PoolError::Service`] if it cannot be opened.
    pub fn with<R>(
        &mut self,
        user: &UserKey,
        f: impl FnOnce(&mut Service) -> R,
    ) -> Result<R, PoolError> {
        let at = match self.open.iter().position(|(key, _)| key == user) {
            Some(at) => at,
            None => self.admit(user)?,
        };
        // Most recently used goes last.
        let entry = self.open.remove(at).expect("the index was just found");
        self.open.push_back(entry);
        let (_, slot) = self.open.back_mut().expect("just pushed");
        Ok(f(&mut slot.service))
    }

    /// Open `user`'s service at the back of the queue, closing the least
    /// recently used one first so the bound holds even while opening. A
    /// failed open leaves the pool one user smaller, never larger.
    fn admit(&mut self, user: &UserKey) -> Result<usize, PoolError> {
        while self.open.len() >= self.capacity.get() {
            self.open.pop_front();
        }
        let dir = self.root.join(user.as_str());
        let lock = DirLock::acquire(&dir, "store.lock")?;
        let service = Service::open(&dir, (self.clock)())?;
        self.open.push_back((
            user.clone(),
            Slot {
                service,
                _lock: lock,
            },
        ));
        Ok(self.open.len() - 1)
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::service::{system_clock, NewTask};

    fn user(name: &str) -> UserKey {
        UserKey::parse(name).unwrap()
    }

    fn pool(root: &Path, capacity: usize) -> ServicePool {
        ServicePool::new(
            root,
            NonZeroUsize::new(capacity).unwrap(),
            Box::new(system_clock),
        )
    }

    fn new_task(list_id: uuid::Uuid) -> NewTask {
        NewTask {
            list_id,
            title: "t".into(),
            ..NewTask::default()
        }
    }

    #[test]
    fn the_default_bound_is_thirty_two() {
        assert_eq!(DEFAULT_MAX_OPEN_USERS.get(), 32);
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
    fn the_least_recently_used_user_is_the_one_closed() {
        let root = tempfile::tempdir().unwrap();
        let mut pool = pool(root.path(), 2);
        pool.with(&user("a"), |_| ()).unwrap();
        pool.with(&user("b"), |_| ()).unwrap();
        pool.with(&user("a"), |_| ()).unwrap(); // a is now newer than b
        pool.with(&user("c"), |_| ()).unwrap(); // closes b
        let open: Vec<&str> = pool.open.iter().map(|(k, _)| k.as_str()).collect();
        assert_eq!(open, ["a", "c"]);
        // b's directory lock was released with its service.
        DirLock::acquire(&root.path().join("b"), "store.lock").unwrap();
    }

    #[test]
    fn a_closed_user_reopens_with_lists_and_tasks_intact() {
        let root = tempfile::tempdir().unwrap();
        let mut pool = pool(root.path(), 1);
        let (list, task) = pool
            .with(&user("alice"), |s| {
                let list = s.create_list("Inbox").unwrap();
                let task = s.create_task(new_task(list.id)).unwrap();
                (list, task)
            })
            .unwrap();
        pool.with(&user("bob"), |_| ()).unwrap(); // evicts alice
        assert_eq!(pool.open_count(), 1);
        let (lists, back) = pool
            .with(&user("alice"), |s| (s.lists(), s.task(task.id).unwrap()))
            .unwrap();
        assert_eq!(lists, [list]);
        assert_eq!(back, task);
    }

    #[test]
    fn users_do_not_see_each_others_lists_or_tasks() {
        let root = tempfile::tempdir().unwrap();
        let mut pool = pool(root.path(), 4);
        let list = pool
            .with(&user("alice"), |s| {
                let list = s.create_list("Private").unwrap();
                s.create_task(new_task(list.id)).unwrap();
                list
            })
            .unwrap();
        let (lists, missing) = pool
            .with(&user("bob"), |s| (s.lists(), s.list(list.id).is_err()))
            .unwrap();
        assert!(lists.is_empty());
        assert!(missing, "bob cannot read alice's list by id");
    }

    #[test]
    fn each_opened_service_reads_its_own_clock() {
        let root = tempfile::tempdir().unwrap();
        let mut pool = ServicePool::new(
            root.path(),
            NonZeroUsize::new(2).unwrap(),
            Box::new(|| Box::new(|| 1_234)),
        );
        let stamped = pool
            .with(&user("alice"), |s| s.create_list("L").unwrap().updated_ms)
            .unwrap();
        assert_eq!(stamped, 1_234);
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
