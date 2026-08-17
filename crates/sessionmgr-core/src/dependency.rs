//! What a `Dependent` session's parent status means for whether the
//! dependent session can start.
//!
//! Pure policy, same reasoning as [`crate::recovery`]: the daemon does
//! the I/O (reads the parent's `state.json`), passes its status in here,
//! and acts on the answer -- which keeps the actual decision
//! unit-testable against every [`crate::session::SessionStatus`] variant
//! with no processes or filesystem involved.

use crate::session::SessionStatus;

/// Whether a waiting dependent session can proceed, given its parent's
/// current status.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum ParentReadiness {
    /// The parent is still doing something. Keep waiting.
    NotYet,
    /// The parent has stopped running, and its workspace still exists.
    /// Start the dependent session now.
    Ready,
    /// The parent's workspace is gone (or is about to be) and never will
    /// exist again for this dependent session to run in. Waiting further
    /// cannot help -- the dependent session must fail, not spin forever.
    Unavailable,
}

/// Decides [`ParentReadiness`] from a parent session's current
/// [`SessionStatus`].
///
/// # The rule
///
/// - Anything that still expects a live worker, plus [`SessionStatus::Waiting`]
///   itself (a dependent chain, waiting on a dependent), means the parent
///   has not finished its own turn yet: [`ParentReadiness::NotYet`].
/// - [`SessionStatus::Finished`]/[`SessionStatus::Errored`]/[`SessionStatus::Crashed`]
///   (the parent's process is done, one way or another) and
///   [`SessionStatus::Closed`] (torn down with **no** disposition, which
///   -- see `Session::teardown_status` -- is the one close outcome that
///   leaves a worktree-owning session's worktree and branch in place)
///   all mean the workspace this dependent session needs is still on
///   disk: [`ParentReadiness::Ready`].
/// - [`SessionStatus::Merged`]/[`SessionStatus::Discarded`] both mean the
///   parent's own close actually removed the worktree (see
///   `Supervisor::dispose_workspace`, which runs `git worktree remove`
///   for *both* dispositions, not just `--discard`): [`ParentReadiness::Unavailable`].
pub fn parent_readiness(parent_status: SessionStatus) -> ParentReadiness {
    use SessionStatus::*;
    match parent_status {
        Created | Running | NeedsInput | Waiting => ParentReadiness::NotYet,
        Finished | Errored | Crashed | Closed => ParentReadiness::Ready,
        Merged | Discarded => ParentReadiness::Unavailable,
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn a_still_active_parent_is_not_yet_ready() {
        for status in [
            SessionStatus::Created,
            SessionStatus::Running,
            SessionStatus::NeedsInput,
            SessionStatus::Waiting,
        ] {
            assert_eq!(
                parent_readiness(status),
                ParentReadiness::NotYet,
                "{status:?} must not be treated as ready"
            );
        }
    }

    #[test]
    fn an_exited_or_bare_closed_parent_is_ready_because_its_workspace_survives() {
        for status in [
            SessionStatus::Finished,
            SessionStatus::Errored,
            SessionStatus::Crashed,
            SessionStatus::Closed,
        ] {
            assert_eq!(
                parent_readiness(status),
                ParentReadiness::Ready,
                "{status:?} leaves the worktree in place, so a dependent session may start"
            );
        }
    }

    #[test]
    fn a_merged_or_discarded_parent_is_permanently_unavailable() {
        for status in [SessionStatus::Merged, SessionStatus::Discarded] {
            assert_eq!(
                parent_readiness(status),
                ParentReadiness::Unavailable,
                "{status:?} means the worktree this dependent session needs is gone"
            );
        }
    }
}
