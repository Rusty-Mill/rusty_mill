//! The `agentgateway.dev.ext_mcp` protocol.
//!
//! These messages are written by hand rather than generated. `tonic-build`
//! wants `protoc` on the machine doing the build, and making everyone who
//! compiles this gateway install a protobuf compiler to get eight small
//! messages is a poor trade. The schema is in `proto/ext_mcp.proto` next to
//! this crate, copied from upstream unchanged, and every field number below is
//! written next to the field it belongs to.
//!
//! Field numbers are the wire contract: renaming a field here is free, but
//! changing a number silently talks to a different field on the other end.
//! [`tests`] pins the encoding against bytes rather than against these
//! declarations, so a typo fails a test rather than a deployment.

use prost::Message;

/// The gateway asking whether a call may proceed.
#[derive(Clone, PartialEq, Message)]
pub struct McpRequest {
    /// Backends this call targets, in their own namespace.
    #[prost(string, repeated, tag = "1")]
    pub service_names: Vec<String>,
    /// The JSON-RPC method.
    #[prost(string, tag = "2")]
    pub method: String,
    /// CEL-evaluated context from the gateway's config.
    #[prost(message, optional, tag = "3")]
    pub metadata_context: Option<prost_types::Struct>,
    /// JSON-RPC `params` as raw JSON, absent when the method has none.
    #[prost(bytes = "vec", optional, tag = "4")]
    pub mcp_request: Option<Vec<u8>>,
    /// Request headers, after filtering. One entry per value.
    #[prost(message, repeated, tag = "5")]
    pub headers: Vec<McpHeader>,
}

/// The gateway asking whether a result may be returned.
#[derive(Clone, PartialEq, Message)]
pub struct McpResponse {
    /// Backends this call targeted.
    #[prost(string, repeated, tag = "1")]
    pub service_names: Vec<String>,
    /// The JSON-RPC method.
    #[prost(string, tag = "2")]
    pub method: String,
    /// CEL-evaluated context from the gateway's config.
    #[prost(message, optional, tag = "3")]
    pub metadata_context: Option<prost_types::Struct>,
    /// The upstream's JSON-RPC `result` as raw JSON. Errors skip this hook.
    #[prost(bytes = "vec", tag = "4")]
    pub mcp_response: Vec<u8>,
}

/// What the processor decided about a request.
#[derive(Clone, PartialEq, Message)]
pub struct McpRequestResult {
    /// Pass, a replacement body, or a refusal.
    #[prost(oneof = "request_result::Result", tags = "1, 2, 3")]
    pub result: Option<request_result::Result>,
    /// Changes to the HTTP request carrying this call. Ignored on a refusal.
    #[prost(message, optional, tag = "4")]
    pub header_mutation: Option<HeaderMutation>,
    /// Values stashed for later filters. Ignored on a refusal.
    #[prost(message, optional, tag = "5")]
    pub metadata: Option<prost_types::Struct>,
}

/// The `McpRequestResult.result` oneof.
pub mod request_result {
    /// Which of the three answers the processor gave.
    #[derive(Clone, PartialEq, prost::Oneof)]
    pub enum Result {
        /// Proceed unchanged.
        #[prost(message, tag = "1")]
        Pass(super::Pass),
        /// Proceed with these JSON-RPC `params` instead.
        #[prost(bytes = "vec", tag = "2")]
        Mutated(Vec<u8>),
        /// Refuse.
        #[prost(message, tag = "3")]
        Error(super::AuthorizationError),
    }
}

/// What the processor decided about a result.
#[derive(Clone, PartialEq, Message)]
pub struct McpResponseResult {
    /// Pass, a replacement result, or a refusal.
    #[prost(oneof = "response_result::Result", tags = "1, 2, 3")]
    pub result: Option<response_result::Result>,
}

/// The `McpResponseResult.result` oneof.
pub mod response_result {
    /// Which of the three answers the processor gave.
    #[derive(Clone, PartialEq, prost::Oneof)]
    pub enum Result {
        /// Return the result unchanged.
        #[prost(message, tag = "1")]
        Pass(super::Pass),
        /// Return this JSON-RPC `result` instead.
        #[prost(bytes = "vec", tag = "2")]
        Mutated(Vec<u8>),
        /// Refuse.
        #[prost(message, tag = "3")]
        Error(super::AuthorizationError),
    }
}

/// Changes to the HTTP request carrying an MCP call.
#[derive(Clone, PartialEq, Message)]
pub struct HeaderMutation {
    /// Headers to overwrite, inserting when absent.
    #[prost(message, repeated, tag = "1")]
    pub set: Vec<McpHeader>,
    /// Header names to remove, matched case-insensitively.
    #[prost(string, repeated, tag = "2")]
    pub remove: Vec<String>,
}

/// One header. The value is bytes: HTTP header values need not be UTF-8.
#[derive(Clone, PartialEq, Message)]
pub struct McpHeader {
    /// Header name.
    #[prost(string, tag = "1")]
    pub key: String,
    /// Raw value bytes.
    #[prost(bytes = "vec", tag = "2")]
    pub value: Vec<u8>,
}

/// "Proceed." Empty by design; the variant carries the meaning.
#[derive(Clone, Copy, PartialEq, Eq, Message)]
pub struct Pass {}

/// A refusal.
#[derive(Clone, PartialEq, Message)]
pub struct AuthorizationError {
    /// One of [`authorization_error::Code`].
    #[prost(enumeration = "authorization_error::Code", tag = "1")]
    pub code: i32,
    /// Human-readable reason, returned to the caller.
    #[prost(string, tag = "2")]
    pub reason: String,
    /// An explicit JSON-RPC error payload, overriding the code's default.
    #[prost(bytes = "vec", optional, tag = "3")]
    pub mcp_error: Option<Vec<u8>>,
}

/// The `AuthorizationError` enums.
pub mod authorization_error {
    /// Why the processor refused.
    #[derive(Clone, Copy, Debug, PartialEq, Eq, PartialOrd, Ord, Hash, prost::Enumeration)]
    #[repr(i32)]
    pub enum Code {
        /// Unspecified.
        Unknown = 0,
        /// The caller may not do this.
        PermissionDenied = 1,
        /// A quota is spent.
        ResourceExhausted = 2,
        /// The call itself is malformed.
        Invalid = 3,
    }
}

/// The `ExtMcp` service, as a `tonic` client.
///
/// Written out rather than generated for the same reason as the messages. Both
/// methods are unary, so the whole client is two calls over one codec.
pub mod client {
    use tonic::client::GrpcService;
    use tonic::codegen::{Body, Bytes, StdError};

    use super::{McpRequest, McpRequestResult, McpResponse, McpResponseResult};

    /// A client for `agentgateway.dev.ext_mcp.ExtMcp`.
    #[derive(Debug, Clone)]
    pub struct ExtMcpClient<T> {
        inner: tonic::client::Grpc<T>,
    }

    impl<T> ExtMcpClient<T>
    where
        T: GrpcService<tonic::body::Body>,
        T::Error: Into<StdError>,
        T::ResponseBody: Body<Data = Bytes> + std::marker::Send + 'static,
        <T::ResponseBody as Body>::Error: Into<StdError> + std::marker::Send,
    {
        /// Wrap a transport.
        pub fn new(inner: T) -> Self {
            ExtMcpClient {
                inner: tonic::client::Grpc::new(inner),
            }
        }

        /// `ExtMcp/CheckRequest`.
        pub async fn check_request(
            &mut self,
            request: McpRequest,
        ) -> Result<McpRequestResult, tonic::Status> {
            self.unary(request, "/agentgateway.dev.ext_mcp.ExtMcp/CheckRequest")
                .await
        }

        /// `ExtMcp/CheckResponse`.
        pub async fn check_response(
            &mut self,
            request: McpResponse,
        ) -> Result<McpResponseResult, tonic::Status> {
            self.unary(request, "/agentgateway.dev.ext_mcp.ExtMcp/CheckResponse")
                .await
        }

        async fn unary<Req, Res>(
            &mut self,
            request: Req,
            path: &'static str,
        ) -> Result<Res, tonic::Status>
        where
            Req: prost::Message + 'static,
            Res: prost::Message + Default + 'static,
        {
            self.inner.ready().await.map_err(|err| {
                tonic::Status::unavailable(format!("the processor is not ready: {}", err.into()))
            })?;
            let codec = tonic_prost::ProstCodec::default();
            let path = http::uri::PathAndQuery::from_static(path);
            self.inner
                .unary(tonic::Request::new(request), path, codec)
                .await
                .map(|response| response.into_inner())
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

/// Convert a JSON value into the protobuf `Struct` field type.
///
/// `prost_types::Value` has no `From<serde_json::Value>`, and the mapping is
/// the obvious one — but numbers are worth a note: protobuf's `number_value`
/// is a `double`, so a `u64` beyond 2^53 loses precision here. JSON says the
/// same thing about its own numbers, so nothing is lost that survived being
/// JSON in the first place.
pub fn to_proto_value(value: serde_json::Value) -> prost_types::Value {
    use prost_types::value::Kind;
    let kind = match value {
        serde_json::Value::Null => Kind::NullValue(0),
        serde_json::Value::Bool(b) => Kind::BoolValue(b),
        serde_json::Value::Number(n) => Kind::NumberValue(n.as_f64().unwrap_or(f64::NAN)),
        serde_json::Value::String(s) => Kind::StringValue(s),
        serde_json::Value::Array(values) => Kind::ListValue(prost_types::ListValue {
            values: values.into_iter().map(to_proto_value).collect(),
        }),
        serde_json::Value::Object(map) => Kind::StructValue(prost_types::Struct {
            fields: map
                .into_iter()
                .map(|(k, v)| (k, to_proto_value(v)))
                .collect(),
        }),
    };
    prost_types::Value { kind: Some(kind) }
}

#[cfg(test)]
mod tests {
    use super::*;

    /// Field numbers pinned against bytes rather than against the declarations
    /// above, so a mistyped tag fails here instead of on the wire.
    ///
    /// A protobuf field is `(number << 3) | wire_type`. Length-delimited is
    /// wire type 2, so `McpRequest.method` at field 2 is byte `0x12`.
    #[test]
    fn field_numbers_match_the_schema() {
        let encoded = McpRequest {
            service_names: vec!["alpha".into()],
            method: "tools/call".into(),
            ..Default::default()
        }
        .encode_to_vec();

        assert_eq!(encoded[0], 0x0a, "service_names is field 1");
        assert_eq!(&encoded[2..7], b"alpha");
        assert_eq!(encoded[7], 0x12, "method is field 2");
        assert_eq!(&encoded[9..19], b"tools/call");
    }

    #[test]
    fn a_request_round_trips() {
        let request = McpRequest {
            service_names: vec!["alpha".into(), "beta".into()],
            method: "tools/call".into(),
            metadata_context: Some(prost_types::Struct {
                fields: [("tenant".to_string(), to_proto_value("acme".into()))].into(),
            }),
            mcp_request: Some(br#"{"name":"echo"}"#.to_vec()),
            headers: vec![McpHeader {
                key: "x-tenant".into(),
                value: b"acme".to_vec(),
            }],
        };

        let decoded =
            McpRequest::decode(request.encode_to_vec().as_slice()).expect("should decode");
        assert_eq!(decoded, request);
    }

    #[test]
    fn an_absent_body_is_distinguishable_from_an_empty_one() {
        // `mcp_request` is `optional`, and the difference carries meaning: a
        // method with no params is not a method whose params are `{}`.
        let absent = McpRequest::decode(
            McpRequest {
                mcp_request: None,
                ..Default::default()
            }
            .encode_to_vec()
            .as_slice(),
        )
        .expect("should decode");
        assert_eq!(absent.mcp_request, None);

        let empty = McpRequest::decode(
            McpRequest {
                mcp_request: Some(Vec::new()),
                ..Default::default()
            }
            .encode_to_vec()
            .as_slice(),
        )
        .expect("should decode");
        assert_eq!(empty.mcp_request, Some(Vec::new()));
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
            let decoded = McpRequestResult::decode(message.encode_to_vec().as_slice())
                .expect("should decode");
            assert_eq!(decoded.result, Some(result));
        }
    }

    #[test]
    fn json_maps_onto_the_struct_type() {
        use prost_types::value::Kind;
        let value = to_proto_value(serde_json::json!({
            "s": "text", "n": 1.5, "b": true, "z": null, "l": [1, 2],
        }));
        let Some(Kind::StructValue(structure)) = value.kind else {
            panic!("an object should map to a struct");
        };
        assert!(matches!(
            structure.fields["s"].kind,
            Some(Kind::StringValue(_))
        ));
        assert!(matches!(
            structure.fields["n"].kind,
            Some(Kind::NumberValue(_))
        ));
        assert!(matches!(
            structure.fields["b"].kind,
            Some(Kind::BoolValue(true))
        ));
        assert!(matches!(
            structure.fields["z"].kind,
            Some(Kind::NullValue(_))
        ));
        assert!(matches!(
            structure.fields["l"].kind,
            Some(Kind::ListValue(_))
        ));
    }
}
