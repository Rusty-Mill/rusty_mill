//! Counting the work in flight, so a stop can wait for it and then refuse new work.

use std::sync::{Condvar, Mutex, MutexGuard, PoisonError};
use std::time::{Duration, Instant};

#[derive(Default)]
struct State {
    running: usize,
    closed: bool,
}

/// The requests and background jobs running now. `enter` while open; once a stop has
/// [`InFlight::close_and_wait`]ed, `enter` refuses, so nothing starts that the stop cannot wait for.
#[derive(Default)]
pub struct InFlight {
    state: Mutex<State>,
    idle: Condvar,
}

/// One unit of work in flight; it leaves when dropped.
pub struct Running<'a>(&'a InFlight);

impl InFlight {
    fn state(&self) -> MutexGuard<'_, State> {
        self.state.lock().unwrap_or_else(PoisonError::into_inner)
    }

    /// Start a unit of work, or `None` once the stop has begun.
    pub fn enter(&self) -> Option<Running<'_>> {
        let mut state = self.state();
        if state.closed {
            return None;
        }
        state.running += 1;
        Some(Running(self))
    }

    /// Refuse new work, then wait up to `within` for the work running to finish. `true` if it all did.
    pub fn close_and_wait(&self, within: Duration) -> bool {
        let end = Instant::now() + within;
        let mut state = self.state();
        state.closed = true;
        while state.running > 0 {
            let Some(left) = end.checked_duration_since(Instant::now()) else {
                return false;
            };
            state = self
                .idle
                .wait_timeout(state, left)
                .unwrap_or_else(PoisonError::into_inner)
                .0;
        }
        true
    }
}

impl Drop for Running<'_> {
    fn drop(&mut self) {
        let mut state = self.0.state();
        state.running -= 1;
        if state.running == 0 {
            self.0.idle.notify_all();
        }
    }
}

#[cfg(test)]
mod tests {
    use std::sync::Arc;

    use super::*;

    #[test]
    fn a_stop_waits_for_work_held_across_it_then_refuses_new_work() {
        let flight = Arc::new(InFlight::default());
        let (held, release) = std::sync::mpsc::channel::<()>();
        let worker = {
            let flight = Arc::clone(&flight);
            std::thread::spawn(move || {
                let _running = flight.enter().expect("open");
                held.send(()).expect("signal");
                std::thread::sleep(Duration::from_millis(150));
            })
        };
        release.recv().expect("worker holds a request");
        let started = Instant::now();
        assert!(flight.close_and_wait(Duration::from_secs(5)));
        assert!(started.elapsed() >= Duration::from_millis(100), "it waited");
        assert!(flight.enter().is_none(), "closed for new work");
        worker.join().expect("worker");
    }

    #[test]
    fn a_stop_gives_up_at_its_deadline_on_work_that_never_ends() {
        let flight = InFlight::default();
        let _stuck = flight.enter().expect("open");
        let started = Instant::now();
        assert!(!flight.close_and_wait(Duration::from_millis(100)));
        assert!(started.elapsed() < Duration::from_secs(5));
        assert!(flight.enter().is_none());
    }

    #[test]
    fn an_idle_stop_returns_at_once() {
        assert!(InFlight::default().close_and_wait(Duration::from_secs(5)));
    }
}
