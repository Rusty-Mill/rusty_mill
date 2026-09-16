//! A destructive-looking tool that asks before acting, via MRTR.
//!
//! The pattern to copy: the first call returns an input request instead of
//! doing anything, and the client retries with the user's answer. Because the
//! protocol is stateless, everything needed on the way back travels in a sealed
//! `requestState` rather than in server memory.

use rmcp::{
    handler::server::{
        tool::{InputResponses, RequestState},
        wrapper::Parameters,
    },
    model::{CallToolResponse, CallToolResult, ContentBlock, ErrorData},
    tool, tool_router,
};
use rusty_mcp::mrtr::{InputGate, Turn};
use schemars::JsonSchema;
use serde::{Deserialize, Serialize};

/// The tool name, shared by the registration and the state binding.
///
/// These must agree: the name is authenticated into the sealed state, so a
/// mismatch shows up as a rejected retry rather than a compile error.
pub const DROP_TABLE: &str = "drop_table";

/// Which table to drop.
#[derive(Debug, Deserialize, JsonSchema)]
pub struct DropTableArgs {
    /// Name of the table.
    pub table: String,
}

/// What has to survive the trip through the client.
///
/// Deliberately small. Anything large or sensitive belongs server-side behind
/// an opaque handle; this crosses the wire, sealed but not encrypted.
#[derive(Debug, Serialize, Deserialize)]
pub struct PendingDrop {
    /// The table the user was asked about.
    pub table: String,
}

#[tool_router(router = confirm_tools, vis = "pub(crate)")]
impl crate::server::DemoServer {
    /// Drop a table, after confirming with the user.
    #[tool(
        name = "drop_table",
        description = "Drop a demo table. Asks the user to confirm before acting."
    )]
    pub async fn drop_table(
        &self,
        Parameters(DropTableArgs { table }): Parameters<DropTableArgs>,
        // The retry arrives as an ordinary call carrying these two extractors.
        RequestState(request_state): RequestState,
        InputResponses(responses): InputResponses,
    ) -> Result<CallToolResponse, ErrorData> {
        match self
            .confirmations
            .turn(DROP_TABLE, request_state.as_deref(), responses.as_ref())?
        {
            Turn::Fresh => {
                let requests = InputGate::<PendingDrop>::confirm(
                    "confirm-drop",
                    format!("Really drop the `{table}` table? This cannot be undone."),
                );

                Ok(CallToolResponse::InputRequired(self.confirmations.ask(
                    DROP_TABLE,
                    &PendingDrop { table },
                    requests,
                )?))
            }

            Turn::Resumed { state, answers } => {
                // The table name comes from the *sealed* state, not from the
                // arguments on the retry. A client that changed `table` between
                // rounds would otherwise get a confirmation for one table and a
                // drop of another.
                if answers.accepted("confirm-drop") {
                    Ok(CallToolResult::success(vec![ContentBlock::text(format!(
                        "dropped `{}`",
                        state.table
                    ))])
                    .into())
                } else {
                    Ok(CallToolResult::success(vec![ContentBlock::text(format!(
                        "left `{}` alone",
                        state.table
                    ))])
                    .into())
                }
            }
        }
    }
}
