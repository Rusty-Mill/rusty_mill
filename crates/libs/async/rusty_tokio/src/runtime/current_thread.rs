//! `block_on` for the current-thread flavor
//! ([`super::Builder::new_current_thread`]): spawned tasks run
//! interleaved with polls of `block_on`'s own future, entirely on the
//! calling thread -- no background worker threads at all, unlike the
//! multi-threaded flavor's dedicated pool.
//!
//! The I/O reactor and timer driver still run on their own dedicated
//! background threads regardless of flavor -- unlike real tokio, whose
//! current-thread runtime drives its I/O reactor inline on the single
//! thread as part of its own `park()`. Collapsing this crate's
//! `Reactor`/`TimerDriver` (already dedicated, already-tested background
//! threads, shared unchanged by both flavors) into the scheduling thread
//! too would be a materially bigger redesign than this issue's actual
//! ask (spawned tasks running without a worker pool) -- not attempted
//! here; a genuinely single-OS-thread runtime is a separate, bigger
//! step.

use super::worker;
use super::Shared;
use std::future::Future;
use std::sync::atomic::{AtomicBool, Ordering};
use std::sync::Arc;
use std::task::{Context, Poll, Wake, Waker};

pub(super) fn block_on<F: Future>(shared: &Arc<Shared>, future: F) -> F::Output {
    // Registers this thread as "worker 0" -- the single local queue a
    // current-thread runtime's `Shared` has -- for the duration of this
    // call, so a nested `spawn()` from a task running here (or from
    // `future` itself) enqueues locally rather than via the injector.
    let _worker_guard = worker::enter_as_worker(0);

    let mut future = std::pin::pin!(future);
    let parking = Arc::new(ParkingWaker {
        shared: shared.clone(),
        woken: AtomicBool::new(false),
    });
    let waker = Waker::from(parking.clone());
    let mut cx = Context::from_waker(&waker);

    loop {
        // Cleared before the poll, so a wake during it (e.g.
        // `yield_now`) is still seen by the auto-advance check below.
        parking.woken.store(false, Ordering::Release);
        if let Poll::Ready(value) = future.as_mut().poll(&mut cx) {
            return value;
        }

        // Drain every task that's ready to run before parking. A
        // spawned task's own wake -- including one that in turn wakes
        // `future`, e.g. completing a channel `future` is waiting on --
        // is handled by re-polling `future` at the top of the next
        // iteration rather than tracked separately.
        let mut ran_any = false;
        while let Some(task) = shared.next_task(0) {
            let start = std::time::Instant::now();
            task.run();
            shared.add_busy_duration(0, start.elapsed());
            ran_any = true;
        }
        if ran_any {
            continue;
        }

        // A paused clock never moves on its own, so with nothing
        // runnable a paused runtime waiting on a timer would wait
        // forever. Jump to the next deadline instead -- tokio's
        // auto-advance, which is what makes `start_paused = true` (and
        // `time::pause()` without manual `advance` calls) usable. Held
        // off while `future` itself was woken (it will make progress on
        // the next poll, e.g. mid-`advance`'s `yield_now`) or blocking
        // work is still running in real time.
        if !parking.woken.load(Ordering::Acquire)
            && !shared.blocking_pool.has_in_flight()
            && shared.timer.auto_advance()
        {
            continue;
        }

        // Nothing runnable right now -- park until `future`'s own waker
        // fires or a task gets scheduled (from a spawned task's own
        // wake, or the reactor/timer background threads), whichever
        // comes first. Same bounded, not-precisely-woken park the
        // multi-threaded worker loop already uses (see `Shared::park`'s
        // docs): a wakeup racing the check-then-park window here is
        // caught by that timeout, not lost outright.
        shared.park(0);
    }
}

struct ParkingWaker {
    shared: Arc<Shared>,
    /// Set by any wake of `block_on`'s own future -- see auto-advance
    /// above.
    woken: AtomicBool,
}

impl Wake for ParkingWaker {
    fn wake(self: Arc<Self>) {
        self.wake_by_ref();
    }

    fn wake_by_ref(self: &Arc<Self>) {
        self.woken.store(true, Ordering::Release);
        self.shared.wake_all_parked();
    }
}
