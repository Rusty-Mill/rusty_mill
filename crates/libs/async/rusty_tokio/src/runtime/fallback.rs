//! An opt-in, process-wide runtime for threads that have none of their own.
//!
//! I/O, timers and `spawn` find their runtime through the thread's ambient
//! [`Handle`]. A future polled by another executor (a tokio worker, a
//! hand-written `block_on`) has none, so they panic there. Once a program
//! calls [`install_fallback`], they use this runtime instead: its reactor and
//! timer threads wake the foreign executor through ordinary `Waker`s, and
//! spawned tasks run on its single worker.
//!
//! Nothing starts it implicitly. A program that drives rusty_tokio code from
//! another runtime calls it once at startup, so the extra threads are a choice
//! made where the runtimes meet rather than a hidden global.

use super::{Builder, Handle};
use std::io;
use std::sync::{Mutex, OnceLock, PoisonError};

static FALLBACK: OnceLock<Handle> = OnceLock::new();
static INIT: Mutex<()> = Mutex::new(());

/// Starts the fallback runtime if it isn't running yet, and returns its handle.
///
/// From then on, rusty_tokio I/O, timers, [`crate::spawn`] and
/// [`crate::spawn_blocking`] called on a thread with no rusty_tokio runtime
/// use this one instead of panicking. A thread inside a rusty_tokio runtime
/// keeps using its own. [`Handle::current`] and [`Handle::try_current`] are
/// unchanged: they still report only the thread's own runtime.
///
/// The runtime has one worker thread plus the usual reactor, timer and
/// blocking-pool threads, and lives until the process exits. Calling this
/// again returns the same handle.
///
/// # Errors
/// Returns the error from creating the I/O reactor (the epoll, kqueue,
/// io_uring or IOCP instance). Nothing has started at that point, and a
/// later call tries again.
///
/// # Panics
/// Panics if the OS refuses to spawn one of the runtime's threads, as
/// [`Builder::build`] does. Threads started before that keep running, and
/// a later call starts a new runtime rather than reusing or stopping them.
pub fn install_fallback() -> io::Result<Handle> {
    if let Some(handle) = FALLBACK.get() {
        return Ok(handle.clone());
    }
    let _init = INIT.lock().unwrap_or_else(PoisonError::into_inner);
    if let Some(handle) = FALLBACK.get() {
        return Ok(handle.clone());
    }
    let runtime = Builder::new_multi_thread()
        .worker_threads(1)
        .thread_name("rusty-tokio-fallback")
        .enable_all()
        .build()?;
    let handle = runtime.handle();
    // It serves the rest of the process; dropping it would shut it down.
    std::mem::forget(runtime);
    Ok(FALLBACK.get_or_init(|| handle).clone())
}

/// The fallback runtime's handle, if [`install_fallback`] has run.
pub(crate) fn get() -> Option<Handle> {
    FALLBACK.get().cloned()
}
