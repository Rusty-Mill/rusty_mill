//! A minimal stdio MCP server, used as a fixture by the federation tests.
//!
//! It exports two tools and echoes which server handled a call, so a test can
//! prove a request reached the *right* target rather than merely reaching one.
//! `MOCK_LABEL` names the instance; `MOCK_TOOLS` overrides the tool list, which
//! is how a test sets up a name collision between two targets. `MOCK_DELAY_MS`
//! makes tool calls slow, so the timeout and concurrency tests have something
//! real to be slow about. The delay is on `call_tool` only, leaving startup
//! and `tools/list` prompt.
//!
//! `MOCK_PROMPTS` and `MOCK_RESOURCES` add prompts and resources. Both default
//! to empty, and a server with neither advertises neither capability — which
//! is what lets a test check that the gateway only claims what its targets can
//! actually do.

use std::sync::Arc;

use rmcp::{
    ErrorData as McpError, ServerHandler, ServiceExt,
    model::{
        CallToolRequestParams, CallToolResponse, CallToolResult, ContentBlock,
        GetPromptRequestParams, GetPromptResponse, GetPromptResult, Implementation,
        ListPromptsResult, ListResourceTemplatesResult, ListResourcesResult, ListToolsResult,
        PaginatedRequestParams, Prompt, PromptMessage, ReadResourceRequestParams,
        ReadResourceResponse, ReadResourceResult, Resource, ResourceContents, ResourceTemplate,
        Role, ServerCapabilities, ServerInfo, Tool,
    },
    service::{RequestContext, RoleServer},
};

#[derive(Clone)]
struct MockServer {
    label: String,
    tools: Vec<String>,
    prompts: Vec<String>,
    /// Resource URIs, as this server publishes them.
    resources: Vec<String>,
    delay: std::time::Duration,
}

impl ServerHandler for MockServer {
    fn get_info(&self) -> ServerInfo {
        // Only what this instance actually has, so a test can check the
        // gateway's own advertisement follows its targets.
        let mut capabilities = ServerCapabilities::default();
        capabilities.tools = Some(Default::default());
        capabilities.prompts = (!self.prompts.is_empty()).then(Default::default);
        capabilities.resources = (!self.resources.is_empty()).then(Default::default);

        ServerInfo::new(capabilities)
            .with_server_info(Implementation::new("mock-mcp-server", "0.1.0"))
    }

    async fn list_prompts(
        &self,
        _request: Option<PaginatedRequestParams>,
        _context: RequestContext<RoleServer>,
    ) -> Result<ListPromptsResult, McpError> {
        Ok(ListPromptsResult {
            prompts: self
                .prompts
                .iter()
                .map(|name| Prompt::new(name, Some(&format!("{name} on {}", self.label)), None))
                .collect(),
            ..Default::default()
        })
    }

    async fn get_prompt(
        &self,
        request: GetPromptRequestParams,
        _context: RequestContext<RoleServer>,
    ) -> Result<GetPromptResponse, McpError> {
        if !self.prompts.iter().any(|p| p.as_str() == request.name) {
            return Err(McpError::invalid_params(
                format!("unknown prompt `{}`", request.name),
                None,
            ));
        }
        Ok(GetPromptResponse::Complete(GetPromptResult::new(vec![
            PromptMessage::new_text(Role::User, format!("{}:{}", self.label, request.name)),
        ])))
    }

    async fn list_resources(
        &self,
        _request: Option<PaginatedRequestParams>,
        _context: RequestContext<RoleServer>,
    ) -> Result<ListResourcesResult, McpError> {
        Ok(ListResourcesResult {
            resources: self
                .resources
                .iter()
                .map(|uri| Resource::new(uri, uri.clone()))
                .collect(),
            ..Default::default()
        })
    }

    async fn list_resource_templates(
        &self,
        _request: Option<PaginatedRequestParams>,
        _context: RequestContext<RoleServer>,
    ) -> Result<ListResourceTemplatesResult, McpError> {
        // One fixed template, so a test has something with a `uriTemplate` to
        // gate without needing another environment variable.
        Ok(ListResourceTemplatesResult {
            resource_templates: if self.resources.is_empty() {
                Vec::new()
            } else {
                vec![ResourceTemplate::new(
                    format!("{}:{{id}}", self.label),
                    "by-id",
                )]
            },
            ..Default::default()
        })
    }

    async fn read_resource(
        &self,
        request: ReadResourceRequestParams,
        _context: RequestContext<RoleServer>,
    ) -> Result<ReadResourceResponse, McpError> {
        if !self.resources.iter().any(|r| r.as_str() == request.uri) {
            return Err(McpError::invalid_params(
                format!("unknown resource `{}`", request.uri),
                None,
            ));
        }
        Ok(ReadResourceResponse::Complete(ReadResourceResult::new(
            vec![ResourceContents::TextResourceContents {
                // The server's own URI, never the federated one -- which is
                // what makes the gateway's re-qualification observable.
                uri: request.uri.clone(),
                mime_type: Some("text/plain".into()),
                text: format!("{}:{}", self.label, request.uri),
                meta: None,
            }],
        )))
    }

    async fn list_tools(
        &self,
        _request: Option<PaginatedRequestParams>,
        _context: RequestContext<RoleServer>,
    ) -> Result<ListToolsResult, McpError> {
        let mut schema = serde_json::Map::new();
        schema.insert("type".into(), serde_json::Value::String("object".into()));
        schema.insert(
            "properties".into(),
            serde_json::Value::Object(serde_json::Map::new()),
        );
        let schema = Arc::new(schema);

        Ok(ListToolsResult {
            tools: self
                .tools
                .iter()
                .map(|name| {
                    Tool::new(
                        name.clone(),
                        format!("{name} on {}", self.label),
                        Arc::clone(&schema),
                    )
                })
                .collect(),
            ..Default::default()
        })
    }

    async fn call_tool(
        &self,
        request: CallToolRequestParams,
        _context: RequestContext<RoleServer>,
    ) -> Result<CallToolResponse, McpError> {
        if !self.tools.iter().any(|t| t.as_str() == request.name) {
            return Err(McpError::invalid_params(
                format!("unknown tool `{}`", request.name),
                None,
            ));
        }
        if !self.delay.is_zero() {
            tokio::time::sleep(self.delay).await;
        }
        // The label is what lets a test tell which target served the call.
        let text = format!("{}:{}", self.label, request.name);
        Ok(CallToolResponse::Complete(CallToolResult::success(vec![
            ContentBlock::text(text),
        ])))
    }
}

#[tokio::main]
async fn main() -> Result<(), Box<dyn std::error::Error>> {
    let label = std::env::var("MOCK_LABEL").unwrap_or_else(|_| "mock".into());
    let tools = std::env::var("MOCK_TOOLS")
        .unwrap_or_else(|_| "echo,ping".into())
        .split(',')
        .filter(|t| !t.is_empty())
        .map(|t| t.to_string())
        .collect();

    let list = |name: &str| {
        std::env::var(name)
            .unwrap_or_default()
            .split(',')
            .filter(|item| !item.is_empty())
            .map(|item| item.to_string())
            .collect::<Vec<_>>()
    };
    let prompts = list("MOCK_PROMPTS");
    let resources = list("MOCK_RESOURCES");

    let delay = std::time::Duration::from_millis(
        std::env::var("MOCK_DELAY_MS")
            .ok()
            .and_then(|raw| raw.parse().ok())
            .unwrap_or(0),
    );

    let service = MockServer {
        label,
        tools,
        prompts,
        resources,
        delay,
    }
    .serve(rmcp::transport::io::stdio())
    .await?;
    service.waiting().await?;
    Ok(())
}
