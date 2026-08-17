//! What a restarting supervisor does with a session record it finds on
//! disk.
//!
//! This is the single most consequential policy in the project, and it is
//! deliberately pure: the daemon does the I/O (read `state.json`, probe
//! the pid), passes the result in here, and acts on the returned
//! [`RecoveryAction`]. That keeps the rule -- including the rule about
//! what *not* to do -- unit-testable with no processes involved.
//!
//! See `docs/plan/PLAN.md` § Process supervision & session persistence.

use crate::session::{Session, WorkerRef};

/// The result of probing a recorded worker pid.
///
/// "Alive" here means the strong form: a live process **that is still the
/// same process** which recorded itself, established by comparing a
/// start-time fingerprint, not merely by the pid existing. The adapter
/// (`sessionmgr-proc`) is what establishes that; this enum is the answer,
/// not the method.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum Liveness {
    Alive,
    Dead,
}

/// What the supervisor should do about one session record.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum RecoveryAction {
    /// The worker is still running: adopt it. Mark the session live and
    /// allow clients to reattach. **Spawn nothing.**
    Adopt,
    /// The worker is gone: record the session as
    /// [`crate::session::SessionStatus::Crashed`].
    /// **Spawn nothing.**
    MarkCrashed,
    /// Nothing to do -- the record is already in a state that expects no
    /// worker.
    LeaveAsIs,
}

/// Decides what to do with `session`, given the [`Liveness`] of its
/// recorded worker.
///
/// # The rule, and why it is this rule
///
/// - A session whose status does not expect a live worker is left alone.
/// - A session that expects a worker, and whose worker is alive, is
///   **adopted** -- not respawned. This is the entire point of the
///   detached-worker architecture: the worker outlives the manager, so
///   the manager coming back finds work still in progress and reattaches
///   to it.
/// - A session that expects a worker whose worker is dead is marked
///   crashed, and **nothing is resurrected**.
///
/// That last point is a deliberate limitation, not an oversight. Silently
/// respawning would mean relaunching an agent CLI into a conversation
/// that is mid-flight, with no way to restore what the CLI itself had in
/// memory -- so the "recovered" session would look alive while having
/// quietly lost its context. Restoring an agent CLI's own prior state is
/// the same unproven per-CLI primitive that PLAN.md gates fork and
/// switch-agent-mid-session behind in Phase 6+. Until that is proven,
/// reporting a crash honestly beats presenting a lobotomised session as
/// healthy.
///
/// # A session with no recorded worker
///
/// `Created` with no worker pid means the daemon died in the window
/// between writing the record and spawning the worker. There is nothing
/// to adopt and nothing to have crashed *yet* -- but leaving it `Created`
/// forever would strand it invisibly, so it is marked crashed too: the
/// user asked for a session and did not get one, which is exactly what
/// `Crashed` communicates.
pub fn decide_recovery(session: &Session, liveness: Option<Liveness>) -> RecoveryAction {
    if !session.status.expects_live_worker() {
        return RecoveryAction::LeaveAsIs;
    }
    match (&session.worker, liveness) {
        (Some(_), Some(Liveness::Alive)) => RecoveryAction::Adopt,
        (Some(_), Some(Liveness::Dead) | None) => RecoveryAction::MarkCrashed,
        // No worker was ever recorded: see the doc comment above.
        (None, _) => RecoveryAction::MarkCrashed,
    }
}

/// The pid pair a close/teardown must terminate.
///
/// Returns the worker pid **and** the child pid, in that order, skipping
/// whichever is absent.
///
/// Both, deliberately. Terminating only the worker leaves the agent CLI
/// it spawned running as an orphan with nothing tracking it and no way
/// for the user to reach it -- the exact gap PLAN.md's adversarial review
/// identified (finding #2) in an earlier design that killed the worker
/// alone. No Job Object is used to get a tree-kill instead, because
/// kill-on-close is structurally incompatible with sessions surviving the
/// manager closing, which is the capability this whole architecture
/// exists to deliver.
///
/// Worker first: it is the process that would otherwise notice its child
/// dying and react (writing an exit record, restarting nothing but
/// logging noisily). Killing it first makes the child's death quiet and
/// expected rather than something a half-dead worker misreports.
pub fn teardown_pids(session: &Session) -> Vec<u32> {
    [session.worker.as_ref(), session.child.as_ref()]
        .into_iter()
        .flatten()
        .map(|w: &WorkerRef| w.pid)
        .collect()
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::session::{SessionId, SessionKind, SessionStatus};

    fn session_with(status: SessionStatus, worker: Option<WorkerRef>) -> Session {
        let mut s = Session::new(
            SessionId::new(1_700_000_000_000, 7),
            SessionKind::PlainTerminal,
            vec!["sh".to_owned()],
            None,
            true,
            1_700_000_000_000,
            None,
        );
        s.status = status;
        s.worker = worker;
        s
    }

    fn worker() -> Option<WorkerRef> {
        Some(WorkerRef {
            pid: 4321,
            start_fingerprint: Some("12345".to_owned()),
        })
    }

    #[test]
    fn a_live_worker_is_adopted_never_respawned() {
        // The behavioural heart of the whole persistence design: the
        // manager closing and reopening must find the work still running.
        for status in [
            SessionStatus::Created,
            SessionStatus::Running,
            SessionStatus::NeedsInput,
        ] {
            assert_eq!(
                decide_recovery(&session_with(status, worker()), Some(Liveness::Alive)),
                RecoveryAction::Adopt,
                "a live worker in {status:?} must be adopted"
            );
        }
    }

    #[test]
    fn a_dead_worker_is_marked_crashed_and_nothing_is_resurrected() {
        for status in [
            SessionStatus::Created,
            SessionStatus::Running,
            SessionStatus::NeedsInput,
        ] {
            let action = decide_recovery(&session_with(status, worker()), Some(Liveness::Dead));
            assert_eq!(action, RecoveryAction::MarkCrashed);
            // There is deliberately no `Respawn` variant to assert the
            // absence of -- the type itself makes silent resurrection
            // unrepresentable. This test documents that intent.
        }
    }

    #[test]
    fn a_session_that_expects_no_worker_is_left_alone() {
        for status in [
            SessionStatus::Finished,
            SessionStatus::Errored,
            SessionStatus::Crashed,
            SessionStatus::Closed,
        ] {
            assert_eq!(
                decide_recovery(&session_with(status, worker()), Some(Liveness::Dead)),
                RecoveryAction::LeaveAsIs,
                "{status:?} expects no worker, so recovery must not touch it"
            );
        }
    }

    #[test]
    fn a_record_with_no_worker_pid_is_marked_crashed_not_left_stranded() {
        // The daemon died between writing `state.json` and spawning the
        // worker. Leaving it `Created` forever would strand it invisibly.
        assert_eq!(
            decide_recovery(&session_with(SessionStatus::Created, None), None),
            RecoveryAction::MarkCrashed
        );
    }

    #[test]
    fn an_unprobeable_worker_counts_as_dead() {
        assert_eq!(
            decide_recovery(&session_with(SessionStatus::Running, worker()), None),
            RecoveryAction::MarkCrashed
        );
    }

    #[test]
    fn teardown_targets_both_the_worker_and_its_child() {
        // Adversarial finding #2: killing only the worker orphans the
        // agent CLI with no remediation path.
        let mut s = session_with(SessionStatus::Running, worker());
        s.child = Some(WorkerRef {
            pid: 9999,
            start_fingerprint: None,
        });
        assert_eq!(
            teardown_pids(&s),
            vec![4321, 9999],
            "worker first, then the child it spawned"
        );
    }

    #[test]
    fn teardown_skips_pids_that_were_never_recorded() {
        let s = session_with(SessionStatus::Created, None);
        assert!(teardown_pids(&s).is_empty());

        let s = session_with(SessionStatus::Running, worker());
        assert_eq!(teardown_pids(&s), vec![4321]);
    }
}
