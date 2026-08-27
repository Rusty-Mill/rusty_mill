//! Field-by-field conversions between the generated `pb::*` (prost) types
//! and this crate's own `crate::types::*`, plus the [`A2aError`] ->
//! [`tonic::Status`] mapping.
//!
//! Shared by both the gRPC server (`server::grpc`, which needs
//! `pb_*_request_to_ours` and `our_*_to_pb`/`our_*_response_to_pb`) and the
//! gRPC client (`client::grpc`, which needs exactly the opposite:
//! `our_*_request_to_pb` and `pb_*_to_ours`/`pb_*_response_to_ours`). Any
//! feature combination that enables only one of them leaves the other
//! direction's functions unused, hence the blanket allow below rather than
//! `#[cfg]`-gating each one individually.
//!
//! Proto3 non-`optional` string/enum fields have no wire-level way to
//! distinguish "unset" from "explicitly set to the zero value" - a plain
//! `string` decodes to `""` either way, a plain `enum` to variant `0`. For
//! fields the A2A data model treats as logically optional (e.g.
//! `Message.context_id`, `ListTasksRequest.status`), this module treats
//! that zero value as "unset": empty string -> `None`, enum `0`
//! (`*_UNSPECIFIED`) -> `None`. This matches every other binding in this
//! crate (JSON omits the field either way) and is the only sensible
//! reading available.
#![allow(dead_code)]

use chrono::{DateTime, TimeZone, Utc};
use serde_json::{Map, Value};
use tonic::{Code, Status};
use tonic_types::{ErrorDetails, StatusExt};

use crate::error::A2aError;
use crate::pb;
use crate::types as ours;

/// The domain JSON-RPC/REST already send in their own `google.rpc.ErrorInfo`
/// (`JsonRpcErrorObject`/`rest::rest_error`); reused here so a `reason`
/// round-trips identically regardless of which binding a client used.
const ERROR_DOMAIN: &str = "a2a-protocol.org";

/// Maps `err` to a `tonic::Status`, attaching a `google.rpc.ErrorInfo`
/// detail (spec Section 10.6: "implementations MUST include a
/// `google.rpc.ErrorInfo` message in the `status.details` array") whenever
/// `err` has one of the nine A2A-specific `reason`s - mirroring
/// `JsonRpcErrorObject::from`/`rest::rest_error`, the JSON-RPC/REST
/// equivalents. Errors with no A2A-specific reason (auth/validation/
/// internal failures, where the gRPC `Code` alone is already precise) get a
/// bare `Status`, same as before.
pub fn a2a_error_to_status(err: A2aError) -> Status {
    let code = match err.grpc_status_name() {
        "NOT_FOUND" => Code::NotFound,
        "FAILED_PRECONDITION" => Code::FailedPrecondition,
        "INVALID_ARGUMENT" => Code::InvalidArgument,
        "INTERNAL" => Code::Internal,
        "UNAUTHENTICATED" => Code::Unauthenticated,
        "PERMISSION_DENIED" => Code::PermissionDenied,
        "UNIMPLEMENTED" => Code::Unimplemented,
        _ => Code::Unknown,
    };
    let message = err.standard_message();
    match err.reason() {
        Some(reason) => {
            let details =
                ErrorDetails::with_error_info(reason, ERROR_DOMAIN, std::collections::HashMap::new());
            Status::with_error_details(code, message, details)
        }
        None => Status::new(code, message),
    }
}

fn non_empty(s: String) -> Option<String> {
    if s.is_empty() {
        None
    } else {
        Some(s)
    }
}

// --- google.protobuf.Struct/Value <-> serde_json ---

fn struct_to_json(s: prost_types::Struct) -> Map<String, Value> {
    s.fields
        .into_iter()
        .map(|(k, v)| (k, prost_value_to_json(v)))
        .collect()
}

fn json_to_struct(m: Map<String, Value>) -> prost_types::Struct {
    prost_types::Struct {
        fields: m.into_iter().map(|(k, v)| (k, json_to_prost_value(v))).collect(),
    }
}

fn prost_value_to_json(v: prost_types::Value) -> Value {
    use prost_types::value::Kind;
    match v.kind {
        None | Some(Kind::NullValue(_)) => Value::Null,
        Some(Kind::NumberValue(n)) => serde_json::Number::from_f64(n)
            .map(Value::Number)
            .unwrap_or(Value::Null),
        Some(Kind::StringValue(s)) => Value::String(s),
        Some(Kind::BoolValue(b)) => Value::Bool(b),
        Some(Kind::StructValue(s)) => Value::Object(struct_to_json(s)),
        Some(Kind::ListValue(l)) => Value::Array(l.values.into_iter().map(prost_value_to_json).collect()),
    }
}

fn json_to_prost_value(v: Value) -> prost_types::Value {
    use prost_types::value::Kind;
    let kind = match v {
        Value::Null => Kind::NullValue(0),
        Value::Bool(b) => Kind::BoolValue(b),
        Value::Number(n) => Kind::NumberValue(n.as_f64().unwrap_or(0.0)),
        Value::String(s) => Kind::StringValue(s),
        Value::Array(a) => Kind::ListValue(prost_types::ListValue {
            values: a.into_iter().map(json_to_prost_value).collect(),
        }),
        Value::Object(o) => Kind::StructValue(json_to_struct(o)),
    };
    prost_types::Value { kind: Some(kind) }
}

// --- google.protobuf.Timestamp <-> chrono ---

fn timestamp_to_chrono(ts: prost_types::Timestamp) -> Option<DateTime<Utc>> {
    Utc.timestamp_opt(ts.seconds, ts.nanos.max(0) as u32).single()
}

fn chrono_to_timestamp(dt: DateTime<Utc>) -> prost_types::Timestamp {
    prost_types::Timestamp {
        seconds: dt.timestamp(),
        nanos: dt.timestamp_subsec_nanos() as i32,
    }
}

// --- enums ---

fn pb_task_state_to_ours(state: i32) -> ours::TaskState {
    match pb::TaskState::try_from(state).unwrap_or(pb::TaskState::Unspecified) {
        pb::TaskState::Unspecified => ours::TaskState::Unspecified,
        pb::TaskState::Submitted => ours::TaskState::Submitted,
        pb::TaskState::Working => ours::TaskState::Working,
        pb::TaskState::Completed => ours::TaskState::Completed,
        pb::TaskState::Failed => ours::TaskState::Failed,
        pb::TaskState::Canceled => ours::TaskState::Canceled,
        pb::TaskState::InputRequired => ours::TaskState::InputRequired,
        pb::TaskState::Rejected => ours::TaskState::Rejected,
        pb::TaskState::AuthRequired => ours::TaskState::AuthRequired,
    }
}

fn our_task_state_to_pb(state: ours::TaskState) -> pb::TaskState {
    match state {
        ours::TaskState::Unspecified => pb::TaskState::Unspecified,
        ours::TaskState::Submitted => pb::TaskState::Submitted,
        ours::TaskState::Working => pb::TaskState::Working,
        ours::TaskState::Completed => pb::TaskState::Completed,
        ours::TaskState::Failed => pb::TaskState::Failed,
        ours::TaskState::Canceled => pb::TaskState::Canceled,
        ours::TaskState::InputRequired => pb::TaskState::InputRequired,
        ours::TaskState::Rejected => pb::TaskState::Rejected,
        ours::TaskState::AuthRequired => pb::TaskState::AuthRequired,
    }
}

fn pb_role_to_ours(role: i32) -> ours::Role {
    match pb::Role::try_from(role).unwrap_or(pb::Role::Unspecified) {
        pb::Role::Unspecified => ours::Role::Unspecified,
        pb::Role::User => ours::Role::User,
        pb::Role::Agent => ours::Role::Agent,
    }
}

fn our_role_to_pb(role: ours::Role) -> pb::Role {
    match role {
        ours::Role::Unspecified => pb::Role::Unspecified,
        ours::Role::User => pb::Role::User,
        ours::Role::Agent => pb::Role::Agent,
    }
}

// --- Part / Message ---

fn pb_part_to_ours(p: pb::Part) -> Result<ours::Part, Status> {
    use pb::part::Content as PC;
    let content = match p.content {
        Some(PC::Text(t)) => ours::PartContent::Text { text: t },
        Some(PC::Raw(b)) => ours::PartContent::Raw { raw: b },
        Some(PC::Url(u)) => ours::PartContent::Url { url: u },
        Some(PC::Data(v)) => ours::PartContent::Data {
            data: prost_value_to_json(v),
        },
        None => return Err(Status::invalid_argument("Part.content is required")),
    };
    Ok(ours::Part {
        content,
        metadata: p.metadata.map(struct_to_json),
        filename: non_empty(p.filename),
        media_type: non_empty(p.media_type),
    })
}

fn our_part_to_pb(p: ours::Part) -> pb::Part {
    let content = Some(match p.content {
        ours::PartContent::Text { text } => pb::part::Content::Text(text),
        ours::PartContent::Raw { raw } => pb::part::Content::Raw(raw),
        ours::PartContent::Url { url } => pb::part::Content::Url(url),
        ours::PartContent::Data { data } => pb::part::Content::Data(json_to_prost_value(data)),
    });
    pb::Part {
        metadata: p.metadata.map(json_to_struct),
        filename: p.filename.unwrap_or_default(),
        media_type: p.media_type.unwrap_or_default(),
        content,
    }
}

fn pb_message_to_ours(m: pb::Message) -> Result<ours::Message, Status> {
    Ok(ours::Message {
        message_id: m.message_id,
        context_id: non_empty(m.context_id),
        task_id: non_empty(m.task_id),
        role: pb_role_to_ours(m.role),
        parts: m
            .parts
            .into_iter()
            .map(pb_part_to_ours)
            .collect::<Result<_, _>>()?,
        metadata: m.metadata.map(struct_to_json),
        extensions: m.extensions,
        reference_task_ids: m.reference_task_ids,
    })
}

fn our_message_to_pb(m: ours::Message) -> pb::Message {
    pb::Message {
        message_id: m.message_id,
        context_id: m.context_id.unwrap_or_default(),
        task_id: m.task_id.unwrap_or_default(),
        role: our_role_to_pb(m.role) as i32,
        parts: m.parts.into_iter().map(our_part_to_pb).collect(),
        metadata: m.metadata.map(json_to_struct),
        extensions: m.extensions,
        reference_task_ids: m.reference_task_ids,
    }
}

// --- TaskStatus / Task / Artifact / events ---

fn our_task_status_to_pb(s: ours::TaskStatus) -> pb::TaskStatus {
    pb::TaskStatus {
        state: our_task_state_to_pb(s.state) as i32,
        message: s.message.map(our_message_to_pb),
        timestamp: s.timestamp.map(chrono_to_timestamp),
    }
}

fn our_artifact_to_pb(a: ours::Artifact) -> pb::Artifact {
    pb::Artifact {
        artifact_id: a.artifact_id,
        name: a.name.unwrap_or_default(),
        description: a.description.unwrap_or_default(),
        parts: a.parts.into_iter().map(our_part_to_pb).collect(),
        metadata: a.metadata.map(json_to_struct),
        extensions: a.extensions,
    }
}

pub fn our_task_to_pb(t: ours::Task) -> pb::Task {
    pb::Task {
        id: t.id,
        context_id: t.context_id.unwrap_or_default(),
        status: Some(our_task_status_to_pb(t.status)),
        artifacts: t.artifacts.into_iter().map(our_artifact_to_pb).collect(),
        history: t.history.into_iter().map(our_message_to_pb).collect(),
        metadata: t.metadata.map(json_to_struct),
    }
}

fn our_status_update_to_pb(e: ours::TaskStatusUpdateEvent) -> pb::TaskStatusUpdateEvent {
    pb::TaskStatusUpdateEvent {
        task_id: e.task_id,
        context_id: e.context_id,
        status: Some(our_task_status_to_pb(e.status)),
        metadata: e.metadata.map(json_to_struct),
    }
}

fn our_artifact_update_to_pb(e: ours::TaskArtifactUpdateEvent) -> pb::TaskArtifactUpdateEvent {
    pb::TaskArtifactUpdateEvent {
        task_id: e.task_id,
        context_id: e.context_id,
        artifact: Some(our_artifact_to_pb(e.artifact)),
        append: e.append,
        last_chunk: e.last_chunk,
        metadata: e.metadata.map(json_to_struct),
    }
}

pub fn our_stream_response_to_pb(r: ours::StreamResponse) -> pb::StreamResponse {
    use pb::stream_response::Payload;
    let payload = Some(match r {
        ours::StreamResponse::Task { task } => Payload::Task(our_task_to_pb(task)),
        ours::StreamResponse::Message { message } => Payload::Message(our_message_to_pb(message)),
        ours::StreamResponse::StatusUpdate { status_update } => {
            Payload::StatusUpdate(our_status_update_to_pb(status_update))
        }
        ours::StreamResponse::ArtifactUpdate { artifact_update } => {
            Payload::ArtifactUpdate(our_artifact_update_to_pb(artifact_update))
        }
    });
    pb::StreamResponse { payload }
}

pub fn our_send_result_to_pb(r: ours::SendMessageResult) -> pb::SendMessageResponse {
    use pb::send_message_response::Payload;
    let payload = Some(match r {
        ours::SendMessageResult::Task { task } => Payload::Task(our_task_to_pb(task)),
        ours::SendMessageResult::Message { message } => Payload::Message(our_message_to_pb(message)),
    });
    pb::SendMessageResponse { payload }
}

// --- push notification config ---

fn pb_auth_info_to_ours(a: pb::AuthenticationInfo) -> ours::AuthenticationInfo {
    ours::AuthenticationInfo {
        scheme: a.scheme,
        credentials: non_empty(a.credentials),
    }
}

fn our_auth_info_to_pb(a: ours::AuthenticationInfo) -> pb::AuthenticationInfo {
    pb::AuthenticationInfo {
        scheme: a.scheme,
        credentials: a.credentials.unwrap_or_default(),
    }
}

pub fn pb_push_config_to_ours(c: pb::TaskPushNotificationConfig) -> ours::TaskPushNotificationConfig {
    ours::TaskPushNotificationConfig {
        tenant: non_empty(c.tenant),
        id: non_empty(c.id),
        task_id: non_empty(c.task_id),
        url: c.url,
        token: non_empty(c.token),
        authentication: c.authentication.map(pb_auth_info_to_ours),
    }
}

pub fn our_push_config_to_pb(c: ours::TaskPushNotificationConfig) -> pb::TaskPushNotificationConfig {
    pb::TaskPushNotificationConfig {
        tenant: c.tenant.unwrap_or_default(),
        id: c.id.unwrap_or_default(),
        task_id: c.task_id.unwrap_or_default(),
        url: c.url,
        token: c.token.unwrap_or_default(),
        authentication: c.authentication.map(our_auth_info_to_pb),
    }
}

pub fn our_list_push_notification_configs_response_to_pb(
    r: ours::ListTaskPushNotificationConfigsResponse,
) -> pb::ListTaskPushNotificationConfigsResponse {
    pb::ListTaskPushNotificationConfigsResponse {
        configs: r.configs.into_iter().map(our_push_config_to_pb).collect(),
        next_page_token: r.next_page_token,
    }
}

pub fn our_list_tasks_response_to_pb(r: ours::ListTasksResponse) -> pb::ListTasksResponse {
    pb::ListTasksResponse {
        tasks: r.tasks.into_iter().map(our_task_to_pb).collect(),
        next_page_token: r.next_page_token,
        page_size: r.page_size,
        total_size: r.total_size,
    }
}

// --- requests (inbound only) ---

fn pb_send_config_to_ours(c: pb::SendMessageConfiguration) -> ours::SendMessageConfiguration {
    ours::SendMessageConfiguration {
        accepted_output_modes: c.accepted_output_modes,
        task_push_notification_config: c.task_push_notification_config.map(pb_push_config_to_ours),
        history_length: c.history_length,
        return_immediately: c.return_immediately,
    }
}

pub fn pb_send_message_request_to_ours(
    r: pb::SendMessageRequest,
) -> Result<ours::SendMessageRequest, Status> {
    let message = r
        .message
        .ok_or_else(|| Status::invalid_argument("message is required"))?;
    Ok(ours::SendMessageRequest {
        tenant: non_empty(r.tenant),
        message: pb_message_to_ours(message)?,
        configuration: r.configuration.map(pb_send_config_to_ours),
        metadata: r.metadata.map(struct_to_json),
    })
}

pub fn pb_get_task_request_to_ours(r: pb::GetTaskRequest) -> ours::GetTaskRequest {
    ours::GetTaskRequest {
        tenant: non_empty(r.tenant),
        id: r.id,
        history_length: r.history_length,
    }
}

pub fn pb_list_tasks_request_to_ours(r: pb::ListTasksRequest) -> ours::ListTasksRequest {
    ours::ListTasksRequest {
        tenant: non_empty(r.tenant),
        context_id: non_empty(r.context_id),
        status: if r.status == 0 {
            None
        } else {
            Some(pb_task_state_to_ours(r.status))
        },
        page_size: r.page_size,
        page_token: non_empty(r.page_token),
        history_length: r.history_length,
        status_timestamp_after: r.status_timestamp_after.and_then(timestamp_to_chrono),
        include_artifacts: r.include_artifacts,
    }
}

pub fn pb_cancel_task_request_to_ours(r: pb::CancelTaskRequest) -> ours::CancelTaskRequest {
    ours::CancelTaskRequest {
        tenant: non_empty(r.tenant),
        id: r.id,
        metadata: r.metadata.map(struct_to_json),
    }
}

pub fn pb_subscribe_to_task_request_to_ours(r: pb::SubscribeToTaskRequest) -> ours::SubscribeToTaskRequest {
    ours::SubscribeToTaskRequest {
        tenant: non_empty(r.tenant),
        id: r.id,
    }
}

pub fn pb_get_push_notification_config_request_to_ours(
    r: pb::GetTaskPushNotificationConfigRequest,
) -> ours::GetTaskPushNotificationConfigRequest {
    ours::GetTaskPushNotificationConfigRequest {
        tenant: non_empty(r.tenant),
        task_id: r.task_id,
        id: r.id,
    }
}

pub fn pb_delete_push_notification_config_request_to_ours(
    r: pb::DeleteTaskPushNotificationConfigRequest,
) -> ours::DeleteTaskPushNotificationConfigRequest {
    ours::DeleteTaskPushNotificationConfigRequest {
        tenant: non_empty(r.tenant),
        task_id: r.task_id,
        id: r.id,
    }
}

pub fn pb_list_push_notification_configs_request_to_ours(
    r: pb::ListTaskPushNotificationConfigsRequest,
) -> ours::ListTaskPushNotificationConfigsRequest {
    ours::ListTaskPushNotificationConfigsRequest {
        tenant: non_empty(r.tenant),
        task_id: r.task_id,
        page_size: if r.page_size == 0 { None } else { Some(r.page_size) },
        page_token: non_empty(r.page_token),
    }
}

// --- AgentCard (outbound only, for GetExtendedAgentCard) ---

fn our_agent_interface_to_pb(i: ours::AgentInterface) -> pb::AgentInterface {
    pb::AgentInterface {
        url: i.url,
        protocol_binding: i.protocol_binding,
        tenant: i.tenant.unwrap_or_default(),
        protocol_version: i.protocol_version,
    }
}

fn our_agent_provider_to_pb(p: ours::AgentProvider) -> pb::AgentProvider {
    pb::AgentProvider {
        url: p.url,
        organization: p.organization,
    }
}

fn our_agent_extension_to_pb(e: ours::AgentExtension) -> pb::AgentExtension {
    pb::AgentExtension {
        uri: e.uri,
        description: e.description,
        required: e.required,
        params: e.params.map(json_to_struct),
    }
}

fn our_agent_capabilities_to_pb(c: ours::AgentCapabilities) -> pb::AgentCapabilities {
    pb::AgentCapabilities {
        streaming: c.streaming,
        push_notifications: c.push_notifications,
        extensions: c.extensions.into_iter().map(our_agent_extension_to_pb).collect(),
        extended_agent_card: c.extended_agent_card,
    }
}

fn our_string_list_to_pb(s: ours::StringList) -> pb::StringList {
    pb::StringList { list: s.list }
}

fn our_security_requirement_to_pb(s: ours::SecurityRequirement) -> pb::SecurityRequirement {
    pb::SecurityRequirement {
        schemes: s
            .schemes
            .into_iter()
            .map(|(k, v)| (k, our_string_list_to_pb(v)))
            .collect(),
    }
}

fn our_agent_skill_to_pb(s: ours::AgentSkill) -> pb::AgentSkill {
    pb::AgentSkill {
        id: s.id,
        name: s.name,
        description: s.description,
        tags: s.tags,
        examples: s.examples,
        input_modes: s.input_modes,
        output_modes: s.output_modes,
        security_requirements: s
            .security_requirements
            .into_iter()
            .map(our_security_requirement_to_pb)
            .collect(),
    }
}

fn our_agent_card_signature_to_pb(s: ours::AgentCardSignature) -> pb::AgentCardSignature {
    pb::AgentCardSignature {
        protected: s.protected,
        signature: s.signature,
        header: s.header.map(json_to_struct),
    }
}

fn our_api_key_scheme_to_pb(s: ours::ApiKeySecurityScheme) -> pb::ApiKeySecurityScheme {
    pb::ApiKeySecurityScheme {
        description: s.description.unwrap_or_default(),
        location: s.location,
        name: s.name,
    }
}

fn our_http_auth_scheme_to_pb(s: ours::HttpAuthSecurityScheme) -> pb::HttpAuthSecurityScheme {
    pb::HttpAuthSecurityScheme {
        description: s.description.unwrap_or_default(),
        scheme: s.scheme,
        bearer_format: s.bearer_format.unwrap_or_default(),
    }
}

fn our_oidc_scheme_to_pb(s: ours::OpenIdConnectSecurityScheme) -> pb::OpenIdConnectSecurityScheme {
    pb::OpenIdConnectSecurityScheme {
        description: s.description.unwrap_or_default(),
        open_id_connect_url: s.open_id_connect_url,
    }
}

fn our_mtls_scheme_to_pb(s: ours::MutualTlsSecurityScheme) -> pb::MutualTlsSecurityScheme {
    pb::MutualTlsSecurityScheme {
        description: s.description.unwrap_or_default(),
    }
}

fn our_auth_code_flow_to_pb(f: ours::AuthorizationCodeOAuthFlow) -> pb::AuthorizationCodeOAuthFlow {
    pb::AuthorizationCodeOAuthFlow {
        authorization_url: f.authorization_url,
        token_url: f.token_url,
        refresh_url: f.refresh_url.unwrap_or_default(),
        scopes: f.scopes,
        pkce_required: f.pkce_required,
    }
}

fn our_client_creds_flow_to_pb(f: ours::ClientCredentialsOAuthFlow) -> pb::ClientCredentialsOAuthFlow {
    pb::ClientCredentialsOAuthFlow {
        token_url: f.token_url,
        refresh_url: f.refresh_url.unwrap_or_default(),
        scopes: f.scopes,
    }
}

fn our_implicit_flow_to_pb(f: ours::ImplicitOAuthFlow) -> pb::ImplicitOAuthFlow {
    pb::ImplicitOAuthFlow {
        authorization_url: f.authorization_url,
        refresh_url: f.refresh_url.unwrap_or_default(),
        scopes: f.scopes,
    }
}

fn our_password_flow_to_pb(f: ours::PasswordOAuthFlow) -> pb::PasswordOAuthFlow {
    pb::PasswordOAuthFlow {
        token_url: f.token_url,
        refresh_url: f.refresh_url.unwrap_or_default(),
        scopes: f.scopes,
    }
}

fn our_device_code_flow_to_pb(f: ours::DeviceCodeOAuthFlow) -> pb::DeviceCodeOAuthFlow {
    pb::DeviceCodeOAuthFlow {
        device_authorization_url: f.device_authorization_url,
        token_url: f.token_url,
        refresh_url: f.refresh_url.unwrap_or_default(),
        scopes: f.scopes,
    }
}

// The proto marks the Implicit/Password flows `deprecated = true` (spec
// recommends Authorization Code + PKCE or Device Code instead), but they
// remain part of the wire format, so this crate still passes them through.
#[allow(deprecated)]
fn our_oauth_flows_to_pb(f: ours::OAuthFlows) -> pb::OAuthFlows {
    use pb::o_auth_flows::Flow;
    let flow = Some(match f {
        ours::OAuthFlows::AuthorizationCode { authorization_code } => {
            Flow::AuthorizationCode(our_auth_code_flow_to_pb(authorization_code))
        }
        ours::OAuthFlows::ClientCredentials { client_credentials } => {
            Flow::ClientCredentials(our_client_creds_flow_to_pb(client_credentials))
        }
        ours::OAuthFlows::Implicit { implicit } => Flow::Implicit(our_implicit_flow_to_pb(implicit)),
        ours::OAuthFlows::Password { password } => Flow::Password(our_password_flow_to_pb(password)),
        ours::OAuthFlows::DeviceCode { device_code } => {
            Flow::DeviceCode(our_device_code_flow_to_pb(device_code))
        }
    });
    pb::OAuthFlows { flow }
}

fn our_oauth2_scheme_to_pb(s: ours::OAuth2SecurityScheme) -> pb::OAuth2SecurityScheme {
    pb::OAuth2SecurityScheme {
        description: s.description.unwrap_or_default(),
        flows: Some(our_oauth_flows_to_pb(s.flows)),
        oauth2_metadata_url: s.oauth2_metadata_url.unwrap_or_default(),
    }
}

fn our_security_scheme_to_pb(s: ours::SecurityScheme) -> pb::SecurityScheme {
    use pb::security_scheme::Scheme;
    let scheme = Some(match s {
        ours::SecurityScheme::ApiKey {
            api_key_security_scheme,
        } => Scheme::ApiKeySecurityScheme(our_api_key_scheme_to_pb(api_key_security_scheme)),
        ours::SecurityScheme::HttpAuth {
            http_auth_security_scheme,
        } => Scheme::HttpAuthSecurityScheme(our_http_auth_scheme_to_pb(http_auth_security_scheme)),
        ours::SecurityScheme::OAuth2 {
            oauth2_security_scheme,
        } => Scheme::Oauth2SecurityScheme(our_oauth2_scheme_to_pb(oauth2_security_scheme)),
        ours::SecurityScheme::OpenIdConnect {
            open_id_connect_security_scheme,
        } => Scheme::OpenIdConnectSecurityScheme(our_oidc_scheme_to_pb(open_id_connect_security_scheme)),
        ours::SecurityScheme::MutualTls { mtls_security_scheme } => {
            Scheme::MtlsSecurityScheme(our_mtls_scheme_to_pb(mtls_security_scheme))
        }
    });
    pb::SecurityScheme { scheme }
}

pub fn our_agent_card_to_pb(c: ours::AgentCard) -> pb::AgentCard {
    pb::AgentCard {
        name: c.name,
        description: c.description,
        supported_interfaces: c
            .supported_interfaces
            .into_iter()
            .map(our_agent_interface_to_pb)
            .collect(),
        provider: c.provider.map(our_agent_provider_to_pb),
        version: c.version,
        documentation_url: c.documentation_url,
        capabilities: Some(our_agent_capabilities_to_pb(c.capabilities)),
        security_schemes: c
            .security_schemes
            .into_iter()
            .map(|(k, v)| (k, our_security_scheme_to_pb(v)))
            .collect(),
        security_requirements: c
            .security_requirements
            .into_iter()
            .map(our_security_requirement_to_pb)
            .collect(),
        default_input_modes: c.default_input_modes,
        default_output_modes: c.default_output_modes,
        skills: c.skills.into_iter().map(our_agent_skill_to_pb).collect(),
        signatures: c
            .signatures
            .into_iter()
            .map(our_agent_card_signature_to_pb)
            .collect(),
        icon_url: c.icon_url,
    }
}

// --- pb -> ours: responses (for the gRPC client) ---
//
// The functions above this point exist to let the *server* decode incoming
// requests (`pb_*_request_to_ours`) and encode outgoing responses
// (`our_*_to_pb`). A gRPC client needs exactly the opposite: encode
// outgoing requests and decode incoming responses. Where a pb -> ours
// helper already exists above (e.g. `pb_push_config_to_ours`,
// `pb_task_state_to_ours`) it's reused rather than duplicated.

fn pb_task_status_to_ours(s: pb::TaskStatus) -> Result<ours::TaskStatus, Status> {
    Ok(ours::TaskStatus {
        state: pb_task_state_to_ours(s.state),
        message: s.message.map(pb_message_to_ours).transpose()?,
        timestamp: s.timestamp.and_then(timestamp_to_chrono),
    })
}

fn pb_artifact_to_ours(a: pb::Artifact) -> Result<ours::Artifact, Status> {
    Ok(ours::Artifact {
        artifact_id: a.artifact_id,
        name: non_empty(a.name),
        description: non_empty(a.description),
        parts: a
            .parts
            .into_iter()
            .map(pb_part_to_ours)
            .collect::<Result<_, _>>()?,
        metadata: a.metadata.map(struct_to_json),
        extensions: a.extensions,
    })
}

pub fn pb_task_to_ours(t: pb::Task) -> Result<ours::Task, Status> {
    Ok(ours::Task {
        id: t.id,
        context_id: non_empty(t.context_id),
        status: t
            .status
            .map(pb_task_status_to_ours)
            .transpose()?
            .ok_or_else(|| Status::invalid_argument("Task.status is required"))?,
        artifacts: t
            .artifacts
            .into_iter()
            .map(pb_artifact_to_ours)
            .collect::<Result<_, _>>()?,
        history: t
            .history
            .into_iter()
            .map(pb_message_to_ours)
            .collect::<Result<_, _>>()?,
        metadata: t.metadata.map(struct_to_json),
    })
}

fn pb_status_update_to_ours(e: pb::TaskStatusUpdateEvent) -> Result<ours::TaskStatusUpdateEvent, Status> {
    Ok(ours::TaskStatusUpdateEvent {
        task_id: e.task_id,
        context_id: e.context_id,
        status: e
            .status
            .map(pb_task_status_to_ours)
            .transpose()?
            .ok_or_else(|| Status::invalid_argument("TaskStatusUpdateEvent.status is required"))?,
        metadata: e.metadata.map(struct_to_json),
    })
}

fn pb_artifact_update_to_ours(
    e: pb::TaskArtifactUpdateEvent,
) -> Result<ours::TaskArtifactUpdateEvent, Status> {
    Ok(ours::TaskArtifactUpdateEvent {
        task_id: e.task_id,
        context_id: e.context_id,
        artifact: e
            .artifact
            .map(pb_artifact_to_ours)
            .transpose()?
            .ok_or_else(|| Status::invalid_argument("TaskArtifactUpdateEvent.artifact is required"))?,
        append: e.append,
        last_chunk: e.last_chunk,
        metadata: e.metadata.map(struct_to_json),
    })
}

pub fn pb_stream_response_to_ours(r: pb::StreamResponse) -> Result<ours::StreamResponse, Status> {
    use pb::stream_response::Payload;
    match r.payload {
        Some(Payload::Task(t)) => Ok(ours::StreamResponse::Task {
            task: pb_task_to_ours(t)?,
        }),
        Some(Payload::Message(m)) => Ok(ours::StreamResponse::Message {
            message: pb_message_to_ours(m)?,
        }),
        Some(Payload::StatusUpdate(s)) => Ok(ours::StreamResponse::StatusUpdate {
            status_update: pb_status_update_to_ours(s)?,
        }),
        Some(Payload::ArtifactUpdate(a)) => Ok(ours::StreamResponse::ArtifactUpdate {
            artifact_update: pb_artifact_update_to_ours(a)?,
        }),
        None => Err(Status::internal("StreamResponse.payload is required")),
    }
}

pub fn pb_send_message_response_to_ours(
    r: pb::SendMessageResponse,
) -> Result<ours::SendMessageResult, Status> {
    use pb::send_message_response::Payload;
    match r.payload {
        Some(Payload::Task(t)) => Ok(ours::SendMessageResult::Task {
            task: pb_task_to_ours(t)?,
        }),
        Some(Payload::Message(m)) => Ok(ours::SendMessageResult::Message {
            message: pb_message_to_ours(m)?,
        }),
        None => Err(Status::internal("SendMessageResponse.payload is required")),
    }
}

pub fn pb_list_tasks_response_to_ours(r: pb::ListTasksResponse) -> Result<ours::ListTasksResponse, Status> {
    Ok(ours::ListTasksResponse {
        tasks: r
            .tasks
            .into_iter()
            .map(pb_task_to_ours)
            .collect::<Result<_, _>>()?,
        next_page_token: r.next_page_token,
        page_size: r.page_size,
        total_size: r.total_size,
    })
}

pub fn pb_list_push_notification_configs_response_to_ours(
    r: pb::ListTaskPushNotificationConfigsResponse,
) -> ours::ListTaskPushNotificationConfigsResponse {
    ours::ListTaskPushNotificationConfigsResponse {
        configs: r.configs.into_iter().map(pb_push_config_to_ours).collect(),
        next_page_token: r.next_page_token,
    }
}

// --- pb -> ours: AgentCard (inbound only, for the gRPC client's
// GetExtendedAgentCard) ---

fn pb_agent_interface_to_ours(i: pb::AgentInterface) -> ours::AgentInterface {
    ours::AgentInterface {
        url: i.url,
        protocol_binding: i.protocol_binding,
        tenant: non_empty(i.tenant),
        protocol_version: i.protocol_version,
    }
}

fn pb_agent_provider_to_ours(p: pb::AgentProvider) -> ours::AgentProvider {
    ours::AgentProvider {
        url: p.url,
        organization: p.organization,
    }
}

fn pb_agent_extension_to_ours(e: pb::AgentExtension) -> ours::AgentExtension {
    ours::AgentExtension {
        uri: e.uri,
        description: e.description,
        required: e.required,
        params: e.params.map(struct_to_json),
    }
}

fn pb_agent_capabilities_to_ours(c: pb::AgentCapabilities) -> ours::AgentCapabilities {
    ours::AgentCapabilities {
        streaming: c.streaming,
        push_notifications: c.push_notifications,
        extensions: c.extensions.into_iter().map(pb_agent_extension_to_ours).collect(),
        extended_agent_card: c.extended_agent_card,
    }
}

fn pb_string_list_to_ours(s: pb::StringList) -> ours::StringList {
    ours::StringList { list: s.list }
}

fn pb_security_requirement_to_ours(s: pb::SecurityRequirement) -> ours::SecurityRequirement {
    ours::SecurityRequirement {
        schemes: s
            .schemes
            .into_iter()
            .map(|(k, v)| (k, pb_string_list_to_ours(v)))
            .collect(),
    }
}

fn pb_agent_skill_to_ours(s: pb::AgentSkill) -> ours::AgentSkill {
    ours::AgentSkill {
        id: s.id,
        name: s.name,
        description: s.description,
        tags: s.tags,
        examples: s.examples,
        input_modes: s.input_modes,
        output_modes: s.output_modes,
        security_requirements: s
            .security_requirements
            .into_iter()
            .map(pb_security_requirement_to_ours)
            .collect(),
    }
}

fn pb_agent_card_signature_to_ours(s: pb::AgentCardSignature) -> ours::AgentCardSignature {
    ours::AgentCardSignature {
        protected: s.protected,
        signature: s.signature,
        header: s.header.map(struct_to_json),
    }
}

fn pb_api_key_scheme_to_ours(s: pb::ApiKeySecurityScheme) -> ours::ApiKeySecurityScheme {
    ours::ApiKeySecurityScheme {
        description: non_empty(s.description),
        location: s.location,
        name: s.name,
    }
}

fn pb_http_auth_scheme_to_ours(s: pb::HttpAuthSecurityScheme) -> ours::HttpAuthSecurityScheme {
    ours::HttpAuthSecurityScheme {
        description: non_empty(s.description),
        scheme: s.scheme,
        bearer_format: non_empty(s.bearer_format),
    }
}

fn pb_oidc_scheme_to_ours(s: pb::OpenIdConnectSecurityScheme) -> ours::OpenIdConnectSecurityScheme {
    ours::OpenIdConnectSecurityScheme {
        description: non_empty(s.description),
        open_id_connect_url: s.open_id_connect_url,
    }
}

fn pb_mtls_scheme_to_ours(s: pb::MutualTlsSecurityScheme) -> ours::MutualTlsSecurityScheme {
    ours::MutualTlsSecurityScheme {
        description: non_empty(s.description),
    }
}

fn pb_auth_code_flow_to_ours(f: pb::AuthorizationCodeOAuthFlow) -> ours::AuthorizationCodeOAuthFlow {
    ours::AuthorizationCodeOAuthFlow {
        authorization_url: f.authorization_url,
        token_url: f.token_url,
        refresh_url: non_empty(f.refresh_url),
        scopes: f.scopes,
        pkce_required: f.pkce_required,
    }
}

fn pb_client_creds_flow_to_ours(f: pb::ClientCredentialsOAuthFlow) -> ours::ClientCredentialsOAuthFlow {
    ours::ClientCredentialsOAuthFlow {
        token_url: f.token_url,
        refresh_url: non_empty(f.refresh_url),
        scopes: f.scopes,
    }
}

fn pb_implicit_flow_to_ours(f: pb::ImplicitOAuthFlow) -> ours::ImplicitOAuthFlow {
    ours::ImplicitOAuthFlow {
        authorization_url: f.authorization_url,
        refresh_url: non_empty(f.refresh_url),
        scopes: f.scopes,
    }
}

fn pb_password_flow_to_ours(f: pb::PasswordOAuthFlow) -> ours::PasswordOAuthFlow {
    ours::PasswordOAuthFlow {
        token_url: f.token_url,
        refresh_url: non_empty(f.refresh_url),
        scopes: f.scopes,
    }
}

fn pb_device_code_flow_to_ours(f: pb::DeviceCodeOAuthFlow) -> ours::DeviceCodeOAuthFlow {
    ours::DeviceCodeOAuthFlow {
        device_authorization_url: f.device_authorization_url,
        token_url: f.token_url,
        refresh_url: non_empty(f.refresh_url),
        scopes: f.scopes,
    }
}

#[allow(deprecated)]
fn pb_oauth_flows_to_ours(f: pb::OAuthFlows) -> Result<ours::OAuthFlows, Status> {
    use pb::o_auth_flows::Flow;
    match f.flow {
        Some(Flow::AuthorizationCode(a)) => Ok(ours::OAuthFlows::AuthorizationCode {
            authorization_code: pb_auth_code_flow_to_ours(a),
        }),
        Some(Flow::ClientCredentials(c)) => Ok(ours::OAuthFlows::ClientCredentials {
            client_credentials: pb_client_creds_flow_to_ours(c),
        }),
        Some(Flow::Implicit(i)) => Ok(ours::OAuthFlows::Implicit {
            implicit: pb_implicit_flow_to_ours(i),
        }),
        Some(Flow::Password(p)) => Ok(ours::OAuthFlows::Password {
            password: pb_password_flow_to_ours(p),
        }),
        Some(Flow::DeviceCode(d)) => Ok(ours::OAuthFlows::DeviceCode {
            device_code: pb_device_code_flow_to_ours(d),
        }),
        None => Err(Status::invalid_argument("OAuthFlows.flow is required")),
    }
}

fn pb_oauth2_scheme_to_ours(s: pb::OAuth2SecurityScheme) -> Result<ours::OAuth2SecurityScheme, Status> {
    Ok(ours::OAuth2SecurityScheme {
        description: non_empty(s.description),
        flows: s
            .flows
            .map(pb_oauth_flows_to_ours)
            .transpose()?
            .ok_or_else(|| Status::invalid_argument("OAuth2SecurityScheme.flows is required"))?,
        oauth2_metadata_url: non_empty(s.oauth2_metadata_url),
    })
}

fn pb_security_scheme_to_ours(s: pb::SecurityScheme) -> Result<ours::SecurityScheme, Status> {
    use pb::security_scheme::Scheme;
    match s.scheme {
        Some(Scheme::ApiKeySecurityScheme(a)) => Ok(ours::SecurityScheme::ApiKey {
            api_key_security_scheme: pb_api_key_scheme_to_ours(a),
        }),
        Some(Scheme::HttpAuthSecurityScheme(h)) => Ok(ours::SecurityScheme::HttpAuth {
            http_auth_security_scheme: pb_http_auth_scheme_to_ours(h),
        }),
        Some(Scheme::Oauth2SecurityScheme(o)) => Ok(ours::SecurityScheme::OAuth2 {
            oauth2_security_scheme: pb_oauth2_scheme_to_ours(o)?,
        }),
        Some(Scheme::OpenIdConnectSecurityScheme(o)) => Ok(ours::SecurityScheme::OpenIdConnect {
            open_id_connect_security_scheme: pb_oidc_scheme_to_ours(o),
        }),
        Some(Scheme::MtlsSecurityScheme(m)) => Ok(ours::SecurityScheme::MutualTls {
            mtls_security_scheme: pb_mtls_scheme_to_ours(m),
        }),
        None => Err(Status::invalid_argument("SecurityScheme.scheme is required")),
    }
}

pub fn pb_agent_card_to_ours(c: pb::AgentCard) -> Result<ours::AgentCard, Status> {
    Ok(ours::AgentCard {
        name: c.name,
        description: c.description,
        supported_interfaces: c
            .supported_interfaces
            .into_iter()
            .map(pb_agent_interface_to_ours)
            .collect(),
        provider: c.provider.map(pb_agent_provider_to_ours),
        version: c.version,
        documentation_url: c.documentation_url,
        capabilities: c
            .capabilities
            .map(pb_agent_capabilities_to_ours)
            .ok_or_else(|| Status::invalid_argument("AgentCard.capabilities is required"))?,
        security_schemes: c
            .security_schemes
            .into_iter()
            .map(|(k, v)| Ok((k, pb_security_scheme_to_ours(v)?)))
            .collect::<Result<_, Status>>()?,
        security_requirements: c
            .security_requirements
            .into_iter()
            .map(pb_security_requirement_to_ours)
            .collect(),
        default_input_modes: c.default_input_modes,
        default_output_modes: c.default_output_modes,
        skills: c.skills.into_iter().map(pb_agent_skill_to_ours).collect(),
        signatures: c
            .signatures
            .into_iter()
            .map(pb_agent_card_signature_to_ours)
            .collect(),
        icon_url: c.icon_url,
    })
}

// --- ours -> pb: requests (outbound only, for the gRPC client) ---

fn our_send_config_to_pb(c: ours::SendMessageConfiguration) -> pb::SendMessageConfiguration {
    pb::SendMessageConfiguration {
        accepted_output_modes: c.accepted_output_modes,
        task_push_notification_config: c.task_push_notification_config.map(our_push_config_to_pb),
        history_length: c.history_length,
        return_immediately: c.return_immediately,
    }
}

pub fn our_send_message_request_to_pb(r: ours::SendMessageRequest) -> pb::SendMessageRequest {
    pb::SendMessageRequest {
        tenant: r.tenant.unwrap_or_default(),
        message: Some(our_message_to_pb(r.message)),
        configuration: r.configuration.map(our_send_config_to_pb),
        metadata: r.metadata.map(json_to_struct),
    }
}

pub fn our_get_task_request_to_pb(r: ours::GetTaskRequest) -> pb::GetTaskRequest {
    pb::GetTaskRequest {
        tenant: r.tenant.unwrap_or_default(),
        id: r.id,
        history_length: r.history_length,
    }
}

pub fn our_list_tasks_request_to_pb(r: ours::ListTasksRequest) -> pb::ListTasksRequest {
    pb::ListTasksRequest {
        tenant: r.tenant.unwrap_or_default(),
        context_id: r.context_id.unwrap_or_default(),
        status: r
            .status
            .map(our_task_state_to_pb)
            .unwrap_or(pb::TaskState::Unspecified) as i32,
        page_size: r.page_size,
        page_token: r.page_token.unwrap_or_default(),
        history_length: r.history_length,
        status_timestamp_after: r.status_timestamp_after.map(chrono_to_timestamp),
        include_artifacts: r.include_artifacts,
    }
}

pub fn our_cancel_task_request_to_pb(r: ours::CancelTaskRequest) -> pb::CancelTaskRequest {
    pb::CancelTaskRequest {
        tenant: r.tenant.unwrap_or_default(),
        id: r.id,
        metadata: r.metadata.map(json_to_struct),
    }
}

pub fn our_subscribe_to_task_request_to_pb(r: ours::SubscribeToTaskRequest) -> pb::SubscribeToTaskRequest {
    pb::SubscribeToTaskRequest {
        tenant: r.tenant.unwrap_or_default(),
        id: r.id,
    }
}

pub fn our_get_push_notification_config_request_to_pb(
    r: ours::GetTaskPushNotificationConfigRequest,
) -> pb::GetTaskPushNotificationConfigRequest {
    pb::GetTaskPushNotificationConfigRequest {
        tenant: r.tenant.unwrap_or_default(),
        task_id: r.task_id,
        id: r.id,
    }
}

pub fn our_delete_push_notification_config_request_to_pb(
    r: ours::DeleteTaskPushNotificationConfigRequest,
) -> pb::DeleteTaskPushNotificationConfigRequest {
    pb::DeleteTaskPushNotificationConfigRequest {
        tenant: r.tenant.unwrap_or_default(),
        task_id: r.task_id,
        id: r.id,
    }
}

pub fn our_list_push_notification_configs_request_to_pb(
    r: ours::ListTaskPushNotificationConfigsRequest,
) -> pb::ListTaskPushNotificationConfigsRequest {
    pb::ListTaskPushNotificationConfigsRequest {
        tenant: r.tenant.unwrap_or_default(),
        task_id: r.task_id,
        page_size: r.page_size.unwrap_or_default(),
        page_token: r.page_token.unwrap_or_default(),
    }
}

pub fn our_get_extended_agent_card_request_to_pb(
    r: ours::GetExtendedAgentCardRequest,
) -> pb::GetExtendedAgentCardRequest {
    pb::GetExtendedAgentCardRequest {
        tenant: r.tenant.unwrap_or_default(),
    }
}
