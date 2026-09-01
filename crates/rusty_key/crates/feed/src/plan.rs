//! Plan-mode tools (PRD 06 / Phase 9). `enter_plan_mode` flips the shared
//! [`PlanController`] to a read-only proposal phase (writes + bash blocked for
//! the rest of the turn); `exit_plan_mode` submits the proposed plan for human
//! approval. The actual mode transition happens when the session resolves the
//! request (Proceed/Reject/Annotate) — these tools never transition on their own.

use std::sync::Arc;

use async_trait::async_trait;
use rk_constrain::PlanController;
use rk_observe::ToolOutcome;
use serde_json::Value;

use crate::tool::{ToolFn, ToolRegistry};

/// `enter_plan_mode`: switch the session into read-only plan mode.
pub struct EnterPlanModeTool {
    controller: Arc<PlanController>,
}

/// `exit_plan_mode`: submit the proposed plan and request approval to execute.
pub struct ExitPlanModeTool {
    controller: Arc<PlanController>,
}

/// Register `enter_plan_mode` / `exit_plan_mode`, backed by `controller`.
pub fn register_plan_tools(registry: &mut ToolRegistry, controller: Arc<PlanController>) {
    registry.insert(Box::new(EnterPlanModeTool {
        controller: controller.clone(),
    }));
    registry.insert(Box::new(ExitPlanModeTool { controller }));
}

#[async_trait]
impl ToolFn for EnterPlanModeTool {
    fn name(&self) -> &str {
        "enter_plan_mode"
    }

    fn schema(&self) -> Value {
        serde_json::json!({
            "type": "object",
            "properties": {},
        })
    }

    async fn call(&self, _args: Value) -> ToolOutcome {
        self.controller.enter_plan();
        ToolOutcome::ok(
            "entered plan mode: writes and bash are blocked. Propose a plan, then \
             call exit_plan_mode to request approval.",
        )
    }
}

#[async_trait]
impl ToolFn for ExitPlanModeTool {
    fn name(&self) -> &str {
        "exit_plan_mode"
    }

    fn schema(&self) -> Value {
        serde_json::json!({
            "type": "object",
            "properties": {"plan": {"type": "string"}},
            "required": ["plan"]
        })
    }

    async fn call(&self, args: Value) -> ToolOutcome {
        let plan = args.get("plan").and_then(Value::as_str).unwrap_or("");
        self.controller.request_exit(plan);
        ToolOutcome::ok("plan submitted for approval (awaiting Proceed/Reject)")
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use rk_constrain::{PermissionMode, PlanDecision};

    #[tokio::test]
    async fn enter_then_exit_round_trips_the_controller() {
        let c = Arc::new(PlanController::new(PermissionMode::Default));
        let enter = EnterPlanModeTool {
            controller: c.clone(),
        };
        let exit = ExitPlanModeTool {
            controller: c.clone(),
        };

        enter.call(serde_json::json!({})).await;
        assert!(c.is_planning());

        exit.call(serde_json::json!({"plan": "do X then Y"})).await;
        assert_eq!(c.pending_exit().as_deref(), Some("do X then Y"));

        c.resolve(PlanDecision::Proceed);
        assert!(!c.is_planning());
    }
}
