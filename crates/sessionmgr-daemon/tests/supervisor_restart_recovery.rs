//! **Phase 1's exit criterion.**
//!
//! This file is the acceptance test for the entire persistence design.
//! Everything else in the project is in service of the property proved
//! here: *the manager application closing does not stop the work*.
//!
//! The scenario is the real one, not an approximation of it. A session is
//! started, the daemon is killed uncleanly from outside (no graceful
//! shutdown, no chance to clean up -- the way closing an app or a crash
//! actually behaves), and then a new daemon is started and must find the
//! work still running and adopt it.
//!
//! PLAN.md § Verification also specifies this be run by hand once against
//! a real Windows box with `taskkill`; see `docs/phase-1-report.md`.

mod common;

use std::time::Duration;

use common::*;

#[test]
fn a_session_survives_the_daemon_being_killed_and_is_adopted_by_its_replacement() {
    let root = TempRoot::new("restart-recovery");
    let id = session_new(root.path(), &long_running());

    let worker = worker_pid(root.path(), &id);
    let child = child_pid(root.path(), &id);
    let daemon = daemon_pid(root.path()).expect("a daemon should be recorded after `new`");
    assert!(is_alive(worker), "the worker should be running");
    assert_eq!(session_status(root.path(), &id), "running");

    // Kill the manager the way a user closing the app (or a crash) would:
    // no warning, no graceful path.
    force_kill(daemon);
    assert!(
        wait_until(|| !is_alive(daemon), Duration::from_secs(10)),
        "the daemon should be gone after being killed"
    );

    // The whole point. The work is still running even though the thing
    // that started it is gone.
    assert!(
        is_alive(worker),
        "the worker MUST survive the daemon being killed -- this is the product's central promise"
    );
    assert!(is_alive(child), "the session's own process must survive too");

    // A client transparently starts a replacement daemon.
    let listing = session_list(root.path());
    assert!(listing.contains(&id), "the session should still be listed");
    assert!(
        listing.contains("Running"),
        "the adopted session should still be Running, not Crashed:\n{listing}"
    );

    let new_daemon = daemon_pid(root.path()).expect("a replacement daemon should be recorded");
    assert_ne!(new_daemon, daemon, "a genuinely new daemon should be running");

    // Adopted, **not respawned**. A new worker pid here would mean the
    // supervisor silently restarted the session -- which would look like
    // recovery while having actually thrown away everything the original
    // process had in memory.
    assert_eq!(
        worker_pid(root.path(), &id),
        worker,
        "the worker must be adopted, not respawned"
    );
    assert!(is_alive(worker), "the original worker should still be the live one");
}

#[test]
fn reattaching_after_a_restart_announces_the_recovery_and_replays_the_transcript() {
    let root = TempRoot::new("restart-attach");
    // Prints, then stays alive -- so there is both a transcript to replay
    // and a live worker to adopt.
    let command = if cfg!(windows) {
        vec!["cmd", "/C", "echo marker-line && ping -n 600 127.0.0.1 > NUL"]
    } else {
        vec!["sh", "-c", "echo marker-line; sleep 600"]
    };
    let id = session_new(root.path(), &command);
    let daemon = daemon_pid(root.path()).expect("daemon recorded");

    // Let the output land in the transcript before killing anything.
    assert!(
        wait_until(
            || std::fs::read_to_string(
                root.path().join("sessions").join(&id).join("transcript.jsonl")
            )
            .map(|t| t.contains("marker-line"))
            .unwrap_or(false),
            Duration::from_secs(10),
        ),
        "the session's output should reach the transcript"
    );

    force_kill(daemon);
    assert!(wait_until(|| !is_alive(daemon), Duration::from_secs(10)));

    let lines = attach_lines(root.path(), &id, 6, Duration::from_secs(15));
    let joined = lines.join("\n");
    assert!(
        joined.contains("survived a manager restart"),
        "reattaching to an adopted session should say so, rather than leaving the user to \
         infer it from a gap in the output:\n{joined}"
    );
    assert!(
        joined.contains("marker-line"),
        "the transcript should be replayed on reattach:\n{joined}"
    );
}

#[test]
fn shutting_the_daemon_down_gracefully_also_leaves_sessions_running() {
    // The ordinary case, as opposed to the crash above: quitting the
    // manager deliberately must not stop the work either. `DaemonShutdown`
    // stopping sessions would be an easy and very wrong "tidy-up" to add.
    let root = TempRoot::new("graceful-shutdown");
    let id = session_new(root.path(), &long_running());
    let worker = worker_pid(root.path(), &id);

    assert_success("daemon shutdown", &run(root.path(), &["daemon", "shutdown"]));

    assert!(
        is_alive(worker),
        "a graceful daemon shutdown must leave running sessions alone"
    );
    assert!(session_list(root.path()).contains("Running"));
}
