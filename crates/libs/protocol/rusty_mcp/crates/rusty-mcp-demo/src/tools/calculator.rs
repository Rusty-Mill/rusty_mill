//! Arithmetic tools.
//!
//! Shows the two result shapes worth knowing: [`rmcp::Json`] for structured
//! output that clients can consume as data, and [`ToolError`] for a call that
//! could not be processed at all.

use std::sync::atomic::Ordering;

use rmcp::{Json, handler::server::wrapper::Parameters, model::ErrorData, tool, tool_router};
use rusty_mcp::ToolError;
use schemars::JsonSchema;
use serde::{Deserialize, Serialize};

use crate::server::DemoServer;

/// Two integer operands.
#[derive(Debug, Deserialize, JsonSchema)]
pub struct BinaryOp {
    /// Left operand.
    pub a: i64,
    /// Right operand.
    pub b: i64,
}

/// Result of an addition.
#[derive(Debug, Serialize, JsonSchema)]
pub struct SumResult {
    /// `a + b`.
    pub sum: i64,
    /// Number of tool calls this server has served, including this one.
    pub calls: u64,
}

/// Result of an integer division.
#[derive(Debug, Serialize, JsonSchema)]
pub struct DivideResult {
    /// Truncated quotient.
    pub quotient: i64,
    /// Remainder, with the sign of the dividend.
    pub remainder: i64,
}

#[tool_router(router = calculator_tools, vis = "pub(crate)")]
impl DemoServer {
    /// Add two integers, returning structured output.
    #[tool(description = "Add two integers and return the sum.")]
    pub async fn add(
        &self,
        Parameters(BinaryOp { a, b }): Parameters<BinaryOp>,
    ) -> Result<Json<SumResult>, ErrorData> {
        let sum = a
            .checked_add(b)
            .ok_or_else(|| ToolError::invalid(format!("{a} + {b} overflows a 64-bit integer")))?;

        let calls = self.state.calls.fetch_add(1, Ordering::Relaxed) + 1;
        Ok(Json(SumResult { sum, calls }))
    }

    /// Divide two integers.
    #[tool(description = "Divide two integers, returning quotient and remainder.")]
    pub async fn divide(
        &self,
        Parameters(BinaryOp { a, b }): Parameters<BinaryOp>,
    ) -> Result<Json<DivideResult>, ErrorData> {
        if b == 0 {
            return Err(ToolError::invalid("cannot divide by zero").into());
        }

        // The one overflowing case for two's complement division.
        let quotient = a
            .checked_div(b)
            .ok_or_else(|| ToolError::invalid(format!("{a} / {b} overflows a 64-bit integer")))?;

        self.state.calls.fetch_add(1, Ordering::Relaxed);
        Ok(Json(DivideResult {
            quotient,
            remainder: a % b,
        }))
    }
}
