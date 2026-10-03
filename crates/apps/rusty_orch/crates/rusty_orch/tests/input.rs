//! The goal file: what is accepted, what is refused, and how indices become ids.

use std::num::NonZeroU32;

use orch_core::task::{Agent, Role, Status};
use orch_core::{GoalId, Ref};
use rusty_orch::input::{build_plan, parse, RoleDraft};

const FULL: &str = include_str!("../examples/goal.json");

fn minimal(tasks: &str) -> String {
    format!(
        r#"{{"goal":"g","done_when":["d"],"out_of_scope":[],"wall_clock_secs":60,"max_calls":3,"stop":"checkpoint","tasks":{tasks}}}"#
    )
}

const ONE_TASK: &str =
    r#"[{"role":"research","instruction":"i","acceptance":["a"],"max_calls":1}]"#;

#[test]
fn the_example_file_parses_and_builds_a_plan() {
    let spec = parse(FULL).expect("example parses");
    assert_eq!(spec.goal.budget().max_calls().get(), 4);
    assert_eq!(spec.tasks.len(), 2);
    assert_eq!(spec.tasks[0].role, RoleDraft::Research);
    assert_eq!(spec.tasks[1].role, RoleDraft::Review { target: 0 });
    assert_eq!(spec.tasks[1].depends_on, vec![0]);
    assert!(matches!(spec.tasks[0].refs[0], Ref::Path(ref p) if p.as_str().ends_with("task.rs")));
    assert_eq!(spec.routing.research, Agent::Codex);
    assert_eq!(spec.routing.triage, Agent::Local);
    assert_eq!(spec.routing.reviewers, vec![Agent::Local, Agent::Codex]);
    // Unspecified routing fields keep the defaults.
    assert_eq!(spec.routing.implement, Agent::Codex);

    let plan = build_plan(&spec, GoalId::from_raw(1)).expect("plan");
    let tasks = plan.tasks();
    assert_eq!(tasks.len(), 2);
    assert_eq!(
        tasks[1].spec().role,
        Role::Review {
            target: tasks[0].id()
        }
    );
    assert_eq!(tasks[1].spec().depends_on, vec![tasks[0].id()]);
    assert_eq!(tasks[1].spec().max_calls, NonZeroU32::new(2).expect("nz"));
    assert!(tasks.iter().all(|t| t.state().status() == Status::Pending));
}

#[test]
fn omitted_routing_and_optional_fields_take_defaults() {
    let spec = parse(&minimal(ONE_TASK)).expect("parses");
    assert_eq!(spec.routing, rusty_orch::input::default_routing());
    assert!(spec.tasks[0].refs.is_empty());
    assert!(spec.tasks[0].depends_on.is_empty());
    assert!(spec.goal.in_scope().is_empty());
}

#[test]
fn goal_contract_problems_are_reported_together() {
    let err = parse(r#"{"goal":"g","tasks":[]}"#).expect_err("rejected");
    let m = err.to_string();
    assert!(m.contains("DONE WHEN is required"), "{m}");
    assert!(m.contains("SCOPE (out) is required"), "{m}");
    assert!(m.contains("BUDGET (max calls) is required"), "{m}");
}

#[test]
fn every_refusal_names_the_json_path() {
    let cases = [
        ("not json", "not JSON"),
        ("[]", "must be a JSON object"),
        (&minimal("[]"), "tasks: must not be empty"),
        (
            &minimal(r#"[{"role":"wizard","instruction":"i","acceptance":["a"],"max_calls":1}]"#),
            "tasks[0].role: unknown role",
        ),
        (
            &minimal(r#"[{"role":"research","acceptance":["a"],"max_calls":1}]"#),
            "tasks[0].instruction: required",
        ),
        (
            &minimal(r#"[{"role":"research","instruction":"i","acceptance":[],"max_calls":1}]"#),
            "tasks[0].acceptance: required",
        ),
        (
            &minimal(r#"[{"role":"research","instruction":"i","acceptance":["a"],"max_calls":0}]"#),
            "tasks[0].max_calls: required, greater than zero",
        ),
        (
            &minimal(
                r#"[{"role":"research","instruction":"i","acceptance":["a"],"refs":["ftp:x"],"max_calls":1}]"#,
            ),
            "tasks[0].refs: unknown ref scheme",
        ),
        (
            &minimal(r#"[{"role":"review","instruction":"i","acceptance":["a"],"max_calls":1}]"#),
            "tasks[0].target: required",
        ),
        (
            &minimal(
                r#"[{"role":"research","instruction":"i","acceptance":["a"],"depends_on":["x"],"max_calls":1}]"#,
            ),
            "tasks[0].depends_on: expected an index",
        ),
        (
            &minimal(ONE_TASK).replace("\"checkpoint\"", "\"maybe\""),
            "stop: expected",
        ),
        (
            &minimal(ONE_TASK).replace("\"tasks\"", "\"routing\":{\"triage\":\"hal\"},\"tasks\""),
            "routing.triage: unknown agent",
        ),
    ];
    for (json, expected) in cases {
        let m = parse(json).expect_err(expected).to_string();
        assert!(m.contains(expected), "expected {expected:?} in {m:?}");
    }
}

#[test]
fn forward_and_self_references_are_refused_when_the_plan_is_built() {
    let forward = minimal(
        r#"[{"role":"research","instruction":"i","acceptance":["a"],"depends_on":[1],"max_calls":1},
            {"role":"research","instruction":"j","acceptance":["a"],"max_calls":1}]"#,
    );
    let spec = parse(&forward).expect("parses");
    let m = build_plan(&spec, GoalId::from_raw(1))
        .expect_err("forward")
        .to_string();
    assert!(
        m.contains("tasks[0].depends_on: index 1 is not an earlier task"),
        "{m}"
    );

    let own = minimal(
        r#"[{"role":"review","target":0,"instruction":"i","acceptance":["a"],"max_calls":1}]"#,
    );
    let spec = parse(&own).expect("parses");
    let m = build_plan(&spec, GoalId::from_raw(1))
        .expect_err("self")
        .to_string();
    assert!(
        m.contains("tasks[0].target: index 0 is not an earlier task"),
        "{m}"
    );
}

#[test]
fn routing_must_be_an_object_when_present() {
    for bad in [r#""local""#, "null", "[]", "3", "true"] {
        let json = minimal(ONE_TASK).replace("\"tasks\"", &format!("\"routing\":{bad},\"tasks\""));
        let m = parse(&json).expect_err(bad).to_string();
        assert!(m.contains("routing: expected an object"), "{bad}: {m}");
    }
    // Omitted and partial objects keep working.
    assert!(parse(&minimal(ONE_TASK)).is_ok());
    let partial =
        minimal(ONE_TASK).replace("\"tasks\"", "\"routing\":{\"triage\":\"codex\"},\"tasks\"");
    let spec = parse(&partial).expect("partial routing");
    assert_eq!(spec.routing.triage, Agent::Codex);
    assert_eq!(spec.routing.research, Agent::Codex);
}
