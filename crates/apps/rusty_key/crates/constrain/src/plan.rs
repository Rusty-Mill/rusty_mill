//! Plan mode (PRD 06 / Phase 9): a read-only proposal phase before destructive
//! execution. [`PlanController`] is the shared, runtime-mutable permission state
//! that [`crate::ModePolicy`] reads live, so `enter_plan_mode` takes effect for
//! the rest of the turn and `exit_plan_mode` (after human approval) transitions
//! the mode for the next turn.

use std::sync::Mutex;

use crate::PermissionMode;

/// The human's decision on an `exit_plan_mode` request.
#[derive(Debug, Clone, PartialEq, Eq)]
pub enum PlanDecision {
    /// Approve the plan — the next turn runs with writes enabled.
    Proceed,
    /// Reject the plan — restore the base mode; no writes.
    Reject,
    /// Reject with feedback to send back to the agent (desktop/CLI).
    Annotate(String),
}

/// Shared permission state for plan mode. The `ModePolicy` reads [`Self::mode`]
/// on every `before_tool`; the plan tools mutate it. One per session, held by
/// both the policy chain and the session.
pub struct PlanController {
    active: Mutex<PermissionMode>,
    base: PermissionMode,
    exit_request: Mutex<Option<String>>,
}

impl PlanController {
    /// Start in `base` (the configured permission mode).
    pub fn new(base: PermissionMode) -> Self {
        Self {
            active: Mutex::new(base.clone()),
            base,
            exit_request: Mutex::new(None),
        }
    }

    /// The mode the policy enforces right now.
    pub fn mode(&self) -> PermissionMode {
        self.active
            .lock()
            .unwrap_or_else(|p| p.into_inner())
            .clone()
    }

    /// Whether plan mode is currently active.
    pub fn is_planning(&self) -> bool {
        matches!(self.mode(), PermissionMode::Plan)
    }

    /// Enter plan mode: writes and bash are blocked until exit.
    pub fn enter_plan(&self) {
        *self.active.lock().unwrap_or_else(|p| p.into_inner()) = PermissionMode::Plan;
    }

    /// Record an `exit_plan_mode` request carrying the proposed plan summary.
    /// Resolution waits for a human decision (PRD: not an intervention).
    pub fn request_exit(&self, summary: &str) {
        *self.exit_request.lock().unwrap_or_else(|p| p.into_inner()) = Some(summary.to_string());
    }

    /// The pending plan summary, if `exit_plan_mode` was called and not resolved.
    pub fn pending_exit(&self) -> Option<String> {
        self.exit_request
            .lock()
            .unwrap_or_else(|p| p.into_inner())
            .clone()
    }

    /// Resolve a pending exit. `Proceed` enables writes (`AcceptEdits`);
    /// `Reject`/`Annotate` restore the base mode. Returns the annotation text to
    /// re-send to the agent, if any.
    pub fn resolve(&self, decision: PlanDecision) -> Option<String> {
        *self.exit_request.lock().unwrap_or_else(|p| p.into_inner()) = None;
        let mut active = self.active.lock().unwrap_or_else(|p| p.into_inner());
        match decision {
            PlanDecision::Proceed => {
                *active = PermissionMode::AcceptEdits;
                None
            }
            PlanDecision::Reject => {
                *active = self.base.clone();
                None
            }
            PlanDecision::Annotate(text) => {
                *active = self.base.clone();
                Some(text)
            }
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn enter_blocks_writes_then_proceed_enables_them() {
        let c = PlanController::new(PermissionMode::Default);
        assert!(c.mode().check("write_file").is_ok());
        c.enter_plan();
        assert!(c.is_planning());
        assert!(c.mode().check("write_file").is_err());
        assert!(c.mode().check("bash").is_err());
        assert!(c.mode().check("read_file").is_ok());

        c.request_exit("the plan");
        assert_eq!(c.pending_exit().as_deref(), Some("the plan"));
        assert_eq!(c.resolve(PlanDecision::Proceed), None);
        assert!(!c.is_planning());
        assert!(c.mode().check("write_file").is_ok()); // AcceptEdits
        assert_eq!(c.pending_exit(), None);
    }

    #[test]
    fn reject_restores_base_mode() {
        let c = PlanController::new(PermissionMode::Default);
        c.enter_plan();
        c.request_exit("nope");
        assert_eq!(c.resolve(PlanDecision::Reject), None);
        assert_eq!(c.mode(), PermissionMode::Default);
    }

    #[test]
    fn annotate_returns_feedback_and_restores_base() {
        let c = PlanController::new(PermissionMode::Default);
        c.enter_plan();
        c.request_exit("plan v1");
        let fb = c.resolve(PlanDecision::Annotate("add tests".into()));
        assert_eq!(fb.as_deref(), Some("add tests"));
        assert_eq!(c.mode(), PermissionMode::Default);
    }
}
