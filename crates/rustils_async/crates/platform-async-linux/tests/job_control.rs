//! Exercises `AsyncChild::wait_job`/`try_wait_job` against a real child —
//! stop, continue, then terminate — and confirms the reap-state stash is
//! shared correctly with the plain `wait`/`try_wait` path.

use std::pin::pin;
use std::sync::atomic::{AtomicUsize, Ordering};
use std::sync::Arc;
use std::task::{Context, Poll, Wake, Waker};
use std::time::{Duration, Instant};

use platform::process::{Command, ExitStatus, Signal};
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

fn block_on<F: std::future::Future>(fut: F) -> F::Output {
    let woken = Arc::new(CountingWaker(AtomicUsize::new(0)));
    let waker = Waker::from(Arc::clone(&woken));
    let mut cx = Context::from_waker(&waker);
    let mut fut = pin!(fut);
    loop {
        match fut.as_mut().poll(&mut cx) {
            Poll::Ready(v) => return v,
            Poll::Pending => {
                let deadline = Instant::now() + Duration::from_secs(5);
                while woken.0.load(Ordering::SeqCst) == 0 {
                    assert!(
                        Instant::now() < deadline,
                        "wait_job's background thread never woke the future"
                    );
                    std::thread::sleep(Duration::from_millis(5));
                }
                woken.0.store(0, Ordering::SeqCst);
            }
        }
    }
}

#[test]
fn wait_job_observes_stop_then_continue_then_terminate() {
    let spawner = AsyncLinuxSpawner::new().expect("construct reactor");
    let mut child = spawner
        .spawn(&Command::new("/bin/sleep", "/").arg("30"))
        .expect("spawn");

    child
        .kill_single(Signal::Stop)
        .expect("send SIGSTOP to child");
    let status = block_on(child.wait_job()).expect("wait_job after stop");
    assert!(
        matches!(status, ExitStatus::Stopped(_)),
        "expected Stopped, got {status:?}"
    );

    child
        .kill_single(Signal::Cont)
        .expect("send SIGCONT to child");
    let status = block_on(child.wait_job()).expect("wait_job after continue");
    assert_eq!(status, ExitStatus::Continued);

    child.kill_single(Signal::Kill).expect("kill child");
    let status = block_on(child.wait_job()).expect("wait_job after kill");
    assert!(
        matches!(status, ExitStatus::Signaled(_)),
        "expected Signaled, got {status:?}"
    );

    // The reap-state stash must be shared: a later call through the
    // plain (non-job-control) path must see the same cached terminal
    // status, not re-waitpid an already-reaped pid.
    let status_again = block_on(child.wait()).expect("wait after wait_job already reaped it");
    assert!(matches!(status_again, ExitStatus::Signaled(_)));
}

#[test]
fn try_wait_job_is_non_blocking_and_none_while_running() {
    let spawner = AsyncLinuxSpawner::new().expect("construct reactor");
    let mut child = spawner
        .spawn(&Command::new("/bin/sleep", "/").arg("30"))
        .expect("spawn");

    assert_eq!(child.try_wait_job().expect("try_wait_job"), None);

    child.kill_single(Signal::Kill).expect("kill child");
    // Give the kernel a moment to actually terminate the process before
    // polling — try_wait_job must not block, so a fast no-answer here
    // would be a false negative, not a bug in the method itself.
    let deadline = Instant::now() + Duration::from_secs(5);
    loop {
        if let Some(status) = child.try_wait_job().expect("try_wait_job") {
            assert!(matches!(status, ExitStatus::Signaled(_)));
            break;
        }
        assert!(Instant::now() < deadline, "child never reported terminated");
        std::thread::sleep(Duration::from_millis(5));
    }
}
