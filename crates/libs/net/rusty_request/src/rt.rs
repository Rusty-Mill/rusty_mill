//! Which async runtime is driving the current task -- decided at run
//! time, per call, rather than at compile time by the `tokio` feature.
//!
//! Cargo features are meant to be additive: with feature unification,
//! *any* crate in a build graph (or `--all-features` on a workspace
//! member) turning on `rusty_request/tokio` turns it on for every other
//! consumer in that same build, including ones that only ever run under
//! `rusty_tokio`. The feature used to *replace* the `rusty_tokio` backend
//! with a real-tokio one wholesale, so such a consumer's requests went
//! looking for a tokio reactor that wasn't there ("there is no reactor
//! running, must be called from the context of a Tokio 1.x runtime")
//! -- a hard failure caused entirely by an unrelated crate's feature
//! choice. Now the feature only *adds* the real-tokio backend alongside
//! the always-present `rusty_tokio` one, and every runtime-touching
//! operation (connecting a socket, the request timeout, retry backoff
//! sleeps, `spawn_blocking` for DNS) asks which runtime the calling task
//! is actually running on and uses that one's primitives.
//!
//! Detection is cheap (two thread-local reads) and happens on the thread
//! polling the future -- i.e. a worker (or `block_on` caller) of whichever
//! runtime owns the task -- so it always answers for the runtime that
//! will be driving the resulting I/O and timers. `rusty_tokio` is checked
//! first: it's this crate's native runtime and its default, and a task
//! running inside it must use its reactor regardless of whether a real
//! tokio handle happens to be entered on the same thread (e.g. a
//! `rusty_tokio` runtime driven from within a tokio program). With no
//! runtime of either kind detected, `rusty_tokio` is used, so the caller
//! gets that runtime's own "no runtime on this thread" diagnostic --
//! exactly what a default-features build has always reported.

use std::fmt;
use std::future::Future;
use std::time::Duration;

/// The runtime the current task is running on -- see the module docs.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub(crate) enum Flavor {
    RustyTokio,
    #[cfg(feature = "tokio")]
    Tokio,
}

/// Detects the runtime driving the calling task. Never panics.
pub(crate) fn current() -> Flavor {
    if rusty_tokio::Handle::try_current().is_some() {
        return Flavor::RustyTokio;
    }
    #[cfg(feature = "tokio")]
    if tokio::runtime::Handle::try_current().is_ok() {
        return Flavor::Tokio;
    }
    Flavor::RustyTokio
}

/// The error [`timeout`] resolves to when `duration` passes before the
/// wrapped future completes -- one shape regardless of which runtime's
/// timer actually fired.
#[derive(Debug)]
pub(crate) struct Elapsed;

/// `rusty_tokio::time::timeout`/`tokio::time::timeout`, whichever the
/// current runtime is.
pub(crate) async fn timeout<F: Future>(
    duration: Duration,
    future: F,
) -> Result<F::Output, Elapsed> {
    match current() {
        Flavor::RustyTokio => rusty_tokio::time::timeout(duration, future)
            .await
            .map_err(|_| Elapsed),
        #[cfg(feature = "tokio")]
        Flavor::Tokio => tokio::time::timeout(duration, future)
            .await
            .map_err(|_| Elapsed),
    }
}

/// `rusty_tokio::time::sleep`/`tokio::time::sleep`, whichever the
/// current runtime is.
pub(crate) async fn sleep(duration: Duration) {
    match current() {
        Flavor::RustyTokio => rusty_tokio::time::sleep(duration).await,
        #[cfg(feature = "tokio")]
        Flavor::Tokio => tokio::time::sleep(duration).await,
    }
}

/// Why a [`spawn_blocking`] task's result never arrived (the runtime's
/// own `JoinError`, flattened to its message so both runtimes' distinct
/// error types fit one signature).
#[derive(Debug)]
pub(crate) struct JoinFailed(String);

impl fmt::Display for JoinFailed {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        f.write_str(&self.0)
    }
}

/// Runs `f` on the current runtime's blocking pool
/// (`rusty_tokio::spawn_blocking`/`tokio::task::spawn_blocking`) and
/// waits for its result.
pub(crate) async fn spawn_blocking<F, T>(f: F) -> Result<T, JoinFailed>
where
    F: FnOnce() -> T + Send + 'static,
    T: Send + 'static,
{
    match current() {
        Flavor::RustyTokio => rusty_tokio::spawn_blocking(f)
            .await
            .map_err(|e| JoinFailed(e.to_string())),
        #[cfg(feature = "tokio")]
        Flavor::Tokio => tokio::task::spawn_blocking(f)
            .await
            .map_err(|e| JoinFailed(e.to_string())),
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn no_runtime_on_this_thread_falls_back_to_rusty_tokio() {
        assert_eq!(current(), Flavor::RustyTokio);
    }

    #[test]
    fn inside_rusty_tokio_detects_rusty_tokio() {
        let rt = rusty_tokio::Runtime::new().expect("rusty_tokio runtime");
        assert_eq!(rt.block_on(async { current() }), Flavor::RustyTokio);
    }

    #[cfg(feature = "tokio")]
    #[test]
    fn inside_real_tokio_detects_tokio() {
        let rt = tokio::runtime::Builder::new_current_thread()
            .enable_all()
            .build()
            .expect("tokio runtime");
        assert_eq!(rt.block_on(async { current() }), Flavor::Tokio);
    }

    /// The exact shape feature unification produces: the `tokio` feature
    /// compiled in, but the task itself running on `rusty_tokio`. The
    /// timer must be `rusty_tokio`'s -- real tokio's would panic here
    /// with "there is no reactor running".
    #[cfg(feature = "tokio")]
    #[test]
    fn rusty_tokio_task_uses_rusty_tokio_timers_even_with_tokio_compiled_in() {
        let rt = rusty_tokio::Runtime::new().expect("rusty_tokio runtime");
        rt.block_on(async {
            sleep(Duration::from_millis(1)).await;
            let hit = timeout(Duration::from_millis(50), std::future::pending::<()>()).await;
            assert!(hit.is_err(), "timeout should have elapsed");
            let ok = timeout(Duration::from_secs(5), async { 7 }).await;
            assert!(matches!(ok, Ok(7)));
            let joined = spawn_blocking(|| 42).await.expect("blocking task joined");
            assert_eq!(joined, 42);
        });
    }
}
