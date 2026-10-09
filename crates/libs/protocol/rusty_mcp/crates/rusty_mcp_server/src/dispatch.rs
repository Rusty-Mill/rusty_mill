//! The sans-IO core: one decoded message in, at most one message out.

use crate::Context;
use rusty_json::Value;
use rusty_mcp_proto::jsonrpc::code;
use rusty_mcp_proto::{
    CallToolParams, CallToolResult, ErrorObject, Implementation, InitializeParams,
    InitializeResult, ListParams, ListToolsResult, Message, RequestId, ServerCapabilities,
};
use std::panic::{catch_unwind, AssertUnwindSafe};

/// Protocol revisions this scaffold can speak, newest first.
pub const SUPPORTED_VERSIONS: [&str; 5] = [
    "2026-07-28",
    "2025-11-25",
    "2025-06-18",
    "2025-03-26",
    "2024-11-05",
];

/// What a server implements. Everything but [`Service::info`] has a default
/// that answers "not offered", so a service overrides only what it supports.
pub trait Service: Send + Sync {
    /// Who this server is, what it offers, and optional usage hints.
    fn info(&self) -> (Implementation, ServerCapabilities, Option<String>);

    /// `tools/list`.
    fn list_tools(&self, _params: &ListParams) -> Result<ListToolsResult, ErrorObject> {
        Ok(ListToolsResult::default())
    }

    /// `tools/call`. A tool that fails returns `Ok` with `is_error` set; an
    /// `Err` is a protocol error such as an unknown tool.
    fn call_tool(
        &self,
        _ctx: &Context,
        params: &CallToolParams,
    ) -> Result<CallToolResult, ErrorObject> {
        Err(ErrorObject::new(
            code::INVALID_PARAMS,
            format!("Unknown tool: {}", params.name),
        ))
    }
}

/// Routes decoded messages to a [`Service`].
pub struct Dispatcher<S> {
    service: S,
}

impl<S: Service> Dispatcher<S> {
    /// A dispatcher over `service`.
    pub fn new(service: S) -> Self {
        Dispatcher { service }
    }

    /// The wrapped service.
    pub fn service(&self) -> &S {
        &self.service
    }

    /// Handle one message. Requests produce a response; notifications and
    /// stray responses produce `None`.
    pub fn handle(&self, ctx: &Context, message: Message) -> Option<Message> {
        match message {
            Message::Request { id, method, params } => {
                Some(match self.request(ctx, &method, params.as_ref()) {
                    Ok(result) => Message::Response { id, result },
                    Err(error) => Message::Error {
                        id: Some(id),
                        error,
                    },
                })
            }
            // Notifications (initialized, cancelled, ...) need no reply, and a
            // server has sent no requests of its own to receive replies to.
            Message::Notification { .. } | Message::Response { .. } | Message::Error { .. } => None,
        }
    }

    fn request(
        &self,
        ctx: &Context,
        method: &str,
        params: Option<&Value>,
    ) -> Result<Value, ErrorObject> {
        let empty = Value::object();
        let params_or_empty = params.unwrap_or(&empty);
        match method {
            "initialize" => {
                let init = InitializeParams::from_value(params_or_empty).map_err(invalid_params)?;
                Ok(self.initialize(&init).to_value())
            }
            "ping" => Ok(Value::object()),
            "tools/list" => {
                let list = ListParams::from_value(params_or_empty).map_err(invalid_params)?;
                self.service.list_tools(&list).map(|r| r.to_value())
            }
            "tools/call" => {
                let call = CallToolParams::from_value(params_or_empty).map_err(invalid_params)?;
                // A panicking handler must not take the server down.
                match catch_unwind(AssertUnwindSafe(|| self.service.call_tool(ctx, &call))) {
                    Ok(result) => result.map(|r| r.to_value()),
                    Err(_) => Err(ErrorObject::new(
                        code::INTERNAL_ERROR,
                        format!("tool {:?} panicked", call.name),
                    )),
                }
            }
            other => Err(ErrorObject::new(
                code::METHOD_NOT_FOUND,
                format!("Method not found: {other}"),
            )),
        }
    }

    fn initialize(&self, init: &InitializeParams) -> InitializeResult {
        let (server_info, capabilities, instructions) = self.service.info();
        let protocol_version = SUPPORTED_VERSIONS
            .iter()
            .find(|v| **v == init.protocol_version)
            .unwrap_or(&SUPPORTED_VERSIONS[0]);
        InitializeResult {
            protocol_version: (*protocol_version).to_string(),
            capabilities,
            server_info,
            instructions,
        }
    }
}

fn invalid_params(error: rusty_mcp_proto::Error) -> ErrorObject {
    ErrorObject::new(code::INVALID_PARAMS, error.to_string())
}

/// A response for input that was not a valid message.
pub(crate) fn parse_failure(error: &rusty_mcp_proto::Error, id: Option<RequestId>) -> Message {
    let code = match error {
        rusty_mcp_proto::Error::Json(_) => code::PARSE_ERROR,
        rusty_mcp_proto::Error::Decode { .. } => code::INVALID_REQUEST,
    };
    Message::Error {
        id,
        error: ErrorObject::new(code, error.to_string()),
    }
}
