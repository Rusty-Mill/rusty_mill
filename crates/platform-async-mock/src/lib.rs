//! In-memory async process backend, wrapping rustils' `platform-mock`
//! [`MockSpawner`](platform_mock::process::MockSpawner) rather than
//! reimplementing scripted-response logic. A scripted child has nothing
//! real to wait for, so [`AsyncChild::wait`] resolves immediately —
//! there is no reactor here, by design; this crate exists purely so
//! consumer logic can be tested against the async trait surface without
//! spawning real processes.

#![forbid(unsafe_code)]

use std::ffi::{OsStr, OsString};

use platform::error::Result;
use platform::process::{Child, Command, ExitStatus, GroupHandle, Signal, Spawner};
use platform_async::process::{AsyncChild, AsyncSpawner, BoxFuture};
use platform_mock::process::MockSpawner;

/// Adapts [`MockSpawner`] to [`AsyncSpawner`].
pub struct AsyncMockSpawner {
    inner: MockSpawner,
}

impl AsyncMockSpawner {
    pub fn new() -> Self {
        Self {
            inner: MockSpawner::new(),
        }
    }

    /// Register a scripted response, same contract as
    /// [`MockSpawner::script`].
    #[must_use]
    pub fn script(mut self, program: impl Into<OsString>, status: ExitStatus) -> Self {
        self.inner = self.inner.script(program, status);
        self
    }

    /// Access to the wrapped [`MockSpawner`] for spawn-log assertions —
    /// the same surface consumer tests already use against the sync
    /// mock.
    pub fn inner(&self) -> &MockSpawner {
        &self.inner
    }
}

impl Default for AsyncMockSpawner {
    fn default() -> Self {
        Self::new()
    }
}

impl AsyncSpawner for AsyncMockSpawner {
    fn spawn(&self, cmd: &Command) -> Result<Box<dyn AsyncChild>> {
        let child = self.inner.spawn(cmd)?;
        Ok(Box::new(AsyncMockChild { inner: child }))
    }

    fn resolve(&self, program: &OsStr) -> Result<OsString> {
        self.inner.resolve(program)
    }

    fn adopt(&self, pid: u32) -> Result<Box<dyn GroupHandle>> {
        self.inner.adopt(pid)
    }

    fn is_alive(&self, pid: u32) -> Result<bool> {
        self.inner.is_alive(pid)
    }
}

struct AsyncMockChild {
    inner: Box<dyn Child>,
}

impl AsyncChild for AsyncMockChild {
    fn wait(self: Box<Self>) -> BoxFuture<'static, Result<ExitStatus>> {
        // A scripted mock child's status is already decided the instant
        // it was spawned — there is nothing to actually wait for, so
        // this resolves on first poll rather than registering with any
        // reactor.
        let result = self.inner.wait();
        Box::pin(std::future::ready(result))
    }

    fn id(&self) -> u32 {
        self.inner.id()
    }

    fn kill_tree(&self, sig: Signal) -> Result<()> {
        self.inner.kill_tree(sig)
    }

    fn kill_single(&self, sig: Signal) -> Result<()> {
        self.inner.kill_single(sig)
    }

    fn try_wait(&mut self) -> Result<Option<ExitStatus>> {
        self.inner.try_wait()
    }

    fn ready(&self) -> BoxFuture<'_, Result<()>> {
        // Same reasoning as `wait`: a scripted child's status is already
        // decided at spawn time, so there is nothing to actually wait
        // for — this always resolves on first poll.
        Box::pin(std::future::ready(Ok(())))
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn block_on<F: std::future::Future>(fut: F) -> F::Output {
        // Minimal, single-threaded, no-op-waker executor — sufficient
        // for a future that resolves on first poll (every future this
        // crate produces). Not a general-purpose executor; this crate
        // deliberately does not depend on one (see the crate doc
        // comment and `reactor-core`'s own no-hidden-runtime stance).
        // Built entirely from safe APIs (`Wake` + `pin!`) so this test
        // stays inside the crate's own `forbid(unsafe_code)`.
        use std::pin::pin;
        use std::sync::Arc;
        use std::task::{Context, Poll, Wake, Waker};

        struct NoopWaker;
        impl Wake for NoopWaker {
            fn wake(self: Arc<Self>) {}
        }

        let waker = Waker::from(Arc::new(NoopWaker));
        let mut cx = Context::from_waker(&waker);
        let mut fut = pin!(fut);
        match fut.as_mut().poll(&mut cx) {
            Poll::Ready(v) => v,
            Poll::Pending => panic!("mock child future did not resolve on first poll"),
        }
    }

    #[test]
    fn spawn_and_wait_resolves_scripted_status() {
        let spawner = AsyncMockSpawner::new().script("echo", ExitStatus::Code(0));
        let cmd = Command::new("echo", "/");
        let child = spawner.spawn(&cmd).expect("spawn");
        let status = block_on(child.wait()).expect("wait");
        assert!(status.success());
    }

    #[test]
    fn wait_any_reports_a_terminated_child_by_index() {
        let spawner = AsyncMockSpawner::new()
            .script("a", ExitStatus::Code(0))
            .script("b", ExitStatus::Code(1));
        let a = spawner.spawn(&Command::new("a", "/")).expect("spawn a");
        let b = spawner.spawn(&Command::new("b", "/")).expect("spawn b");
        let mut children: Vec<Box<dyn AsyncChild>> = vec![a, b];

        let index = block_on(spawner.wait_any(&mut children, None))
            .expect("wait_any")
            .expect("Some(index), not a timeout");
        // Both scripted children are "ready" immediately; the first one
        // in the slice wins.
        assert_eq!(index, 0);
        let wait_fut = children.remove(index).wait();
        assert!(block_on(wait_fut).unwrap().success());
    }

    #[test]
    fn wait_any_rejects_an_empty_slice() {
        let spawner = AsyncMockSpawner::new();
        let mut children: Vec<Box<dyn AsyncChild>> = Vec::new();
        let err = block_on(spawner.wait_any(&mut children, None)).expect_err("must fail");
        assert_eq!(err.kind, platform::error::ErrorKind::InvalidInput);
    }
}
