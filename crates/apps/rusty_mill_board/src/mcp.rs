//! The MCP surface: every [`crate::api`] operation as a tool.

use std::sync::Arc;

use rusty_mcp_server::json::Value as WireValue;
use rusty_mcp_server::proto::{CallToolResult, ContentBlock, ErrorCode, ErrorData, Tool};
use rusty_mcp_server::{BuildError, Server};

use crate::api::{CallError, Shared, ops};

fn wire(v: &serde_json::Value) -> Result<WireValue, ErrorData> {
    WireValue::from_json_str(&v.to_string())
        .map_err(|e| ErrorData::new(ErrorCode::INTERNAL_ERROR, e.to_string()))
}

/// Build the MCP server over `board`.
///
/// # Errors
/// If the tool set is invalid (a duplicate name).
pub fn server(board: Arc<Shared>) -> Result<Arc<Server>, BuildError> {
    let mut builder = Server::builder(env!("CARGO_PKG_NAME"), env!("CARGO_PKG_VERSION")).instructions(
        "Kanban board shared by humans and agents. Pass your `actor` name and `role` on every write. \
         Typical flow: card_claim, work, card_move to the next column (moving out of building/qa/review \
         requires your claim). Refusals come back as tool errors with the reason. Poll board_events to \
         follow changes.",
    );
    for op in ops() {
        let input = WireValue::from_json_str(&op.schema.to_string())
            .unwrap_or_else(|_| WireValue::object());
        let mut tool = Tool::new(op.name, input);
        tool.description = Some(op.description.to_owned());
        let board = Arc::clone(&board);
        builder = builder.tool(tool, move |_ctx, params| {
            let args = params.arguments.as_ref().map_or_else(
                || serde_json::json!({}),
                |a| serde_json::from_str(&a.to_json_string()).unwrap_or_default(),
            );
            match op.call(&board, args) {
                Ok(out) => Ok(CallToolResult {
                    content: vec![ContentBlock::text(out.to_string())],
                    structured_content: Some(wire(&out)?),
                    ..CallToolResult::default()
                }),
                Err(CallError::BadArgs(why)) => Err(ErrorData::new(
                    ErrorCode::INVALID_PARAMS,
                    format!("invalid arguments: {why}"),
                )),
                Err(CallError::Board(e)) => Ok(CallToolResult {
                    content: vec![ContentBlock::text(e.to_string())],
                    is_error: Some(true),
                    ..CallToolResult::default()
                }),
            }
        });
    }
    Ok(Arc::new(builder.build()?))
}
