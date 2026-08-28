//! # coreutils-async — the forcing consumer for `platform-async`
//!
//! Exists so this workspace's async trait surface has at least one real
//! caller (`arun`, `src/bin/arun.rs`) even though `docs/adr/0001-native-async-rustils.md`
//! records that this repository was not built for a named consumer in
//! the first place. See that ADR for the full reasoning.

#![forbid(unsafe_code)]

use std::future::Future;
use std::pin::pin;
use std::sync::Arc;
use std::task::{Context, Poll, Wake, Waker};

/// A minimal, single-task, disclosed executor — deliberately not a
/// general-purpose runtime (matches `reactor-core`'s own no-hidden-
/// runtime stance: this crate is a consumer, not another runtime for
/// other consumers to depend on). Sufficient to drive exactly one
/// future to completion on the calling thread, which is all a CLI's
/// `main` needs. Parks the thread between polls rather than busy-
/// looping; the `platform-async-linux` reactor thread wakes it via the
/// standard `Waker` contract.
pub fn block_on<F: Future>(fut: F) -> F::Output {
    struct ThreadWaker(std::thread::Thread);

    impl Wake for ThreadWaker {
        fn wake(self: Arc<Self>) {
            self.0.unpark();
        }

        fn wake_by_ref(self: &Arc<Self>) {
            self.0.unpark();
        }
    }

    let waker = Waker::from(Arc::new(ThreadWaker(std::thread::current())));
    let mut cx = Context::from_waker(&waker);
    let mut fut = pin!(fut);
    loop {
        match fut.as_mut().poll(&mut cx) {
            Poll::Ready(v) => return v,
            Poll::Pending => std::thread::park(),
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn block_on_drives_a_ready_future_to_completion() {
        assert_eq!(block_on(std::future::ready(42)), 42);
    }

    #[test]
    fn block_on_parks_until_woken() {
        // A future that is Pending exactly once, waking itself from a
        // background thread — proves block_on actually parks and
        // resumes rather than only handling the trivial first-poll-
        // ready case.
        struct WakeLater {
            polled: bool,
        }
        impl Future for WakeLater {
            type Output = &'static str;
            fn poll(
                mut self: std::pin::Pin<&mut Self>,
                cx: &mut Context<'_>,
            ) -> Poll<Self::Output> {
                if self.polled {
                    return Poll::Ready("done");
                }
                self.polled = true;
                let waker = cx.waker().clone();
                std::thread::spawn(move || {
                    std::thread::sleep(std::time::Duration::from_millis(20));
                    waker.wake();
                });
                Poll::Pending
            }
        }
        assert_eq!(block_on(WakeLater { polled: false }), "done");
    }
}
