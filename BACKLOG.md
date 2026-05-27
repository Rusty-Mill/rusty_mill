# Rusty Keys — development roadmap

Kanban for the full system. Items are sequenced by dependency; each phase
is a working, runnable system — not a milestone toward one.

Issues are tracked on GitHub. Each item below links to its issue.

---

## Phase 1 — Runnable skeleton (MVP) · H1

The smallest complete system: aisdk kernel + policy + core tools + CLI.
No memory, no verification. Proves the Session architecture and the
`#[tool]` macro integration.

- [ ] Cargo workspace layout — `kernel`, `constrain`, `feed`, `observe`, `compose`, `app`, `config` · #1
- [ ] `Config` crate: env-var resolution, `RUSTYKEYS_MODEL`, `RUSTYKEYS_WORKSPACE` · #2
- [ ] `Constrain` crate: `Policy` trait + `WorkspacePolicy` + `PolicyChain` · #3
- [ ] `Feed` crate: `ToolRegistry`, `#[tool]` dispatch, built-in `read_file` / `list_directory` · #4
- [ ] `Observe` crate: `Tracer`, `Episode`, `ToolEvent`, structured trace logging · #5
- [ ] `Kernel` crate: aisdk `LanguageModelRequest` loop, tool dispatch, error handling · #6
- [ ] `App` crate: `Session::send()`, thin CLI REPL, startup banner · #7

---

## Phase 2 — Observe + Compose · H3 (deterministic)

Structured visibility and deterministic verification.

- [ ] `EvidenceJournal`: append-only JSONL at `.rustykeys/evidence.jsonl`
- [ ] `InterventionLogger`: M-HIR metric, `.rustykeys/interventions.jsonl`
- [ ] `Verifier`: `Check` trait, `NoToolErrors`, `CleanTermination`
- [ ] `VerificationReport`: `render()`, `as_observation()`, `limits` field
- [ ] Failure attribution: `(category, layer)` diagnosis on failed checks
- [ ] `/verify` and `/mhir` CLI commands

---

## Phase 3 — Memory (Observe + Orient) · H2

Short-term stream → long-term graph. Completes the OODA loop.

- [ ] Short-term `Stream` trait + SQLite implementation (`rusqlite`)
- [ ] Long-term `Store` trait + SQLite implementation (FTS5 lexical recall)
- [ ] Recall: relevance + recency + importance scoring, 1-hop graph expansion
- [ ] Tiered consolidation: idle / sleep / explicit (aisdk LLM call)
- [ ] Skill grooming: refine / merge / split operations
- [ ] Skills exempt from pruning
- [ ] `/memory`, `/reflect`, `/sleep`, `/groom` CLI commands

---

## Phase 4 — Task State + Semantic Verification · H2 + H3 (semantic)

Working-memory tier and LLM-judge criteria check.

- [ ] `TaskState`: goal + success criteria, persisted to `.rustykeys/task.json`
- [ ] `set_task` / `complete_task` as `#[tool]`-registered agent tools
- [ ] Task prompt injection (drift prevention) + recall anchoring
- [ ] `CriteriaJudge`: async aisdk call, per-criterion pass/fail, `CheckResult`
- [ ] `criteria_unmet@compose/semantic` attribution branch
- [ ] `/task` CLI command

---

## Phase 5 — DuckDB + Embeddings

Optional backend for semantic recall at scale.

- [ ] `Store` implementation over `duckdb-rs`
- [ ] Native vector search via `list_cosine_similarity`
- [ ] Embedding model support via aisdk embed API
- [ ] `RUSTYKEYS_LONG_TERM_BACKEND=duckdb` env var
- [ ] Lexical fallback when no embed model configured

---

## Phase 6 — Full tool suite (Claude Code parity)

Expand from 2 built-in tools to the core set a coding agent requires.

- [ ] `bash`, `edit_file`, `write_file`, `glob`, `grep` — with `BashGuard` security checkers · #8
- [ ] `web_fetch`, `web_search` — opt-in via `RUSTYKEYS_ALLOW_WEB` · #9
- [ ] `agent` tool — subagent spawning with `AgentDepthPolicy` · #10
- [ ] Task management tools: `task_create`, `task_get`, `task_list`, `task_update`, `task_stop`, `task_output` · #11

---

## Phase 7 — Permission system

Full permission mode system with security checkers and interactive approval.

- [ ] `PermissionMode` enum: Default / Plan / AcceptEdits / ReadOnly / Restricted / Bypass · #14
- [ ] Security checkers: `CommandInjectionCheck`, `PrivilegeEscalationCheck`, `PathTraversalCheck`, `NetworkExfilCheck`, `DestructiveCommandCheck`
- [ ] `SecurityEvent` log at `.rustykeys/security.jsonl`
- [ ] `ApprovalGate`: channel-based interactive approval for high-risk calls
- [ ] `/permissions` CLI command

---

## Phase 8 — Token and context management

Keep the kernel within the model's context window indefinitely.

- [ ] `TokenBudget`: per-turn tracking, session totals · #15
- [ ] Micro-compact (Tier 1): drop oldest turn-pairs at 80% context
- [ ] Session summary (Tier 2): aisdk summarisation at 90% context
- [ ] Full compact (Tier 3): `/compact` command or 95% threshold
- [ ] `/cost` CLI command

---

## Phase 9 — Plan mode

Read-only proposal phase before destructive execution.

- [ ] `enter_plan_mode` / `exit_plan_mode` agent tools · #16
- [ ] `Plan` permission mode enforced at policy layer
- [ ] CLI approval prompt on `exit_plan_mode`
- [ ] `/plan` CLI shortcut

---

## Phase 10 — H3 episode packages

Formal reproduce → attribute → fix → verify → report workflow.

- [ ] `DeterministicCheck` registry, `checks.toml` loading · #21
- [ ] `reproduce` agent tool — records observed vs expected
- [ ] `attribute_failure` agent tool — structured pre-edit attribution
- [ ] `verification_report` agent tool — requirement-evidence links
- [ ] `ReproduceBeforeEdit` and `VerificationReportRequired` verifier checks (H3 mode)
- [ ] Five-label outcome classifier: `autonomous_verified_success` / `assisted_verified_success` / `unverified_success` / `failed` / `unsafe_invalid`
- [ ] Episode package written to `.rustykeys/episodes/`

---

## Phase 11 — Entropy auditor ⭐

Detect and record maintenance burden introduced by the agent. No equivalent
in Claude Code or hermes-agent.

- [ ] `EntropyAudit`, `EntropyFinding`, `EntropyCategory` types · #19
- [ ] Heuristics: Residue, TestWeakening, StaleDocs, DependencyChurn, BoundaryViolation
- [ ] Runs in post-turn `tokio::join!` alongside criteria judge
- [ ] `.rustykeys/entropy.jsonl` append-only log
- [ ] `/entropy` CLI command

---

## Phase 12 — MCP integration

- [ ] MCP client: consume external MCP servers (stdio + SSE transports), `mcp.toml` config · #12
- [ ] MCP server mode: expose `Session::send()` as an MCP server for IDE consumption · #13

---

## Phase 13 — Extended CLI commands

- [ ] `/compact`, `/model`, `/cost`, `/stats` — session management · #20
- [ ] `/init` — generate `AGENT_GUIDE.md` workspace context artifact
- [ ] `/commit`, `/diff`, `/branch`, `/review` — git integration
- [ ] `/config`, `/env`, `/help` — configuration and diagnostics
- [ ] `/doctor` — environment health check

---

## Phase 14 — Web gateway

- [ ] `axum` HTTP server over `Session::send()` · #18
- [ ] `POST /chat`, `GET /stream` (SSE), `GET /health`, `GET /verify`, `GET /evidence`, `GET /mhir`
- [ ] Single-session and multi-session modes
- [ ] Bearer token auth, CORS config
- [ ] `--gateway` binary flag

---

## Phase 15 — Desktop frontend

**Stack: Tauri 2 + SolidJS + CodeMirror 6 + xterm.js. All AI SDK calls on
the Rust side. Frontend is a pure reactive rendering layer over Tauri IPC.**

Tauri IPC events: `rk://token`, `rk://tool_event`, `rk://turn_complete`,
`rk://approval_request`, `rk://entropy`, `rk://bash_output`.

- [ ] Tauri 2 shell: SolidJS + Tailwind v4 + Vite, resizable panel layout · #22
- [ ] Session panel: `TurnCard`, `ToolEventRow`, `VerificationBadge`, `TaskStateBanner`, streaming · #23
- [ ] Context panel: xterm.js terminal, CM6 diff editor, git, memory browser, web preview · #24
- [ ] Composer: multi-line input, `@file` / `#memory` attachment, `/command` palette, approval gate, plan confirmation · #25
- [ ] Harness dashboard: verification stream, evidence journal, entropy chart, M-HIR trend, token budget · #26 ⭐
- [ ] Settings: provider/model, OS keychain API keys, permissions, MCP servers, harness tuning, themes · #27

---

## Backlog (post-phase)

### Streaming output to CLI
Surface `stream_text()` from aisdk so tokens appear in the terminal REPL
as they arrive. The desktop frontend already streams via `rk://token` events;
this brings the same behaviour to the TUI.

### Rich terminal UI (ratatui)
`ratatui`-based TUI for the CLI: streaming token display, syntax highlighting,
status bar (model / mode / tokens / M-HIR), vim keybindings. · #17

### Hierarchical temporal consolidation
Multi-cadence rollup summaries: idle / hourly / daily / weekly. Each level
summarises the tier below.

### OpenTelemetry observability
Wire aisdk's OTel support (when available) to the observe layer: spans per
kernel turn, tool call attributes, token counters.

### Multi-agent orchestration
`Session` as a unit of composition: one session calling another via the
`agent` tool. `AgentCoordinator` for parallel subagent execution.

### Instruction boundaries + bounded retry
JSON-schema instruction boundaries and bounded-retry-then-escalate rather
than unlimited retries.

### Entropy dashboard in CLI
`/entropy history` and cumulative entropy score surfaced in the terminal
alongside `/mhir`.

---

## Reference material

| Source | What it informs |
|---|---|
| `docs/prd/` | Component design for all harness crates |
| `docs/research/2605.13357v1.pdf` | H0–H3 ladder, episode packages, M-HIR, entropy audit, outcome taxonomy |
| baileyrd/claude-code | Tool suite (53 tools), permission modes, 3-tier compaction, 5-tier memory |
| nousresearch/hermes-agent | Skill/memory consolidation, background fork pattern, tool guardrails |
| crynta/terax-ai | Frontend UI capabilities: xterm.js, CM6 diff, Tauri 2, plan mode, approval gates |
| harness/harness-ai | Future MCP client use case (CI/CD tools via MCP server) |
