# Rusty Keys — development roadmap

Kanban for the harness subsystem. Items below are sequenced by dependency;
each phase is a working, runnable system — not a milestone toward one.

---

## Phase 1 — Runnable skeleton (MVP)

The smallest complete system: aisdk kernel + policy + tools + CLI.
No memory, no verification. Proves the Session architecture and the
`#[tool]` macro integration.

- [ ] Cargo workspace layout (`kernel`, `constrain`, `feed`, `observe`, `compose`, `app`)
- [ ] aisdk integration: `LanguageModelRequest` builder, async kernel loop
- [ ] `#[tool]` macro wired to tool registry — type-safe dispatch
- [ ] Policy layer: `before_tool` hook, workspace filesystem allowlist
- [ ] `Session` struct: message-passing interface, `send()` returns `(String, Report)`
- [ ] CLI adapter: thin `tokio::main` REPL over `Session`
- [ ] `Config`: env-var resolution, `RUSTYKEYS_MODEL`, `RUSTYKEYS_WORKSPACE`

---

## Phase 2 — Observe + Compose

Structured visibility and deterministic verification. Completes H3 without
memory.

- [ ] `Tracer`: per-turn tool events, token counter, `final_reached`, episode
- [ ] `EvidenceJournal`: append-only JSONL at `.rustykeys/evidence.jsonl`
- [ ] `InterventionLogger`: M-HIR metric, `.rustykeys/interventions.jsonl`
- [ ] `Verifier`: `Check` trait, `no_tool_errors`, `clean_termination`
- [ ] `VerificationReport`: `render()`, `as_observation()`, `limits` field
- [ ] Failure attribution: `(category, layer)` diagnosis on failed checks
- [ ] `/verify` and `/mhir` CLI commands

---

## Phase 3 — Memory (Observe + Orient)

Short-term stream → long-term graph. Completes the OODA loop.

- [ ] Short-term `Stream` trait + SQLite implementation (`rusqlite`)
- [ ] Long-term `Store` trait + SQLite implementation (FTS5 lexical recall)
- [ ] Recall: relevance + recency + importance scoring, 1-hop graph expansion
- [ ] Tiered consolidation: idle / sleep / explicit (aisdk LLM call)
- [ ] Skill grooming: refine / merge / split operations
- [ ] Skills exempt from pruning (ADR-019 equivalent)
- [ ] `/memory`, `/reflect`, `/sleep`, `/groom` CLI commands

---

## Phase 4 — Task State + Semantic Verification

Working-memory tier and LLM-judge criteria check. Completes H2 + full H3.

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

## Backlog (post-MVP)

### Streaming output
Surface `stream_text()` from aisdk to the CLI/Session so tokens appear as they
arrive rather than after the full response. Requires the Session's reply channel
to carry a stream handle, not a completed string.

### Web gateway
An `axum` HTTP adapter over `Session` — the same `send()` interface, different
transport. Compatible with Vercel AI SDK UI (aisdk supports the wire protocol).

### Hierarchical temporal consolidation
Multi-cadence rollup summaries: break / hourly / daily / weekly. Each level
summarises the tier below. Mirrors Keystone's BACKLOG item.

### OpenTelemetry observability
aisdk's roadmap includes OTel support. Wire it to the observe layer once
available — spans per kernel turn, tool call attributes, token counters.

### Multi-agent composition
`Session` as a unit of composition: one session calling another via tools.
The constrain layer controls inter-session permissions.

### Instruction boundaries + bounded retry
Minimal-pipeline safety upgrades from the whitepaper: JSON-schema instruction
boundaries and bounded-retry-then-escalate rather than unlimited retries.
