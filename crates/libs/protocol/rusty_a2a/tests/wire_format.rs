//! Wire-format conformance tests for `rusty_a2a::types`.
//!
//! The data model's central claim is that it serializes to exactly the JSON
//! the A2A spec mandates. Nothing else in the suite tests that: the
//! integration tests drive this crate's own client against this crate's own
//! server, so a symmetric mistake — a field misnamed in both directions, an
//! enum spelled the same way on each side — passes every one of them and
//! still fails against a Python or Go agent.
//!
//! The authority is the vendored [`spec/a2a.proto`](../spec/a2a.proto), which
//! the spec makes normative. proto3's canonical JSON mapping says a field's
//! JSON name is the lowerCamelCase of its proto name, and an enum value's is
//! its proto name verbatim. So the expectations here are written as **proto
//! field names**, camel-cased by [`camel`] at assert time: each list can be
//! diffed line-for-line against the `message` block it cites, and nothing has
//! to be hand-transcribed into camelCase (which is where a typo would hide).
//!
//! These tests need no features — `types` is always compiled.

use std::collections::HashMap;

use chrono::{DateTime, Utc};
use rusty_a2a::types::*;
use serde::Serialize;
use serde_json::{json, Value};

/// proto3 canonical JSON: a field's JSON name is the lowerCamelCase of its
/// proto name (`media_type` -> `mediaType`).
fn camel(proto_field: &str) -> String {
    let mut out = String::with_capacity(proto_field.len());
    let mut upper_next = false;
    for c in proto_field.chars() {
        if c == '_' {
            upper_next = true;
        } else if upper_next {
            out.extend(c.to_uppercase());
            upper_next = false;
        } else {
            out.push(c);
        }
    }
    out
}

/// Asserts that `value` serializes to an object whose keys are exactly the
/// camelCase forms of `proto_fields`.
#[track_caller]
fn assert_fields<T: Serialize>(value: &T, proto_fields: &[&str]) {
    let serialized = serde_json::to_value(value).expect("serializes");
    let object = serialized
        .as_object()
        .unwrap_or_else(|| panic!("expected a JSON object, got {serialized}"));

    let mut got: Vec<String> = object.keys().cloned().collect();
    let mut want: Vec<String> = proto_fields.iter().map(|f| camel(f)).collect();
    got.sort();
    want.sort();
    assert_eq!(got, want, "field names differ from the proto");
}

/// Asserts that a value survives JSON without changing shape. Most of these
/// types do not implement `PartialEq`, and JSON-level idempotence is the
/// stronger property anyway: it catches a field that serializes under one
/// name and deserializes under another.
#[track_caller]
fn assert_round_trips<T>(value: &T)
where
    T: Serialize + for<'de> serde::Deserialize<'de>,
{
    let once = serde_json::to_value(value).expect("serializes");
    let parsed: T = serde_json::from_value(once.clone()).expect("deserializes");
    let twice = serde_json::to_value(&parsed).expect("re-serializes");
    assert_eq!(once, twice, "value changed shape through a JSON round trip");
}

fn timestamp() -> DateTime<Utc> {
    DateTime::parse_from_rfc3339("2026-08-07T00:01:45.123Z")
        .unwrap()
        .with_timezone(&Utc)
}

fn metadata() -> serde_json::Map<String, Value> {
    json!({"trace": "abc"}).as_object().unwrap().clone()
}

// ---------------------------------------------------------------------------
// Field naming
// ---------------------------------------------------------------------------

/// proto `message Task`.
#[test]
fn task_field_names() {
    let mut task = Task::new("task-1", "ctx-1", TaskState::Working);
    task.artifacts = vec![Artifact::new("a-1", vec![Part::text("hi")])];
    task.history = vec![Message::user_text("hello")];
    task.metadata = Some(metadata());

    assert_fields(
        &task,
        &["id", "context_id", "status", "artifacts", "history", "metadata"],
    );
    assert_round_trips(&task);
}

/// proto `message TaskStatus`.
#[test]
fn task_status_field_names() {
    let status = TaskStatus {
        state: TaskState::Completed,
        message: Some(Message::agent_text("done")),
        timestamp: Some(timestamp()),
    };
    assert_fields(&status, &["state", "message", "timestamp"]);
    assert_round_trips(&status);
}

/// proto `message Artifact`.
#[test]
fn artifact_field_names() {
    let artifact = Artifact {
        artifact_id: "a-1".into(),
        name: Some("report".into()),
        description: Some("the report".into()),
        parts: vec![Part::text("body")],
        metadata: Some(metadata()),
        extensions: vec!["https://example.com/ext".into()],
    };
    assert_fields(
        &artifact,
        &[
            "artifact_id",
            "name",
            "description",
            "parts",
            "metadata",
            "extensions",
        ],
    );
    assert_round_trips(&artifact);
}

/// proto `message Message`.
#[test]
fn message_field_names() {
    let mut message = Message::user_text("hello");
    message.context_id = Some("ctx-1".into());
    message.task_id = Some("task-1".into());
    message.metadata = Some(metadata());
    message.extensions = vec!["https://example.com/ext".into()];
    message.reference_task_ids = vec!["task-0".into()];

    assert_fields(
        &message,
        &[
            "message_id",
            "context_id",
            "task_id",
            "role",
            "parts",
            "metadata",
            "extensions",
            "reference_task_ids",
        ],
    );
    assert_round_trips(&message);
}

/// proto `message Part`: the `content` oneof plus three plain fields.
#[test]
fn part_field_names() {
    let part = Part::text("hi")
        .with_filename("note.txt")
        .with_media_type("text/plain");
    let mut part = part;
    part.metadata = Some(metadata());

    assert_fields(&part, &["text", "metadata", "filename", "media_type"]);
    assert_round_trips(&part);
}

/// proto `message TaskStatusUpdateEvent`.
#[test]
fn task_status_update_event_field_names() {
    let event = TaskStatusUpdateEvent {
        task_id: "task-1".into(),
        context_id: "ctx-1".into(),
        status: TaskStatus::new(TaskState::Working),
        metadata: Some(metadata()),
    };
    assert_fields(&event, &["task_id", "context_id", "status", "metadata"]);
    assert_round_trips(&event);
}

/// proto `message TaskArtifactUpdateEvent`.
#[test]
fn task_artifact_update_event_field_names() {
    let event = TaskArtifactUpdateEvent {
        task_id: "task-1".into(),
        context_id: "ctx-1".into(),
        artifact: Artifact::new("a-1", vec![Part::text("chunk")]),
        append: true,
        last_chunk: false,
        metadata: Some(metadata()),
    };
    assert_fields(
        &event,
        &[
            "task_id",
            "context_id",
            "artifact",
            "append",
            "last_chunk",
            "metadata",
        ],
    );
    assert_round_trips(&event);
}

/// proto `message AgentInterface`.
#[test]
fn agent_interface_field_names() {
    let mut interface = AgentInterface::json_rpc("http://localhost:8080");
    interface.tenant = Some("t-1".into());
    assert_fields(
        &interface,
        &["url", "protocol_binding", "tenant", "protocol_version"],
    );
    assert_round_trips(&interface);
}

/// proto `message AgentCapabilities`.
#[test]
fn agent_capabilities_field_names() {
    let capabilities = AgentCapabilities {
        streaming: Some(true),
        push_notifications: Some(true),
        extensions: vec![AgentExtension {
            uri: "https://example.com/ext".into(),
            description: "an extension".into(),
            required: false,
            params: None,
        }],
        extended_agent_card: Some(true),
    };
    assert_fields(
        &capabilities,
        &[
            "streaming",
            "push_notifications",
            "extensions",
            "extended_agent_card",
        ],
    );
    assert_round_trips(&capabilities);
}

/// proto `message AgentExtension`.
#[test]
fn agent_extension_field_names() {
    let extension = AgentExtension {
        uri: "https://example.com/ext".into(),
        description: "an extension".into(),
        required: true,
        params: Some(metadata()),
    };
    assert_fields(&extension, &["uri", "description", "required", "params"]);
    assert_round_trips(&extension);
}

/// proto `message AgentSkill`.
#[test]
fn agent_skill_field_names() {
    let skill = AgentSkill {
        id: "echo".into(),
        name: "Echo".into(),
        description: "Repeats you.".into(),
        tags: vec!["util".into()],
        examples: vec!["say hi".into()],
        input_modes: vec!["text/plain".into()],
        output_modes: vec!["text/plain".into()],
        security_requirements: vec![SecurityRequirement {
            schemes: HashMap::from([(
                "bearer".to_string(),
                StringList {
                    list: vec!["read".into()],
                },
            )]),
        }],
    };
    assert_fields(
        &skill,
        &[
            "id",
            "name",
            "description",
            "tags",
            "examples",
            "input_modes",
            "output_modes",
            "security_requirements",
        ],
    );
    assert_round_trips(&skill);
}

/// proto `message AgentCardSignature`.
#[test]
fn agent_card_signature_field_names() {
    let signature = AgentCardSignature {
        protected: "eyJhbGciOiJFUzI1NiJ9".into(),
        signature: "c2ln".into(),
        header: Some(metadata()),
    };
    assert_fields(&signature, &["protected", "signature", "header"]);
    assert_round_trips(&signature);
}

/// proto `message AgentProvider`.
#[test]
fn agent_provider_field_names() {
    let provider = AgentProvider {
        url: "https://example.com".into(),
        organization: "Example".into(),
    };
    assert_fields(&provider, &["url", "organization"]);
    assert_round_trips(&provider);
}

/// proto `message AgentCard`.
#[test]
fn agent_card_field_names() {
    let card = fully_populated_card();
    assert_fields(
        &card,
        &[
            "name",
            "description",
            "supported_interfaces",
            "provider",
            "version",
            "documentation_url",
            "capabilities",
            "security_schemes",
            "security_requirements",
            "default_input_modes",
            "default_output_modes",
            "skills",
            "signatures",
            "icon_url",
        ],
    );
    assert_round_trips(&card);
}

/// proto `message TaskPushNotificationConfig` and `message AuthenticationInfo`.
#[test]
fn push_notification_config_field_names() {
    let config = TaskPushNotificationConfig {
        tenant: Some("t-1".into()),
        id: Some("cfg-1".into()),
        task_id: Some("task-1".into()),
        url: "https://example.com/hook".into(),
        token: Some("tok".into()),
        authentication: Some(AuthenticationInfo {
            scheme: "Bearer".into(),
            credentials: Some("secret".into()),
        }),
    };
    assert_fields(
        &config,
        &["tenant", "id", "task_id", "url", "token", "authentication"],
    );
    assert_fields(
        config.authentication.as_ref().unwrap(),
        &["scheme", "credentials"],
    );
    assert_round_trips(&config);
}

/// proto `message SendMessageConfiguration` and `message SendMessageRequest`.
#[test]
fn send_message_field_names() {
    let configuration = SendMessageConfiguration {
        accepted_output_modes: vec!["text/plain".into()],
        task_push_notification_config: Some(TaskPushNotificationConfig::new("https://example.com/hook")),
        history_length: Some(5),
        return_immediately: true,
    };
    assert_fields(
        &configuration,
        &[
            "accepted_output_modes",
            "task_push_notification_config",
            "history_length",
            "return_immediately",
        ],
    );

    let request = SendMessageRequest {
        tenant: Some("t-1".into()),
        message: Message::user_text("hi"),
        configuration: Some(configuration),
        metadata: Some(metadata()),
    };
    assert_fields(&request, &["tenant", "message", "configuration", "metadata"]);
    assert_round_trips(&request);
}

/// proto `message GetTaskRequest`, `CancelTaskRequest`, `SubscribeToTaskRequest`.
#[test]
fn task_request_field_names() {
    assert_fields(
        &GetTaskRequest {
            tenant: Some("t-1".into()),
            id: "task-1".into(),
            history_length: Some(3),
        },
        &["tenant", "id", "history_length"],
    );
    assert_fields(
        &CancelTaskRequest {
            tenant: Some("t-1".into()),
            id: "task-1".into(),
            metadata: Some(metadata()),
        },
        &["tenant", "id", "metadata"],
    );
    assert_fields(
        &SubscribeToTaskRequest {
            tenant: Some("t-1".into()),
            id: "task-1".into(),
        },
        &["tenant", "id"],
    );
}

/// proto `message ListTasksRequest` and `message ListTasksResponse`.
#[test]
fn list_tasks_field_names() {
    let request = ListTasksRequest {
        tenant: Some("t-1".into()),
        context_id: Some("ctx-1".into()),
        status: Some(TaskState::Working),
        page_size: Some(10),
        page_token: Some("tok".into()),
        history_length: Some(3),
        status_timestamp_after: Some(timestamp()),
        include_artifacts: Some(true),
    };
    assert_fields(
        &request,
        &[
            "tenant",
            "context_id",
            "status",
            "page_size",
            "page_token",
            "history_length",
            "status_timestamp_after",
            "include_artifacts",
        ],
    );
    assert_round_trips(&request);

    let response = ListTasksResponse {
        tasks: vec![Task::new("task-1", "ctx-1", TaskState::Submitted)],
        next_page_token: "tok".into(),
        page_size: 10,
        total_size: 1,
    };
    assert_fields(
        &response,
        &["tasks", "next_page_token", "page_size", "total_size"],
    );
    assert_round_trips(&response);
}

/// proto `message GetTaskPushNotificationConfigRequest`,
/// `DeleteTaskPushNotificationConfigRequest`,
/// `ListTaskPushNotificationConfigsRequest` and its response.
#[test]
fn push_config_request_field_names() {
    assert_fields(
        &GetTaskPushNotificationConfigRequest {
            tenant: Some("t-1".into()),
            task_id: "task-1".into(),
            id: "cfg-1".into(),
        },
        &["tenant", "task_id", "id"],
    );
    assert_fields(
        &DeleteTaskPushNotificationConfigRequest {
            tenant: Some("t-1".into()),
            task_id: "task-1".into(),
            id: "cfg-1".into(),
        },
        &["tenant", "task_id", "id"],
    );
    assert_fields(
        &ListTaskPushNotificationConfigsRequest {
            tenant: Some("t-1".into()),
            task_id: "task-1".into(),
            page_size: Some(10),
            page_token: Some("tok".into()),
        },
        &["tenant", "task_id", "page_size", "page_token"],
    );
    assert_fields(
        &ListTaskPushNotificationConfigsResponse {
            configs: vec![TaskPushNotificationConfig::new("https://example.com/hook")],
            next_page_token: "tok".into(),
        },
        &["configs", "next_page_token"],
    );
}

// ---------------------------------------------------------------------------
// Enums
// ---------------------------------------------------------------------------

/// proto3 canonical JSON writes an enum as its proto value name verbatim.
/// proto `enum TaskState`.
#[test]
fn task_state_uses_proto_enum_names() {
    let cases = [
        (TaskState::Unspecified, "TASK_STATE_UNSPECIFIED"),
        (TaskState::Submitted, "TASK_STATE_SUBMITTED"),
        (TaskState::Working, "TASK_STATE_WORKING"),
        (TaskState::Completed, "TASK_STATE_COMPLETED"),
        (TaskState::Failed, "TASK_STATE_FAILED"),
        (TaskState::Canceled, "TASK_STATE_CANCELED"),
        (TaskState::InputRequired, "TASK_STATE_INPUT_REQUIRED"),
        (TaskState::Rejected, "TASK_STATE_REJECTED"),
        (TaskState::AuthRequired, "TASK_STATE_AUTH_REQUIRED"),
    ];
    for (state, expected) in cases {
        assert_eq!(serde_json::to_value(state).unwrap(), json!(expected));
        let parsed: TaskState = serde_json::from_value(json!(expected)).unwrap();
        assert_eq!(parsed, state);
    }
}

/// proto `enum Role`.
#[test]
fn role_uses_proto_enum_names() {
    let cases = [
        (Role::Unspecified, "ROLE_UNSPECIFIED"),
        (Role::User, "ROLE_USER"),
        (Role::Agent, "ROLE_AGENT"),
    ];
    for (role, expected) in cases {
        assert_eq!(serde_json::to_value(role).unwrap(), json!(expected));
        let parsed: Role = serde_json::from_value(json!(expected)).unwrap();
        assert_eq!(parsed, role);
    }
}

/// An unrecognized enum value is rejected rather than silently defaulting —
/// a peer that sends a state from a newer spec revision should produce an
/// error, not a task that looks `UNSPECIFIED`.
#[test]
fn unknown_enum_values_are_rejected() {
    assert!(serde_json::from_value::<TaskState>(json!("TASK_STATE_PAUSED")).is_err());
    assert!(serde_json::from_value::<TaskState>(json!("submitted")).is_err());
    assert!(serde_json::from_value::<Role>(json!("user")).is_err());
}

// ---------------------------------------------------------------------------
// Oneofs
// ---------------------------------------------------------------------------

/// proto `message Part`, `oneof content`: exactly one content key, named for
/// the proto field that carries it.
#[test]
fn part_content_oneof_keys() {
    let cases: [(Part, &str); 4] = [
        (Part::text("hi"), "text"),
        (Part::raw(*b"hi"), "raw"),
        (Part::url("https://example.com/f"), "url"),
        (Part::data(json!({"k": 1})), "data"),
    ];
    for (part, expected_key) in cases {
        let serialized = serde_json::to_value(&part).unwrap();
        let object = serialized.as_object().unwrap();
        let content_keys: Vec<&String> = object
            .keys()
            .filter(|k| !matches!(k.as_str(), "metadata" | "filename" | "mediaType"))
            .collect();
        assert_eq!(
            content_keys,
            vec![expected_key],
            "expected exactly one content key"
        );
        assert_round_trips(&part);
    }
}

/// Spec Section 4.1.6: "A Part MUST contain exactly one of the following:
/// text, raw, url, data" - a JSON object with zero or more than one of
/// those keys must be rejected, not silently resolved to whichever
/// variant happens to be tried first.
#[test]
fn part_content_rejects_zero_or_multiple_content_keys() {
    let zero = json!({});
    let two = json!({"text": "hi", "data": {"k": 1}});
    for bad in [zero, two] {
        let full = {
            let mut obj = bad.as_object().unwrap().clone();
            obj.insert("mediaType".to_string(), json!("text/plain"));
            Value::Object(obj)
        };
        let err = serde_json::from_value::<Part>(full.clone())
            .expect_err(&format!("expected an error deserializing {full}"));
        assert!(
            err.to_string().contains("exactly one"),
            "expected an 'exactly one' validation error, got: {err}"
        );
    }
}

/// proto `message SecurityScheme`, `oneof scheme`.
#[test]
fn security_scheme_oneof_keys() {
    let cases: [(SecurityScheme, &str); 5] = [
        (
            SecurityScheme::ApiKey {
                api_key_security_scheme: ApiKeySecurityScheme {
                    description: None,
                    location: "header".into(),
                    name: "X-Key".into(),
                },
            },
            "api_key_security_scheme",
        ),
        (
            SecurityScheme::HttpAuth {
                http_auth_security_scheme: HttpAuthSecurityScheme {
                    description: None,
                    scheme: "Bearer".into(),
                    bearer_format: Some("JWT".into()),
                },
            },
            "http_auth_security_scheme",
        ),
        (
            SecurityScheme::OAuth2 {
                oauth2_security_scheme: OAuth2SecurityScheme {
                    description: None,
                    flows: OAuthFlows::ClientCredentials {
                        client_credentials: ClientCredentialsOAuthFlow {
                            token_url: "https://example.com/token".into(),
                            refresh_url: None,
                            scopes: HashMap::new(),
                        },
                    },
                    oauth2_metadata_url: None,
                },
            },
            "oauth2_security_scheme",
        ),
        (
            SecurityScheme::OpenIdConnect {
                open_id_connect_security_scheme: OpenIdConnectSecurityScheme {
                    description: None,
                    open_id_connect_url: "https://example.com/.well-known/openid".into(),
                },
            },
            "open_id_connect_security_scheme",
        ),
        (
            SecurityScheme::MutualTls {
                mtls_security_scheme: MutualTlsSecurityScheme { description: None },
            },
            "mtls_security_scheme",
        ),
    ];
    for (scheme, proto_field) in cases {
        assert_fields(&scheme, &[proto_field]);
        assert_round_trips(&scheme);
    }
}

/// Spec Section 4.5.1: "A SecurityScheme MUST contain exactly one of the
/// following: apiKeySecurityScheme, httpAuthSecurityScheme,
/// oauth2SecurityScheme, openIdConnectSecurityScheme, mtlsSecurityScheme".
#[test]
fn security_scheme_rejects_zero_or_multiple_scheme_keys() {
    let zero = json!({});
    let two = json!({
        "httpAuthSecurityScheme": {"scheme": "Bearer"},
        "mtlsSecurityScheme": {},
    });
    for bad in [zero, two] {
        let err = serde_json::from_value::<SecurityScheme>(bad.clone())
            .expect_err(&format!("expected an error deserializing {bad}"));
        assert!(
            err.to_string().contains("exactly one"),
            "expected an 'exactly one' validation error, got: {err}"
        );
    }
}

/// proto `message OAuthFlows`, `oneof flow`.
#[test]
fn oauth_flow_oneof_keys() {
    let scopes = || HashMap::from([("read".to_string(), "Read access".to_string())]);
    let cases: [(OAuthFlows, &str); 5] = [
        (
            OAuthFlows::AuthorizationCode {
                authorization_code: AuthorizationCodeOAuthFlow {
                    authorization_url: "https://example.com/auth".into(),
                    token_url: "https://example.com/token".into(),
                    refresh_url: None,
                    scopes: scopes(),
                    pkce_required: true,
                },
            },
            "authorization_code",
        ),
        (
            OAuthFlows::ClientCredentials {
                client_credentials: ClientCredentialsOAuthFlow {
                    token_url: "https://example.com/token".into(),
                    refresh_url: None,
                    scopes: scopes(),
                },
            },
            "client_credentials",
        ),
        (
            OAuthFlows::Implicit {
                implicit: ImplicitOAuthFlow {
                    authorization_url: "https://example.com/auth".into(),
                    refresh_url: None,
                    scopes: scopes(),
                },
            },
            "implicit",
        ),
        (
            OAuthFlows::Password {
                password: PasswordOAuthFlow {
                    token_url: "https://example.com/token".into(),
                    refresh_url: None,
                    scopes: scopes(),
                },
            },
            "password",
        ),
        (
            OAuthFlows::DeviceCode {
                device_code: DeviceCodeOAuthFlow {
                    device_authorization_url: "https://example.com/device".into(),
                    token_url: "https://example.com/token".into(),
                    refresh_url: None,
                    scopes: scopes(),
                },
            },
            "device_code",
        ),
    ];
    for (flow, proto_field) in cases {
        assert_fields(&flow, &[proto_field]);
        assert_round_trips(&flow);
    }
}

/// Spec Section 4.5.7: "A OAuthFlows MUST contain exactly one of the
/// following: authorizationCode, clientCredentials, implicit, password,
/// deviceCode".
#[test]
fn oauth_flows_rejects_zero_or_multiple_flow_keys() {
    let zero = json!({});
    let two = json!({
        "implicit": {"authorizationUrl": "https://example.com/auth"},
        "password": {"tokenUrl": "https://example.com/token"},
    });
    for bad in [zero, two] {
        let err = serde_json::from_value::<OAuthFlows>(bad.clone())
            .expect_err(&format!("expected an error deserializing {bad}"));
        assert!(
            err.to_string().contains("exactly one"),
            "expected an 'exactly one' validation error, got: {err}"
        );
    }
}

/// proto `message AuthorizationCodeOAuthFlow` and `DeviceCodeOAuthFlow`, the
/// two flows whose field names camel-case non-trivially.
#[test]
fn oauth_flow_field_names() {
    assert_fields(
        &AuthorizationCodeOAuthFlow {
            authorization_url: "https://example.com/auth".into(),
            token_url: "https://example.com/token".into(),
            refresh_url: Some("https://example.com/refresh".into()),
            scopes: HashMap::new(),
            pkce_required: true,
        },
        &[
            "authorization_url",
            "token_url",
            "refresh_url",
            "scopes",
            "pkce_required",
        ],
    );
    assert_fields(
        &DeviceCodeOAuthFlow {
            device_authorization_url: "https://example.com/device".into(),
            token_url: "https://example.com/token".into(),
            refresh_url: Some("https://example.com/refresh".into()),
            scopes: HashMap::new(),
        },
        &["device_authorization_url", "token_url", "refresh_url", "scopes"],
    );
}

/// proto `message SendMessageResponse`, `oneof payload`.
#[test]
fn send_message_result_oneof_keys() {
    assert_fields(
        &SendMessageResult::Task {
            task: Task::new("task-1", "ctx-1", TaskState::Completed),
        },
        &["task"],
    );
    assert_fields(
        &SendMessageResult::Message {
            message: Message::agent_text("hi"),
        },
        &["message"],
    );
}

/// proto `message StreamResponse`, `oneof payload`.
#[test]
fn stream_response_oneof_keys() {
    assert_fields(
        &StreamResponse::Task {
            task: Task::new("task-1", "ctx-1", TaskState::Working),
        },
        &["task"],
    );
    assert_fields(
        &StreamResponse::Message {
            message: Message::agent_text("hi"),
        },
        &["message"],
    );
    assert_fields(
        &StreamResponse::StatusUpdate {
            status_update: TaskStatusUpdateEvent {
                task_id: "task-1".into(),
                context_id: "ctx-1".into(),
                status: TaskStatus::new(TaskState::Working),
                metadata: None,
            },
        },
        &["status_update"],
    );
    assert_fields(
        &StreamResponse::ArtifactUpdate {
            artifact_update: TaskArtifactUpdateEvent {
                task_id: "task-1".into(),
                context_id: "ctx-1".into(),
                artifact: Artifact::new("a-1", vec![Part::text("x")]),
                append: false,
                last_chunk: true,
                metadata: None,
            },
        },
        &["artifact_update"],
    );
}

/// A stream payload must deserialize back into the variant it was written
/// from. The union is untagged, so a wrapper key that overlapped another
/// variant's shape would silently resolve to the wrong arm.
#[test]
fn stream_response_variants_are_distinguishable() {
    let events: Vec<StreamResponse> = vec![
        Task::new("task-1", "ctx-1", TaskState::Working).into(),
        Message::agent_text("hi").into(),
        TaskStatusUpdateEvent {
            task_id: "task-1".into(),
            context_id: "ctx-1".into(),
            status: TaskStatus::new(TaskState::Completed),
            metadata: None,
        }
        .into(),
        TaskArtifactUpdateEvent {
            task_id: "task-1".into(),
            context_id: "ctx-1".into(),
            artifact: Artifact::new("a-1", vec![Part::text("x")]),
            append: false,
            last_chunk: true,
            metadata: None,
        }
        .into(),
    ];

    for event in &events {
        let json = serde_json::to_value(event).unwrap();
        let parsed: StreamResponse = serde_json::from_value(json).unwrap();
        assert_eq!(
            std::mem::discriminant(&parsed),
            std::mem::discriminant(event),
            "round trip landed on a different variant"
        );
    }
}

// ---------------------------------------------------------------------------
// Scalar encodings
// ---------------------------------------------------------------------------

/// proto3 canonical JSON encodes `bytes` as base64 with the standard
/// alphabet and padding. `Part.raw` is the only `bytes` field in the spec.
#[test]
fn raw_bytes_are_standard_padded_base64() {
    // Two bytes so the encoding needs padding, and 0xFB/0xFF so the last two
    // alphabet positions ('+' and '/') appear — the characters that differ
    // between the standard and URL-safe alphabets.
    let part = Part::raw(vec![0xFB, 0xFF]);
    assert_eq!(serde_json::to_value(&part).unwrap(), json!({"raw": "+/8="}));

    let parsed: Part = serde_json::from_value(json!({"raw": "+/8="})).unwrap();
    assert_eq!(serde_json::to_value(&parsed).unwrap(), json!({"raw": "+/8="}));
}

/// `google.protobuf.Timestamp` fields are RFC 3339, UTC, `Z`-suffixed.
#[test]
fn timestamps_are_rfc3339_utc() {
    let status = TaskStatus {
        state: TaskState::Working,
        message: None,
        timestamp: Some(timestamp()),
    };
    assert_eq!(
        serde_json::to_value(&status).unwrap()["timestamp"],
        json!("2026-08-07T00:01:45.123Z")
    );
}

/// A non-UTC offset is accepted on the way in and normalized to `Z` on the
/// way out: peers are entitled to send any RFC 3339 offset.
#[test]
fn non_utc_timestamps_are_normalized() {
    let parsed: TaskStatus = serde_json::from_value(json!({
        "state": "TASK_STATE_WORKING",
        "timestamp": "2026-08-06T19:01:45.123-05:00",
    }))
    .unwrap();
    assert_eq!(
        serde_json::to_value(&parsed).unwrap()["timestamp"],
        json!("2026-08-07T00:01:45.123Z")
    );
}

// ---------------------------------------------------------------------------
// Presence
// ---------------------------------------------------------------------------

/// Absent optional fields are omitted, never `null`. A peer distinguishing
/// "unset" from "explicitly null" would read a null as a present value.
#[test]
fn absent_optionals_are_omitted_not_null() {
    let task = Task {
        id: "task-1".into(),
        context_id: None,
        status: TaskStatus {
            state: TaskState::Submitted,
            message: None,
            timestamp: None,
        },
        artifacts: vec![],
        history: vec![],
        metadata: None,
    };
    let serialized = serde_json::to_value(&task).unwrap();
    assert_eq!(
        serialized,
        json!({"id": "task-1", "status": {"state": "TASK_STATE_SUBMITTED"}})
    );

    let message = Message::user_text("hi");
    let serialized = serde_json::to_value(&message).unwrap();
    let object = serialized.as_object().unwrap();
    assert!(!object.values().any(Value::is_null), "found a null: {serialized}");
    // Repeated fields are omitted when empty rather than sent as `[]`.
    for absent in [
        "contextId",
        "taskId",
        "metadata",
        "extensions",
        "referenceTaskIds",
    ] {
        assert!(!object.contains_key(absent), "{absent} should be omitted");
    }
}

/// Required fields are always present, even at their type's default value —
/// a `Task` with no artifacts still has to say which state it is in.
#[test]
fn required_fields_are_always_present() {
    let response = ListTasksResponse {
        tasks: vec![],
        next_page_token: String::new(),
        page_size: 0,
        total_size: 0,
    };
    assert_fields(
        &response,
        &["tasks", "next_page_token", "page_size", "total_size"],
    );

    let skill = AgentSkill::new("echo", "Echo", "Repeats you.");
    let serialized = serde_json::to_value(&skill).unwrap();
    // `tags` is REQUIRED in the proto, so it ships as `[]` rather than vanishing.
    assert_eq!(serialized["tags"], json!([]));
}

/// Unknown fields are ignored, so a peer on a later spec revision that adds
/// a field does not break this one.
#[test]
fn unknown_fields_are_ignored() {
    let parsed: Task = serde_json::from_value(json!({
        "id": "task-1",
        "status": {"state": "TASK_STATE_WORKING", "somethingNew": 1},
        "aFieldFromTheFuture": {"nested": true},
    }))
    .unwrap();
    assert_eq!(parsed.id, "task-1");
    assert_eq!(parsed.status.state, TaskState::Working);
}

// ---------------------------------------------------------------------------
// A whole document
// ---------------------------------------------------------------------------

fn fully_populated_card() -> AgentCard {
    AgentCard {
        name: "Echo Agent".into(),
        description: "Echoes back whatever you send it.".into(),
        supported_interfaces: vec![AgentInterface::json_rpc("http://localhost:8080")],
        provider: Some(AgentProvider {
            url: "https://example.com".into(),
            organization: "Example".into(),
        }),
        version: "0.1.0".into(),
        documentation_url: Some("https://example.com/docs".into()),
        capabilities: AgentCapabilities {
            streaming: Some(true),
            push_notifications: Some(true),
            extensions: vec![AgentExtension {
                uri: "https://example.com/ext".into(),
                description: "an extension".into(),
                required: false,
                params: None,
            }],
            extended_agent_card: Some(false),
        },
        security_schemes: HashMap::from([(
            "bearer".to_string(),
            SecurityScheme::HttpAuth {
                http_auth_security_scheme: HttpAuthSecurityScheme {
                    description: Some("A bearer token.".into()),
                    scheme: "Bearer".into(),
                    bearer_format: Some("JWT".into()),
                },
            },
        )]),
        security_requirements: vec![SecurityRequirement {
            schemes: HashMap::from([(
                "bearer".to_string(),
                StringList {
                    list: vec!["read".into()],
                },
            )]),
        }],
        default_input_modes: vec!["text/plain".into()],
        default_output_modes: vec!["text/plain".into()],
        skills: vec![AgentSkill::new("echo", "Echo", "Repeats your message back.").with_tags(["util"])],
        signatures: vec![AgentCardSignature {
            protected: "eyJhbGciOiJFUzI1NiJ9".into(),
            signature: "c2ln".into(),
            header: None,
        }],
        icon_url: Some("https://example.com/icon.png".into()),
    }
}

/// The document served at `/.well-known/agent-card.json`, spelled out. This
/// is what another SDK actually parses at discovery time, so it is asserted
/// literally rather than derived — if any nested name drifts, the diff shows
/// exactly what a peer would have choked on.
#[test]
fn agent_card_matches_its_published_json() {
    let expected = json!({
        "name": "Echo Agent",
        "description": "Echoes back whatever you send it.",
        "supportedInterfaces": [{
            "url": "http://localhost:8080",
            "protocolBinding": "JSONRPC",
            "protocolVersion": "1.0",
        }],
        "provider": {"url": "https://example.com", "organization": "Example"},
        "version": "0.1.0",
        "documentationUrl": "https://example.com/docs",
        "capabilities": {
            "streaming": true,
            "pushNotifications": true,
            "extensions": [{
                "uri": "https://example.com/ext",
                "description": "an extension",
                "required": false,
            }],
            "extendedAgentCard": false,
        },
        "securitySchemes": {
            "bearer": {
                "httpAuthSecurityScheme": {
                    "description": "A bearer token.",
                    "scheme": "Bearer",
                    "bearerFormat": "JWT",
                },
            },
        },
        "securityRequirements": [{"schemes": {"bearer": {"list": ["read"]}}}],
        "defaultInputModes": ["text/plain"],
        "defaultOutputModes": ["text/plain"],
        "skills": [{
            "id": "echo",
            "name": "Echo",
            "description": "Repeats your message back.",
            "tags": ["util"],
        }],
        "signatures": [{
            "protected": "eyJhbGciOiJFUzI1NiJ9",
            "signature": "c2ln",
        }],
        "iconUrl": "https://example.com/icon.png",
    });

    assert_eq!(serde_json::to_value(fully_populated_card()).unwrap(), expected);

    // ...and it parses back from that exact document.
    let parsed: AgentCard = serde_json::from_value(expected.clone()).unwrap();
    assert_eq!(serde_json::to_value(&parsed).unwrap(), expected);
}

/// A `Task` carrying every part kind, spelled out the same way. This is the
/// payload a peer receives from `GetTask`.
#[test]
fn task_matches_its_published_json() {
    let task = Task {
        id: "task-1".into(),
        context_id: Some("ctx-1".into()),
        status: TaskStatus {
            state: TaskState::Completed,
            message: Some(Message {
                message_id: "msg-1".into(),
                context_id: None,
                task_id: None,
                role: Role::Agent,
                parts: vec![Part::text("all done")],
                metadata: None,
                extensions: vec![],
                reference_task_ids: vec![],
            }),
            timestamp: Some(timestamp()),
        },
        artifacts: vec![Artifact {
            artifact_id: "a-1".into(),
            name: Some("result".into()),
            description: None,
            parts: vec![
                Part::text("summary"),
                Part::data(json!({"score": 42})),
                Part::url("https://example.com/full.pdf").with_media_type("application/pdf"),
                Part::raw(vec![0xFB, 0xFF]).with_filename("blob.bin"),
            ],
            metadata: None,
            extensions: vec![],
        }],
        history: vec![],
        metadata: None,
    };

    let expected = json!({
        "id": "task-1",
        "contextId": "ctx-1",
        "status": {
            "state": "TASK_STATE_COMPLETED",
            "message": {
                "messageId": "msg-1",
                "role": "ROLE_AGENT",
                "parts": [{"text": "all done"}],
            },
            "timestamp": "2026-08-07T00:01:45.123Z",
        },
        "artifacts": [{
            "artifactId": "a-1",
            "name": "result",
            "parts": [
                {"text": "summary"},
                {"data": {"score": 42}},
                {"url": "https://example.com/full.pdf", "mediaType": "application/pdf"},
                {"raw": "+/8=", "filename": "blob.bin"},
            ],
        }],
    });

    assert_eq!(serde_json::to_value(&task).unwrap(), expected);

    let parsed: Task = serde_json::from_value(expected.clone()).unwrap();
    assert_eq!(serde_json::to_value(&parsed).unwrap(), expected);
}
