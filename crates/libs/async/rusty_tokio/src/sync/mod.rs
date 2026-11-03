//! Async-aware synchronization primitives: things that suspend the
//! *task* while waiting, rather than blocking the worker thread the way
//! `std::sync` equivalents do.

pub mod broadcast;
pub mod mpsc;
pub mod oneshot;
pub mod watch;

mod barrier;
mod cancellation_token;
mod mutex;
mod notify;
mod once_cell;
mod rwlock;
mod semaphore;
mod set_once;
mod wait_list;

pub use barrier::{Barrier, BarrierWaitResult};
// tokio-util's, not tokio's (#901): here so a migration needs no second crate.
pub use cancellation_token::{
    CancellationToken, DropGuard, WaitForCancellationFuture, WaitForCancellationFutureOwned,
};
pub use mutex::{MappedMutexGuard, Mutex, MutexGuard, OwnedMappedMutexGuard, OwnedMutexGuard};
pub use notify::{Notified, Notify, OwnedNotified};
pub use once_cell::{OnceCell, SetError};
pub use rwlock::{
    OwnedRwLockMappedWriteGuard, OwnedRwLockReadGuard, OwnedRwLockWriteGuard, RwLock,
    RwLockMappedWriteGuard, RwLockReadGuard, RwLockWriteGuard,
};
pub use semaphore::{
    AcquireError, OwnedSemaphorePermit, Semaphore, SemaphorePermit, TryAcquireError,
};
pub use set_once::{SetOnce, SetOnceError};
