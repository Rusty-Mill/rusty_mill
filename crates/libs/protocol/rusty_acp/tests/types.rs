//! Wire-format tests: every type must round-trip through the protocol's JSON.

use rusty_acp::types::*;
use serde_json::json;

#[test]
fn role_round_trips_all_three_forms() {
    for (role, wire) in [
        (Role::User, "user"),
        (Role::Agent, "agent"),
        (Role::agent("summarizer"), "agent/summarizer"),
        (Role::agent("data_processor"), "agent/data_processor"),
        (Role::agent("multi-word-name"), "agent/multi-word-name"),
    ] {
        assert_eq!(serde_json::to_value(&role).unwrap(), json!(wire));
        assert_eq!(serde_json::from_value::<Role>(json!(wire)).unwrap(), role);
    }
}

#[test]
fn role_rejects_malformed_values() {
    for bad in ["", "system", "agent/", "agent//x", "Agent", "user/x", "agent/bad name"] {
        assert!(Role::parse(bad).is_err(), "{bad:?} should be rejected");
    }
}

#[test]
fn agent_name_enforces_rfc_1123_dns_label() {
    for good in ["chat", "a", "my-agent-1", "0", &"a".repeat(63)] {
        assert!(AgentName::new(good).is_ok(), "{good:?} should be accepted");
    }
    for bad in ["", "-chat", "chat-", "Chat", "my_agent", "my.agent", &"a".repeat(64)] {
        assert!(AgentName::new(bad).is_err(), "{bad:?} should be rejected");
    }
}

#[test]
fn message_part_defaults_content_type_to_text_plain() {
    let part: MessagePart = serde_json::from_value(json!({ "content": "hi" })).unwrap();
    assert_eq!(part.content_type, "text/plain");
    assert_eq!(part.encoding(), ContentEncoding::Plain);
    assert_eq!(part.as_text(), Some("hi"));
}

#[test]
fn message_part_rejects_content_and_content_url_together() {
    let part = MessagePart {
        content: Some("inline".into()),
        content_url: Some("https://example.com/a".into()),
        ..Default::default()
    };
    assert_eq!(part.validate().unwrap_err().code, ErrorCode::InvalidInput);
}

#[test]
fn message_text_concatenates_only_plain_text_parts() {
    let message = Message::new(
        Role::Agent,
        [
            MessagePart::text("Hello, "),
            MessagePart::inline("image/png", "iVBORw0KGgo=").with_encoding(ContentEncoding::Base64),
            MessagePart::text("world"),
            MessagePart::from_url("text/plain", "https://example.com/tail.txt"),
        ],
    );
    assert_eq!(message.text(), "Hello, world");
}

#[test]
fn message_requires_at_least_one_part() {
    let message = Message { role: Role::User, parts: vec![], created_at: None, completed_at: None };
    assert!(message.validate().is_err());
}

#[test]
fn part_metadata_is_discriminated_by_kind() {
    let citation = MessagePart::text("cited").with_metadata(CitationMetadata {
        start_index: Some(0),
        end_index: Some(5),
        url: Some("https://example.com".into()),
        ..Default::default()
    });
    let value = serde_json::to_value(&citation).unwrap();
    assert_eq!(value["metadata"]["kind"], "citation");
    assert_eq!(serde_json::from_value::<MessagePart>(value).unwrap(), citation);

    let trajectory = MessagePart::trajectory(TrajectoryMetadata {
        tool_name: Some("search".into()),
        tool_input: Some(json!({ "q": "acp" })),
        ..Default::default()
    });
    let value = serde_json::to_value(&trajectory).unwrap();
    assert_eq!(value["metadata"]["kind"], "trajectory");
    assert_eq!(serde_json::from_value::<MessagePart>(value).unwrap(), trajectory);
}

#[test]
fn run_status_uses_kebab_case_on_the_wire() {
    let cases = [
        (RunStatus::Created, "created"),
        (RunStatus::InProgress, "in-progress"),
        (RunStatus::Awaiting, "awaiting"),
        (RunStatus::Cancelling, "cancelling"),
        (RunStatus::Cancelled, "cancelled"),
        (RunStatus::Completed, "completed"),
        (RunStatus::Failed, "failed"),
    ];
    for (status, wire) in cases {
        assert_eq!(serde_json::to_value(status).unwrap(), json!(wire));
        assert_eq!(status.to_string(), wire);
        assert_eq!(serde_json::from_value::<RunStatus>(json!(wire)).unwrap(), status);
    }
}

#[test]
fn only_terminal_statuses_are_terminal() {
    assert!(RunStatus::Completed.is_terminal());
    assert!(RunStatus::Failed.is_terminal());
    assert!(RunStatus::Cancelled.is_terminal());
    assert!(!RunStatus::Cancelling.is_terminal());
    assert!(!RunStatus::Awaiting.is_terminal());
    assert!(!RunStatus::InProgress.is_terminal());
    assert!(!RunStatus::Created.is_terminal());
}

#[test]
fn events_are_tagged_by_type() {
    let run = Run::new(AgentName::new("chat").unwrap(), None);
    let cases = vec![
        (Event::MessagePart { part: MessagePart::text("hi") }, "message.part"),
        (Event::MessageCreated { message: Message::agent("hi") }, "message.created"),
        (Event::MessageCompleted { message: Message::agent("hi") }, "message.completed"),
        (Event::generic(json!({ "progress": 0.5 })), "generic"),
        (Event::RunCreated { run: Box::new(run.clone()) }, "run.created"),
        (Event::RunInProgress { run: Box::new(run.clone()) }, "run.in-progress"),
        (Event::RunAwaiting { run: Box::new(run.clone()) }, "run.awaiting"),
        (Event::RunCompleted { run: Box::new(run.clone()) }, "run.completed"),
        (Event::RunFailed { run: Box::new(run.clone()) }, "run.failed"),
        (Event::RunCancelled { run: Box::new(run) }, "run.cancelled"),
        (Event::Error { error: Error::server_error("boom") }, "error"),
    ];
    for (event, wire) in cases {
        let value = serde_json::to_value(&event).unwrap();
        assert_eq!(value["type"], json!(wire), "{wire}");
        assert_eq!(event.event_type(), wire);
        assert_eq!(serde_json::from_value::<Event>(value).unwrap(), event);
    }
}

#[test]
fn only_stream_ending_events_are_terminal() {
    let run = Box::new(Run::new(AgentName::new("chat").unwrap(), None));
    assert!(Event::RunCompleted { run: run.clone() }.is_terminal());
    assert!(Event::RunFailed { run: run.clone() }.is_terminal());
    assert!(Event::RunCancelled { run: run.clone() }.is_terminal());
    assert!(Event::RunAwaiting { run: run.clone() }.is_terminal());
    assert!(Event::Error { error: Error::server_error("x") }.is_terminal());
    assert!(!Event::RunCreated { run: run.clone() }.is_terminal());
    assert!(!Event::RunInProgress { run }.is_terminal());
    assert!(!Event::MessagePart { part: MessagePart::text("x") }.is_terminal());
}

#[test]
fn error_codes_map_to_http_statuses() {
    assert_eq!(ErrorCode::ServerError.http_status(), 500);
    assert_eq!(ErrorCode::InvalidInput.http_status(), 422);
    assert_eq!(ErrorCode::NotFound.http_status(), 404);
    assert_eq!(serde_json::to_value(ErrorCode::InvalidInput).unwrap(), json!("invalid_input"));
}

#[test]
fn run_create_request_parses_the_specs_example_shape() {
    let request: RunCreateRequest = serde_json::from_value(json!({
        "agent_name": "chat",
        "input": [{
            "role": "user",
            "parts": [{ "content_type": "text/plain", "content": "Hello" }]
        }],
        "mode": "stream"
    }))
    .unwrap();

    assert_eq!(request.agent_name.as_str(), "chat");
    assert_eq!(request.mode(), RunMode::Stream);
    assert_eq!(request.input[0].text(), "Hello");
    request.validate().unwrap();
}

#[test]
fn run_create_request_defaults_to_sync_mode() {
    let request = RunCreateRequest::new(AgentName::new("chat").unwrap(), [Message::user("hi")]);
    assert_eq!(request.mode(), RunMode::Sync);
    assert_eq!(RunMode::default(), RunMode::Sync);
}

#[test]
fn run_create_request_rejects_empty_input() {
    let request = RunCreateRequest::new(AgentName::new("chat").unwrap(), []);
    assert!(request.validate().is_err());
}

#[test]
fn run_create_request_rejects_conflicting_session_ids() {
    let request = RunCreateRequest::new(AgentName::new("chat").unwrap(), [Message::user("hi")])
        .with_session_id(SessionId::new())
        .with_session(Session::new());
    assert!(request.validate().is_err());
}

#[test]
fn await_payloads_carry_typed_values() {
    #[derive(serde::Serialize, serde::Deserialize, PartialEq, Debug)]
    struct Question {
        question: String,
    }

    let question = Question { question: "Your name?".into() };
    let request = AwaitRequest::from_value(&question).unwrap();
    assert_eq!(request.as_value(), &json!({ "question": "Your name?" }));
    assert_eq!(request.deserialize::<Question>().unwrap(), question);
}

#[test]
fn content_type_matching_honours_wildcards() {
    assert!(content_type_matches("*/*", "image/png"));
    assert!(content_type_matches("image/*", "image/png"));
    assert!(content_type_matches("text/plain", "text/plain"));
    assert!(content_type_matches("text/plain", "text/plain; charset=utf-8"));
    assert!(!content_type_matches("image/*", "text/plain"));
    assert!(!content_type_matches("text/plain", "text/html"));
}

#[test]
fn manifest_accepts_only_declared_content_types() {
    let manifest = AgentManifest::new(AgentName::new("vision").unwrap(), "Sees things")
        .with_input_content_types(["text/plain", "image/*"])
        .with_output_content_types(["text/plain"]);

    assert!(manifest.accepts_input("text/plain"));
    assert!(manifest.accepts_input("image/jpeg"));
    assert!(!manifest.accepts_input("application/pdf"));
    assert!(manifest.produces_output("text/plain"));
    assert!(!manifest.produces_output("image/png"));
    manifest.validate().unwrap();
}

#[test]
fn manifest_round_trips_with_full_metadata() {
    let manifest = AgentManifest::new(AgentName::new("chat").unwrap(), "Conversational agent")
        .with_metadata(
            Metadata::new()
                .with_license("Apache-2.0")
                .with_programming_language("Rust")
                .with_framework("rusty-acp")
                .with_tags([Tag::CHAT, "custom-tag"])
                .with_capabilities([Capability::new("Conversational AI", "Multi-turn chat")])
                .with_links([Link::new(LinkType::SourceCode, "https://example.com/repo")])
                .with_author(Person::new("Ada Lovelace")),
        )
        .with_status(Status { success_rate: Some(99.5), ..Default::default() });

    let value = serde_json::to_value(&manifest).unwrap();
    assert_eq!(value["metadata"]["links"][0]["type"], "source-code");
    assert_eq!(value["metadata"]["tags"][0], "Chat");
    assert_eq!(serde_json::from_value::<AgentManifest>(value).unwrap(), manifest);
}

#[test]
fn session_matches_the_distributed_session_shape() {
    let value = json!({
        "id": "8b1a9953-4c2b-4e2d-9f1a-1c2d3e4f5a6b",
        "history": [
            "http://server-a/session/x/messages/0",
            "http://server-b/session/x/messages/1"
        ],
        "state": "http://server-b/session/x/state"
    });
    let session: Session = serde_json::from_value(value.clone()).unwrap();
    assert_eq!(session.history.len(), 2);
    assert_eq!(session.state.as_deref(), Some("http://server-b/session/x/state"));
    assert_eq!(serde_json::to_value(&session).unwrap(), value);
}

#[test]
fn optional_run_fields_are_omitted_when_absent() {
    let run = Run::new(AgentName::new("chat").unwrap(), None);
    let value = serde_json::to_value(&run).unwrap();
    let object = value.as_object().unwrap();
    assert!(!object.contains_key("session_id"));
    assert!(!object.contains_key("await_request"));
    assert!(!object.contains_key("error"));
    assert!(!object.contains_key("finished_at"));
    // Required by the spec even when empty.
    assert!(object.contains_key("output"));
    assert!(object.contains_key("status"));
    assert!(object.contains_key("created_at"));
}

#[test]
fn failed_run_converts_into_its_error() {
    let mut run = Run::new(AgentName::new("chat").unwrap(), None);
    run.status = RunStatus::Failed;
    run.error = Some(Error::server_error("model unavailable"));
    let error = run.into_result().unwrap_err();
    assert_eq!(error.code, ErrorCode::ServerError);
    assert_eq!(error.message, "model unavailable");
}

#[test]
fn artifacts_are_message_parts_with_a_name() {
    let part = MessagePart::artifact("result.json", "application/json", r#"{"ok":true}"#);

    // The spec defines no artifact type: it is exactly a named part.
    let value = serde_json::to_value(&part).unwrap();
    assert_eq!(value["name"], "result.json");
    assert_eq!(value["content_type"], "application/json");
    assert_eq!(value["content"], r#"{"ok":true}"#);
    // Plain is the default, so it should not be written out.
    assert!(value.get("content_encoding").is_none());

    assert!(part.is_artifact());
    assert_eq!(part.artifact_name(), Some("result.json"));
    assert_eq!(serde_json::from_value::<MessagePart>(value).unwrap(), part);
}

#[test]
fn binary_artifacts_encode_and_declare_base64_together() {
    let bytes = [0x89, 0x50, 0x4e, 0x47, 0x0d, 0x0a];
    let part = MessagePart::binary_artifact("chart.png", "image/png", bytes);

    let value = serde_json::to_value(&part).unwrap();
    assert_eq!(value["content_encoding"], "base64");
    assert_eq!(value["content"], "iVBORw0K");

    // Round-trips back to the original bytes.
    assert_eq!(part.decoded_content().unwrap().unwrap(), bytes.to_vec());
}

#[test]
fn a_plain_part_is_not_an_artifact() {
    let part = MessagePart::text("just prose");
    assert!(!part.is_artifact());
    assert_eq!(part.artifact_name(), None);
    assert_eq!(part.decoded_content().unwrap().unwrap(), b"just prose".to_vec());
}

#[test]
fn decoding_content_reports_malformed_base64() {
    let part =
        MessagePart::inline("image/png", "not base64!!").with_encoding(ContentEncoding::Base64);
    let error = part.decoded_content().unwrap_err();
    assert_eq!(error.code, ErrorCode::InvalidInput);
}

#[test]
fn a_part_with_no_inline_content_decodes_to_none() {
    let part = MessagePart::from_url("image/png", "https://example.com/chart.png");
    assert!(part.decoded_content().unwrap().is_none());
}
