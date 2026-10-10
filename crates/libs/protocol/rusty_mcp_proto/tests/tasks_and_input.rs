//! Invariants and helpers of the task, subscription and input types.

use rusty_json::Value;
use rusty_mcp_proto::{
    DetailedTask, ElicitParams, GetTaskResult, Implementation, InputRequest, InputRequests,
    InputRequiredResult, ListenResult, RequestId, Task, TaskPayload, TaskStatus, Wire,
};

fn task(status: TaskStatus) -> Task {
    Task::new("t1", status, "2026-07-28T10:00:00Z", "2026-07-28T10:00:01Z")
}

#[test]
fn a_detailed_task_cannot_disagree_with_its_payload() {
    let result = Value::object();
    // Built as "working" but given a completed payload: the payload wins.
    let t = DetailedTask::new(task(TaskStatus::Working), TaskPayload::Completed { result });
    assert_eq!(t.task().status, TaskStatus::Completed);
    assert_eq!(t.payload().status(), TaskStatus::Completed);

    let wire = GetTaskResult {
        task: t,
        meta: None,
    }
    .to_json();
    let back = GetTaskResult::from_json(&wire).expect("round trip");
    assert_eq!(back.task.task().status, TaskStatus::Completed);
}

#[test]
fn terminal_statuses() {
    use TaskStatus::*;
    for (s, terminal) in [
        (Working, false),
        (InputRequired, false),
        (Completed, true),
        (Failed, true),
        (Cancelled, true),
    ] {
        assert_eq!(s.is_terminal(), terminal, "{s:?}");
    }
}

#[test]
fn unlimited_ttl_is_an_explicit_null_on_the_wire() {
    let v = Value::from_json_str(&task_json(None)).expect("json");
    assert_eq!(v.get("ttlMs"), Some(&Value::Null));
    let v = Value::from_json_str(&task_json(Some(5))).expect("json");
    assert_eq!(v.get("ttlMs").and_then(Value::as_u64), Some(5));
}

fn task_json(ttl_ms: Option<u64>) -> String {
    let mut t = task(TaskStatus::Working);
    t.ttl_ms = ttl_ms;
    GetTaskResult {
        task: DetailedTask::new(t, TaskPayload::Working),
        meta: None,
    }
    .to_json()
}

#[test]
fn a_task_payload_must_be_an_object() {
    for text in [
        r#"{"resultType":"complete","taskId":"t","status":"completed","createdAt":"a","lastUpdatedAt":"b","ttlMs":null,"result":[]}"#,
        r#"{"resultType":"complete","taskId":"t","status":"failed","createdAt":"a","lastUpdatedAt":"b","ttlMs":null,"error":"boom"}"#,
    ] {
        assert!(GetTaskResult::from_json(text).is_err(), "accepted {text}");
    }
}

#[test]
fn listen_result_names_its_subscription() {
    let r = ListenResult::complete(&RequestId::String("sub-1".into()))
        .with_server_info(&Implementation::new("s", "1"));
    assert_eq!(r.subscription_id(), Some(RequestId::String("sub-1".into())));
    let back = ListenResult::from_json(&r.to_json()).expect("round trip");
    assert_eq!(back, r);
    // A subscription id that is not a number or string is refused.
    assert!(ListenResult::from_json(
        r#"{"resultType":"complete","_meta":{"io.modelcontextprotocol/subscriptionId":true}}"#
    )
    .is_err());
}

#[test]
fn input_required_round_trips_an_elicitation() {
    let mut schema = Value::object();
    schema.insert("type", "object");
    schema.insert("properties", Value::object());
    let ask = ElicitParams::Form {
        message: "Name?".into(),
        requested_schema: schema,
        meta: None,
    };
    let mut requests = InputRequests::new();
    requests.insert("who".into(), InputRequest::elicitation(&ask));
    let result = InputRequiredResult::from_input_requests(requests);
    let back = InputRequiredResult::from_json(&result.to_json()).expect("round trip");
    assert_eq!(back, result);
    let sent = back.input_requests.expect("requests");
    let params = sent["who"].params.as_ref().expect("params");
    assert_eq!(ElicitParams::from_value(params).expect("elicit"), ask);
}
