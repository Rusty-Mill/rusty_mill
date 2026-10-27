//! The decision half of [`crate::serve::LimitedHandler`], with no I/O.
//!
//! Counting requests in flight and describing the refusal are pure logic: an
//! atomic counter and some constants. The adapter turns a [`Rejection`] into
//! its own response type.

use std::sync::{
    Arc,
    atomic::{AtomicUsize, Ordering},
};

use rusty_http::StatusCode;

/// `Retry-After` sent with a shed response, in seconds.
///
/// Short on purpose: the point is a brief pause while the backlog clears, not
/// a client that goes away for a minute.
const RETRY_AFTER_SECS: u32 = 1;

/// Why a request was refused, and how to say so.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub(crate) struct Rejection {
    pub(crate) status: StatusCode,
    /// Seconds for a `Retry-After` header, when the client should come back.
    pub(crate) retry_after_secs: Option<u32>,
    pub(crate) body: &'static str,
}

impl Rejection {
    /// The concurrency limit is reached. Shed now rather than queue.
    pub(crate) const OVERLOADED: Self = Self {
        status: StatusCode::SERVICE_UNAVAILABLE,
        retry_after_secs: Some(RETRY_AFTER_SECS),
        body: "Service Unavailable: too many requests in flight",
    };
}

/// A bound on requests in flight. Cheap to clone; clones share one count,
/// which is the whole point — a limit each clone counted separately would
/// bound nothing.
///
/// Admission never waits: over the limit it refuses. A wait here would be a
/// queue, which turns a capacity problem into a latency problem.
#[derive(Debug, Clone)]
pub(crate) struct Admission {
    in_flight: Arc<AtomicUsize>,
    max: usize,
}

impl Admission {
    /// At most `max` in flight. Zero is treated as one, since a limit of zero
    /// would refuse everything.
    pub(crate) fn new(max: usize) -> Self {
        Self {
            in_flight: Arc::new(AtomicUsize::new(0)),
            max: max.max(1),
        }
    }

    /// Slots free right now.
    pub(crate) fn available(&self) -> usize {
        self.max
            .saturating_sub(self.in_flight.load(Ordering::Acquire))
    }

    /// Take a slot, or refuse with [`Rejection::OVERLOADED`]. The slot is
    /// returned when the [`Permit`] drops.
    pub(crate) fn try_admit(&self) -> Result<Permit, Rejection> {
        self.in_flight
            .fetch_update(Ordering::AcqRel, Ordering::Acquire, |n| {
                (n < self.max).then_some(n + 1)
            })
            .map(|_| Permit {
                in_flight: Arc::clone(&self.in_flight),
            })
            .map_err(|_| Rejection::OVERLOADED)
    }
}

/// A slot in flight, held until dropped.
#[derive(Debug)]
pub(crate) struct Permit {
    in_flight: Arc<AtomicUsize>,
}

impl Drop for Permit {
    fn drop(&mut self) {
        self.in_flight.fetch_sub(1, Ordering::AcqRel);
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn admits_up_to_the_limit_then_refuses() {
        let admission = Admission::new(2);
        let _a = admission.try_admit().expect("first");
        let _b = admission.try_admit().expect("second");
        assert_eq!(
            admission.try_admit().expect_err("over the limit"),
            Rejection::OVERLOADED
        );
    }

    #[test]
    fn dropping_a_permit_frees_its_slot() {
        let admission = Admission::new(1);
        for _ in 0..3 {
            let permit = admission.try_admit().expect("a leaked slot would refuse");
            assert_eq!(admission.available(), 0);
            drop(permit);
            assert_eq!(admission.available(), 1);
        }
    }

    #[test]
    fn a_refusal_takes_no_slot() {
        // Otherwise being overloaded would make recovery harder.
        let admission = Admission::new(1);
        let held = admission.try_admit().expect("first");
        for _ in 0..5 {
            assert!(admission.try_admit().is_err());
        }
        drop(held);
        assert_eq!(admission.available(), 1);
    }

    #[test]
    fn clones_share_one_count() {
        let admission = Admission::new(1);
        let other = admission.clone();
        let _held = admission.try_admit().expect("first");
        assert!(other.try_admit().is_err());
    }

    #[test]
    fn a_limit_of_zero_is_clamped_to_one() {
        let admission = Admission::new(0);
        assert_eq!(admission.available(), 1);
        let _held = admission.try_admit().expect("one slot");
        assert!(admission.try_admit().is_err(), "and only one");
    }

    #[test]
    fn concurrent_admission_never_exceeds_the_limit() {
        // The check and the increment are one atomic step; a load then store
        // would let two threads both take the last slot.
        const MAX: usize = 4;
        let admission = Admission::new(MAX);
        let admitted = Arc::new(AtomicUsize::new(0));
        let barrier = Arc::new(std::sync::Barrier::new(16));

        let threads: Vec<_> = (0..16)
            .map(|_| {
                let (admission, admitted, barrier) =
                    (admission.clone(), admitted.clone(), barrier.clone());
                std::thread::spawn(move || {
                    barrier.wait();
                    admission.try_admit().ok().inspect(|_| {
                        admitted.fetch_add(1, Ordering::SeqCst);
                    })
                })
            })
            .collect();
        let permits: Vec<_> = threads
            .into_iter()
            .filter_map(|t| t.join().expect("thread"))
            .collect();

        assert_eq!(permits.len(), MAX);
        assert_eq!(admitted.load(Ordering::SeqCst), MAX);
    }

    #[test]
    fn the_refusal_says_what_the_adapter_must_send() {
        assert_eq!(
            Rejection::OVERLOADED.status,
            StatusCode::SERVICE_UNAVAILABLE
        );
        assert_eq!(Rejection::OVERLOADED.retry_after_secs, Some(1));
    }
}
