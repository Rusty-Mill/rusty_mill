//! Channels whose receiver is a [`Stream`](crate::Stream).

/// Multi-producer, single-consumer channels.
pub mod mpsc {
    use std::collections::VecDeque;
    use std::fmt;
    use std::pin::Pin;
    use std::sync::{Arc, Mutex, MutexGuard};
    use std::task::{Context, Poll, Waker};

    use crate::Stream;

    struct Inner<T> {
        queue: VecDeque<T>,
        receiver_waker: Option<Waker>,
        senders: usize,
        receiver_alive: bool,
    }

    type Shared<T> = Arc<Mutex<Inner<T>>>;

    fn lock<T>(shared: &Shared<T>) -> MutexGuard<'_, Inner<T>> {
        // The queue holds plain data, so a panic elsewhere cannot leave it
        // inconsistent; recover from poisoning.
        shared
            .lock()
            .unwrap_or_else(|poisoned| poisoned.into_inner())
    }

    /// An unbounded channel. Sending never waits; the receiver yields values
    /// in send order and ends once every sender is dropped.
    pub fn unbounded<T>() -> (UnboundedSender<T>, UnboundedReceiver<T>) {
        let shared = Arc::new(Mutex::new(Inner {
            queue: VecDeque::new(),
            receiver_waker: None,
            senders: 1,
            receiver_alive: true,
        }));
        (
            UnboundedSender {
                shared: Arc::clone(&shared),
            },
            UnboundedReceiver { shared },
        )
    }

    /// The error from [`UnboundedSender::unbounded_send`]: the receiver is
    /// gone. Holds the value that was not sent.
    #[derive(PartialEq, Eq)]
    pub struct SendError<T>(pub T);

    impl<T> SendError<T> {
        /// The value that was not sent.
        pub fn into_inner(self) -> T {
            self.0
        }
    }

    impl<T> fmt::Debug for SendError<T> {
        fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
            f.debug_struct("SendError").finish_non_exhaustive()
        }
    }

    impl<T> fmt::Display for SendError<T> {
        fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
            f.write_str("send failed because the receiver is gone")
        }
    }

    impl<T> std::error::Error for SendError<T> {}

    /// The sending half of [`unbounded`]. Cloneable.
    pub struct UnboundedSender<T> {
        shared: Shared<T>,
    }

    impl<T> UnboundedSender<T> {
        /// Queue `value` for the receiver, or hand it back if the receiver
        /// has been dropped.
        pub fn unbounded_send(&self, value: T) -> Result<(), SendError<T>> {
            let mut inner = lock(&self.shared);
            if !inner.receiver_alive {
                return Err(SendError(value));
            }
            inner.queue.push_back(value);
            if let Some(waker) = inner.receiver_waker.take() {
                waker.wake();
            }
            Ok(())
        }

        /// Whether the receiver has been dropped.
        pub fn is_closed(&self) -> bool {
            !lock(&self.shared).receiver_alive
        }
    }

    impl<T> Clone for UnboundedSender<T> {
        fn clone(&self) -> Self {
            lock(&self.shared).senders += 1;
            Self {
                shared: Arc::clone(&self.shared),
            }
        }
    }

    impl<T> Drop for UnboundedSender<T> {
        fn drop(&mut self) {
            let mut inner = lock(&self.shared);
            inner.senders -= 1;
            if inner.senders == 0 {
                if let Some(waker) = inner.receiver_waker.take() {
                    waker.wake();
                }
            }
        }
    }

    impl<T> fmt::Debug for UnboundedSender<T> {
        fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
            f.debug_struct("UnboundedSender").finish_non_exhaustive()
        }
    }

    /// The receiving half of [`unbounded`]: a [`Stream`] of the sent values.
    pub struct UnboundedReceiver<T> {
        shared: Shared<T>,
    }

    impl<T> Stream for UnboundedReceiver<T> {
        type Item = T;

        fn poll_next(self: Pin<&mut Self>, cx: &mut Context<'_>) -> Poll<Option<T>> {
            let mut inner = lock(&self.shared);
            if let Some(value) = inner.queue.pop_front() {
                return Poll::Ready(Some(value));
            }
            if inner.senders == 0 {
                return Poll::Ready(None);
            }
            inner.receiver_waker = Some(cx.waker().clone());
            Poll::Pending
        }
    }

    impl<T> Drop for UnboundedReceiver<T> {
        fn drop(&mut self) {
            let mut inner = lock(&self.shared);
            inner.receiver_alive = false;
            inner.queue.clear();
        }
    }

    impl<T> fmt::Debug for UnboundedReceiver<T> {
        fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
            f.debug_struct("UnboundedReceiver").finish_non_exhaustive()
        }
    }
}
