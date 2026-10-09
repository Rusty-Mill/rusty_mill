//! A [`Service`] built from registered tools.

use crate::{Context, Service};
use rusty_json::Value;
use rusty_mcp_proto::jsonrpc::code;
use rusty_mcp_proto::{
    CallToolParams, CallToolResult, CompleteParams, CompleteResult, Completion, ErrorObject,
    GetPromptParams, GetPromptResult, Implementation, ListCapability, ListParams,
    ListPromptsResult, ListResourceTemplatesResult, ListResourcesResult, ListToolsResult, Prompt,
    ReadResourceParams, ReadResourceResult, Resource, ResourceContents, ResourceTemplate,
    ResourcesCapability, ServerCapabilities, Tool,
};
use std::collections::BTreeMap;

/// Why a tool failed. It becomes a tool result with `isError` set, which the
/// model can read, not a protocol error.
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct ToolError(pub String);

impl From<String> for ToolError {
    fn from(message: String) -> Self {
        ToolError(message)
    }
}

impl From<&str> for ToolError {
    fn from(message: &str) -> Self {
        ToolError(message.to_string())
    }
}

type ToolFn = Box<dyn Fn(&Context, &Value) -> Result<CallToolResult, ToolError> + Send + Sync>;
type ResourceFn =
    Box<dyn Fn(&Context, &str) -> Result<Vec<ResourceContents>, ToolError> + Send + Sync>;
type FallbackFn =
    Box<dyn Fn(&Context, &str) -> Option<Result<Vec<ResourceContents>, ToolError>> + Send + Sync>;
type PromptFn = Box<
    dyn Fn(&Context, &BTreeMap<String, String>) -> Result<GetPromptResult, ToolError> + Send + Sync,
>;
type CompleteFn = Box<dyn Fn(&Context, &CompleteParams) -> Completion + Send + Sync>;

/// A server assembled from tools.
pub struct Server {
    info: Implementation,
    instructions: Option<String>,
    tools: Vec<(Tool, ToolFn)>,
    resources: Vec<(Resource, ResourceFn)>,
    templates: Vec<ResourceTemplate>,
    fallback: Option<FallbackFn>,
    prompts: Vec<(Prompt, PromptFn)>,
    completer: Option<CompleteFn>,
}

impl Server {
    /// A server with no tools.
    pub fn new(name: impl Into<String>, version: impl Into<String>) -> Self {
        Server {
            info: Implementation::new(name, version),
            instructions: None,
            tools: Vec::new(),
            resources: Vec::new(),
            templates: Vec::new(),
            fallback: None,
            prompts: Vec::new(),
            completer: None,
        }
    }

    /// Usage hints sent in the `initialize` result.
    pub fn instructions(mut self, text: impl Into<String>) -> Self {
        self.instructions = Some(text.into());
        self
    }

    /// Register a tool. `handler` gets the arguments object (`Null` when the
    /// caller sent none). Registering a name twice replaces the earlier tool.
    pub fn tool<F>(mut self, tool: Tool, handler: F) -> Self
    where
        F: Fn(&Context, &Value) -> Result<CallToolResult, ToolError> + Send + Sync + 'static,
    {
        self.tools.retain(|(t, _)| t.name != tool.name);
        self.tools.push((tool, Box::new(handler)));
        self
    }
}

impl Server {
    /// Register a fixed resource, read by exact URI. Registering a URI twice
    /// replaces the earlier resource.
    pub fn resource<F>(mut self, resource: Resource, reader: F) -> Self
    where
        F: Fn(&Context, &str) -> Result<Vec<ResourceContents>, ToolError> + Send + Sync + 'static,
    {
        self.resources.retain(|(r, _)| r.uri != resource.uri);
        self.resources.push((resource, Box::new(reader)));
        self
    }

    /// Advertise a resource template. Matching a URI to a template is up to
    /// the handler given to [`Server::read_other`].
    pub fn resource_template(mut self, template: ResourceTemplate) -> Self {
        self.templates.push(template);
        self
    }

    /// Read URIs that are not fixed resources (those a template covers).
    /// Return `None` for a URI the handler does not recognise.
    pub fn read_other<F>(mut self, reader: F) -> Self
    where
        F: Fn(&Context, &str) -> Option<Result<Vec<ResourceContents>, ToolError>>
            + Send
            + Sync
            + 'static,
    {
        self.fallback = Some(Box::new(reader));
        self
    }

    /// Register a prompt. `render` gets the supplied arguments, after the
    /// server has checked that every `required` one is present.
    pub fn prompt<F>(mut self, prompt: Prompt, render: F) -> Self
    where
        F: Fn(&Context, &BTreeMap<String, String>) -> Result<GetPromptResult, ToolError>
            + Send
            + Sync
            + 'static,
    {
        self.prompts.retain(|(p, _)| p.name != prompt.name);
        self.prompts.push((prompt, Box::new(render)));
        self
    }

    /// Answer `completion/complete`.
    pub fn completer<F>(mut self, complete: F) -> Self
    where
        F: Fn(&Context, &CompleteParams) -> Completion + Send + Sync + 'static,
    {
        self.completer = Some(Box::new(complete));
        self
    }
}

impl Service for Server {
    fn info(&self) -> (Implementation, ServerCapabilities, Option<String>) {
        let offered = |any: bool| any.then(ListCapability::default);
        let capabilities = ServerCapabilities {
            tools: offered(!self.tools.is_empty()),
            resources: (!self.resources.is_empty()
                || !self.templates.is_empty()
                || self.fallback.is_some())
            .then(ResourcesCapability::default),
            prompts: offered(!self.prompts.is_empty()),
            logging: false,
            completions: self.completer.is_some(),
        };
        (self.info.clone(), capabilities, self.instructions.clone())
    }

    fn list_tools(&self, _params: &ListParams) -> Result<ListToolsResult, ErrorObject> {
        Ok(ListToolsResult {
            tools: self.tools.iter().map(|(t, _)| t.clone()).collect(),
            next_cursor: None,
        })
    }

    fn call_tool(
        &self,
        ctx: &Context,
        params: &CallToolParams,
    ) -> Result<CallToolResult, ErrorObject> {
        let Some((_, handler)) = self.tools.iter().find(|(t, _)| t.name == params.name) else {
            return Err(ErrorObject::new(
                code::INVALID_PARAMS,
                format!("Unknown tool: {}", params.name),
            ));
        };
        let null = Value::Null;
        let arguments = params.arguments.as_ref().unwrap_or(&null);
        Ok(handler(ctx, arguments)
            .unwrap_or_else(|ToolError(message)| CallToolResult::error(message)))
    }

    fn list_resources(&self, _params: &ListParams) -> Result<ListResourcesResult, ErrorObject> {
        Ok(ListResourcesResult {
            resources: self.resources.iter().map(|(r, _)| r.clone()).collect(),
            next_cursor: None,
        })
    }

    fn list_resource_templates(
        &self,
        _params: &ListParams,
    ) -> Result<ListResourceTemplatesResult, ErrorObject> {
        Ok(ListResourceTemplatesResult {
            resource_templates: self.templates.clone(),
            next_cursor: None,
        })
    }

    fn read_resource(
        &self,
        ctx: &Context,
        params: &ReadResourceParams,
    ) -> Result<ReadResourceResult, ErrorObject> {
        let read = self
            .resources
            .iter()
            .find(|(r, _)| r.uri == params.uri)
            .map(|(_, reader)| reader(ctx, &params.uri))
            .or_else(|| self.fallback.as_ref().and_then(|f| f(ctx, &params.uri)));
        match read {
            Some(Ok(contents)) => Ok(ReadResourceResult { contents }),
            Some(Err(ToolError(message))) => Err(ErrorObject::new(code::INTERNAL_ERROR, message)),
            None => Err(ErrorObject::new(
                code::RESOURCE_NOT_FOUND,
                format!("Resource not found: {}", params.uri),
            )),
        }
    }

    fn list_prompts(&self, _params: &ListParams) -> Result<ListPromptsResult, ErrorObject> {
        Ok(ListPromptsResult {
            prompts: self.prompts.iter().map(|(p, _)| p.clone()).collect(),
            next_cursor: None,
        })
    }

    fn get_prompt(
        &self,
        ctx: &Context,
        params: &GetPromptParams,
    ) -> Result<GetPromptResult, ErrorObject> {
        let Some((prompt, render)) = self.prompts.iter().find(|(p, _)| p.name == params.name)
        else {
            return Err(ErrorObject::new(
                code::INVALID_PARAMS,
                format!("Unknown prompt: {}", params.name),
            ));
        };
        if let Some(missing) = prompt
            .arguments
            .iter()
            .find(|a| a.required == Some(true) && !params.arguments.contains_key(&a.name))
        {
            return Err(ErrorObject::new(
                code::INVALID_PARAMS,
                format!("Missing required argument: {}", missing.name),
            ));
        }
        render(ctx, &params.arguments)
            .map_err(|ToolError(message)| ErrorObject::new(code::INTERNAL_ERROR, message))
    }

    fn complete(
        &self,
        ctx: &Context,
        params: &CompleteParams,
    ) -> Result<CompleteResult, ErrorObject> {
        match &self.completer {
            Some(complete) => Ok(CompleteResult {
                completion: complete(ctx, params),
            }),
            None => Err(ErrorObject::new(
                code::METHOD_NOT_FOUND,
                "Method not found: completion/complete",
            )),
        }
    }
}
