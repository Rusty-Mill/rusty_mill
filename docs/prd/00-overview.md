# PRD 00 — System overview

## Product summary

**Rusty Keys** is an AI-native application skeleton in Rust. It carries forward
the harness philosophy of [Keystone](https://github.com/baileyrd/Keystone) —
the model's agent loop is the kernel; the application is the harness built around
it — and realises that architecture in a language whose properties (ownership,
async, zero-cost abstractions) are a natural fit for what a harness actually does.

The skeleton is deliberately small and runnable at each phase (see BACKLOG), but
every layer is a documented seam for growth: web gateways, richer tools, persisted
trajectories, multi-agent composition.

## Conceptual grounding

Rusty Keys rests on the same premise as Keystone:

```
C_system = F(C_model, C_harness, C_environment, T)
```

Capability is an emergent property of the whole runtime system. Once an agent has
tools, a wrong inference is no longer a bad sentence but a destructive *action*;
the harness exists to constrain, observe, and verify that action.

**Ashby's Law of Requisite Variety** justifies the harness as variety-reduction: a
regulator must have at least as much variety as the system it governs. An
unconstrained model produces effectively infinite possibilities; the harness
supplies the necessary reduction via state tracking, tool permissions, and
deterministic checks.

## Why Rust

The harness is a runtime substrate — it mediates every model action, enforces
policy on every tool call, persists every observation. These responsibilities are
not LLM-bound (slow, network I/O); they are execution-bound (fast, local). Rust's
ownership model makes the constrain layer provably correct, its async runtime
(tokio) makes the LLM I/O genuinely concurrent, and its zero-cost abstractions
mean the harness adds no overhead in the hot path.

The LLM calls are network I/O in any language. The harness layers that wrap them
benefit from being in a language where resource management, thread safety, and
compile-time correctness are the defaults.

## LLM provider: aisdk

[aisdk](https://aisdk.rs) is the Rust equivalent of LiteLLM: provider-agnostic,
73+ providers, native tokio async, streaming, structured output, and a `#[tool]`
proc macro that generates JSON schema from Rust function signatures. It replaces
both LiteLLM and the manual `Tool` dataclass from Keystone.

Model identity is still a config string (`RUSTYKEYS_MODEL`). Swapping providers
requires no code changes.

## Goals

- Realise the kernel/harness split in Rust with Session-first architecture.
- Provider-agnostic via aisdk; model is a config string.
- Native async: every LLM call is `await`; post-turn work runs concurrently.
- Human-like memory: capture → reflection → recall across sessions.
- Local-first storage; runs on an LLM API key alone.
- Minimal: no speculative abstractions; each seam earns its place.

## Non-goals (current skeleton)

- No production auth, multi-tenant, or TLS termination yet.
- No distributed vector index (brute-force or DuckDB native for now).
- No wall-clock scheduler (consolidation is event-driven; see BACKLOG).
- No streaming output to the CLI yet (Phase 5).

## Session-first architecture

The central architectural difference from Keystone is that the AI loop is a
**first-class object**, not code interleaved with `input()` calls.

```rust
pub struct Session { ... }

impl Session {
    pub async fn send(&mut self, message: &str) -> (String, VerificationReport);
}
```

`Session::send()` owns the full turn cycle: observe → orient → kernel → compose.
The CLI is a thin adapter over `Session`. Any future gateway (web, desktop, API)
is a different adapter over the same `Session`. The kernel thread architecture
that was aspirational in Keystone is the default here: `Session` runs on a
`tokio` task; the CLI communicates via `mpsc` channels.

## Component map

| # | Component | Crate/Module | One-liner |
|---|-----------|-------------|-----------|
| 01 | kernel | `kernel` | aisdk agent loop: Decide + Act. |
| 02 | constrain | `constrain` | Policy that vets every tool call before dispatch. |
| 03 | feed | `feed` | Context, tools (`#[tool]`), memory, Task State. |
| 04 | observe | `observe` | Structured episode trace + intervention logger. |
| 05 | compose | `compose` | Verification, failure attribution, evidence journal. |
| 06 | app | `app` | Session struct + CLI adapter. |
| 07 | config | `config` | Runtime config from environment. |

## Architecture Decision Log

### ADR-001 — Rust as implementation language
The harness is execution-bound, not LLM-bound. Rust's ownership model enforces
correct resource handling at compile time; tokio gives native async I/O for the
LLM calls; zero-cost abstractions mean the substrate adds no overhead. **Trade-off:**
higher barrier to entry and longer compile times than Python; justified by
correctness guarantees and the performance headroom for future scale.

### ADR-002 — aisdk as LLM provider abstraction
aisdk gives 73+ providers, native async, streaming, and a `#[tool]` proc macro
— a complete replacement for LiteLLM + the manual `Tool` struct from Keystone.
Model identity remains a config string. **Trade-off:** aisdk is newer than LiteLLM
and has not accumulated the same production edge-case coverage; watch for rough
edges in provider normalization.

### ADR-003 — tokio as async runtime
aisdk is tokio-native; the rest of the harness (SQLite, file I/O) uses tokio's
`spawn_blocking` where needed. All LLM calls are `await`; post-turn work
(criteria judge + consolidation) runs concurrently via `tokio::join!`. **Trade-off:**
async propagates through the call stack — every layer that touches an LLM call
must be `async fn`. Accepted as the correct default for a network-I/O-heavy system.

### ADR-004 — Session-first, not REPL-first
`Session::send()` owns the full turn cycle and is transport-agnostic. CLI,
web, and other gateways are thin adapters. **Why:** in Keystone the UI loop and
AI loop were interleaved in `main()`; the split was aspirational. Here it is the
starting point. **Trade-off:** slightly more structure upfront; justified by the
gateway reuse the comment in Keystone's `build_kernel()` already promised.

### ADR-005 — Harness decomposed into constrain / feed / observe / compose
Same four-verb decomposition as Keystone. Each verb has one obvious module; every
cross-cutting concern has a stable home. **Trade-off:** modules are thin at phase 1;
accepted as intentional placeholders with documented seams.

### ADR-006 — `#[tool]` proc macro for tool registration
Tools are Rust functions annotated with `#[tool]`; aisdk generates the JSON
schema from the function signature. **Why:** eliminates the manual schema
authorship that Keystone's `Tool` dataclass required and makes tool signatures
type-safe at compile time. **Trade-off:** proc macros add compile-time complexity;
aisdk owns this cost, not the harness.

### ADR-007 — Policy vets tool calls before dispatch; errors returned, not panicked
`Policy::before_tool()` runs synchronously before the aisdk dispatcher.
Violations are returned as `Err(PolicyError)` and surfaced to the model as a
`BLOCKED` string — the model can recover rather than the process crashing.
**Trade-off:** the model sees error text (prompt surface); acceptable.

### ADR-008 — Memory is the Observe + Orient half of the OODA loop
Same OODA framing as Keystone. Short-term stream captures every event (Observe);
recall assembles working memory each turn (Orient); kernel is Decide + Act;
outputs feed back as observations. **Trade-off:** couples memory's mental model
to OODA — embraced deliberately.

### ADR-009 — Tiered consolidation: idle / sleep / explicit
Distillation of short-term → long-term at three tempos: micro (idle), sleep
(session end), explicit (user command). **Trade-off:** consolidation quality
depends on an async aisdk call; token cost per consolidation.

### ADR-010 — Pluggable storage via Rust traits
`Stream` and `Store` are traits; SQLite (`rusqlite`) is the default
implementation. DuckDB (`duckdb-rs`) is an optional feature for vector search.
**Trade-off:** trait objects add a vtable; negligible compared to LLM latency.

### ADR-011 — Skills exempt from pruning
Lessons learned from mistakes are stored as `skill` memories and are not subject
to decay-based pruning. Importance decay still reduces recall priority; skills
are never deleted. Skill grooming (refine / merge / split) is the release valve.

### ADR-012 — Post-turn compose runs concurrently
After the kernel returns a reply, the criteria judge and idle consolidation are
independent. They run via `tokio::join!` while the reply is already in the
caller's hands. **Why:** in Keystone, the sequential post-turn LLM calls added
visible latency. With async, both calls overlap with the user reading the reply.
**Trade-off:** if consolidation fires before the criteria judge completes, it may
miss the judge's learning signal — mitigated by joining both before observing.

### ADR-013 — Verification carries its limits
`VerificationReport` always includes a `limits` field describing what the checks
did *not* verify. A "verified" result is never read as more than it is. The
`CriteriaJudge` check, when active, upgrades `limits` from "deterministic only"
to "LLM-judge on active task criteria included".

### ADR-014 — Intervention Logger + M-HIR in observe layer
Human interventions (task overrides, manual consolidations, unverified followups)
are recorded to `.rustykeys/interventions.jsonl`. M-HIR (Missing-Harness Human
Intervention Rate) is computed as `interventions / total_turns`. A rising rate
signals harness gaps; a falling rate signals improvement.

### ADR-015 — Evidence journal is append-only JSONL
Every turn's verification package and every consolidation changelog are appended
to `.rustykeys/evidence.jsonl`. Completion is an auditable record, not a claim.
Rotation/retention is a future seam.

## Maturity self-assessment (H0–H3)

| Level | Definition | Rusty Keys target |
|---|---|---|
| H0 | Task description, repository files | Phase 1 baseline |
| H1 | Tool registry, tool-usage protocol | Phase 1 ✓ |
| H2 | Project memory, Task State, context-selection | Phase 3–4 |
| H3 | Deterministic checks, attribution, verification protocol | Phase 2 |

Phase 1 targets H1. Phases 2–4 complete H3 (including semantic criteria judge)
and H2 (Task State). The full H3 self-improvement loop — Reproduce → Attribute →
Fix → Verify → Report — is the same loop as Keystone, realised natively in async
Rust.

## Relationship to Keystone

Rusty Keys is not a port — it is a redesign informed by Keystone's lessons. The
harness philosophy, OODA framing, H0–H3 maturity model, and four-verb
decomposition are identical. What changes:

| Keystone | Rusty Keys |
|---|---|
| Python, synchronous | Rust, async (tokio) |
| LiteLLM | aisdk |
| Manual `Tool` dataclass + JSON schema | `#[tool]` proc macro |
| REPL-first (`main()` owns the loop) | Session-first (`Session::send()`) |
| Aspirational thread architecture | Native async by default |
| Streaming not implemented | `stream_text()` available (Phase 5) |
