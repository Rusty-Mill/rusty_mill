//! The feature methods of a [`Connection`]: tools, prompts, resources and
//! completion. Each is a thin shell over the registry in [`Server`]; the
//! generic work (revision, pagination, cache hints, argument checks, error
//! codes) lives here so every feature behaves the same.

use crate::connection::{CancelToken, Connection};
use crate::page::{self, Kind};
use crate::server::MAX_COMPLETION_VALUES;
use rusty_json::Value;
use rusty_mcp_proto::rpc::params as decode_params;
use rusty_mcp_proto::{
    CacheScope, CallToolParams, CompleteParams, CompleteResult, ErrorCode, ErrorData,
    GetPromptParams, ListPromptsResult, ListResourceTemplatesResult, ListResourcesResult,
    ListToolsResult, PaginatedParams, Paging, ProtocolVersion, ReadResourceParams, Reference,
    RequestId, ResultType, Wire,
};
use std::ops::Range;

fn invalid_params(why: impl std::fmt::Display) -> ErrorData {
    ErrorData::new(ErrorCode::INVALID_PARAMS, why.to_string())
}

fn not_offered(what: &str) -> ErrorData {
    ErrorData::new(
        ErrorCode::METHOD_NOT_FOUND,
        format!("this server has no {what}"),
    )
}

/// Fill in `resultType` for a revision that has one.
fn complete_type(version: &ProtocolVersion, slot: &mut Option<ResultType>) {
    if version.is_stateless() && slot.is_none() {
        *slot = Some(ResultType(ResultType::COMPLETE.to_owned()));
    }
}

impl Connection {
    /// The common front of every list method: the revision in force, the
    /// slice of the list the request asks for, and the paging members of the
    /// reply (next cursor, and the cache hints a stateless revision carries).
    fn list_window(
        &self,
        params: &Option<Value>,
        kind: Kind,
        len: usize,
    ) -> Result<(Range<usize>, Paging), ErrorData> {
        let (version, _) = self.version_for(params)?;
        let p: PaginatedParams = decode_params(params).map_err(invalid_params)?;
        let (start, end, next_cursor) =
            page::window(kind, p.cursor.as_deref(), len, self.server().page_size)
                .ok_or_else(|| invalid_params("invalid cursor"))?;
        let mut paging = Paging {
            next_cursor,
            ..Paging::default()
        };
        if version.is_stateless() {
            paging.ttl_ms = Some(0);
            paging.cache_scope = Some(CacheScope::Public);
        }
        Ok((start..end, paging))
    }

    pub(crate) fn tools_list(&self, params: &Option<Value>) -> Result<Value, ErrorData> {
        let tools = &self.server().tools;
        if tools.is_empty() {
            return Err(not_offered("tools"));
        }
        let (range, paging) = self.list_window(params, Kind::Tools, tools.len())?;
        Ok(ListToolsResult {
            tools: tools[range].iter().map(|t| t.tool.clone()).collect(),
            paging,
        }
        .to_value())
    }

    pub(crate) fn tools_call(
        &self,
        id: &RequestId,
        params: &Option<Value>,
        token: &CancelToken,
    ) -> Result<Value, ErrorData> {
        let (version, meta) = self.version_for(params)?;
        let call: CallToolParams = decode_params(params).map_err(invalid_params)?;
        let entry = self
            .server()
            .tools
            .iter()
            .find(|t| t.tool.name == call.name)
            .ok_or_else(|| invalid_params(format!("unknown tool {:?}", call.name)))?;
        let ctx = self.call_context(id, version.clone(), meta, token);
        let mut result = (entry.handler)(&ctx, call)?;
        complete_type(&version, &mut result.result_type);
        Ok(result.to_value())
    }

    pub(crate) fn prompts_list(&self, params: &Option<Value>) -> Result<Value, ErrorData> {
        let prompts = &self.server().prompts;
        if prompts.is_empty() {
            return Err(not_offered("prompts"));
        }
        let (range, paging) = self.list_window(params, Kind::Prompts, prompts.len())?;
        Ok(ListPromptsResult {
            prompts: prompts[range].iter().map(|p| p.prompt.clone()).collect(),
            paging,
        }
        .to_value())
    }

    pub(crate) fn prompts_get(
        &self,
        id: &RequestId,
        params: &Option<Value>,
        token: &CancelToken,
    ) -> Result<Value, ErrorData> {
        if self.server().prompts.is_empty() {
            return Err(not_offered("prompts"));
        }
        let (version, meta) = self.version_for(params)?;
        let get: GetPromptParams = decode_params(params).map_err(invalid_params)?;
        let entry = self
            .server()
            .prompts
            .iter()
            .find(|p| p.prompt.name == get.name)
            .ok_or_else(|| invalid_params(format!("unknown prompt {:?}", get.name)))?;
        // Arguments the prompt declares required must be given.
        for arg in entry.prompt.arguments.iter().flatten() {
            let given = get
                .arguments
                .as_ref()
                .is_some_and(|a| a.get(&arg.name).is_some_and(|v| !v.is_null()));
            if arg.required == Some(true) && !given {
                return Err(invalid_params(format!(
                    "missing required argument {:?}",
                    arg.name
                )));
            }
        }
        let ctx = self.call_context(id, version.clone(), meta, token);
        let mut result = (entry.handler)(&ctx, get)?;
        complete_type(&version, &mut result.result_type);
        Ok(result.to_value())
    }

    pub(crate) fn resources_list(&self, params: &Option<Value>) -> Result<Value, ErrorData> {
        let s = self.server();
        if s.resources.is_empty() && s.templates.is_empty() {
            return Err(not_offered("resources"));
        }
        let (range, paging) = self.list_window(params, Kind::Resources, s.resources.len())?;
        Ok(ListResourcesResult {
            resources: s.resources[range]
                .iter()
                .map(|r| r.resource.clone())
                .collect(),
            paging,
        }
        .to_value())
    }

    pub(crate) fn templates_list(&self, params: &Option<Value>) -> Result<Value, ErrorData> {
        let s = self.server();
        if s.resources.is_empty() && s.templates.is_empty() {
            return Err(not_offered("resources"));
        }
        let (range, paging) = self.list_window(params, Kind::Templates, s.templates.len())?;
        Ok(ListResourceTemplatesResult {
            resource_templates: s.templates[range]
                .iter()
                .map(|t| t.template.clone())
                .collect(),
            paging,
        }
        .to_value())
    }

    pub(crate) fn resources_read(
        &self,
        id: &RequestId,
        params: &Option<Value>,
        token: &CancelToken,
    ) -> Result<Value, ErrorData> {
        let s = self.server();
        if s.resources.is_empty() && s.templates.is_empty() {
            return Err(not_offered("resources"));
        }
        let (version, meta) = self.version_for(params)?;
        let read: ReadResourceParams = decode_params(params).map_err(invalid_params)?;
        let ctx = self.call_context(id, version.clone(), meta, token);
        let mut result = if let Some(r) = s.resources.iter().find(|r| r.resource.uri == read.uri) {
            (r.handler)(&ctx, read)?
        } else if let Some((t, vars)) = s
            .templates
            .iter()
            .find_map(|t| t.matcher.matches(&read.uri).map(|vars| (t, vars)))
        {
            (t.handler)(&ctx, &vars, read)?
        } else {
            let mut data = Value::object();
            data.insert("uri", read.uri.as_str());
            return Err(ErrorData {
                code: ErrorCode::RESOURCE_NOT_FOUND,
                message: format!("resource not found: {}", read.uri),
                data: Some(data),
            });
        };
        complete_type(&version, &mut result.result_type);
        Ok(result.to_value())
    }

    pub(crate) fn complete(
        &self,
        id: &RequestId,
        params: &Option<Value>,
        token: &CancelToken,
    ) -> Result<Value, ErrorData> {
        let s = self.server();
        let Some(completer) = &s.completer else {
            return Err(not_offered("completions"));
        };
        let (version, meta) = self.version_for(params)?;
        let req: CompleteParams = decode_params(params).map_err(invalid_params)?;
        // The reference must name something this server offers.
        match &req.reference {
            Reference::Prompt { name, .. } => {
                let prompt = s
                    .prompts
                    .iter()
                    .find(|p| &p.prompt.name == name)
                    .ok_or_else(|| invalid_params(format!("unknown prompt {name:?}")))?;
                let declared = prompt.prompt.arguments.iter().flatten();
                if !declared.clone().any(|a| a.name == req.argument.name) {
                    return Err(invalid_params(format!(
                        "prompt {name:?} has no argument {:?}",
                        req.argument.name
                    )));
                }
            }
            Reference::Resource { uri } => {
                if !s.templates.iter().any(|t| &t.template.uri_template == uri) {
                    return Err(invalid_params(format!("unknown resource template {uri:?}")));
                }
            }
        }
        let ctx = self.call_context(id, version.clone(), meta, token);
        let mut completion = completer(&ctx, req)?;
        if completion.values.len() > MAX_COMPLETION_VALUES {
            completion
                .total
                .get_or_insert(completion.values.len() as u64);
            completion.values.truncate(MAX_COMPLETION_VALUES);
            completion.has_more = Some(true);
        }
        let mut result = CompleteResult {
            result_type: None,
            completion,
            meta: None,
        };
        complete_type(&version, &mut result.result_type);
        Ok(result.to_value())
    }
}
