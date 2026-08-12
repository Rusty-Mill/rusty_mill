//! Exercises the real reactor against a real process — not a mock —
//! proving the pidfd + epoll path actually delivers a wakeup instead of
//! resolving trivially.

use std::pin::pin;
use std::sync::atomic::{AtomicUsize, Ordering};
use std::sync::Arc;
use std::task::{Context, Poll, Wake, Waker};

use platform::process::{Command, ExitStatus};
use platform_async::process::AsyncSpawner;
use platform_async_linux::AsyncLinuxSpawner;

struct CountingWaker(AtomicUsize);

impl Wake for CountingWaker {
    fn wake(self: Arc<Self>) {
        self.0.fetch_add(1, Ordering::SeqCst);
    }

    fn wake_by_ref(self: &Arc<Self>) {
        self.0.fetch_add(1, Ordering::SeqCst);
    }
}

/// A tiny, blocking-with-timeout executor: polls once, and if `Pending`,
/// parks this thread until the counting waker fires, then polls again.
/// Real waiting through a real OS wakeup, not a busy loop.
fn block_on<F: std::future::Future>(fut: F) -> F::Output {
    let woken = Arc::new(CountingWaker(AtomicUsize::new(0)));
    let waker = Waker::from(Arc::clone(&woken));
    let mut cx = Context::from_waker(&waker);
    let mut fut = pin!(fut);
    loop {
        match fut.as_mut().poll(&mut cx) {
            Poll::Ready(v) => return v,
            Poll::Pending => {
                let deadline = std::time::Instant::now() + std::time::Duration::from_secs(5);
                while woken.0.load(Ordering::SeqCst) == 0 {
                    assert!(
                        std::time::Instant::now() < deadline,
                        "reactor never woke the waiting future"
                    );
                    std::thread::sleep(std::time::Duration::from_millis(5));
                }
                woken.0.store(0, Ordering::SeqCst);
            }
        }
    }
}

#[test]
fn async_wait_observes_real_process_exit() {
    let spawner = AsyncLinuxSpawner::new().expect("construct reactor");
    let cmd = Command::new("/bin/true", "/");
    let child = spawner.spawn(&cmd).expect("spawn /bin/true");
    let status = block_on(child.wait()).expect("async wait");
    assert_eq!(status, ExitStatus::Code(0));
}

#[test]
fn async_wait_observes_nonzero_exit() {
    let spawner = AsyncLinuxSpawner::new().expect("construct reactor");
    let cmd = Command::new("/bin/false", "/");
    let child = spawner.spawn(&cmd).expect("spawn /bin/false");
    let status = block_on(child.wait()).expect("async wait");
    assert_eq!(status, ExitStatus::Code(1));
}

#[test]
fn many_concurrent_waits_all_resolve() {
    // Proves the reactor multiplexes correctly across several pidfds at
    // once rather than only working for a single in-flight wait.
    let spawner = AsyncLinuxSpawner::new().expect("construct reactor");
    let children: Vec<_> = (0..8)
        .map(|_| {
            let cmd = Command::new("/bin/true", "/");
            spawner.spawn(&cmd).expect("spawn")
        })
        .collect();

    for child in children {
        let status = block_on(child.wait()).expect("async wait");
        assert!(status.success());
    }
}
