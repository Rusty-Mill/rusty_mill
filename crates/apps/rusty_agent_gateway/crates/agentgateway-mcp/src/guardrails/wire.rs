//! The `agentgateway.dev.ext_mcp` protocol.
//!
//! These messages are written as tables on `rusty_prost` rather than generated:
//! `tonic-build` wants `protoc` on the machine doing the build, and making
//! everyone who compiles this gateway install a protobuf compiler to get eight
//! small messages is a poor trade. The schema is in `proto/ext_mcp.proto` next
//! to this crate, copied from upstream unchanged, and every field number below
//! is written next to the field it belongs to.
//!
//! Field numbers are the wire contract: renaming a field here is free, but
//! changing a number silently talks to a different field on the other end.
//! `tests/wire_vectors.rs` pins the encoding against bytes produced by Google's
//! protobuf runtime from the `.proto`, so a typo fails a test rather than a
//! deployment.

use rusty_prost::wkt::{Struct, Value};
use rusty_prost::{message, oneof, open_enum};

message! {
    /// The gateway asking whether a call may proceed.
    McpRequest {
        // Backends this call targets, in their own namespace.
        1 service_names: Vec<String> = rep_string,
        // The JSON-RPC method.
        2 method: String = string,
        // CEL-evaluated context from the gateway's config.
        3 metadata_context: Option<Struct> = msg,
        // JSON-RPC `params` as raw JSON, absent when the method has none.
        4 mcp_request: Option<Vec<u8>> = opt_bytes,
        // Request headers, after filtering. One entry per value.
        5 headers: Vec<McpHeader> = rep_msg,
    }
}

message! {
    /// The gateway asking whether a result may be returned.
    McpResponse {
        // Backends this call targeted.
        1 service_names: Vec<String> = rep_string,
        // The JSON-RPC method.
        2 method: String = string,
        // CEL-evaluated context from the gateway's config.
        3 metadata_context: Option<Struct> = msg,
        // The upstream's JSON-RPC `result` as raw JSON. Errors skip this hook.
        4 mcp_response: Vec<u8> = bytes,
    }
}

message! {
    /// What the processor decided about a request.
    McpRequestResult {
        // Pass, a replacement body, or a refusal.
        oneof result: request_result::Result;
        // Changes to the HTTP request carrying this call. Ignored on a refusal.
        4 header_mutation: Option<HeaderMutation> = msg,
        // Values stashed for later filters. Ignored on a refusal.
        5 metadata: Option<Struct> = msg,
    }
}

/// The `McpRequestResult.result` oneof.
pub mod request_result {
    use super::*;

    oneof! {
        /// Which of the three answers the processor gave.
        Result {
            // Proceed unchanged.
            1 Pass(Pass) = msg,
            // Proceed with these JSON-RPC `params` instead.
            2 Mutated(Vec<u8>) = bytes,
            // Refuse.
            3 Error(AuthorizationError) = msg,
        }
    }
}

message! {
    /// What the processor decided about a result.
    McpResponseResult {
        // Pass, a replacement result, or a refusal.
        oneof result: response_result::Result;
    }
}

/// The `McpResponseResult.result` oneof.
pub mod response_result {
    use super::*;

    oneof! {
        /// Which of the three answers the processor gave.
        Result {
            // Return the result unchanged.
            1 Pass(Pass) = msg,
            // Return this JSON-RPC `result` instead.
            2 Mutated(Vec<u8>) = bytes,
            // Refuse.
            3 Error(AuthorizationError) = msg,
        }
    }
}

message! {
    /// Changes to the HTTP request carrying an MCP call.
    HeaderMutation {
        // Headers to overwrite, inserting when absent.
        1 set: Vec<McpHeader> = rep_msg,
        // Header names to remove, matched case-insensitively.
        2 remove: Vec<String> = rep_string,
    }
}

message! {
    /// One header. The value is bytes: HTTP header values need not be UTF-8.
    McpHeader {
        // Header name.
        1 key: String = string,
        // Raw value bytes.
        2 value: Vec<u8> = bytes,
    }
}

message! {
    /// "Proceed." Empty by design; the variant carries the meaning.
    Pass {}
}

message! {
    /// A refusal.
    AuthorizationError {
        // One of [`authorization_error::Code`].
        1 code: i32 = enumv,
        // Human-readable reason, returned to the caller.
        2 reason: String = string,
        // An explicit JSON-RPC error payload, overriding the code's default.
        3 mcp_error: Option<Vec<u8>> = opt_bytes,
    }
}

/// The `AuthorizationError` enums.
pub mod authorization_error {
    use super::*;

    open_enum! {
        /// Why the processor refused.
        Code {
            // Unspecified.
            Unknown = 0: "UNKNOWN",
            // The caller may not do this.
            PermissionDenied = 1: "PERMISSION_DENIED",
            // A quota is spent.
            ResourceExhausted = 2: "RESOURCE_EXHAUSTED",
            // The call itself is malformed.
            Invalid = 3: "INVALID",
        }
    }
}

/// The `ExtMcp` service, as a `rusty_tonic` client.
///
/// Written out rather than generated for the same reason as the messages. Both
/// methods are unary, so the whole client is two calls over one channel.
pub mod client {
    use rusty_prost::Message;
    use rusty_tonic::Status;
    use rusty_tonic::bridge::spawn;
    use rusty_tonic::client::{Channel, Request};

    use super::{McpRequest, McpRequestResult, McpResponse, McpResponseResult};

    /// A client for `agentgateway.dev.ext_mcp.ExtMcp`.
    #[derive(Clone)]
    pub struct ExtMcpClient {
        channel: Channel,
    }

    impl ExtMcpClient {
        /// Wrap a channel.
        pub fn new(channel: Channel) -> Self {
            ExtMcpClient { channel }
        }

        /// `ExtMcp/CheckRequest`.
        pub async fn check_request(&self, request: McpRequest) -> Result<McpRequestResult, Status> {
            self.unary(request, "/agentgateway.dev.ext_mcp.ExtMcp/CheckRequest")
                .await
        }

        /// `ExtMcp/CheckResponse`.
        pub async fn check_response(
            &self,
            request: McpResponse,
        ) -> Result<McpResponseResult, Status> {
            self.unary(request, "/agentgateway.dev.ext_mcp.ExtMcp/CheckResponse")
                .await
        }

        /// One call, made on the transport's runtime (this one is `tokio`) and cancelled on the
        /// wire if the caller drops it.
        async fn unary<Req, Res>(&self, request: Req, path: &'static str) -> Result<Res, Status>
        where
            Req: Message + Send + 'static,
            Res: Message + Send + 'static,
        {
            let channel = self.channel.clone();
            spawn(async move {
                channel
                    .unary::<Req, Res>(path, Request::new(request))
                    .await
                    .map(|reply| reply.message)
            })?
            .await?
        }
    }
}

/// Convert a CEL result into JSON, or `None` when it has no JSON form.
///
/// `Function` and `Opaque` values have no representation in a protobuf
/// `Struct`, so a metadata key whose expression produces one is dropped rather
/// than sent as something it is not. Bytes become a string when they are valid
/// UTF-8 and are dropped otherwise, for the same reason.
pub fn cel_to_json(value: cel::Value) -> Option<serde_json::Value> {
    use serde_json::Value as Json;
    Some(match value {
        cel::Value::Null => Json::Null,
        cel::Value::Bool(b) => Json::Bool(b),
        cel::Value::Int(i) => Json::Number(i.into()),
        cel::Value::UInt(u) => Json::Number(u.into()),
        cel::Value::Float(f) => serde_json::Number::from_f64(f).map(Json::Number)?,
        cel::Value::String(s) => Json::String(s.to_string()),
        cel::Value::Bytes(b) => Json::String(String::from_utf8(b.to_vec()).ok()?),
        cel::Value::List(items) => Json::Array(
            items
                .iter()
                .map(|item| cel_to_json(item.clone()))
                .collect::<Option<Vec<_>>>()?,
        ),
        cel::Value::Map(map) => Json::Object(
            map.map
                .iter()
                .map(|(key, value)| {
                    let key = match key {
                        cel::objects::Key::String(s) => s.to_string(),
                        cel::objects::Key::Int(i) => i.to_string(),
                        cel::objects::Key::Uint(u) => u.to_string(),
                        cel::objects::Key::Bool(b) => b.to_string(),
                    };
                    cel_to_json(value.clone()).map(|value| (key, value))
                })
                .collect::<Option<serde_json::Map<_, _>>>()?,
        ),
        _ => return None,
    })
}

/// Convert a protobuf `Struct` field value back into JSON.
///
/// The inverse of [`to_proto_value`], for reading what a processor sent.
pub fn proto_to_json(value: Value) -> serde_json::Value {
    match value {
        Value::Null => serde_json::Value::Null,
        Value::Bool(b) => serde_json::Value::Bool(b),
        Value::Number(n) => serde_json::Number::from_f64(n)
            .map(serde_json::Value::Number)
            // NaN and the infinities have no JSON form. Rendering them as text
            // keeps the key rather than dropping the processor's answer.
            .unwrap_or_else(|| serde_json::Value::String(n.to_string())),
        Value::String(s) => serde_json::Value::String(s),
        Value::List(values) => {
            serde_json::Value::Array(values.into_iter().map(proto_to_json).collect())
        }
        Value::Struct(structure) => serde_json::Value::Object(
            structure
                .fields
                .into_iter()
                .map(|(k, v)| (k, proto_to_json(v)))
                .collect(),
        ),
    }
}

/// Convert a JSON value into the protobuf `Struct` field type.
///
/// Numbers are worth a note: protobuf's `number_value` is a `double`, so a
/// `u64` beyond 2^53 loses precision here. JSON says the same thing about its
/// own numbers, so nothing is lost that survived being JSON in the first place.
pub fn to_proto_value(value: serde_json::Value) -> Value {
    match value {
        serde_json::Value::Null => Value::Null,
        serde_json::Value::Bool(b) => Value::Bool(b),
        serde_json::Value::Number(n) => Value::Number(n.as_f64().unwrap_or(f64::NAN)),
        serde_json::Value::String(s) => Value::String(s),
        serde_json::Value::Array(values) => {
            Value::List(values.into_iter().map(to_proto_value).collect())
        }
        serde_json::Value::Object(map) => Value::Struct(Struct {
            fields: map
                .into_iter()
                .map(|(k, v)| (k, to_proto_value(v)))
                .collect(),
        }),
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use rusty_prost::Message;

    #[test]
    fn a_request_round_trips() {
        let request = McpRequest {
            service_names: vec!["alpha".into(), "beta".into()],
            method: "tools/call".into(),
            metadata_context: Some(Struct {
                fields: [("tenant".to_string(), to_proto_value("acme".into()))].into(),
            }),
            mcp_request: Some(br#"{"name":"echo"}"#.to_vec()),
            headers: vec![McpHeader {
                key: "x-tenant".into(),
                value: b"acme".to_vec(),
            }],
        };

        let decoded = McpRequest::decode(&request.encode_to_vec().expect("should encode"))
            .expect("should decode");
        assert_eq!(decoded, request);
    }

    #[test]
    fn an_absent_body_is_distinguishable_from_an_empty_one() {
        // `mcp_request` is `optional`, and the difference carries meaning: a
        // method with no params is not a method whose params are `{}`.
        let round = |mcp_request| {
            let request = McpRequest {
                mcp_request,
                ..Default::default()
            };
            McpRequest::decode(&request.encode_to_vec().expect("should encode"))
                .expect("should decode")
                .mcp_request
        };
        assert_eq!(round(None), None);
        assert_eq!(round(Some(Vec::new())), Some(Vec::new()));
    }

    #[test]
    fn each_result_variant_round_trips() {
        for result in [
            request_result::Result::Pass(Pass {}),
            request_result::Result::Mutated(b"{}".to_vec()),
            request_result::Result::Error(AuthorizationError {
                code: authorization_error::Code::PermissionDenied as i32,
                reason: "nope".into(),
                mcp_error: None,
            }),
        ] {
            let message = McpRequestResult {
                result: Some(result.clone()),
                ..Default::default()
            };
            let decoded =
                McpRequestResult::decode(&message.encode_to_vec().expect("should encode"))
                    .expect("should decode");
            assert_eq!(decoded.result, Some(result));
        }
    }

    #[test]
    fn json_maps_onto_the_struct_type() {
        let value = to_proto_value(serde_json::json!({
            "s": "text", "n": 1.5, "b": true, "z": null, "l": [1, 2],
        }));
        let Value::Struct(structure) = value else {
            panic!("an object should map to a struct");
        };
        assert!(matches!(structure.fields["s"], Value::String(_)));
        assert!(matches!(structure.fields["n"], Value::Number(_)));
        assert!(matches!(structure.fields["b"], Value::Bool(true)));
        assert!(matches!(structure.fields["z"], Value::Null));
        assert!(matches!(structure.fields["l"], Value::List(_)));
    }

    #[test]
    fn json_survives_a_trip_through_the_struct_type() {
        let json = serde_json::json!({"a": [1.5, "x", null, {"b": true}], "c": "d"});
        assert_eq!(proto_to_json(to_proto_value(json.clone())), json);
    }
}
