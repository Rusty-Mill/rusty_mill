# PRD 00 — System overview

> **What this document is.** A product brief and index. It states *why* Rusty Keys
> exists (the conceptual grounding, the bet on Rust, goals/non-goals, the
> relationship to Keystone) and points at the authoritative docs for everything
> else. It is the entry point, not the system reference.
>
> **Where the details live:**
> - System structure — component map, crate DAG, concurrency, topologies, faithfulness map: [`../ARCHITECTURE.md`](../ARCHITECTURE.md)
> - On-disk state — `.rustykeys/` tree, SQLite DDL, all schemas, versioning: [`../architecture/data-model.md`](../architecture/data-model.md)
> - Decisions and trade-offs: [`../adr/`](../adr/) (ADR-0001 … ADR-0028)
> - Runtime configuration — every `RUSTYKEYS_*` var: [`../reference/configuration.md`](../reference/configuration.md)
> - Per-component depth: the sibling PRDs [`01-kernel`](01-kernel.md) · [`02-constrain`](02-constrain.md) · [`03-feed`](03-feed.md) · [`04-observe`](04-observe.md) · [`05-compose`](05-compose.md) · [`06-app`](06-app.md) · [`07-mcp`](07-mcp.md) · [`08-frontend`](08-frontend.md)
> - Phasing / roadmap: [`../../BACKLOG.md`](../../BACKLOG.md)

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

## Component map (summary)

The harness is a Cargo workspace of **eight crates** plus a (non-crate) desktop
`frontend/`. At a glance: `config` (settings) · `observe` (trace, interventions,
entropy) · `constrain` (policy, security, approval) · `feed` (tools, memory, Task
State) · `kernel` (the aisdk agent loop) · `mcp` (MCP client + server) · `compose`
(verification, attribution, evidence) · `app` (the `Session` and its CLI / gateway /
MCP / Tauri adapters).

> The original overview listed seven crates and omitted `mcp`; it is a real crate
> (PRD 07). **[ARCHITECTURE.md §4](../ARCHITECTURE.md#4-logical-view--components)
> is the authoritative component map**, with §5 the crate dependency DAG and import
> rules and §3 the H0–H3 maturity model. The per-crate PRDs (01–08) carry the depth.

## Architecture decision log

The 15 decisions that were inlined here have been extracted to [`../adr/`](../adr/)
as **ADR-0001 … ADR-0015** (Rust, aisdk, tokio, Session-first, the four-verb
decomposition, `#[tool]`, policy-before-dispatch, memory-as-OODA, tiered
consolidation, pluggable storage, skill-exemption, concurrent compose,
verification-carries-limits, intervention logger + M-HIR, append-only evidence).
The five-persona review added **ADR-0016 … ADR-0028** (async `before_tool`,
`SessionFactory`, faithfulness divergences, error model, serde and versioning
conventions, redaction, H0 selectability). The ADR directory is the single source
of truth for rationale and trade-offs; this PRD no longer restates them.

## Maturity model (summary)

Rusty Keys climbs the paper's H0–H3 ladder across the roadmap: **H1** (tool
registry) at Phase 1, **H3** deterministic checks at Phase 2, **H2** (project
memory, Task State, context selection) at Phases 3–4, and the semantic + episode-
package layers of H3 later still. H0 (no tool registry) is the ablation floor and
is **selectable-or-eval-only** (ADR-0028). The full H3 self-improvement loop —
Reproduce → Attribute → Fix → Verify → Report — is the same loop as Keystone,
realised natively in async Rust. The authoritative ladder, with what each level
sees, is **[ARCHITECTURE.md §3](../ARCHITECTURE.md#3-maturity-model-h0h3)**.

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
