// The IPC contract, mirrored on the JS side. These names are the single source
// of truth in Rust (`rk_app::contract`); this file restates them so the
// frontend's `invoke`/`listen` calls cannot typo a name. The Rust IPC smoke test
// asserts the Rust catalogs match the contract; keep this list in lockstep.

/// Canonical `rk://` event names (scheme stripped). Listen via `rk://<name>`.
export const EVENT = {
  TURN_START: "turn_start",
  TOKEN: "token",
  TOOL_EVENT: "tool_event",
  TURN_COMPLETE: "turn_complete",
  APPROVAL_REQUEST: "approval_request",
  PLAN_EXIT: "plan_exit",
  ENTROPY: "entropy",
  BASH_OUTPUT: "bash_output",
  CONSOLIDATION: "consolidation",
} as const;

export type EventName = (typeof EVENT)[keyof typeof EVENT];

/// The full `rk://<name>` URI for a canonical event.
export const eventUri = (name: EventName): string => `rk://${name}`;

/// Tauri `invoke` command names.
export const COMMAND = {
  SESSION_SEND: "session_send",
  SESSION_COMMAND: "session_command",
  SESSION_LAST_REPORT: "session_last_report",
  SESSION_MHIR: "session_mhir",
  SESSION_CONFIG: "session_config",
  CONFIG_SET: "config_set",
  SESSION_MEMORY_SNAPSHOT: "session_memory_snapshot",
  SESSION_EVIDENCE_RECENT: "session_evidence_recent",
  SESSION_ENTROPY_HISTORY: "session_entropy_history",
  SESSION_TOKEN_BUDGET: "session_token_budget",
  APPROVAL_RESPOND: "approval_respond",
  SECRETS_SET: "secrets_set",
  SECRETS_GET: "secrets_get",
  SECRETS_DELETE: "secrets_delete",
  MCP_SERVERS_LIST: "mcp_servers_list",
  MCP_SERVER_ADD: "mcp_server_add",
  MCP_SERVER_REMOVE: "mcp_server_remove",
  MCP_SERVER_TEST: "mcp_server_test",
  FS_LIST_WORKSPACE: "fs_list_workspace",
  SESSION_MEMORY_SEARCH: "session_memory_search",
  SESSION_COMMANDS_LIST: "session_commands_list",
  GIT_STATUS: "git_status",
  GIT_DIFF: "git_diff",
} as const;

export type CommandName = (typeof COMMAND)[keyof typeof COMMAND];

/// The six boundary-error surface kinds (`invoke` rejection `kind`).
export type BoundaryErrorKind =
  | "provider_error"
  | "timeout"
  | "rate_limited"
  | "auth_error"
  | "policy_block"
  | "internal";

export interface BoundaryError {
  kind: BoundaryErrorKind;
  message: string;
}

// ---- Payload shapes (owned by the producing Rust types; partial mirrors) ----

export interface TurnResult {
  reply: string;
  verified: boolean;
  limits: string;
}

export interface TurnCompleteEvent extends TurnResult {
  turn_id: string;
}

export interface ToolOutcome {
  status: string;
  [k: string]: unknown;
}

export interface ToolEvent {
  name: string;
  args: unknown;
  outcome: ToolOutcome;
}

export interface ApprovalRequest {
  tool: string;
  args: unknown;
  trigger: string;
}

export interface EntropyAudit {
  turn_id?: string;
  delta?: number;
  findings?: EntropyFinding[];
  [k: string]: unknown;
}

export interface EntropyFinding {
  severity: number;
  category: string;
  description: string;
}

export interface TokenBudget {
  used: number;
  limit: number;
  fraction: number;
  session_total: number;
  compactions: number;
}

export interface MhirReport {
  rate: number;
  n_interventions: number;
  n_turns: number;
  n_unavoidable: number;
  n_benign: number;
  breakdown?: Record<string, number>;
  /// Per-session M-HIR rate, oldest→newest (the current session is last).
  trend?: number[];
}

export interface SessionConfig {
  permission_mode: string;
  isolation: string;
  explore_enabled: boolean;
}

export interface MemoryEntry {
  title: string;
  body: string;
  mem_type?: string;
  validated?: boolean;
  importance?: number;
  [k: string]: unknown;
}

export interface ConsolidationStats {
  created: number;
  updated: number;
  pruned: number;
  groomed: number;
}

export interface GitFile {
  path: string;
  x: string;
  y: string;
  staged: boolean;
  unstaged: boolean;
  untracked: boolean;
}

export interface GitStatus {
  repo: boolean;
  branch?: string;
  ahead?: number;
  behind?: number;
  files?: GitFile[];
}
