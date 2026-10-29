//! Bounded custody of children whose rollback could not be confirmed
//! (design §11.2.1). A spawn reserves a slot here before `CreateProcessW`,
//! so reservation is the last step that can fail for lack of resources.
//! Should the child's rollback fail to confirm its exit, the reserved slot
//! takes its process handle and console without allocating or starting
//! anything, and one worker thread per pool keeps them until the child has
//! exited. Ownership is released only on a confirmed exit, never on a
//! failed wait.

use std::io;
use std::sync::atomic::{AtomicBool, Ordering};
use std::sync::{Arc, Mutex, MutexGuard, PoisonError};
use std::time::Duration;

use platform::error::{ErrorKind, OsCode, PlatformError, Result};

use crate::sys::condrv as sys;
use crate::sys::handle::OwnedWinHandle;
use crate::sys::proc;

/// Children the process-wide pool holds at once.
const CAPACITY: usize = 8;
/// How often the worker rechecks its children, whatever it was woken by.
const RESCAN: Duration = Duration::from_secs(1);

/// Starts the worker thread: `Ok` once it runs, `Err` if it can't start.
pub(super) type StartWorker = dyn FnOnce(Box<dyn FnOnce() + Send>) -> io::Result<()>;

/// A pool of cleanup slots and the worker that empties them. Cloning
/// shares the pool.
#[derive(Clone)]
pub(super) struct Pool {
    shared: Arc<Shared>,
}

struct Shared {
    state: Mutex<State>,
    /// Wakes the worker after a hand-off; it rescans every [`RESCAN`]
    /// regardless, so a lost wake only delays it.
    wake: OwnedWinHandle,
    /// Cleared when the worker stops; a pool without one reserves nothing.
    alive: AtomicBool,
    capacity: usize,
}

struct State {
    /// Slots taken, by reservations and by held children together.
    reserved: usize,
    /// Allocated to `capacity` up front, so a hand-off never allocates.
    held: Vec<Held>,
    /// The worker's latest failed wait or exit check, kept for diagnosis.
    /// The child it concerned stays held.
    last_error: Option<PlatformError>,
}

/// A child whose exit is not yet confirmed, with the console it may use.
struct Held {
    process: OwnedWinHandle,
    _server: sys::Server,
}

/// One slot, taken before `CreateProcessW`. Dropping it frees the slot;
/// [`Reservation::hand_off`] fills it instead.
pub(super) struct Reservation {
    shared: Arc<Shared>,
    armed: bool,
}

static GLOBAL: Mutex<Option<Pool>> = Mutex::new(None);

/// The process-wide pool, started on first use. Fails, before any child
/// exists, when its event or thread can't be created. A pool whose worker
/// has stopped is kept, not replaced, so the children it holds stay held;
/// it then refuses every reservation.
pub(super) fn global() -> Result<Pool> {
    let mut global = GLOBAL.lock().unwrap_or_else(PoisonError::into_inner);
    if let Some(pool) = global.as_ref() {
        return Ok(pool.clone());
    }
    let pool = Pool::start(CAPACITY, Box::new(os_thread))?;
    *global = Some(pool.clone());
    Ok(pool)
}

fn os_thread(work: Box<dyn FnOnce() + Send>) -> io::Result<()> {
    std::thread::Builder::new()
        .name("condrv-cleanup".into())
        .spawn(work)
        .map(drop)
}

impl Pool {
    /// A pool of `capacity` slots whose worker `start` runs.
    pub(super) fn start(capacity: usize, start: Box<StartWorker>) -> Result<Pool> {
        let shared = Arc::new(Shared {
            state: Mutex::new(State {
                reserved: 0,
                held: Vec::with_capacity(capacity),
                last_error: None,
            }),
            wake: sys::auto_reset_event()?,
            alive: AtomicBool::new(true),
            capacity,
        });
        let alive = AliveGuard(Arc::clone(&shared));
        start(Box::new(move || work(alive))).map_err(|_| {
            PlatformError::new(
                ErrorKind::Other,
                OsCode::None,
                "condrv::spawn_attached: the cleanup worker could not start",
            )
        })?;
        Ok(Pool { shared })
    }

    /// Take a slot. `WouldBlock` when every slot is taken; `Other` when the
    /// worker has stopped.
    pub(super) fn reserve(&self) -> Result<Reservation> {
        if !self.shared.alive.load(Ordering::Acquire) {
            return Err(PlatformError::new(
                ErrorKind::Other,
                OsCode::None,
                "condrv::spawn_attached: the cleanup worker has stopped",
            ));
        }
        let mut state = self.shared.lock();
        if state.reserved >= self.shared.capacity {
            return Err(PlatformError::new(
                ErrorKind::WouldBlock,
                OsCode::None,
                "condrv::spawn_attached: every cleanup slot holds an unfinished child",
            ));
        }
        state.reserved += 1;
        Ok(Reservation {
            shared: Arc::clone(&self.shared),
            armed: true,
        })
    }

    #[cfg(test)]
    pub(super) fn reserved(&self) -> usize {
        self.shared.lock().reserved
    }

    #[cfg(test)]
    pub(super) fn held(&self) -> usize {
        self.shared.lock().held.len()
    }
}

impl Shared {
    fn lock(&self) -> MutexGuard<'_, State> {
        self.state.lock().unwrap_or_else(PoisonError::into_inner)
    }
}

impl Reservation {
    /// Give the slot a child whose exit is not confirmed. Infallible: the
    /// slot was counted and its storage allocated when it was reserved.
    pub(super) fn hand_off(mut self, process: OwnedWinHandle, server: sys::Server) {
        self.armed = false;
        self.shared.lock().held.push(Held {
            process,
            _server: server,
        });
        // A failed wake only delays the worker to its next rescan.
        let _ = sys::set_event(&self.shared.wake);
    }
}

impl Drop for Reservation {
    fn drop(&mut self) {
        if self.armed {
            self.shared.lock().reserved -= 1;
        }
    }
}

/// Marks the pool's worker stopped when the worker ends, or when it never
/// ran because its closure was dropped.
struct AliveGuard(Arc<Shared>);

impl Drop for AliveGuard {
    fn drop(&mut self) {
        self.0.alive.store(false, Ordering::Release);
    }
}

/// The worker: wait on the wake event and every held child, then release
/// the children that have exited. A failed wait releases nothing; the next
/// rescan tries again. It ends once nothing but itself can reach the pool
/// and the pool holds no child, which the process-wide pool never reaches.
fn work(alive: AliveGuard) {
    let shared = &alive.0;
    loop {
        let handles: Vec<_> = {
            let state = shared.lock();
            if state.held.is_empty() && Arc::strong_count(shared) == 1 {
                return;
            }
            std::iter::once(shared.wake.as_raw())
                .chain(state.held.iter().map(|h| h.process.as_raw()))
                .collect()
        };
        // Only this thread removes held children, so every handle stays
        // open for the wait.
        if let Err(e) = proc::wait_many(&handles, Some(RESCAN)) {
            shared.lock().last_error = Some(e);
            std::thread::sleep(RESCAN);
        }
        release_exited(shared, has_exited);
    }
}

/// A process handle is signalled only once the process has exited.
fn has_exited(process: &OwnedWinHandle) -> Result<bool> {
    Ok(proc::wait_many(&[process.as_raw()], Some(Duration::ZERO))?.is_some())
}

/// Release the held children `exited` confirms have exited. One whose
/// check fails stays held, and the error is kept in `last_error`.
fn release_exited(shared: &Shared, exited: impl Fn(&OwnedWinHandle) -> Result<bool>) {
    let gone: Vec<Held> = {
        let mut state = shared.lock();
        let mut gone = Vec::new();
        let mut i = 0;
        while i < state.held.len() {
            match exited(&state.held[i].process) {
                Ok(true) => gone.push(state.held.swap_remove(i)),
                Ok(false) => i += 1,
                Err(e) => {
                    state.last_error = Some(e);
                    i += 1;
                }
            }
        }
        state.reserved -= gone.len();
        gone
    };
    // Closed outside the lock: dropping a server may wait on its driver.
    drop(gone);
}

#[cfg(test)]
mod tests {
    use super::*;

    type Work = Box<dyn FnOnce() + Send>;

    /// A pool whose worker closure is parked in the returned slot: the
    /// worker counts as running until the slot is emptied, and never runs.
    fn parked(capacity: usize) -> (Pool, Arc<Mutex<Option<Work>>>) {
        let slot = Arc::new(Mutex::new(None));
        let keep = Arc::clone(&slot);
        let pool = Pool::start(
            capacity,
            Box::new(move |w| {
                *keep.lock().expect("slot") = Some(w);
                Ok(())
            }),
        )
        .expect("start");
        (pool, slot)
    }

    fn stand_in() -> (OwnedWinHandle, sys::Server) {
        let process = sys::auto_reset_event().expect("event");
        (process, sys::Server::create().expect("Server::create"))
    }

    #[test]
    fn a_worker_that_cannot_start_gives_no_pool() {
        let e = Pool::start(1, Box::new(|_| Err(io::Error::other("injected"))))
            .err()
            .expect("a pool without a worker");
        assert_eq!(e.kind, ErrorKind::Other);
    }

    #[test]
    fn reservations_are_bounded_and_released_on_drop() {
        let (pool, _worker) = parked(2);
        let a = pool.reserve().expect("first");
        let _b = pool.reserve().expect("second");
        let e = pool.reserve().err().expect("a third slot");
        assert_eq!(e.kind, ErrorKind::WouldBlock);
        drop(a);
        assert_eq!(pool.reserved(), 1);
        pool.reserve().expect("the freed slot");
    }

    /// A failed exit check keeps the child and records why; only a
    /// confirmed exit releases it and its slot.
    #[test]
    fn only_a_confirmed_exit_releases_a_held_child() {
        let (pool, _worker) = parked(1);
        let (process, server) = stand_in();
        pool.reserve().expect("slot").hand_off(process, server);

        release_exited(&pool.shared, |_| {
            Err(PlatformError::new(ErrorKind::Other, OsCode::None, "check"))
        });
        assert_eq!((pool.held(), pool.reserved()), (1, 1));
        let last = pool.shared.lock().last_error.as_ref().map(|e| e.op);
        assert_eq!(last, Some("check"));

        release_exited(&pool.shared, |_| Ok(false));
        assert_eq!((pool.held(), pool.reserved()), (1, 1));

        release_exited(&pool.shared, |_| Ok(true));
        assert_eq!((pool.held(), pool.reserved()), (0, 0));
    }

    /// A worker that stops keeps what it holds and refuses new slots.
    #[test]
    fn a_stopped_worker_keeps_its_children_and_reserves_nothing() {
        let (pool, worker) = parked(2);
        let (process, server) = stand_in();
        pool.reserve().expect("slot").hand_off(process, server);
        drop(worker.lock().expect("slot").take());
        let e = pool.reserve().err().expect("a slot from a stopped worker");
        assert_eq!(e.kind, ErrorKind::Other);
        assert_eq!((pool.held(), pool.reserved()), (1, 1));
    }
}
