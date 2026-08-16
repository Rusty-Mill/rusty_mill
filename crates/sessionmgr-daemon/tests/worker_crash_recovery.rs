//! Proves the **"no silent respawn"** rule.
//!
//! Its counterpart, `supervisor_restart_recovery.rs`, proves that a
//! surviving worker is adopted. This one proves the other half: a worker
//! that is genuinely gone is reported as crashed and is *not* quietly
//! restarted.
//!
//! That restraint is a deliberate design decision, not a missing feature.
//! Respawning would relaunch a session whose process had accumulated
//! state this tool cannot restore, so the "recovered" session would
//! present as healthy while having silently lost everything it knew.
//! Restoring an agent CLI's own prior state is the unproven per-CLI
//! primitive PLAN.md gates fork and switch-agent behind in Phase 6+.

mod common;

use std::time::Duration;

use common::*;

#[test]
fn a_killed_worker_is_reported_as_crashed_and_never_respawned() {
    let root = TempRoot::new("worker-crash");
    let id = session_new(root.path(), &long_running());
    let worker = worker_pid(root.path(), &id);

    force_kill(worker);
    assert!(
        wait_until(|| !is_alive(worker), Duration::from_secs(10)),
        "the worker should be gone after being killed"
    );

    let listing = session_list(root.path());
    assert!(
        listing.contains("Crashed"),
        "a session whose worker died must be reported as crashed, not left showing Running:\n{listing}"
    );
    assert_eq!(session_status(root.path(), &id), "crashed");

    // Nothing was resurrected: the recorded pid is unchanged and still
    // dead. A different (live) pid here would mean a silent respawn.
    assert_eq!(worker_pid(root.path(), &id), worker);
    assert!(!is_alive(worker));

    // And it stays that way -- a second look must not "helpfully" start
    // something either.
    let _ = session_list(root.path());
    assert_eq!(session_status(root.path(), &id), "crashed");
}

#[test]
fn a_crashed_session_is_still_closeable() {
    // A crashed session still owns a transcript (and, from Phase 2, a
    // worktree), so it must remain tearable-down rather than being stuck.
    let root = TempRoot::new("close-crashed");
    let id = session_new(root.path(), &long_running());
    let worker = worker_pid(root.path(), &id);

    force_kill(worker);
    assert!(wait_until(|| !is_alive(worker), Duration::from_secs(10)));
    let _ = session_list(root.path()); // reconcile it to Crashed

    assert_success("close", &run(root.path(), &["close", &id]));
    assert_eq!(session_status(root.path(), &id), "closed");
}

#[test]
fn attaching_to_a_crashed_session_still_shows_what_it_produced() {
    // Refusing to open a crashed session would hide the output that
    // explains *why* it crashed -- exactly when the user needs it most.
    let root = TempRoot::new("attach-crashed");
    let command = if cfg!(windows) {
        vec!["cmd", "/C", "echo before-the-crash && ping -n 600 127.0.0.1 > NUL"]
    } else {
        vec!["sh", "-c", "echo before-the-crash; sleep 600"]
    };
    let id = session_new(root.path(), &command);
    let worker = worker_pid(root.path(), &id);

    assert!(
        wait_until(
            || transcript_contains(root.path(), &id, "before-the-crash"),
            Duration::from_secs(10),
        ),
        "the output should reach the transcript before the crash"
    );

    force_kill(worker);
    assert!(wait_until(|| !is_alive(worker), Duration::from_secs(10)));

    let joined = attach_lines(root.path(), &id, 6, Duration::from_secs(15)).join("\n");
    assert!(
        joined.contains("before-the-crash"),
        "a crashed session's output must still be readable:\n{joined}"
    );
}

#[test]
fn killing_the_child_alone_is_recorded_as_an_exit_not_a_crash() {
    // The worker is still alive and watching, so it sees a real exit
    // status. That is PLAN.md's tier-2 signal -- the one status source
    // that is free and cannot be wrong -- and it must win over the
    // liveness-probe path, which knows nothing about *why* something
    // stopped.
    let root = TempRoot::new("child-killed");
    let id = session_new(root.path(), &long_running());
    let child = child_pid(root.path(), &id);

    force_kill(child);

    assert!(
        wait_until(
            || {
                let status = session_status(root.path(), &id);
                status == "errored" || status == "finished"
            },
            Duration::from_secs(15),
        ),
        "a killed child should be recorded via its exit status, but the session is {}",
        session_status(root.path(), &id)
    );
}
