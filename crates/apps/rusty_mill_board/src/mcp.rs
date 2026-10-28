//! The MCP surface: every [`crate::api`] operation as a tool.

use std::sync::Arc;

use rusty_mcp_server::json::Value as WireValue;
use rusty_mcp_server::proto::{CallToolResult, ContentBlock, ErrorCode, ErrorData, Tool};
use rusty_mcp_server::{BuildError, Server};

use crate::api::{CallError, Identity, Shared, ops};
use crate::drain::InFlight;

fn wire(v: &serde_json::Value) -> Result<WireValue, ErrorData> {
    WireValue::from_json_str(&v.to_string())
        .map_err(|e| ErrorData::new(ErrorCode::INTERNAL_ERROR, e.to_string()))
}

/// Build the MCP server over `board`.
///
/// # Errors
/// If the tool set is invalid (a duplicate name).
pub fn server(board: Arc<Shared>) -> Result<Arc<Server>, BuildError> {
    server_with(board, Arc::default())
}

/// [`server`], counting every tool call in `flight` for as long as it runs. The HTTP layer runs a
/// call on a worker of its own and may answer with a stream while it works, so the request
/// counting alone does not see a call whose client has gone; this does. Once `flight` is closed a
/// call is refused.
///
/// # Errors
/// If the tool set is invalid (a duplicate name).
pub fn server_with(board: Arc<Shared>, flight: Arc<InFlight>) -> Result<Arc<Server>, BuildError> {
    let mut builder = Server::builder(env!("CARGO_PKG_NAME"), env!("CARGO_PKG_VERSION")).instructions(
        "Kanban board shared by humans and models. Start with board_whoami. If it says you are \
         authenticated, your actor and role are fixed: omit them. Otherwise pass `actor` and `role` on \
         every write. Working loop: card_take (your next card, claimed for you; null means nothing is \
         waiting), do the work, card_comment what you did and anything the next person needs, then \
         card_move it to the next column (leaving building, qa or review needs your claim). Humans move \
         cards into Ready and out of Review. A card in building or qa whose PR already merged on the \
         forge may be moved straight to done by a role holding `close`: comment with the merged PR first. \
         Merged work with no card: card_add with done: true. A missing project: project_add. If a move is refused, the reason says which capability is \
         missing, so do not try to work around it: comment and leave the card. Poll board_events \
         (after = the last event id you saw) to follow changes.",
    );
    for op in ops() {
        let input = WireValue::from_json_str(&op.schema.to_string())
            .unwrap_or_else(|_| WireValue::object());
        let mut tool = Tool::new(op.name, input);
        tool.description = Some(op.description.to_owned());
        let (board, flight) = (Arc::clone(&board), Arc::clone(&flight));
        builder = builder.tool(tool, move |ctx, params| {
            let Some(_running) = flight.enter() else {
                return Ok(CallToolResult {
                    content: vec![ContentBlock::text("the board is shutting down")],
                    is_error: Some(true),
                    ..CallToolResult::default()
                });
            };
            let identity: Option<Identity> = ctx
                .caller()
                .principal
                .as_ref()
                .and_then(|p| serde_json::from_str(&p.to_json_string()).ok());
            let args = params.arguments.as_ref().map_or_else(
                || serde_json::json!({}),
                |a| serde_json::from_str(&a.to_json_string()).unwrap_or_default(),
            );
            match op.call(&board, identity.as_ref(), args) {
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
