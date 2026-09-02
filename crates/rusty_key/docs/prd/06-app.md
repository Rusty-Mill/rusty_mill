# PRD 06 — App (Session + CLI + Gateways)

## Responsibility

The app layer has three parts:

- **`Session`**: the transport-agnostic AI loop. Owns the full turn cycle —
  observe → orient → kernel → compose — and exposes a single `send()` method.
  This is the centre of the system.
- **CLI adapter**: a `tokio::main` REPL over `Session`. The only part of the
  codebase that touches `stdin`/`stdout`.
- **Web gateway**: an `axum` HTTP server over `Session`. The transport for the
  desktop frontend and any HTTP client.

These are deliberately separate. The CLI and gateway are adapters over the same
`Session`; they share no state-management logic.

## Session

```rust
pub struct Session {
    config:        Config,
    kernel:        Kernel,
    registry:      ToolRegistry,
    policy:        Box<dyn Policy>,
    approval_gate: Option<ApprovalGate>,
    memory:        Memory,
    tracer:        Tracer,
    entropy:       EntropyAuditor,
    verifier:      Verifier,
    judge:         CriteriaJudge,
    journal:       EvidenceJournal,
    interventions: InterventionLogger,
    token_budget:  TokenBudget,
    history:       Vec<Message>,
    last_report:   Option<VerificationReport>,
    plan_mode:     bool,
}
```

### `send()`

```rust
impl Session {
    pub async fn send(&mut self, message: &str) -> Result<(String, VerificationReport)>;
}
```

Full turn cycle:

```
1.  Detect unverified_followup (if last_report is Some and !verified)
2.  Detect tool_block intervention (if last ApprovalGate response was Block)
3.  memory.observe(user message)
4.  history.push(user message)
5.  token_budget.check_and_compact(&mut history).await  → micro/session/full compact if needed
6.  oriented = memory.orient(&history).await   // Oriented { extra_context, context_entries } (PRD 03)
7.  tracer.start_episode(); record oriented.context_entries → context_trace (ADR-0036); emit rk://turn_start (desktop only)
8.  reply = kernel.run(&history, &registry, &policy, oriented.extra_context, &mut tracer).await
9.  history.push(assistant reply)
10. memory.observe(reply)

Concurrent post-turn:
11a. report = verifier.verify_with_judge(&reply, tracer.episode(), &judge).await
11b. idle_stats = memory.consolidate(Idle).await
11c. entropy_audit = entropy.audit(tracer.episode()).await

12. memory.observe(report.as_observation())
13. journal.record_turn(&reply, tracer.episode(), &report)  // or record_episode at H3
14. last_report = Some(report.clone())
15. Emit Tauri event rk://turn_complete (desktop only)
16. return (reply, report)
```

Steps 11a–11c run via `tokio::join!` — the criteria judge, idle consolidation,
and entropy audit overlap while the reply is already in the caller's hands.

### Task and episode identity — `task_id` stability (F19)

RK's episode = one `send()` turn, but task-level metrics regroup a task's turns
via `episode_id = "ep_<task_id>"` (ADR-0018). For that regrouping key to be
stable, **`task_id` must stay constant across every turn of the same task** —
otherwise turns of one task would scatter across distinct `episode_id`s and the
aggregation would silently undercount.

The mechanism that holds it constant:

- **`task_id` is assigned once, at task creation.** When `/task [goal]` (or the
  agent's `set_task` tool, PRD 03) opens a task, the `TaskStore` mints a `task_id`
  and persists it in `task.json` (data-model §8). It is **not** regenerated
  per turn.
- **`Session::send()` reads the active `task_id`, never writes a new one.** Each
  turn reads the current `TaskState` from the `TaskStore` and stamps that same
  `task_id` (hence the same `episode_id`) onto the turn record / episode package.
  A turn with no active task carries the session's fallback id, so non-task turns
  do not collide with task turns.
- **The id changes only on an explicit task boundary.** `task_id` is replaced
  only when the active task transitions to `done` and a *new* task is opened, or
  when `/task` overrides the active task (a `task_override` intervention, PRD 04).
  An idle→active resume of the *same* task keeps its `task_id`. The session JSON
  (`sessions/<session_id>.json`, data-model §6) carries `task_id` so `/resume`
  rehydrates the same task identity, keeping `episode_id` stable even across a
  session restart.

So `episode_id` is a deterministic function of a `task_id` that is stable for the
task's whole lifetime — the regrouping key the eval plan aggregates over
(ADR-0018; [`eval-plan.md`](../dev/eval-plan.md) §3) is reliable by construction.

### Token budget and compaction

```rust
pub struct TokenBudget {
    pub context_limit: usize,
    pub used_tokens: usize,
    pub session_total_tokens: u64,
    pub compaction_count: usize,
}

impl TokenBudget {
    pub async fn check_and_compact(&mut self, history: &mut Vec<Message>) -> CompactionResult;
}
```

Four compaction tiers triggered in step 5:

| Tier | Threshold | Behaviour |
|---|---|---|
| Warn | ≈10pp below micro (e.g. 70%) | Light micro: trim only the very oldest turn-pairs (keeps 2× as many as micro); no LLM call (P4 finer tier) |
| Micro-compact | 80% of context limit | Drop oldest turn-pairs; prepend `"[compacted N turns]"`; no LLM call |
| Session summary | 90% | aisdk summarisation call over oldest half of history; insert `[SUMMARY]` message |
| Full compact | 95% (or `/compact` command) | Full summary of all history; history reset to single summary message |

All compaction events recorded in `EvidenceJournal` as `kind: "compaction"`.
Configured via `RUSTYKEYS_COMPACT_MICRO` (0.80), `RUSTYKEYS_COMPACT_SESSION`
(0.90), `RUSTYKEYS_COMPACT_FULL` (0.95).

**Real-token calibration (P4).** The thresholds fire on a token *estimate*
(≈4 chars/token), corrected by a calibration factor `real / estimate` learned
from the provider's reported usage each turn (`run_turn`/`stream_turn` surface
`TurnUsage`). So compaction fires on real tokens, not raw length; a provider
that omits usage (e.g. the offline fake) leaves the factor at `1.0`. `used_tokens`
(shown by `/cost`) is the provider's real input-token count when reported.

### Plan mode lifecycle

```
Agent calls enter_plan_mode()
  → Session sets plan_mode = true
  → WorkspacePolicy switches to Plan mode (writes + bash blocked)
  → All subsequent tool calls in this turn are read-only

Agent calls exit_plan_mode()
  → Session emits approval request to CLI or Tauri event rk://plan_exit
  → Human responds: Proceed / Reject / Annotate

On Proceed:
  → plan_mode = false
  → WorkspacePolicy switches to AcceptEdits for the next turn

On Reject:
  → plan_mode = false
  → WorkspacePolicy stays at Default

On Annotate (desktop only):
  → Human's annotation sent as follow-up message
  → plan_mode = false; agent receives feedback and re-proposes
```

Plan approval is NOT recorded as an intervention (it is expected behaviour).
Tool blocks during plan mode are recorded as `tool_block` interventions.

### Session on its own task

For the web gateway and responsive CLI, `Session` runs on a `tokio` task with
an `mpsc` channel pair:

```rust
let (tx_in, rx_in) = mpsc::channel::<SessionMessage>(1);
let (tx_out, rx_out) = mpsc::channel::<SessionResult>(1);

tokio::spawn(async move {
    let mut session = Session::new(config).await?;
    while let Some(msg) = rx_in.recv().await {
        match msg {
            SessionMessage::Send(text) => {
                let result = session.send(&text).await?;
                tx_out.send(SessionResult::Turn(result)).await?;
            }
            SessionMessage::Command(cmd) => {
                session.handle_command(&cmd).await?;
            }
            SessionMessage::Shutdown => break,
        }
    }
    session.shutdown().await?;
    Ok::<_, Error>(())
});
```

The channel is sized 1 to preserve turn-by-turn ordering.

### `shutdown()`

```rust
impl Session {
    pub async fn shutdown(&mut self) -> Result<()>;
}
```

Called on exit. Runs sleep-tempo consolidation, skill grooming, flushes the
evidence journal, and records any final entropy audit.

### Subagent spawning — `SessionFactory`

The `agent` subagent tool lives in the `feed` crate but must construct a
`Session` (in `app`). To keep the crate DAG acyclic, `app` does not get imported
by `feed`; instead a **`SessionFactory`** (spawn) trait lives in a low crate, the
`agent` tool depends on the trait, and `app` injects the concrete implementation
at startup (**[ADR-0017](../adr/0017-subagent-spawning-via-sessionfactory-trait.md)**).
The factory implementation also handles subagent system-prompt inheritance and
the `RUSTYKEYS_MAX_AGENT_DEPTH` recursion bound. A spawned subagent turn is
linked to its parent via `parent_turn_id` in the evidence journal (data-model
§4.1).

> **Hot-reload vs restart:** a subagent inherits the parent's resolved `Config`.
> Workspace and model are restart-only (see Config below), so a subagent cannot
> be spawned into a different workspace or model than its parent.

## CLI adapter

The CLI is a `tokio::main` binary. All AI logic lives in `Session`.

### Full command set

| Command | Action | Intervention logged |
|---|---|---|
| `/task [goal]` | Show or set active task | `task_override` if task already active |
| `/memory` | Print long-term memory snapshot | — |
| `/reflect` | Explicit consolidation | `manual_reflect` |
| `/sleep` | Deep consolidation + skill groom | `manual_reflect` |
| `/groom` | Skill grooming pass only | `manual_groom` |
| `/verify` | Show last verification report | `manual_verify` |
| `/evidence` | Show recent evidence journal | — |
| `/mhir` | Show M-HIR rate and breakdown | — |
| `/entropy` | Show last entropy audit and cumulative delta | — |
| `/compact` | Trigger full compaction immediately | — |
| `/cost` | Show token usage and session cost | — |
| `/stats` | Combined: tokens, turns, tool calls, M-HIR, entropy delta | — |
| `/model [name]` | Show or switch active model | — |
| `/permissions [mode]` | Show or change permission mode | — |
| `/plan [goal]` | Enter plan mode with optional task | — |
| `/init` | Generate `AGENT_GUIDE.md` in workspace root | — |
| `/commit [msg]` | Stage and commit via agent | — |
| `/diff` | Show `git diff` for current workspace | — |
| `/branch [name]` | Create or switch git branch | — |
| `/review` | Code review of current diff via agent | — |
| `/config` | Show all env-var config with current values | — |
| `/config set KEY VALUE` | Override config for this session | — |
| `/env` | Show all `RUSTYKEYS_*` env vars set | — |
| `/help [command]` | List commands or detail for one command | — |
| `/doctor` | Check environment: model, workspace, SQLite, MCP | — |
| `/resume [id]` | Resume a named previous session | — |
| `/export` | Export session to JSONL | — |
| `/mcp` | List connected MCP servers and their tools | — |
| `exit` / `quit` | Shutdown and exit | — |

### Startup banner

```
Rusty Keys :: model=anthropic/claude-opus-4-7 :: workspace=/path/to/ws :: mode=default
memory :: short-term=sqlite :: long-term=sqlite :: recall=lexical
tokens :: 0 / 200000 (0%) :: compactions=0
Commands: /help for full list. exit to quit.
```

### `/mhir` output

```
M-HIR: 3 interventions / 12 turns = 25.0%
  unverified_followup: 2
  manual_verify: 1
```

## Web gateway

The web gateway exposes `Session::send()` over HTTP, making Rusty Keys
accessible to the desktop frontend and any HTTP client.

```bash
rusty-keys --gateway
# or
RUSTYKEYS_MODE=gateway rusty-keys
```

### Endpoints

| Method | Path | Description |
|---|---|---|
| `POST` | `/chat` | `session.send(message)` → `{ reply, verified, checks, outcome }` |
| `GET` | `/stream` | SSE stream mirroring the canonical `rk://` events (framing below; DEFER → Phase 14) |
| `GET` | `/health` | `{ status: "ok", model, mode, tokens_used, tokens_limit }` |
| `GET` | `/verify` | Last `VerificationReport` as JSON |
| `GET` | `/evidence` | Recent `EvidenceJournal` entries as JSON array |
| `GET` | `/mhir` | `MhirReport` as JSON |
| `GET` | `/entropy` | Last `EntropyAudit` and cumulative delta |
| `GET` | `/memory` | Long-term memory snapshot |
| `GET` | `/config` | Active `Config` as JSON |
| `POST` | `/command` | Send a slash command (e.g. `{ "command": "/compact" }`) |
| `POST` | `/approval` | Respond to an approval request: `{ "approved": true, "always": false }` |

### SSE `/stream` framing (DEFER → Phase 14)

> The full framing, `/chat`↔`/stream` correlation model, and backpressure /
> cancellation policy are **deferred to Phase 14**. The protocol *shape* is
> sketched here so the frontend's web-adapter seam (PRD 08) is real.

`GET /stream` is one SSE channel that mirrors the canonical `rk://` event table
above — the gateway does not collapse the turn to bare token chunks. Each SSE
frame's `event:` name is the `rk://` event with the scheme stripped:

```
event: turn_start
id: turn_20260527_143022_abc123
data: {"turn_id":"turn_20260527_143022_abc123"}

event: token
data: "Fixed"

event: tool_event
data: {"name":"edit_file","status":"ok", ...}

event: turn_complete
id: turn_20260527_143022_abc123
data: { ...TurnResult... }

event: done
data: {"turn_id":"turn_20260527_143022_abc123"}
```

- **Named events** mirror `rk://token`, `rk://tool_event`, `rk://turn_complete`,
  `rk://approval_request`, `rk://entropy`, `rk://bash_output`, etc.
- **`id:`** carries the `turn_id` so a client can resume with `Last-Event-ID`.
- **Terminal sentinels:** a turn ends with either `event: done` (success) or
  `event: error` (a boundary-error-taxonomy frame: `data` is
  `{ "error": <kind>, "message": … }`). The terminating frame is how a streaming
  client learns the turn finished, since SSE has no per-turn close.

### `/health` liveness vs readiness (DEFER → Phase 14)

The Phase-1 `/health` returns `{ status: "ok", model, mode, tokens_used,
tokens_limit }` — a flat liveness probe. A gateway behind a load balancer needs
**readiness** too (provider reachable, MCP servers up, token budget headroom).
Splitting liveness from readiness and expressing degraded states is **deferred
to Phase 14**; the Phase-1 shape stays as-is until then.

### Session model

- **Single-session** (`RUSTYKEYS_GATEWAY_MODE=single`): one `Session` per server
  instance, shared via `Arc<Mutex<Session>>`. For local / desktop use.
- **Multi-session** (`RUSTYKEYS_GATEWAY_MODE=multi`): `session_id` header routes
  to a `HashMap<String, Session>`. For hosted / shared deployments.

### Multi-session lifecycle

> **DEFER → Phase 14** for the full implementation; the contract is pinned here
> so the data model and auth design account for it now.

In `multi` mode each `Session` holds a kernel, SQLite handles, and history, so it
cannot live forever. The lifecycle contract:

- **Idle TTL** — a session with no activity for `RUSTYKEYS_SESSION_TTL_SECS`
  (default `3600`) is evicted: `Session::shutdown()` runs (consolidation,
  journal flush) and the entry is dropped.
- **Max sessions** — at most `RUSTYKEYS_MAX_SESSIONS` (default `64`) concurrent
  sessions; a new `session_id` beyond the cap is rejected (or evicts the
  least-recently-used, decided in Phase 14). This bounds memory and DB handles.
- **Eviction on disconnect / shutdown** — closing the underlying connection (and
  process shutdown) drains every live session through `shutdown()` so no journal
  write is lost. MCP applies the same rule on client disconnect (PRD 07).
- **`session_id` ↔ auth binding** — when `RUSTYKEYS_GATEWAY_SECRET` is set, the
  bearer token scopes which `session_id`s a caller may reach; a caller cannot
  attach to or guess another tenant's session. Without a secret, `multi` mode is
  for trusted local use only.

The on-disk shape of a persisted/named session (`sessions/<session_id>.json` —
`session_id`, timestamps, model, harness level, history, `task_id`) is defined in
**[data-model §6](../architecture/data-model.md#6-sessions--sessionssession_idjson)**;
that file is what `/resume [id]` rehydrates and what a gateway/MCP `session_id`
maps to. TTL/eviction parameters live in
[configuration.md](../reference/configuration.md#gateway-rustykeys_modegateway).

### Auth and CORS

- Bearer token auth: `RUSTYKEYS_GATEWAY_SECRET` — if set, all requests require
  `Authorization: Bearer <secret>`.
- CORS: `RUSTYKEYS_GATEWAY_CORS_ORIGIN` (default `*` for local use).

### Tauri event bridge — canonical `rk://` event table

When the desktop frontend is active, `Session` emits Tauri events in addition
to returning HTTP responses.

**This table is the single canonical `rk://` event catalog.** PRD 08 (frontend)
and the SSE `/stream` sketch below both cite it rather than redefining their own
lists; the gateway SSE channel mirrors these named events one-for-one. Earlier
drafts disagreed on the count (BACKLOG listed 6, this PRD listed 8, PRD 08 used
a 9th — `rk://turn_start` — in its Composer lock logic without ever listing it).
The reconciled set is the **nine** events below; `rk://turn_start` is now
included.

| Event | Payload | Trigger |
|---|---|---|
| `rk://turn_start` | `{ turn_id }` | Turn begins (kernel about to run); UI locks the composer |
| `rk://token` | `string` (token chunk) | Each token during streaming |
| `rk://tool_event` | `ToolEvent` | Each tool call fires |
| `rk://turn_complete` | `TurnResult` | After post-turn work completes |
| `rk://approval_request` | `ApprovalRequest` | Approval gate triggered |
| `rk://plan_exit` | `string` (plan text) | Agent calls `exit_plan_mode` |
| `rk://entropy` | `EntropyAudit` | Post-turn entropy audit complete |
| `rk://bash_output` | `string` | Bash tool stdout/stderr chunk |
| `rk://consolidation` | `ConsolidationStats` | Idle consolidation complete |

`rk://turn_start` is emitted just before step 8 of the turn cycle (kernel run);
`rk://turn_complete` is step 15. `ToolEvent` payloads are redaction-scrubbed
before emission (data-model §11, ADR-0026), so `rk://tool_event` never carries a
raw secret.

## Boundary error taxonomy

`Session::send()` returns `Result<(String, VerificationReport)>`; tool *failures*
are values inside the reply (the `ToolOutcome` contract — they don't surface as
`Err`), but a turn can still fail at the boundary (provider down, timeout, auth,
a hard policy block, an internal bug). Each adapter — CLI, HTTP gateway, Tauri —
must render the same failure consistently, so the boundary speaks one small,
closed taxonomy rather than letting every surface invent its own strings.

The internal error model (the per-crate `thiserror` enums — `KernelError`,
`ToolError`, `PolicyError`, `ComposeError`, …, and the `ToolOutcome` tool-result
contract) is owned by **[`docs/dev/error-handling.md`](../dev/error-handling.md)**
(forward-ref — lands with Phase 1). The taxonomy below is the *boundary
projection* of those internal errors: `app` collapses the typed error from any
layer into one of six surface kinds.

| Kind | Maps from (internal) | Meaning |
|---|---|---|
| `ProviderError` | `KernelError::Provider { retryable: false }` | Provider returned a non-retryable error (e.g. 4xx, bad request) |
| `Timeout` | `KernelError::Timeout`, `ToolError::Timeout` | Per-call / tool timeout after retries exhausted |
| `RateLimited` | `KernelError::Provider` from a `429` after `RUSTYKEYS_RETRY_MAX` | Provider rate limit; `Retry-After` already honored internally |
| `AuthError` | provider 401/403; gateway/MCP bearer-token mismatch | Caller or provider credential rejected |
| `PolicyBlock` | `PolicyError::*` that aborts the turn (not a single recoverable tool block) | A policy decision the caller must act on |
| `Internal` | `<Crate>Error::Internal`, escaped panic caught at `Session::send` | Bug / unexpected state; turn recorded as aborted (ARCHITECTURE §10) |

Note the distinction from a *recoverable* tool block: a single `before_tool`
denial is returned to the model as a `BLOCKED …` `ToolOutcome` and the loop
continues (the turn just verifies UNVERIFIED). `PolicyBlock` here is the boundary
kind for a policy failure that *ends* the turn.

### Per-surface mapping

| Kind | CLI (text to stderr) | HTTP (`/chat` status + body) | Tauri (`invoke` rejection) |
|---|---|---|---|
| `ProviderError` | `error: provider: <msg>` | `502 Bad Gateway` · `{ "error": "provider_error", "message": … }` | reject with `{ kind: "provider_error", message }` |
| `Timeout` | `error: timed out after <ms>ms` | `504 Gateway Timeout` · `{ "error": "timeout", … }` | reject with `{ kind: "timeout", … }` |
| `RateLimited` | `error: rate limited, retry later` | `429 Too Many Requests` (+ `Retry-After` if known) · `{ "error": "rate_limited", … }` | reject with `{ kind: "rate_limited", … }` |
| `AuthError` | `error: auth failed: <msg>` | `401 Unauthorized` · `{ "error": "auth_error", … }` | reject with `{ kind: "auth_error", … }` |
| `PolicyBlock` | `blocked: <policy reason>` | `403 Forbidden` · `{ "error": "policy_block", "message": … }` | reject with `{ kind: "policy_block", message }` |
| `Internal` | `error: internal error (see trace)` | `500 Internal Server Error` · `{ "error": "internal", … }` | reject with `{ kind: "internal", … }` |

The HTTP `error` field and the Tauri `kind` field both use the snake_case kind
name (serde convention, data-model §7). On the SSE `/stream` channel a boundary
error is delivered as a terminal `event: error` frame (see the sketch below), not
an HTTP status — the HTTP status applies to the non-streaming `POST /chat`. PRD
08 cites this taxonomy for its `invoke`-error handling.

## Config

All configuration resolves from environment variables at startup (the `config`
crate). The **full, authoritative `RUSTYKEYS_*` table — every variable, default,
and the hot-reload-vs-restart-only rules — lives in
[`docs/reference/configuration.md`](../reference/configuration.md)** (the SSOT).
This PRD does not duplicate it; `/config` and `/config set KEY VALUE` operate on
the same set.

Key vars an operator of the app layer touches most often (see the reference for
the rest):

- `RUSTYKEYS_MODE` (`cli` | `gateway` | `mcp`) — which adapter this binary runs;
  restart-only.
- `RUSTYKEYS_MODEL` — the kernel model; rebinding it is restart-only.
- `RUSTYKEYS_WORKSPACE` — the policy boundary; restart-only (changing it
  mid-session would void the canonicalized `WorkspacePolicy`).
- `RUSTYKEYS_GATEWAY_MODE` / `_PORT` / `_SECRET` / `_CORS_ORIGIN` — gateway
  transport + auth.
- `RUSTYKEYS_SESSION_TTL_SECS` / `_MAX_SESSIONS` — multi-session lifecycle
  bounds (see below).
- `RUSTYKEYS_PERMISSION_MODE`, `RUSTYKEYS_HARNESS_LEVEL`, `RUSTYKEYS_VERIFY` —
  safety / maturity / verification toggles.

### Hot-reload vs restart-only

`/config set` (and the `config_set` IPC command) applies for the current
session. **Restart-only** keys are rejected mid-session because mutating them
would invalidate live state: `RUSTYKEYS_WORKSPACE` (canonicalized policy
boundary), `RUSTYKEYS_MODEL` (rebinds the kernel), `RUSTYKEYS_MODE`, the
backend selectors, and the gateway/MCP transport+port. Recall/compaction
tunables, per-role models, harness level (within a level that doesn't change the
tool registry), and the redaction toggle are hot-reloadable. The authoritative
list is in [configuration.md](../reference/configuration.md#hot-reload-vs-restart-only).

## Cargo workspace layout

```
rusty-keys/
├── Cargo.toml               # workspace
├── crates/
│   ├── kernel/              # Kernel struct, aisdk integration
│   ├── constrain/           # Policy trait, WorkspacePolicy, PermissionMode,
│   │                        # SecurityCheckers, ApprovalGate
│   ├── feed/                # ToolRegistry, full tool suite, Memory, TaskStore,
│   │                        # context assembly, H3 verification tools
│   │   └── memory/          # Stream, Store, consolidate, refine, recall
│   ├── observe/             # Tracer, InterventionLogger, EntropyAuditor
│   ├── compose/             # Verifier, checks, CriteriaJudge, EvidenceJournal,
│   │                        # H3 episode packages, outcome taxonomy
│   ├── app/                 # Session, CLI, web gateway, Tauri event bridge
│   ├── config/              # Config
│   └── mcp/                 # MCP client + server (PRD 07)
├── frontend/                # Desktop UI (PRD 08)
│   ├── src/                 # SolidJS components
│   ├── src-tauri/           # Tauri Rust backend
│   └── package.json
└── BACKLOG.md
```

Crate boundaries enforce the architectural separation: `kernel` cannot import
`feed` or `compose`; `observe` cannot import `compose`; `app` imports everything.
The dependency graph is a DAG.

## Seams

- **Streaming CLI**: `stream_text()` from aisdk emitted to the terminal token
  by token; also sent as `rk://token` Tauri events. Tracked in BACKLOG.
- **Per-request sessions**: for multi-user gateway, `Session::new()` is called
  per connection; `Config` is shared, session state is not. The TTL / max-session
  / eviction / auth-binding contract is in *Multi-session lifecycle* above
  (DEFER → Phase 14).
- **MCP server mode**: `RUSTYKEYS_MODE=mcp` starts the MCP server — see PRD 07.
