//! The hand-written `pb` messages against vectors produced by Google's Python protobuf runtime
//! (`tests/golden/gen_vectors.py`): every message of `spec/a2a.proto` decodes and re-encodes to
//! the reference's bytes, whatever order, repetition or unknown fields the input has.
#![allow(clippy::unwrap_used)]

use rusty_a2a::pb;
use rusty_prost::Message;
use serde::Deserialize;

#[derive(Deserialize)]
struct Vector {
    r#type: String,
    name: String,
    input: String,
    canonical: String,
}

#[derive(Deserialize)]
struct File {
    vectors: Vec<Vector>,
}

fn hex(s: &str) -> Vec<u8> {
    (0..s.len())
        .step_by(2)
        .map(|i| u8::from_str_radix(&s[i..i + 2], 16).unwrap())
        .collect()
}

fn vectors() -> Vec<Vector> {
    let file: File = serde_json::from_str(include_str!("golden/a2a_vectors.json")).unwrap();
    file.vectors
}

/// `proto message name => pb type` for every message in `spec/a2a.proto`, and the two functions
/// the tests need over a type picked by name.
macro_rules! messages {
    ($($proto:literal => $ty:ident),* $(,)?) => {
        const NAMES: &[&str] = &[$($proto),*];

        /// Decodes `bytes` as the named message and encodes it back.
        fn reencode(name: &str, bytes: &[u8]) -> Result<Vec<u8>, rusty_prost::Error> {
            match name {
                $($proto => pb::$ty::decode(bytes)?.encode_to_vec(),)*
                other => panic!("no pb type for {other}"),
            }
        }
    };
}

messages! {
    "SendMessageConfiguration" => SendMessageConfiguration,
    "Task" => Task,
    "TaskStatus" => TaskStatus,
    "Part" => Part,
    "Message" => Message,
    "Artifact" => Artifact,
    "TaskStatusUpdateEvent" => TaskStatusUpdateEvent,
    "TaskArtifactUpdateEvent" => TaskArtifactUpdateEvent,
    "AuthenticationInfo" => AuthenticationInfo,
    "AgentInterface" => AgentInterface,
    "AgentCard" => AgentCard,
    "AgentProvider" => AgentProvider,
    "AgentCapabilities" => AgentCapabilities,
    "AgentExtension" => AgentExtension,
    "AgentSkill" => AgentSkill,
    "AgentCardSignature" => AgentCardSignature,
    "TaskPushNotificationConfig" => TaskPushNotificationConfig,
    "StringList" => StringList,
    "SecurityRequirement" => SecurityRequirement,
    "SecurityScheme" => SecurityScheme,
    "APIKeySecurityScheme" => ApiKeySecurityScheme,
    "HTTPAuthSecurityScheme" => HttpAuthSecurityScheme,
    "OAuth2SecurityScheme" => OAuth2SecurityScheme,
    "OpenIdConnectSecurityScheme" => OpenIdConnectSecurityScheme,
    "MutualTlsSecurityScheme" => MutualTlsSecurityScheme,
    "OAuthFlows" => OAuthFlows,
    "AuthorizationCodeOAuthFlow" => AuthorizationCodeOAuthFlow,
    "ClientCredentialsOAuthFlow" => ClientCredentialsOAuthFlow,
    "ImplicitOAuthFlow" => ImplicitOAuthFlow,
    "PasswordOAuthFlow" => PasswordOAuthFlow,
    "DeviceCodeOAuthFlow" => DeviceCodeOAuthFlow,
    "SendMessageRequest" => SendMessageRequest,
    "GetTaskRequest" => GetTaskRequest,
    "ListTasksRequest" => ListTasksRequest,
    "ListTasksResponse" => ListTasksResponse,
    "CancelTaskRequest" => CancelTaskRequest,
    "GetTaskPushNotificationConfigRequest" => GetTaskPushNotificationConfigRequest,
    "DeleteTaskPushNotificationConfigRequest" => DeleteTaskPushNotificationConfigRequest,
    "SubscribeToTaskRequest" => SubscribeToTaskRequest,
    "ListTaskPushNotificationConfigsRequest" => ListTaskPushNotificationConfigsRequest,
    "GetExtendedAgentCardRequest" => GetExtendedAgentCardRequest,
    "SendMessageResponse" => SendMessageResponse,
    "StreamResponse" => StreamResponse,
    "ListTaskPushNotificationConfigsResponse" => ListTaskPushNotificationConfigsResponse,
}

fn vector(typ: &str, name: &str) -> Vector {
    vectors()
        .into_iter()
        .find(|v| v.r#type == typ && v.name == name)
        .unwrap_or_else(|| panic!("no vector {typ}/{name}"))
}

// ---- the reference's bytes ------------------------------------------------------------------

/// Where `got` and `want` first differ, for a failure message that is not a wall of bytes.
fn first_difference(got: &[u8], want: &[u8]) -> String {
    let at = got
        .iter()
        .zip(want)
        .position(|(a, b)| a != b)
        .unwrap_or(got.len().min(want.len()));
    format!(
        "first difference at byte {at} (got {} bytes, want {})",
        got.len(),
        want.len()
    )
}

/// Runs `check` on every vector and fails with all the vectors it rejected.
fn for_every_vector(check: impl Fn(&Vector) -> Result<(), String>) {
    let failures: Vec<String> = vectors()
        .iter()
        .filter_map(|v| check(v).err().map(|e| format!("{}/{}: {e}", v.r#type, v.name)))
        .collect();
    assert!(
        failures.is_empty(),
        "{} vectors failed:\n{}",
        failures.len(),
        failures.join("\n")
    );
}

#[test]
fn every_vector_re_encodes_to_the_references_bytes() {
    for_every_vector(|v| {
        let got = reencode(&v.r#type, &hex(&v.input)).map_err(|e| format!("decode failed: {e}"))?;
        let want = hex(&v.canonical);
        (got == want)
            .then_some(())
            .ok_or_else(|| first_difference(&got, &want))
    });
}

#[test]
fn re_encoding_is_stable() {
    for_every_vector(|v| {
        let canonical = hex(&v.canonical);
        let got = reencode(&v.r#type, &canonical).map_err(|e| format!("decode failed: {e}"))?;
        (got == canonical)
            .then_some(())
            .ok_or_else(|| first_difference(&got, &canonical))
    });
}

#[test]
fn the_vectors_cover_every_message_and_the_table_every_vector() {
    let mut in_vectors: Vec<String> = vectors().into_iter().map(|v| v.r#type).collect();
    in_vectors.sort();
    in_vectors.dedup();
    let mut in_table: Vec<String> = NAMES.iter().map(|n| (*n).to_string()).collect();
    in_table.sort();
    assert_eq!(in_vectors, in_table);
    // Several random vectors for each, not just the empty message.
    for name in NAMES {
        let n = vectors().iter().filter(|v| v.r#type == *name).count();
        assert!(n >= 6, "{name}: only {n} vectors");
    }
}

// ---- the values behind some of them ---------------------------------------------------------

#[test]
fn a_oneof_keeps_its_last_member_and_a_default_member_is_still_set() {
    let last = pb::Part::decode(&hex(&vector("Part", "oneof: the last member wins").input)).unwrap();
    assert_eq!(last.content, Some(pb::part::Content::Url("http://u".into())));
    let empty = pb::Part::decode(&hex(&vector(
        "Part",
        "oneof: a member set to its default is still set",
    )
    .input))
    .unwrap();
    assert_eq!(empty.content, Some(pb::part::Content::Text(String::new())));
    let raw = pb::Part::decode(&hex(&vector("Part", "bytes member").input)).unwrap();
    assert_eq!(raw.content, Some(pb::part::Content::Raw(vec![0, 0xff, 0x10])));
}

#[test]
fn an_enum_value_the_schema_does_not_know_is_kept_and_refused_by_try_from() {
    let status = pb::TaskStatus::decode(&hex(&vector(
        "TaskStatus",
        "an enum value the schema does not know",
    )
    .input))
    .unwrap();
    assert_eq!(status.state, 99);
    assert!(pb::TaskState::try_from(status.state).is_err());
    // Neither side of the known range is a member.
    assert!(pb::TaskState::try_from(-1).is_err());
    assert!(pb::TaskState::try_from(9).is_err());
    assert!(pb::Role::try_from(-1).is_err());
    assert_eq!(pb::TaskState::try_from(3).unwrap(), pb::TaskState::Completed);
    assert_eq!(pb::Role::try_from(2).unwrap(), pb::Role::Agent);
}

#[test]
fn enum_names_round_trip() {
    for s in [
        pb::TaskState::Unspecified,
        pb::TaskState::Submitted,
        pb::TaskState::Working,
        pb::TaskState::Completed,
        pb::TaskState::Failed,
        pb::TaskState::Canceled,
        pb::TaskState::InputRequired,
        pb::TaskState::Rejected,
        pb::TaskState::AuthRequired,
    ] {
        assert_eq!(pb::TaskState::from_str_name(s.as_str_name()), Some(s));
        assert_eq!(pb::TaskState::try_from(s as i32).unwrap(), s);
    }
    assert_eq!(pb::TaskState::Working.as_str_name(), "TASK_STATE_WORKING");
    assert_eq!(pb::Role::User.as_str_name(), "ROLE_USER");
    assert_eq!(pb::Role::from_str_name("nope"), None);
}

#[test]
fn optional_scalars_distinguish_zero_from_absent() {
    let zero = pb::GetTaskRequest::decode(&hex(
        &vector("GetTaskRequest", "optional int32 present as zero").input
    ))
    .unwrap();
    assert_eq!((zero.id.as_str(), zero.history_length), ("t", Some(0)));
    let absent = pb::GetTaskRequest::decode(&[]).unwrap();
    assert_eq!(absent.history_length, None);
    let negative =
        pb::GetTaskRequest::decode(&hex(&vector("GetTaskRequest", "optional int32 negative").input)).unwrap();
    assert_eq!(negative.history_length, Some(-5));
}

#[test]
fn a_repeated_map_key_is_replaced_and_a_missing_value_is_the_default() {
    let card = pb::AgentCard::decode(&hex(&vector(
        "AgentCard",
        "map entries: a repeated key is replaced",
    )
    .input))
    .unwrap();
    assert_eq!(card.security_schemes.len(), 1);
    let only_key =
        pb::AgentCard::decode(&hex(&vector("AgentCard", "map entry with no value").input)).unwrap();
    assert_eq!(
        only_key.security_schemes.get("only-key"),
        Some(&pb::SecurityScheme::default())
    );
}

#[test]
fn a_present_but_empty_sub_message_is_present() {
    let task = pb::Task::decode(&hex(&vector("Task", "present but empty sub-message").input)).unwrap();
    assert_eq!(task.status, Some(pb::TaskStatus::default()));
    assert_eq!(pb::Task::decode(&[]).unwrap().status, None);
}

// ---- hostile input --------------------------------------------------------------------------

#[test]
fn every_prefix_of_every_vector_decodes_or_fails_without_panicking() {
    for v in vectors() {
        let bytes = hex(&v.canonical);
        for end in 0..bytes.len() {
            let _ = reencode(&v.r#type, &bytes[..end]);
        }
    }
}

#[test]
fn a_string_that_is_not_utf8_is_an_error() {
    // Task.id = 0xff
    assert!(pb::Task::decode(&[0x0a, 0x01, 0xff]).is_err());
}

#[test]
fn a_length_that_claims_more_than_the_input_has_is_an_error() {
    assert!(pb::Task::decode(&[0x0a, 0xff, 0xff, 0xff, 0xff, 0x0f, b'a']).is_err());
}

#[test]
fn nesting_past_the_depth_limit_is_an_error_not_a_stack_overflow() {
    let mut value = rusty_prost::wkt::Value::List(Vec::new());
    for _ in 0..200 {
        value = rusty_prost::wkt::Value::List(vec![value]);
    }
    let part = pb::Part {
        content: Some(pb::part::Content::Data(value)),
        ..pb::Part::default()
    };
    let bytes = part.encode_to_vec().unwrap();
    assert!(pb::Part::decode(&bytes).is_err());
    // A modest depth is fine.
    let mut shallow = rusty_prost::wkt::Value::List(Vec::new());
    for _ in 0..20 {
        shallow = rusty_prost::wkt::Value::List(vec![shallow]);
    }
    let part = pb::Part {
        content: Some(pb::part::Content::Data(shallow)),
        ..pb::Part::default()
    };
    let bytes = part.encode_to_vec().unwrap();
    assert_eq!(pb::Part::decode(&bytes).unwrap(), part);
}

#[test]
fn random_bytes_never_panic_in_any_message() {
    // xorshift: a fixed stream of arbitrary buffers, some short enough to look like fields.
    let mut state = 0x9E37_79B9_7F4A_7C15_u64;
    let mut next = move || {
        state ^= state << 13;
        state ^= state >> 7;
        state ^= state << 17;
        state
    };
    for _ in 0..3_000 {
        let len = (next() % 64) as usize;
        let buf: Vec<u8> = (0..len).map(|_| next() as u8).collect();
        for name in NAMES {
            let _ = reencode(name, &buf);
        }
    }
}
