//! The IPC contract (PRD 06 §canonical `rk://` table; PRD 08 Tauri bridge).
//!
//! This module is the **single source of truth** for the names that cross the
//! IPC boundary — the `rk://` events the harness emits, the Tauri `invoke`
//! commands the frontend calls, and the boundary error taxonomy every adapter
//! renders. The gateway SSE channel (Phase 14), the ACP `session/update`
//! notifications (Phase 16), and the (future) Tauri desktop bridge (Phase 15)
//! all reference these names rather than re-deriving their own, so the
//! event-contract drift the round-3 audit flagged cannot recur.
//!
//! Payload *shapes* live with their owning types (`observe::ToolEvent`,
//! `observe::EntropyAudit`, `compose::VerificationReport`, …); this module pins
//! the **names** + the cross-boundary [`TurnResult`] and [`BoundaryError`].

use serde::Serialize;

/// The canonical `rk://` event names (scheme stripped). The nine-event set is
/// reconciled in PRD 06 §"Tauri event bridge". `uri()` prepends `rk://`.
pub mod event {
    /// Turn begins (kernel about to run); the UI locks the composer.
    pub const TURN_START: &str = "turn_start";
    /// One token chunk during streaming. Payload: `string`.
    pub const TOKEN: &str = "token";
    /// A tool call fired. Payload: `observe::ToolEvent` (redacted).
    pub const TOOL_EVENT: &str = "tool_event";
    /// Post-turn work complete. Payload: [`super::TurnResult`].
    pub const TURN_COMPLETE: &str = "turn_complete";
    /// The approval gate triggered. Payload: an approval request.
    pub const APPROVAL_REQUEST: &str = "approval_request";
    /// The agent called `exit_plan_mode`. Payload: the proposed plan `string`.
    pub const PLAN_EXIT: &str = "plan_exit";
    /// Post-turn entropy audit complete. Payload: `observe::EntropyAudit`.
    pub const ENTROPY: &str = "entropy";
    /// A bash stdout/stderr chunk. Payload: `string`.
    pub const BASH_OUTPUT: &str = "bash_output";
    /// Idle consolidation complete. Payload: `feed::ConsolidationStats`.
    pub const CONSOLIDATION: &str = "consolidation";

    /// Every canonical event name, in catalog order.
    pub const ALL: [&str; 9] = [
        TURN_START,
        TOKEN,
        TOOL_EVENT,
        TURN_COMPLETE,
        APPROVAL_REQUEST,
        PLAN_EXIT,
        ENTROPY,
        BASH_OUTPUT,
        CONSOLIDATION,
    ];

    /// The full `rk://<name>` URI for a canonical event name.
    pub fn uri(name: &str) -> String {
        format!("rk://{name}")
    }
}

/// The Tauri `invoke` command names the desktop frontend calls (PRD 08). Each
/// maps to a `Session` method or app-level operation; the desktop bridge (Phase
/// 15) registers a `#[tauri::command]` per name.
pub mod command {
    /// `session.send(message)` → [`super::TurnResult`].
    pub const SESSION_SEND: &str = "session_send";
    /// Run a slash command (`/compact`, `/reflect`, …).
    pub const SESSION_COMMAND: &str = "session_command";
    /// The last `VerificationReport`.
    pub const SESSION_LAST_REPORT: &str = "session_last_report";
    /// The `MhirReport`.
    pub const SESSION_MHIR: &str = "session_mhir";
    /// The active `Config`.
    pub const SESSION_CONFIG: &str = "session_config";
    /// Override a config key for this session.
    pub const CONFIG_SET: &str = "config_set";
    /// Long-term memory snapshot.
    pub const SESSION_MEMORY_SNAPSHOT: &str = "session_memory_snapshot";
    /// Recent evidence-journal entries.
    pub const SESSION_EVIDENCE_RECENT: &str = "session_evidence_recent";
    /// Entropy-audit history.
    pub const SESSION_ENTROPY_HISTORY: &str = "session_entropy_history";
    /// The token budget snapshot.
    pub const SESSION_TOKEN_BUDGET: &str = "session_token_budget";
    /// Respond to an approval request (`{ approved, always }`).
    pub const APPROVAL_RESPOND: &str = "approval_respond";
    /// Store a provider key in the OS keychain.
    pub const SECRETS_SET: &str = "secrets_set";
    /// Retrieve a provider key from the OS keychain.
    pub const SECRETS_GET: &str = "secrets_get";
    /// Delete a provider key from the OS keychain.
    pub const SECRETS_DELETE: &str = "secrets_delete";
    /// List connected MCP servers.
    pub const MCP_SERVERS_LIST: &str = "mcp_servers_list";
    /// Add an MCP server.
    pub const MCP_SERVER_ADD: &str = "mcp_server_add";
    /// Remove an MCP server.
    pub const MCP_SERVER_REMOVE: &str = "mcp_server_remove";
    /// Test-connect an MCP server.
    pub const MCP_SERVER_TEST: &str = "mcp_server_test";
    /// Workspace file list (for the `@file` picker).
    pub const FS_LIST_WORKSPACE: &str = "fs_list_workspace";
    /// Memory search (for the `#memory` picker).
    pub const SESSION_MEMORY_SEARCH: &str = "session_memory_search";
    /// Slash-command list (for the `/command` palette).
    pub const SESSION_COMMANDS_LIST: &str = "session_commands_list";
    /// Working-tree status (branch, ahead/behind, changed files) for the Git tab.
    pub const GIT_STATUS: &str = "git_status";
    /// Unified diff for a path / staged scope (the Git tab's CM6 view).
    pub const GIT_DIFF: &str = "git_diff";

    /// Every command name, in catalog order.
    pub const ALL: [&str; 23] = [
        SESSION_SEND,
        SESSION_COMMAND,
        SESSION_LAST_REPORT,
        SESSION_MHIR,
        SESSION_CONFIG,
        CONFIG_SET,
        SESSION_MEMORY_SNAPSHOT,
        SESSION_EVIDENCE_RECENT,
        SESSION_ENTROPY_HISTORY,
        SESSION_TOKEN_BUDGET,
        APPROVAL_RESPOND,
        SECRETS_SET,
        SECRETS_GET,
        SECRETS_DELETE,
        MCP_SERVERS_LIST,
        MCP_SERVER_ADD,
        MCP_SERVER_REMOVE,
        MCP_SERVER_TEST,
        FS_LIST_WORKSPACE,
        SESSION_MEMORY_SEARCH,
        SESSION_COMMANDS_LIST,
        GIT_STATUS,
        GIT_DIFF,
    ];
}

/// The `turn_complete` / `POST /chat` / `session_send` payload — the boundary
/// projection of a [`crate::TurnOutcome`] (the full `VerificationReport` is
/// available via the `session_last_report` command / `/verify`).
#[derive(Debug, Clone, Serialize)]
pub struct TurnResult {
    /// The model's final reply.
    pub reply: String,
    /// Whether the turn verified.
    pub verified: bool,
    /// What was not verified (the report's `limits`).
    pub limits: String,
}

impl TurnResult {
    /// Project a turn outcome into the boundary result.
    pub fn from_outcome(outcome: &crate::TurnOutcome) -> Self {
        Self {
            reply: outcome.reply.clone(),
            verified: outcome.report.verified,
            limits: outcome.report.limits.to_string(),
        }
    }
}

/// The boundary error taxonomy (PRD 06): every adapter — CLI, gateway, ACP,
/// Tauri — collapses a typed internal error into one of these six surface kinds
/// so the boundary speaks one closed vocabulary. Rendered as the `invoke`
/// rejection payload, the SSE `error` frame, and the ACP error body.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize)]
#[serde(rename_all = "snake_case")]
pub enum BoundaryError {
    /// Provider returned a non-retryable error (e.g. 4xx).
    ProviderError,
    /// Per-call / tool timeout after retries exhausted.
    Timeout,
    /// Provider rate limit (`Retry-After` already honored internally).
    RateLimited,
    /// Caller or provider credential rejected.
    AuthError,
    /// A policy decision that ends the turn (not a recoverable tool block).
    PolicyBlock,
    /// Bug / unexpected state; the turn is recorded as aborted.
    Internal,
}

impl BoundaryError {
    /// Wire name (snake_case).
    pub fn as_str(self) -> &'static str {
        match self {
            BoundaryError::ProviderError => "provider_error",
            BoundaryError::Timeout => "timeout",
            BoundaryError::RateLimited => "rate_limited",
            BoundaryError::AuthError => "auth_error",
            BoundaryError::PolicyBlock => "policy_block",
            BoundaryError::Internal => "internal",
        }
    }

    /// Every boundary kind.
    pub const ALL: [BoundaryError; 6] = [
        BoundaryError::ProviderError,
        BoundaryError::Timeout,
        BoundaryError::RateLimited,
        BoundaryError::AuthError,
        BoundaryError::PolicyBlock,
        BoundaryError::Internal,
    ];
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn event_catalog_is_the_canonical_nine() {
        assert_eq!(event::ALL.len(), 9);
        // No duplicates, no scheme prefix in the names.
        for name in event::ALL {
            assert!(!name.contains("rk://"), "{name} must be scheme-stripped");
            assert_eq!(event::ALL.iter().filter(|n| **n == name).count(), 1);
        }
        assert_eq!(event::uri(event::TURN_START), "rk://turn_start");
    }

    #[test]
    fn command_catalog_has_no_duplicates() {
        for name in command::ALL {
            assert_eq!(command::ALL.iter().filter(|n| **n == name).count(), 1);
        }
        assert_eq!(command::ALL.len(), 23);
    }

    #[test]
    fn boundary_errors_serialize_snake_case() {
        assert_eq!(
            serde_json::to_string(&BoundaryError::PolicyBlock).unwrap(),
            "\"policy_block\""
        );
        assert_eq!(BoundaryError::ALL.len(), 6);
        for e in BoundaryError::ALL {
            assert_eq!(
                serde_json::to_string(&e).unwrap(),
                format!("\"{}\"", e.as_str())
            );
        }
    }
}
