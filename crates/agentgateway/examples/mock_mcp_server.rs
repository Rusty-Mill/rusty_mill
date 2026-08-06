//! A minimal stdio MCP server, used as a fixture by the federation tests.
//!
//! It exports two tools and echoes which server handled a call, so a test can
//! prove a request reached the *right* target rather than merely reaching one.
//! `MOCK_LABEL` names the instance; `MOCK_TOOLS` overrides the tool list, which
//! is how a test sets up a name collision between two targets.

use std::sync::Arc;

use rmcp::{
    ErrorData as McpError, ServerHandler, ServiceExt,
    model::{
        CallToolRequestParams, CallToolResponse, CallToolResult, ContentBlock, Implementation,
        ListToolsResult, PaginatedRequestParams, ServerCapabilities, ServerInfo, Tool,
    },
    service::{RequestContext, RoleServer},
};

#[derive(Clone)]
struct MockServer {
    label: String,
    tools: Vec<String>,
}

impl ServerHandler for MockServer {
    fn get_info(&self) -> ServerInfo {
        ServerInfo::new(ServerCapabilities::builder().enable_tools().build())
            .with_server_info(Implementation::new("mock-mcp-server", "0.1.0"))
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

    let service = MockServer { label, tools }
        .serve(rmcp::transport::io::stdio())
        .await?;
    service.waiting().await?;
    Ok(())
}
