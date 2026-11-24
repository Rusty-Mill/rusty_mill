//! The MCP surface: every [`crate::api`] operation as a tool.

use std::sync::Arc;

use rusty_mcp_server::json::Value as WireValue;
use rusty_mcp_server::proto::CallToolParams;
use rusty_mcp_server::proto::{CallToolResult, ContentBlock, ErrorCode, ErrorData, Tool};
use rusty_mcp_server::{BuildError, CallContext, Server, ServerBuilder};

use crate::api::{CallError, Identity, Shared, ops};
use crate::drain::InFlight;
use crate::settings::{Settings, tools};

/// Build the MCP server over `board`.
///
/// # Errors
/// If the tool set is invalid (a duplicate name).
pub fn server(board: Arc<Shared>) -> Result<Arc<Server>, BuildError> {
    server_with(board, Arc::default(), None)
}

/// [`server`], counting every tool call in `flight` for as long as it runs. The HTTP layer runs a
/// call on a worker of its own and may answer with a stream while it works, so the request
/// counting alone does not see a call whose client has gone; this does. Once `flight` is closed a
/// call is refused. With `settings`, the settings operations an agent may call are tools too
/// (see [`tools`]); each needs the `admin` capability.
///
/// # Errors
/// If the tool set is invalid (a duplicate name).
pub fn server_with(
    board: Arc<Shared>,
    flight: Arc<InFlight>,
    settings: Option<Arc<Settings>>,
) -> Result<Arc<Server>, BuildError> {
    let mut builder = Server::builder(env!("CARGO_PKG_NAME"), env!("CARGO_PKG_VERSION")).instructions(
        "Kanban board shared by humans and models. Start with board_whoami. If it says you are \
         authenticated, your actor and role are fixed: omit them. Otherwise pass `actor` and `role` on \
         every write. On a connector sign-in (claude.ai) call agent_register once first, then pass its \
         `agent_id` and `agent_key` on every write; reads need neither. You act with the connector's \
         role unless an admin gave you your own; you may ask for less (`agent_role`, `agent_projects`). \
         Working loop: card_take (your next card, claimed for you; null means nothing is \
         waiting), do the work, card_comment what you did and anything the next person needs, then \
         card_move it to the next column (leaving a work column needs your claim). board_columns lists \
         the columns, their kinds and the moves between them with the capability each needs, and says \
         whether movement rules are on or off (off: the transition list is not enforced, but role, \
         project scope, claims, gates and card state still apply); a move marked `gate` is a human \
         approval. Listing is not authorization: attempt card_move only on a card you are already \
         authorized to move, and if refused report the reason verbatim. Say whether a limit is the \
         board's or a written rule you were given. A card whose PR already merged on the forge may be moved \
         straight to done by a role holding `close`, where the workflow allows it: record the merged \
         PR with card_evidence first (a Done card counts as shipped only with evidence). \
         Merged work with no card: card_add with done: true and evidence: [the PR link]. A missing project: project_add. If a move is refused, the reason says which capability is \
         missing, so do not try to work around it: comment and leave the card. Poll board_events \
         (after = the last event id you saw) to follow changes.",
    );
    for op in ops() {
        let input = WireValue::from_json_str(&op.schema.to_string())
            .unwrap_or_else(|_| WireValue::object());
        let mut tool = Tool::new(op.name, input);
        tool.description = Some(op.description.to_owned());
        let board = Arc::clone(&board);
        let refusals = settings.clone();
        builder = add(builder, tool, &flight, move |i, a| {
            let out = op.call(&board, i, a.clone());
            if let (Err(e), Some(settings)) = (&out, &refusals) {
                settings.audit_refusal(i, op.name, &a, e);
            }
            out
        });
    }
    if let Some(settings) = &settings {
        for t in tools() {
            let input = WireValue::from_json_str(&t.schema.to_string())
                .unwrap_or_else(|_| WireValue::object());
            let mut tool = Tool::new(t.name, input);
            tool.description = Some(t.description.to_owned());
            let settings = Arc::clone(settings);
            builder = add(builder, tool, &flight, move |i, a| {
                settings
                    .call(t.name, i, a)
                    .unwrap_or_else(|| Err(CallError::BadArgs("no such operation".to_owned())))
            });
        }
    }
    Ok(Arc::new(builder.build()?))
}

/// Register `tool`, running `call` as the caller's identity while counted in `flight`.
fn add(
    builder: ServerBuilder,
    tool: Tool,
    flight: &Arc<InFlight>,
    call: impl Fn(Option<&Identity>, serde_json::Value) -> Result<serde_json::Value, CallError>
    + Send
    + Sync
    + 'static,
) -> ServerBuilder {
    let flight = Arc::clone(flight);
    builder.tool(tool, move |ctx: &CallContext, params: CallToolParams| {
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
        match call(identity.as_ref(), args) {
            // The JSON goes out once, as text. Tools declare no `outputSchema`, so
            // `structuredContent` would only repeat it and double every result.
            Ok(out) => Ok(CallToolResult {
                content: vec![ContentBlock::text(out.to_string())],
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
    })
}
