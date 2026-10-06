//! Custom log sink — Rust-idiomatic equivalent of whisper.cpp's
//! `whisper_log_set`. A settable closure hook, not a dependency on the
//! `log` crate (this project is zero-dependency): default behavior mirrors
//! whisper.cpp's own default sink, writing messages to stderr.
//!
//! The installed sink is never run, and never dropped, while the slot's
//! lock is held: [`log`] clones the shared handle out of the slot and
//! releases the lock before calling, and [`set_log_sink`] /
//! [`reset_log_sink`] swap the handle out and let the previous sink drop
//! after the lock is released. A sink may therefore log again, replace
//! itself, or own values whose destructors log, without deadlocking, and
//! a sink that panics cannot leave the slot poisoned for the rest of the
//! process. The flip side of cloning the handle out: a `log` call that
//! already took its clone keeps using that sink even if another thread
//! replaces it meanwhile, so the old sink is only truly gone once every
//! such in-flight call has returned.

use std::sync::{Arc, Mutex, OnceLock, PoisonError};

type LogFn = Arc<dyn Fn(&str) + Send + Sync>;

fn default_sink(msg: &str) {
    eprintln!("{msg}");
}

static SINK: OnceLock<Mutex<LogFn>> = OnceLock::new();

fn sink() -> &'static Mutex<LogFn> {
    SINK.get_or_init(|| Mutex::new(Arc::new(default_sink)))
}

/// Replaces the installed sink and returns the previous one, so the
/// caller can drop it after the lock is gone. A poisoned lock (a sink
/// that panicked while being swapped) is recovered rather than
/// propagated: the slot only ever holds a valid handle.
fn swap_sink(next: LogFn) -> LogFn {
    let mut guard = sink().lock().unwrap_or_else(PoisonError::into_inner);
    std::mem::replace(&mut *guard, next)
}

/// Install a custom log sink, replacing the default (stderr) one —
/// `whisper_log_set`. Applies to every subsequent log call for the
/// lifetime of the process (or until [`reset_log_sink`] is called).
///
/// The sink is invoked with no lock held, so it may run on several
/// threads at once (every logging thread calls it directly, including
/// the audio capture thread) and must not rely on being serialised. It
/// may itself call [`set_log_sink`] or [`reset_log_sink`]; the sink
/// installed that way takes effect for the *next* message. Replacement
/// is not a barrier: a concurrent `log` call that has already picked up
/// the previous sink finishes with it, and the previous sink is dropped
/// once the last such call releases its handle.
pub fn set_log_sink(f: impl Fn(&str) + Send + Sync + 'static) {
    let previous = swap_sink(Arc::new(f));
    drop(previous);
}

/// Restore the default stderr sink.
pub fn reset_log_sink() {
    let previous = swap_sink(Arc::new(default_sink));
    drop(previous);
}

/// Route a message through the currently installed sink. Library code
/// calls this instead of `eprintln!` directly so callers can intercept,
/// silence, or redirect it via [`set_log_sink`].
pub(crate) fn log(msg: impl AsRef<str>) {
    let current = sink()
        .lock()
        .unwrap_or_else(PoisonError::into_inner)
        .clone();
    current(msg.as_ref());
}

#[cfg(test)]
mod tests {
    use super::*;
    use std::process::{Command, Stdio};
    use std::sync::atomic::{AtomicUsize, Ordering};
    use std::sync::{Arc, Mutex as StdMutex};
    use std::time::{Duration, Instant};

    #[test]
    fn set_log_sink_intercepts_and_reset_restores_default() {
        // One test covering set -> log -> assert -> reset: the sink is a
        // process-global static, so keeping the whole sequence in a single
        // test avoids racing against other tests that might touch it.
        let captured = Arc::new(StdMutex::new(Vec::<String>::new()));
        let captured_clone = captured.clone();
        set_log_sink(move |msg| captured_clone.lock().unwrap().push(msg.to_string()));

        log("hello");
        log(format!("world {}", 42));

        // Other tests running concurrently may log through this
        // process-global sink too (e.g. model loading) while it's
        // installed here, so check our own messages appear in order
        // rather than asserting an exact, possibly-interleaved vector.
        let got = captured.lock().unwrap();
        let hello_pos = got.iter().position(|m| m == "hello");
        let world_pos = got.iter().position(|m| m == "world 42");
        assert!(hello_pos.is_some() && world_pos.is_some());
        assert!(hello_pos.unwrap() < world_pos.unwrap());
        drop(got);

        reset_log_sink();
        // No assertion on stderr output itself (nothing to capture without
        // process-level redirection); this just verifies reset doesn't panic
        // and a subsequent log() call runs through the default sink path.
        log("back to stderr");
    }

    // ---------------------------------------------------------------
    // Scenarios that a regression would turn into a hang or a poisoned
    // global run in a child process: this same test binary re-executed
    // with one scenario selected by environment variable and a wall-clock
    // bound. A deadlock becomes a timeout failure in the parent, and a
    // panic cannot poison the sink the other tests in this process share.
    // Cleanup of a hung child is itself bounded and best-effort: the
    // failure message says whether it was reaped.
    // ---------------------------------------------------------------

    const SCENARIO_VAR: &str = "RUSTY_WHISPER_LOG_SCENARIO";
    const CHILD_TIMEOUT: Duration = Duration::from_secs(20);

    /// How long cleanup of a misbehaving child may take before it is
    /// given up on and reported as still running.
    const CLEANUP_TIMEOUT: Duration = Duration::from_secs(5);

    /// The two process operations cleanup needs, so the bounded cleanup
    /// below can be tested against a child that refuses to die.
    trait Terminable {
        fn kill(&mut self) -> std::io::Result<()>;
        fn try_wait(&mut self) -> std::io::Result<Option<std::process::ExitStatus>>;
    }

    impl Terminable for std::process::Child {
        fn kill(&mut self) -> std::io::Result<()> {
            std::process::Child::kill(self)
        }
        fn try_wait(&mut self) -> std::io::Result<Option<std::process::ExitStatus>> {
            std::process::Child::try_wait(self)
        }
    }

    /// Attempts to terminate `child` and reap it within `deadline`,
    /// polling rather than blocking so a child that is deadlocked and
    /// cannot be killed does not hang the test. Returns a description of
    /// the outcome; cleanup can fail, and when it does the description
    /// says so explicitly instead of claiming the child is gone.
    fn terminate(child: &mut impl Terminable, deadline: Duration) -> String {
        let kill = child.kill();
        let started = Instant::now();
        let reaped = loop {
            match child.try_wait() {
                Ok(Some(status)) => break Ok(Some(status)),
                Ok(None) if started.elapsed() >= deadline => break Ok(None),
                Ok(None) => std::thread::sleep(Duration::from_millis(10)),
                Err(e) => break Err(e),
            }
        };
        match (kill, reaped) {
            (Ok(()), Ok(Some(status))) => format!("child killed and reaped ({status})"),
            (Ok(()), Ok(None)) => {
                format!("child killed but NOT reaped within {deadline:?}; it may still be running")
            }
            (Ok(()), Err(e)) => {
                format!("child killed but reaping failed: {e}; it may still be running")
            }
            (Err(k), Ok(Some(status))) => {
                format!("kill failed ({k}) but the child exited anyway ({status})")
            }
            (Err(k), Ok(None)) => {
                format!("kill FAILED ({k}) and the child is still running after {deadline:?}")
            }
            (Err(k), Err(w)) => {
                format!(
                    "kill FAILED ({k}) and polling failed ({w}); the child may still be running"
                )
            }
        }
    }

    #[test]
    fn terminate_reports_a_child_that_cannot_be_killed_within_the_deadline() {
        struct Unkillable;
        impl Terminable for Unkillable {
            fn kill(&mut self) -> std::io::Result<()> {
                Err(std::io::Error::from(std::io::ErrorKind::PermissionDenied))
            }
            fn try_wait(&mut self) -> std::io::Result<Option<std::process::ExitStatus>> {
                Ok(None)
            }
        }
        let started = Instant::now();
        let report = terminate(&mut Unkillable, Duration::from_millis(60));
        assert!(
            started.elapsed() < Duration::from_secs(2),
            "cleanup must be bounded"
        );
        assert!(report.contains("kill FAILED"), "{report}");
        assert!(report.contains("still running"), "{report}");
    }

    /// Runs `scenario` in a child process and asserts it exits cleanly
    /// within the timeout. In the child (selected by `SCENARIO_VAR`), runs
    /// the scenario body itself and returns `true` so the caller stops.
    /// Every failure path (timeout, poll error) attempts a bounded
    /// termination of the child before panicking and reports the outcome,
    /// including when termination or reaping did not succeed.
    fn run_isolated(scenario: &str, body: fn()) -> bool {
        if std::env::var(SCENARIO_VAR).as_deref() == Ok(scenario) {
            body();
            return true;
        }
        let test_name = format!("log::tests::{scenario}");
        let exe = std::env::current_exe().expect("locate the running test binary");
        let mut child = Command::new(exe)
            .args(["--exact", &test_name, "--nocapture", "--test-threads=1"])
            .env(SCENARIO_VAR, scenario)
            .stdout(Stdio::null())
            .stderr(Stdio::piped())
            .spawn()
            .expect("spawn child test process");
        let started = Instant::now();
        loop {
            match child.try_wait() {
                Ok(Some(status)) => {
                    let output = child.wait_with_output().expect("collect child output");
                    let stderr = String::from_utf8_lossy(&output.stderr);
                    assert!(
                        status.success(),
                        "child scenario `{scenario}` failed ({status}):\n{stderr}"
                    );
                    return false;
                }
                Ok(None) => {}
                Err(poll_error) => {
                    let cleanup = terminate(&mut child, CLEANUP_TIMEOUT);
                    panic!("polling child scenario `{scenario}` failed: {poll_error}; {cleanup}");
                }
            }
            if started.elapsed() > CHILD_TIMEOUT {
                let cleanup = terminate(&mut child, CLEANUP_TIMEOUT);
                panic!("child scenario `{scenario}` hung: lock held while the sink ran? {cleanup}");
            }
            std::thread::sleep(Duration::from_millis(20));
        }
    }

    #[test]
    fn reentrant_sink_does_not_deadlock() {
        run_isolated("reentrant_sink_does_not_deadlock", || {
            // The sink logs once more from inside itself (bounded by a
            // depth counter) and then replaces itself: both re-enter the
            // slot while this sink is executing.
            let depth = Arc::new(AtomicUsize::new(0));
            let seen = Arc::new(StdMutex::new(Vec::<String>::new()));
            let (depth_in, seen_in) = (depth.clone(), seen.clone());
            set_log_sink(move |msg| {
                seen_in.lock().unwrap().push(msg.to_string());
                if depth_in.fetch_add(1, Ordering::SeqCst) == 0 {
                    log("nested");
                    reset_log_sink();
                }
            });
            log("outer");
            let got = seen.lock().unwrap();
            assert_eq!(got.as_slice(), ["outer", "nested"]);
            drop(got);
            // The reset installed from inside the sink is in effect now.
            log("default sink again");
        });
    }

    #[test]
    fn replaced_sink_is_dropped_outside_the_lock() {
        run_isolated("replaced_sink_is_dropped_outside_the_lock", || {
            // A value captured by the sink logs from its destructor. The
            // old sink (and its captures) must be dropped after the slot's
            // lock is released, or this deadlocks.
            struct LogsOnDrop;
            impl Drop for LogsOnDrop {
                fn drop(&mut self) {
                    log("dropping the old sink");
                }
            }
            let token = LogsOnDrop;
            set_log_sink(move |_msg| {
                let _keep = &token;
            });
            let seen = Arc::new(StdMutex::new(Vec::<String>::new()));
            let seen_in = seen.clone();
            // Replacing the sink drops `token`, whose destructor logs into
            // the newly installed sink.
            set_log_sink(move |msg| seen_in.lock().unwrap().push(msg.to_string()));
            assert_eq!(seen.lock().unwrap().as_slice(), ["dropping the old sink"]);
            reset_log_sink();
        });
    }

    #[test]
    fn panicking_sink_does_not_poison_logging() {
        run_isolated("panicking_sink_does_not_poison_logging", || {
            set_log_sink(|_msg| panic!("sink exploded"));
            let result = std::panic::catch_unwind(|| log("boom"));
            assert!(
                result.is_err(),
                "the panicking sink must propagate its panic"
            );
            // Logging and replacing the sink still work afterwards.
            let seen = Arc::new(StdMutex::new(Vec::<String>::new()));
            let seen_in = seen.clone();
            set_log_sink(move |msg| seen_in.lock().unwrap().push(msg.to_string()));
            log("after the panic");
            assert_eq!(seen.lock().unwrap().as_slice(), ["after the panic"]);
            reset_log_sink();
            log("default sink works too");
        });
    }
}
