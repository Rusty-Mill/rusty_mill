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
6.  context = memory.orient(&history).await
7.  tracer.start_episode()
8.  reply = kernel.run(&history, &registry, &policy, context, &mut tracer).await
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

Three compaction tiers triggered in step 5:

| Tier | Threshold | Behaviour |
|---|---|---|
| Micro-compact | 80% of context limit | Drop oldest turn-pairs; prepend `"[compacted N turns]"`; no LLM call |
| Session summary | 90% | aisdk summarisation call over oldest half of history; insert `[SUMMARY]` message |
| Full compact | 95% (or `/compact` command) | Full summary of all history; history reset to single summary message |

All compaction events recorded in `EvidenceJournal` as `kind: "compaction"`.
Configured via `RUSTYKEYS_COMPACT_MICRO` (0.80), `RUSTYKEYS_COMPACT_SESSION`
(0.90), `RUSTYKEYS_COMPACT_FULL` (0.95).

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
| `GET` | `/stream` | SSE stream of token chunks (`rk://token` events) |
| `GET` | `/health` | `{ status: "ok", model, mode, tokens_used, tokens_limit }` |
| `GET` | `/verify` | Last `VerificationReport` as JSON |
| `GET` | `/evidence` | Recent `EvidenceJournal` entries as JSON array |
| `GET` | `/mhir` | `MhirReport` as JSON |
| `GET` | `/entropy` | Last `EntropyAudit` and cumulative delta |
| `GET` | `/memory` | Long-term memory snapshot |
| `GET` | `/config` | Active `Config` as JSON |
| `POST` | `/command` | Send a slash command (e.g. `{ "command": "/compact" }`) |
| `POST` | `/approval` | Respond to an approval request: `{ "approved": true, "always": false }` |

### Session model

- **Single-session** (`RUSTYKEYS_GATEWAY_MODE=single`): one `Session` per server
  instance, shared via `Arc<Mutex<Session>>`. For local / desktop use.
- **Multi-session** (`RUSTYKEYS_GATEWAY_MODE=multi`): `session_id` header routes
  to a `HashMap<String, Session>`. For hosted / shared deployments.

### Auth and CORS

- Bearer token auth: `RUSTYKEYS_GATEWAY_SECRET` — if set, all requests require
  `Authorization: Bearer <secret>`.
- CORS: `RUSTYKEYS_GATEWAY_CORS_ORIGIN` (default `*` for local use).

### Tauri event bridge

When the desktop frontend is active, `Session` emits Tauri events in addition
to returning HTTP responses:

| Event | Payload | Trigger |
|---|---|---|
| `rk://token` | `string` (token chunk) | Each token during streaming |
| `rk://tool_event` | `ToolEvent` | Each tool call fires |
| `rk://turn_complete` | `TurnResult` | After post-turn work completes |
| `rk://approval_request` | `ApprovalRequest` | Approval gate triggered |
| `rk://plan_exit` | `string` (plan text) | Agent calls `exit_plan_mode` |
| `rk://entropy` | `EntropyAudit` | Post-turn entropy audit complete |
| `rk://bash_output` | `string` | Bash tool stdout/stderr chunk |
| `rk://consolidation` | `ConsolidationStats` | Idle consolidation complete |

## Config

All configuration resolved from environment variables at startup.

| Variable | Default | Description |
|---|---|---|
| `RUSTYKEYS_MODEL` | `anthropic/claude-opus-4-7` | Any aisdk model string |
| `RUSTYKEYS_WORKSPACE` | `.` | Workspace root (policy boundary) |
| `RUSTYKEYS_TRACE` | `1` | Enable trace logging to stderr |
| `RUSTYKEYS_VERIFY` | `1` | Enable verification + evidence journal |
| `RUSTYKEYS_MAX_STEPS` | `10` | Kernel loop step limit |
| `RUSTYKEYS_PERMISSION_MODE` | `default` | Permission mode (see PRD 02) |
| `RUSTYKEYS_ALLOW_WEB` | `0` | Enable web_fetch / web_search tools |
| `RUSTYKEYS_HARNESS_LEVEL` | `h1` | `h1` / `h2` / `h3` |
| `RUSTYKEYS_SHORT_TERM_BACKEND` | `sqlite` | `sqlite` |
| `RUSTYKEYS_LONG_TERM_BACKEND` | `sqlite` | `sqlite` or `duckdb` |
| `RUSTYKEYS_EMBED_MODEL` | _(none)_ | aisdk embed string; absent = lexical recall |
| `RUSTYKEYS_RECALL_K` | `6` | Top-k memories to retrieve |
| `RUSTYKEYS_RECALL_WINDOW` | `6` | Recent turns used as recall query |
| `RUSTYKEYS_IDLE_THRESHOLD` | `8` | Observations before idle consolidation fires |
| `RUSTYKEYS_SKILL_GROOM_THRESHOLD` | `12` | Skills before grooming fires on sleep |
| `RUSTYKEYS_CONTEXT_LIMIT` | `200000` | Token limit for context management |
| `RUSTYKEYS_COMPACT_MICRO` | `0.80` | Micro-compact threshold |
| `RUSTYKEYS_COMPACT_SESSION` | `0.90` | Session summary threshold |
| `RUSTYKEYS_COMPACT_FULL` | `0.95` | Full compact threshold |
| `RUSTYKEYS_MAX_AGENT_DEPTH` | `3` | Max subagent recursion depth |
| `RUSTYKEYS_SEARCH_PROVIDER` | `brave` | Web search backend |
| `RUSTYKEYS_SEARCH_API_KEY` | _(none)_ | API key for search provider |
| `RUSTYKEYS_MCP_CONFIG` | `.rustykeys/mcp.toml` | MCP server config file |
| `RUSTYKEYS_EVIDENCE_LOG` | `.rustykeys/evidence.jsonl` | Evidence journal path |
| `RUSTYKEYS_INTERVENTIONS_LOG` | `.rustykeys/interventions.jsonl` | Intervention log path |
| `RUSTYKEYS_ENTROPY_LOG` | `.rustykeys/entropy.jsonl` | Entropy audit log path |
| `RUSTYKEYS_SECURITY_LOG` | `.rustykeys/security.jsonl` | Security event log path |
| `RUSTYKEYS_TASK_FILE` | `.rustykeys/task.json` | Task State persistence path |
| `RUSTYKEYS_GATEWAY_MODE` | `single` | `single` or `multi` session |
| `RUSTYKEYS_GATEWAY_PORT` | `3000` | HTTP gateway port |
| `RUSTYKEYS_GATEWAY_SECRET` | _(none)_ | Bearer token for gateway auth |
| `RUSTYKEYS_GATEWAY_CORS_ORIGIN` | `*` | CORS origin header |
| `RUSTYKEYS_MODE` | `cli` | `cli` / `gateway` / `mcp` (binary mode) |

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
  per connection; `Config` is shared, session state is not.
- **MCP server mode**: `RUSTYKEYS_MODE=mcp` starts the MCP server — see PRD 07.
