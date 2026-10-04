//! The reply schema agrees with the parser about kinds under both rules.
//! Strict Structured Outputs form is the Codex adapter's requirement and is
//! checked by its contract test.

use orch_cli::{allowed_kinds, output_schema};
use orch_core::goal::StopRule;
use orch_core::task::Role;
use rusty_json::Value;

fn kinds(stop: StopRule) -> Vec<String> {
    let schema = Value::parse(&output_schema(stop)).expect("schema is JSON");
    let mut kinds: Vec<String> = schema
        .get("properties")
        .and_then(|p| p.get("entries"))
        .and_then(|e| e.get("items"))
        .and_then(|i| i.get("anyOf"))
        .and_then(|a| a.as_array())
        .expect("one variant per kind")
        .iter()
        .filter_map(|v| {
            v.get("properties")?
                .get("kind")?
                .get("enum")?
                .as_array()?
                .first()?
                .as_str()
                .map(str::to_owned)
        })
        .collect();
    kinds.sort_unstable();
    kinds
}

#[test]
fn checkpoint_schema_lists_every_kind_the_roles_may_write() {
    assert_eq!(
        kinds(StopRule::Checkpoint),
        ["assumption", "finding", "question", "review"]
    );
    for kind in allowed_kinds(Role::Research, StopRule::Checkpoint) {
        assert!(kinds(StopRule::Checkpoint).iter().any(|k| k == kind));
    }
}

#[test]
fn best_effort_schema_has_no_question_variant() {
    assert_eq!(
        kinds(StopRule::BestEffort),
        ["assumption", "finding", "review"]
    );
    assert!(!allowed_kinds(Role::Research, StopRule::BestEffort).contains(&"question"));
}
