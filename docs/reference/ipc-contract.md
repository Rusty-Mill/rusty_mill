# IPC contract reference

**Single source of truth** for everything that crosses an adapter boundary
between the Rust harness and a client (Tauri desktop, web gateway, ACP editor):

- the **`rk://` events** the harness emits,
- the **`invoke` commands** a client calls,
- the **boundary error taxonomy** every adapter renders.

The names below are pinned in code at [`crates/app/src/contract.rs`](../../crates/app/src/contract.rs)
(`app::contract`) and unit-tested for completeness. The gateway SSE channel
(Phase 14), the ACP `session/update` notifications (Phase 16), and the Tauri
desktop bridge (Phase 15) **reference these names** rather than re-deriving their
own — this is the anti-drift contract the round-3 audit required.

Authoritative companions: [`docs/prd/06-app.md`](../prd/06-app.md) §"Tauri event
bridge" (the canonical 9-event table) and §"Boundary error taxonomy";
[`docs/prd/08-frontend.md`](../prd/08-frontend.md) (the `invoke` surface).
Payload *shapes* are owned by the producing types (`observe::ToolEvent`,
`observe::EntropyAudit`, `compose::VerificationReport`, …); this doc pins the
**names** and the cross-boundary `TurnResult`/`BoundaryError`.

---

## 1. Events — harness → client (`rk://`, the canonical nine)

Emitted as Tauri `rk://<name>` events, mirrored one-for-one by the gateway SSE
channel (`event:` = the name, scheme stripped) and by ACP `session/update`.
`ToolEvent` payloads are redaction-scrubbed before emission (ADR-0026).

| Event (`rk://`) | Payload | Trigger |
|---|---|---|
| `turn_start` | `{ turn_id }` | Turn begins (kernel about to run); UI locks the composer |
| `token` | `string` | Each token during streaming |
| `tool_event` | `observe::ToolEvent` | A tool call fires |
| `turn_complete` | [`TurnResult`](#3-turnresult) | After post-turn work completes |
| `approval_request` | approval request `{ tool, args, trigger }` | The approval gate triggered |
| `plan_exit` | `string` (plan text) | The agent called `exit_plan_mode` |
| `entropy` | `observe::EntropyAudit` | Post-turn entropy audit complete |
| `bash_output` | `string` | A bash stdout/stderr chunk |
| `consolidation` | `feed::ConsolidationStats` | Idle consolidation complete |

**Surface mapping:**
- **Tauri:** `listen('rk://<name>', …)`.
- **Gateway SSE (`GET /stream`):** `event: <name>` frames, plus the SSE-specific
  terminal sentinels `done` (success) / `error` (a [`BoundaryError`](#4-boundary-error-taxonomy)
  frame) — these two are transport sentinels, not `rk://` events.
- **ACP (`session/update`):** the agent maps these onto ACP's `sessionUpdate`
  vocabulary (e.g. `agent_message_chunk`, `verification`); the *trigger set* is
  the same nine.

> v1 note: the gateway and ACP emit `turn_start`/`turn_complete` at turn
> boundaries (token-level `token`/`bash_output` streaming via the kernel's
> `stream_turn` is a flagged follow-on).

---

## 2. Commands — client → harness (Tauri `invoke`)

Each maps to a `Session` method or an app-level operation. The desktop bridge
(Phase 15) registers one `#[tauri::command]` per name; the gateway exposes the
read endpoints as `GET` routes and `POST /command` for slash commands.

| Command | Args | Returns | Backed by |
|---|---|---|---|
| `session_send` | `{ message, attachments? }` | `TurnResult` | `Session::send` |
| `session_command` | `{ command }` | `void` | slash command (`/compact`, …) |
| `session_last_report` | — | `VerificationReport` | `Session::last_report` |
| `session_mhir` | — | `MhirReport` | `Session::mhir` |
| `session_config` | — | `Config` | `Session` config |
| `config_set` | `{ key, value }` | `AppConfig` | session override (restart-only keys flagged) |
| `session_memory_snapshot` | — | `MemorySnapshot` | `Session::memory_recent` |
| `session_evidence_recent` | `{ n }` | `EvidenceEntry[]` | `Session::evidence_recent` |
| `session_entropy_history` | — | `EntropyAudit[]` | `Session::entropy_recent` |
| `session_token_budget` | — | `TokenBudget` | `Session::cost` |
| `approval_respond` | `{ approved, always }` | `void` | `ApprovalGate` response |
| `secrets_set` | `{ provider, key }` | `void` | OS keychain |
| `secrets_get` | `{ provider }` | `string` | OS keychain |
| `secrets_delete` | `{ provider }` | `void` | OS keychain |
| `mcp_servers_list` | — | `McpServer[]` | `Session::mcp_summary` |
| `mcp_server_add` | `{ server }` | `void` | MCP manager |
| `mcp_server_remove` | `{ name }` | `void` | MCP manager |
| `mcp_server_test` | `{ name }` | `McpTestResult` | MCP manager |
| `fs_list_workspace` | — | `string[]` | `@file` picker |
| `session_memory_search` | `{ q }` | `MemoryEntry[]` | `#memory` picker |
| `session_commands_list` | — | `string[]` | `/command` palette |

Secrets never leave the Rust side longer than the `invoke` call that passes them
(PRD 08): they are stored in the OS keychain, never in JS memory.

---

## 3. `TurnResult`

The `turn_complete` / `POST /chat` / `session_send` payload — the boundary
projection of a `TurnOutcome`. The full `VerificationReport` (checks +
attributions) is fetched separately via `session_last_report` / `GET /verify`.

```jsonc
{ "reply": "…", "verified": true, "limits": "deterministic checks only; …" }
```

---

## 4. Boundary error taxonomy

`Session::send()` returns tool *failures* as values inside the reply (the
`ToolOutcome` contract), but a turn can still fail at the boundary. Every adapter
collapses the typed internal error into one of **six** surface kinds, rendered as
the Tauri `invoke` rejection, the SSE `error` frame, and the ACP error body.

| Kind (`snake_case`) | Maps from | Meaning |
|---|---|---|
| `provider_error` | `KernelError::Provider { retryable: false }` | Provider returned a non-retryable error |
| `timeout` | `KernelError::Timeout`, `ToolError::Timeout` | Per-call / tool timeout after retries |
| `rate_limited` | provider `429` after `RUSTYKEYS_RETRY_MAX` | Rate limit (`Retry-After` honored internally) |
| `auth_error` | provider 401/403; bearer mismatch | Credential rejected |
| `policy_block` | a `PolicyError` that *ends* the turn | A policy decision the caller must act on |
| `internal` | `<Crate>Error::Internal`, caught panic | Bug / unexpected state; turn aborted |

A `policy_block` boundary error is **distinct** from a recoverable per-tool
`before_tool` denial (which returns a `BLOCKED` `ToolOutcome` and the turn
continues, verifying UNVERIFIED). The boundary kind is only for a policy failure
that ends the turn.
