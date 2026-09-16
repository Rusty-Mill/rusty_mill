//! H3 workflow tools (PRD 05 / Phase 10): `reproduce`, `attribute_failure`,
//! `verification_report`. They write the per-turn [`H3Scratch`] that the
//! `compose` assembler projects into the episode package and that the H3 checks
//! (`reproduce_before_edit`, `verification_report_required`) read. Registered
//! only at harness level H3.

use std::sync::Arc;

use async_trait::async_trait;
use rk_observe::{AgentAttribution, H3Scratch, ReproductionLog, Requirement, ToolOutcome};
use serde_json::Value;

use crate::tool::{ToolFn, ToolRegistry};

struct ReproduceTool {
    scratch: Arc<H3Scratch>,
}

struct AttributeFailureTool {
    scratch: Arc<H3Scratch>,
}

struct VerificationReportTool {
    scratch: Arc<H3Scratch>,
}

/// Register the H3 workflow tools, backed by the turn's `scratch`.
pub fn register_h3_tools(registry: &mut ToolRegistry, scratch: Arc<H3Scratch>) {
    registry.insert(Box::new(ReproduceTool {
        scratch: scratch.clone(),
    }));
    registry.insert(Box::new(AttributeFailureTool {
        scratch: scratch.clone(),
    }));
    registry.insert(Box::new(VerificationReportTool { scratch }));
}

fn s(args: &Value, key: &str) -> String {
    args.get(key)
        .and_then(Value::as_str)
        .unwrap_or("")
        .to_string()
}

#[async_trait]
impl ToolFn for ReproduceTool {
    fn name(&self) -> &str {
        "reproduce"
    }

    fn schema(&self) -> Value {
        serde_json::json!({
            "type": "object",
            "properties": {
                "check": {"type": "string"},
                "observed": {"type": "string"},
                "expected": {"type": "string"}
            },
            "required": ["check", "observed", "expected"]
        })
    }

    async fn call(&self, args: Value) -> ToolOutcome {
        self.scratch.set_reproduction(ReproductionLog {
            check: s(&args, "check"),
            observed: s(&args, "observed"),
            expected: s(&args, "expected"),
        });
        ToolOutcome::ok("reproduction recorded")
    }
}

#[async_trait]
impl ToolFn for AttributeFailureTool {
    fn name(&self) -> &str {
        "attribute_failure"
    }

    fn schema(&self) -> Value {
        serde_json::json!({
            "type": "object",
            "properties": {
                "observed": {"type": "string"},
                "expected": {"type": "string"},
                "failure_type": {"type": "string"},
                "evidence": {"type": "string"},
                "next_action": {"type": "string"}
            },
            "required": ["failure_type", "evidence"]
        })
    }

    async fn call(&self, args: Value) -> ToolOutcome {
        self.scratch.add_attribution(AgentAttribution {
            observed: s(&args, "observed"),
            expected: s(&args, "expected"),
            failure_type: s(&args, "failure_type"),
            evidence: s(&args, "evidence"),
            next_action: s(&args, "next_action"),
        });
        ToolOutcome::ok("attribution recorded")
    }
}

#[async_trait]
impl ToolFn for VerificationReportTool {
    fn name(&self) -> &str {
        "verification_report"
    }

    fn schema(&self) -> Value {
        serde_json::json!({
            "type": "object",
            "properties": {
                "requirements": {
                    "type": "array",
                    "items": {
                        "type": "object",
                        "properties": {
                            "requirement": {"type": "string"},
                            "met": {"type": "boolean"},
                            "evidence": {"type": "string"}
                        },
                        "required": ["requirement", "met"]
                    }
                }
            },
            "required": ["requirements"]
        })
    }

    async fn call(&self, args: Value) -> ToolOutcome {
        let reqs: Vec<Requirement> = args
            .get("requirements")
            .and_then(Value::as_array)
            .map(|a| {
                a.iter()
                    .map(|r| Requirement {
                        requirement: r
                            .get("requirement")
                            .and_then(Value::as_str)
                            .unwrap_or("")
                            .to_string(),
                        met: r.get("met").and_then(Value::as_bool).unwrap_or(false),
                        evidence: r
                            .get("evidence")
                            .and_then(Value::as_str)
                            .unwrap_or("")
                            .to_string(),
                    })
                    .collect()
            })
            .unwrap_or_default();
        if reqs.is_empty() {
            return ToolOutcome::error(
                "verification_report: 'requirements' must be a non-empty array",
            );
        }
        let n = reqs.len();
        self.scratch.set_requirements(reqs);
        ToolOutcome::ok(format!("verification report recorded ({n} requirements)"))
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[tokio::test]
    async fn tools_populate_scratch() {
        let scratch = Arc::new(H3Scratch::new());
        let repro = ReproduceTool {
            scratch: scratch.clone(),
        };
        let report = VerificationReportTool {
            scratch: scratch.clone(),
        };
        repro
            .call(serde_json::json!({"check": "probe", "observed": "panics", "expected": "ok"}))
            .await;
        assert!(scratch.reproduction().is_some());
        report
            .call(serde_json::json!({"requirements": [{"requirement": "req-1", "met": true, "evidence": "test passes"}]}))
            .await;
        assert!(scratch.has_report());
        assert_eq!(scratch.requirements()[0].requirement, "req-1");
    }
}
