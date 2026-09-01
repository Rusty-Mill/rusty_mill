// Typed wrappers over Tauri `invoke`/`listen`, keyed off the contract names.
// Every harness call goes through here so the rest of the app never touches a raw
// command/event string.

import { invoke } from "@tauri-apps/api/core";
import { listen, type Event, type UnlistenFn } from "@tauri-apps/api/event";
import {
  COMMAND,
  EVENT,
  eventUri,
  type BoundaryError,
  type ConsolidationStats,
  type EntropyAudit,
  type EventName,
  type GitStatus,
  type MemoryEntry,
  type MhirReport,
  type SessionConfig,
  type TokenBudget,
  type TurnResult,
} from "./contract";

/// Normalize a Tauri `invoke` rejection into the boundary error taxonomy.
export function asBoundaryError(err: unknown): BoundaryError {
  if (err && typeof err === "object" && "kind" in err && "message" in err) {
    return err as BoundaryError;
  }
  return { kind: "internal", message: String(err) };
}

// ---- Commands ----

export const ipc = {
  sessionSend: (message: string, attachments?: string[]) =>
    invoke<TurnResult>(COMMAND.SESSION_SEND, { message, attachments }),
  sessionCommand: (command: string) =>
    invoke<void>(COMMAND.SESSION_COMMAND, { command }),
  lastReport: () => invoke<unknown | null>(COMMAND.SESSION_LAST_REPORT),
  mhir: () => invoke<MhirReport>(COMMAND.SESSION_MHIR),
  config: () => invoke<SessionConfig>(COMMAND.SESSION_CONFIG),
  configSet: (key: string, value: unknown) =>
    invoke<unknown>(COMMAND.CONFIG_SET, { key, value }),
  memorySnapshot: () =>
    invoke<{ recent: MemoryEntry[] }>(COMMAND.SESSION_MEMORY_SNAPSHOT),
  evidenceRecent: (n: number) =>
    invoke<unknown[]>(COMMAND.SESSION_EVIDENCE_RECENT, { n }),
  entropyHistory: () =>
    invoke<{ recent: EntropyAudit[]; cumulative_delta: number }>(
      COMMAND.SESSION_ENTROPY_HISTORY,
    ),
  tokenBudget: () => invoke<TokenBudget>(COMMAND.SESSION_TOKEN_BUDGET),
  approvalRespond: (approved: boolean, always: boolean) =>
    invoke<void>(COMMAND.APPROVAL_RESPOND, { approved, always }),
  secretsSet: (provider: string, key: string) =>
    invoke<void>(COMMAND.SECRETS_SET, { provider, key }),
  secretsGet: (provider: string) =>
    invoke<string>(COMMAND.SECRETS_GET, { provider }),
  secretsDelete: (provider: string) =>
    invoke<void>(COMMAND.SECRETS_DELETE, { provider }),
  mcpServersList: () =>
    invoke<{ name: string; tool_count: number }[]>(COMMAND.MCP_SERVERS_LIST),
  mcpServerAdd: (server: unknown) =>
    invoke<void>(COMMAND.MCP_SERVER_ADD, { server }),
  mcpServerRemove: (name: string) =>
    invoke<void>(COMMAND.MCP_SERVER_REMOVE, { name }),
  mcpServerTest: (name: string) =>
    invoke<unknown>(COMMAND.MCP_SERVER_TEST, { name }),
  fsListWorkspace: () => invoke<string[]>(COMMAND.FS_LIST_WORKSPACE),
  memorySearch: (q: string) =>
    invoke<MemoryEntry[]>(COMMAND.SESSION_MEMORY_SEARCH, { q }),
  commandsList: () => invoke<string[]>(COMMAND.SESSION_COMMANDS_LIST),
  gitStatus: () => invoke<GitStatus>(COMMAND.GIT_STATUS),
  gitDiff: (path?: string, staged?: boolean) =>
    invoke<{ diff: string }>(COMMAND.GIT_DIFF, { path, staged }),
};

export type { ConsolidationStats };

// ---- Events ----

/// Listen for a canonical `rk://` event. Returns the unlisten handle.
export function on<T>(
  name: EventName,
  handler: (payload: T, event: Event<T>) => void,
): Promise<UnlistenFn> {
  return listen<T>(eventUri(name), (e) => handler(e.payload, e));
}

export { EVENT, COMMAND };
