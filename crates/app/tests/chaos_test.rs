//! Chaos / resilience eval tier, v1 (eval-plan §7).
//!
//! Injects each fault class at the tool-dispatch / `ToolOutcome` seam and drives
//! the real path (kernel loop → policy-vetted registry → tracer → verifier). The
//! asserted property is the verification thesis as a metric: **never
//! verified-success on top of an injected fault** — under every fault the turn is
//! UNVERIFIED, `no_tool_errors` fires, and an `f_tool` attribution is recorded.

use std::sync::Arc;

use rk_compose::{FailureType, Verifier};
use rk_constrain::{Policy, PolicyError};
use rk_feed::chaos::{Fault, FaultyTool};
use rk_feed::ToolRegistry;
use rk_kernel::fake::{FakeLanguageModel, Scripted};
use rk_kernel::run_turn;
use rk_observe::Tracer;
use serde_json::{json, Value};

/// Allow-all policy so the fault, not the boundary, is what trips verification.
struct AllowAll;
#[async_trait::async_trait]
impl Policy for AllowAll {
    async fn before_tool(&self, _: &str, _: &Value) -> Result<(), PolicyError> {
        Ok(())
    }
}

async fn run_with_fault(fault: Fault) -> (bool, Vec<String>) {
    let tracer = Arc::new(Tracer::new());
    let mut registry = ToolRegistry::new(Arc::new(AllowAll)).with_tracer(tracer.clone());
    registry.insert(Box::new(FaultyTool::new("flaky", fault)));
    let dispatch: Arc<dyn rk_constrain::ToolDispatch> = Arc::new(registry);

    let model = FakeLanguageModel::new(vec![
        vec![Scripted::ToolCall {
            name: "flaky".into(),
            args: json!({}),
        }],
        vec![Scripted::Text("the tool seemed to work".into())],
    ]);

    tracer.start_episode();
    let reply = run_turn(model, "sys", "use the tool", dispatch, 10)
        .await
        .unwrap();
    tracer.set_final_reached(true);

    let report = Verifier::deterministic().verify(&reply, &tracer.episode());
    let categories = report
        .attributions
        .iter()
        .map(|a| a.category.clone())
        .collect();
    // Resilience invariant: an injected fault must never read as verified, and the
    // attribution must always be f_tool.
    assert!(report
        .attributions
        .iter()
        .all(|a| a.failure_type == FailureType::FTool));
    (report.verified, categories)
}

#[tokio::test(flavor = "multi_thread", worker_threads = 2)]
async fn never_verified_success_on_injected_fault() {
    for (fault, expected) in [
        (Fault::Error, "tool_error"),
        (Fault::Timeout, "tool_timeout"),
        (Fault::Truncated, "tool_truncated"),
    ] {
        let (verified, categories) = run_with_fault(fault).await;
        assert!(!verified, "fault {fault:?} must not verify");
        assert!(
            categories.iter().any(|c| c == expected),
            "fault {fault:?} expected {expected}, got {categories:?}"
        );
    }
}

#[tokio::test(flavor = "multi_thread", worker_threads = 2)]
async fn baseline_clean_tool_verifies() {
    // A non-faulty tool (Ok) is the baseline: the harness *can* verify when there
    // is no fault — so the chaos failures above are real signal, not a stuck "no".
    let tracer = Arc::new(Tracer::new());
    let mut registry = ToolRegistry::new(Arc::new(AllowAll)).with_tracer(tracer.clone());
    // list_directory on the temp dir succeeds (Ok).
    let dir = std::env::temp_dir();
    rk_feed::register_builtins(&mut registry, dir.clone());
    let dispatch: Arc<dyn rk_constrain::ToolDispatch> = Arc::new(registry);

    let abs = dir.to_string_lossy().into_owned();
    let model = FakeLanguageModel::new(vec![
        vec![Scripted::ToolCall {
            name: "list_directory".into(),
            args: json!({ "path": abs }),
        }],
        vec![Scripted::Text("listed".into())],
    ]);

    tracer.start_episode();
    let reply = run_turn(model, "sys", "list", dispatch, 10).await.unwrap();
    tracer.set_final_reached(true);

    let report = Verifier::deterministic().verify(&reply, &tracer.episode());
    assert!(report.verified);
}
