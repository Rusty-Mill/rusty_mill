//! Argument completions for the demo server's prompts and resources.
//!
//! Two shapes worth seeing: a fixed list, and one computed per request from the
//! same data the resource reader uses — so the suggestions cannot drift from
//! what a read will actually accept.

use rmcp::model::Reference;
use rusty_mcp::completion::{CompletionRegistry, CompletionRequest};

/// Build the demo completion registry.
pub fn registry() -> CompletionRegistry {
    CompletionRegistry::new()
        // Fixed candidates, known at startup.
        .with_values(
            Reference::for_prompt("explain-error"),
            "language",
            ["rust", "python", "typescript", "go", "java"],
        )
        // Computed per request, from the same table list `db://tables/{table}`
        // reads from. One source of truth means a suggestion is never a table
        // the read would reject.
        .with_completer(
            Reference::for_resource("db://tables/{table}"),
            "table",
            |_req: CompletionRequest| async move {
                Ok(crate::resources::TABLES
                    .iter()
                    .map(|(name, _)| (*name).to_string())
                    .collect())
            },
        )
}
