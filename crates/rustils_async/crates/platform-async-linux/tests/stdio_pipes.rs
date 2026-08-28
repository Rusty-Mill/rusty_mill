#![cfg(target_os = "linux")]

//! Exercises `AsyncChild::take_stdin`/`take_stdout` against a real
//! `/bin/cat` child through a real OS pipe — proves the parent-side
//! handles actually round-trip bytes, not just that the API compiles.

use std::pin::pin;
use std::sync::atomic::{AtomicUsize, Ordering};
use std::sync::Arc;
use std::task::{Context, Poll, Wake, Waker};

use platform::process::{Command, Stdio};
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
fn stdin_and_stdout_pipes_round_trip_through_cat() {
    let spawner = AsyncLinuxSpawner::new().expect("construct reactor");
    let cmd = Command {
        stdin: Stdio::Pipe,
        stdout: Stdio::Pipe,
        ..Command::new("/bin/cat", "/")
    };
    let mut child = spawner.spawn(&cmd).expect("spawn /bin/cat");

    let mut stdin = child.take_stdin().expect("stdin was piped");
    // Second call must yield nothing — `Some` exactly once, per contract.
    assert!(child.take_stdin().is_none());

    let mut stdout = child.take_stdout().expect("stdout was piped");
    assert!(child.take_stdout().is_none());

    let message = b"hello async pipes\n";
    let mut written = 0;
    while written < message.len() {
        written += stdin.write(&message[written..]).expect("write to stdin");
    }
    // Drop stdin to deliver EOF, so `cat` stops reading and exits.
    drop(stdin);

    let mut received = Vec::new();
    let mut buf = [0u8; 64];
    loop {
        let n = stdout.read(&mut buf).expect("read from stdout");
        if n == 0 {
            break;
        }
        received.extend_from_slice(&buf[..n]);
    }
    assert_eq!(received, message);

    let status = block_on(child.wait()).expect("async wait");
    assert!(status.success());
}
