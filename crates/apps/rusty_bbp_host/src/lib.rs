//! Host for the Blackboard Protocol over a `rusty_bbp` file store.
//!
//! - [`mcp`]: the per-turn MCP server an agent harness talks to over stdio.
//!   One process serves one turn; when that turn ends every call is refused.
//! - [`human`]: the human channel as CLI actions.
//! - [`runner`]: the test supervisor. Applies a candidate in a fresh checkout,
//!   runs the frozen profile set under a sandbox, stores log and report.
//! - [`profiles`]: the profile set a task freezes at open.
//! - [`admin`]: open a task and assign roles.
//! - [`args`]: the small argument parser the `bbp` binary uses.

#![forbid(unsafe_code)]

pub mod admin;
pub mod args;
pub mod human;
pub mod mcp;
pub mod profiles;
pub mod runner;

use rusty_bbp::*;
use std::path::Path;

/// Wall-clock milliseconds since the Unix epoch, as the core's `Time`.
pub fn now() -> Time {
    let ms = std::time::SystemTime::now()
        .duration_since(std::time::UNIX_EPOCH)
        .map(|d| d.as_millis() as u64)
        .unwrap_or(0);
    Time(ms)
}

/// Open the file store at `dir` and rebuild `task`'s state.
pub fn open_driver(dir: &Path, task: &TaskId) -> Result<Driver<FsStore>, String> {
    let store = FsStore::open(dir).map_err(|e| e.to_string())?;
    let mut d = Driver::new(store, task.clone());
    d.reload().map_err(|e| format!("{e:?}"))?;
    Ok(d)
}

/// A fresh operation id for a call the agent did not name.
pub fn fresh_op() -> OpId {
    let bytes = rusty_rand::bytes(8).unwrap_or_else(|_| now().0.to_le_bytes().to_vec());
    OpId(format!(
        "auto-{}",
        bytes.iter().map(|b| format!("{b:02x}")).collect::<String>()
    ))
}

/// Render a response as JSON text.
pub fn response_json(r: &Response) -> String {
    rusty_serde::json::to_string(r).unwrap_or_else(|e| format!("{{\"error\":\"{e}\"}}"))
}
