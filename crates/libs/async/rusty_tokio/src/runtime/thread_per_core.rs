//! The thread-per-core scheduling flavor
//! ([`super::Builder::new_thread_per_core`]): N OS threads, each pinned to
//! its own CPU core via `sched_setaffinity` on Linux (a silent no-op
//! elsewhere -- the flavor still runs correctly on other targets, just
//! without the pinning guarantee), each running its own independent
//! [`super::Shared`] -- its own reactor, timer driver, blocking pool, and
//! single-queue scheduler. This reuses exactly the same
//! [`super::LocalQueues::CurrentThread`]-shaped `Shared` the plain
//! current-thread flavor already builds (see `Builder::new_core_shared`),
//! just N of them instead of one, each handed to its own dedicated worker
//! thread instead of relying on a `block_on` caller to drive it.
//!
//! ## No cross-core work-stealing
//!
//! Task placement happens once, at spawn time, and never migrates
//! afterward: [`super::Runtime::spawn`] round-robins across the N cores'
//! `Shared`s to pick a freshly spawned task's home core, and every
//! re-wake after that goes through that exact same `Shared`'s own
//! `schedule()` (a task's `Waker` closes over the `Arc<Shared>` it was
//! created against, not "whichever core happens to be idle"). There is no
//! shared injector, no stealer list, and no cross-core queue at all for
//! this flavor -- each core's `Shared.local` is the single-queue
//! `LocalQueues::CurrentThread` variant, so there is nothing *to* steal
//! from even if a worker wanted to.
//!
//! A task spawned from *inside* another task via the ordinary ambient
//! `crate::spawn`/`Handle::spawn` lands on whichever core's thread happens
//! to be running it: each pinned worker thread below enters this crate's
//! ambient "current runtime" context ([`context::enter`]) once, for its
//! entire life, bound to its own `Shared` -- so every socket/timer/spawn
//! call this crate's `Handle::current()`-based plumbing already makes
//! (`TcpStream::connect`, `time::sleep`, `crate::spawn`, ...) transparently
//! resolves to that thread's own reactor/timer/queue with zero code
//! changes anywhere else in the crate. That's also what gives each core
//! its own *real* io_uring/epoll reactor instance, not a shared one --
//! see [`super::Builder::new_core_shared`]'s doc comment.
//!
//! ## `block_on`/`Handle` from outside the pool
//!
//! [`super::Runtime::block_on`]/[`super::Runtime::handle`] -- called from
//! a thread that isn't one of these N pinned workers at all (e.g. the
//! thread that built the `Runtime`) -- use core 0 as an "honorary" ambient
//! `Shared` (documented on those methods): a nested `crate::spawn` from
//! inside a `block_on`'d future lands on core 0's queue, momentarily
//! shared (safely -- it's `Mutex`-guarded) between the calling thread and
//! core 0's own dedicated worker thread. Use [`super::Runtime::spawn`]
//! directly (or [`super::Runtime::core_handle`] for a specific core) for
//! genuine round-robin placement across every core.

use super::context;
use super::Shared;
use std::sync::Arc;
use std::time::Instant;

/// Pins the calling thread to CPU `core_id` -- Linux only
/// (`sched_setaffinity`); a silent no-op on every other target, so this
/// flavor still runs correctly there, just without the pinning guarantee
/// the acceptance criteria's `strace`/scheduling check specifically wants
/// on Linux. A failure here (`core_id` beyond the process's own affinity
/// mask, a sandboxed/containerized environment that simply forbids it,
/// ...) isn't fatal to correctness -- this core's worker just runs
/// unpinned, same as every other target -- so the return value is
/// deliberately not checked.
#[cfg(target_os = "linux")]
fn pin_to_core(core_id: usize) {
    // SAFETY: `set` is a plain stack-local `cpu_set_t` this thread alone
    // owns and initializes before use; `sched_setaffinity(0, ..)` targets
    // the calling thread (pid 0 means "self", not an arbitrary process),
    // and the size/pointer passed match `cpu_set_t` exactly.
    unsafe {
        let mut set: libc::cpu_set_t = std::mem::zeroed();
        libc::CPU_ZERO(&mut set);
        libc::CPU_SET(core_id, &mut set);
        libc::sched_setaffinity(0, std::mem::size_of::<libc::cpu_set_t>(), &set);
    }
}

#[cfg(not(target_os = "linux"))]
fn pin_to_core(_core_id: usize) {}

/// Spawns and pins one core's dedicated worker thread -- headless (no
/// `block_on`-supplied future to interleave with, unlike
/// [`super::current_thread::block_on`]'s otherwise-identical pop-run-park
/// loop): it only ever runs tasks that land on `shared`'s own queue,
/// whether from [`super::Runtime::spawn`]'s round-robin or a nested
/// `crate::spawn` from a task already running here.
pub(super) fn spawn_core_worker(
    shared: Arc<Shared>,
    core_id: usize,
    thread_config: super::ThreadConfig,
) -> std::thread::JoinHandle<()> {
    thread_config
        .thread_builder(|| format!("rusty_tokio-core-{core_id}"))
        .spawn(move || {
            pin_to_core(core_id);
            // Entered once, for this thread's entire life -- unlike
            // `worker::spawn_worker`'s `_guard` (also held for that
            // thread's whole run loop, but with a `WORKER_INDEX`/
            // `LOCAL_QUEUE` thread-local pairing this flavor doesn't
            // need: `Shared::next_task`/`schedule` don't consult either
            // one for the single-queue `LocalQueues::CurrentThread`
            // shape every core here uses), this thread never needs to
            // restore any other ambient context afterward -- it simply
            // exits once `run` returns.
            let _guard = context::enter(shared.clone());
            run(&shared);
        })
        .expect("failed to spawn rusty_tokio thread-per-core worker thread")
}

fn run(shared: &Arc<Shared>) {
    while !shared.is_shutdown() {
        match shared.next_task(0) {
            Some(task) => {
                let start = Instant::now();
                task.run();
                shared.add_busy_duration(0, start.elapsed());
            }
            None => shared.park(0),
        }
    }
}
