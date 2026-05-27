# Coding standards

> **Authoritative source** for the engineering baseline: MSRV and `rust-toolchain.toml`, the rustfmt + clippy lint policy, the async-trait mechanism, the trait-object-vs-generics convention, the public-API visibility policy, the Cargo `[features]` table, and the CI/CD pipeline plan. Other documents link here. The no-panic *rule* and the error taxonomy live in [`error-handling.md`](./error-handling.md); on-disk serde conventions live in [`../architecture/data-model.md`](../architecture/data-model.md) §7. Decision: [ADR-0024](../adr/0024-trait-objects-at-plugin-seams-async-trait-msrv.md) (trait objects + async-trait + MSRV); [ADR-0023](../adr/0023-error-model-thiserror-per-crate-anyhow-in-app-no-panic.md) (lints).

Pinned values below (MSRV number, lint levels, feature defaults) are **v1 intent** — the baseline to build against and revisit after the Phase 1 spike. This substrate lands **with Phase 1**.

Related: [`../ARCHITECTURE.md`](../ARCHITECTURE.md) §9 (feature matrix, topologies) · [`error-handling.md`](./error-handling.md) · [`testing-strategy.md`](./testing-strategy.md).

---

## 1. MSRV & toolchain

- **MSRV: pin ≥ 1.82**, well above the 1.75 async-fn-in-trait floor, in a committed `rust-toolchain.toml` at the workspace root, so every contributor and CI job use the same compiler:

```toml
# rust-toolchain.toml (workspace root)
[toolchain]
channel = "1.82"          # MSRV; CI also runs `stable`
components = ["rustfmt", "clippy"]
```

- The MSRV is restated as `rust-version = "1.82"` in the workspace `Cargo.toml` `[workspace.package]` so `cargo` enforces it on publish/build.
- CI's build matrix runs **both** `1.82` (MSRV) and `stable` (§7).

## 2. rustfmt

- **Default rustfmt**, checked in CI with `cargo fmt --all --check`. A minimal `rustfmt.toml` (e.g. `edition = "2021"`) only — no bikeshed config. Formatting is not a review topic; the tool decides.

## 3. clippy lint policy

Configured once in the workspace `Cargo.toml` `[workspace.lints]` and inherited by every crate (`[lints] workspace = true`). CI runs `cargo clippy --workspace --all-targets -- -D warnings`.

- **`-D warnings`** workspace-wide: no warning merges.
- **No-panic lints in library crates** (the [ADR-0023](../adr/0023-error-model-thiserror-per-crate-anyhow-in-app-no-panic.md) enforcement, detailed in [error-handling §4](./error-handling.md#4-the-no-panic-rule-enforced)): `clippy::unwrap_used`, `clippy::expect_used`, `clippy::panic`, `clippy::indexing_slicing` — `deny`. **Allowed in tests** via `#![cfg_attr(test, allow(...))]`.
- A light `clippy::pedantic` is **`warn`, not `deny`** in v1 (advisory; avoids churn on a pre-implementation skeleton). `clippy::correctness` and `clippy::suspicious` stay at their default `deny`.

Naming is already consistent in the corpus and is hereby a written convention: `XxxError` (errors), `XxxPolicy` / `XxxCheck` (seams), `rk://…` (events), `RUSTYKEYS_*` (env), `mcp__server__tool` (MCP tool ids). These are stylistic, not lint-enforced.

## 4. Async in traits ([ADR-0024](../adr/0024-trait-objects-at-plugin-seams-async-trait-msrv.md))

Many plugin-seam traits declare `async fn` — `Stream`/`Store` (PRD 03), `CriteriaJudge` (PRD 05), `McpClient` (PRD 07), `ToolFn::call` (PRD 02/07) — **and** are used as `dyn` (§5). Native async-fn-in-trait alone is not `dyn`-compatible. Decision:

- **Native async-fn-in-trait** where the trait is used by static dispatch / `impl`.
- **`trait-variant`** (`#[trait_variant::make(Send)]`) to add the `Send` bound and produce a `dyn`-compatible form for the seams that are stored as `Box<dyn …>` / `Arc<dyn …>`. This is preferred over the legacy `async-trait` macro (no per-call `Box::pin` allocation, no macro-rewritten signatures).
- Every `dyn`-used async trait must either avoid async on its `dyn` surface or go through `trait-variant`; reviewers enforce this against §5.

## 5. Trait objects vs generics

**Convention: trait objects at every plugin seam; no generic-parameterised seams in v1.** The harness is bounded by LLM and I/O latency, never a tight CPU loop, so a vtable indirection is negligible ([ADR-0024](../adr/0024-trait-objects-at-plugin-seams-async-trait-msrv.md), recording ADR-0010 concretely).

- **Dynamic at seams:** `Box<dyn ToolFn>`, `Box<dyn Check>`, `Box<dyn Policy>`, `Box<dyn SecurityCheck>`, `Box<dyn SessionFactory>`, `Arc<dyn McpClient>`.
- **`Stream` / `Store` storage form is pinned as `Arc<dyn Stream>` / `Arc<dyn Store>`** (the form the PRDs left unstated): `Arc`, not `Box`, because `Memory` is shared read-side across the post-turn `tokio::join!` tasks and `CriteriaJudge` holds `Arc<TaskStore>` ([ARCHITECTURE.md §5](../ARCHITECTURE.md#5-crate-dependency-dag)).
- **Generics** remain available for internal, non-seam hot-path code only.

## 6. Public-API visibility policy

Pre-code signature freezing is premature; a **policy** is not. The rule (not a frozen surface):

- Each crate exports the **minimal trait(s) + constructor(s)** a downstream crate names; concrete impls are `pub(crate)` unless a higher crate in the [DAG](../ARCHITECTURE.md#5-crate-dependency-dag) provably needs the type.
- Error enums ([error-handling §2](./error-handling.md#2-per-crate-error-enums)) are `pub` (callers `match` on them); internal helper errors are `pub(crate)`.
- **`app` is the only crate with a `[[bin]]`**; libraries are `lib`-only.
- Re-exports for downstream ergonomics go through an explicit `pub use` in the crate root, never a glob.

## 7. Cargo `[features]` table

Mirrors [ARCHITECTURE.md §9](../ARCHITECTURE.md#9-deployment--runtime-topologies); this is the Cargo-level convention (default set, which heavy dep each flag gates). **Runtime gates (`RUSTYKEYS_ALLOW_WEB`, `RUSTYKEYS_ALLOW_BYPASS`) are env vars, not compile features** ([configuration.md](../reference/configuration.md)).

| Feature | Default | Gates | Heavy deps |
|---|---|---|---|
| `duckdb` | off | DuckDB long-term backend (Phase 5) | `duckdb-rs` |
| `gateway` | off | axum HTTP gateway | `axum`, `tower` |
| `mcp` | on | MCP client + server | jsonrpc, `reqwest` (SSE) |
| `web-tools` | on | `web_fetch` / `web_search` (still runtime-gated by `RUSTYKEYS_ALLOW_WEB`) | `reqwest`, HTML strip |
| `frontend` | off | builds the Tauri desktop app | tauri toolchain (node/vite) |

CI must build `--no-default-features` and `--all-features` (§8) so feature combinations stay compiling.

## 8. CI/CD pipeline plan

> **Note:** the actual `.github/workflows/ci.yml` **lands with Phase 1**, not now. A workflow with no code to build would fail on every push — this section is the plan that file will implement.

A single workflow with parallel jobs; the Rust jobs gate on the build matrix:

1. **Build matrix** — `{ stable, 1.82 (MSRV) }` × `cargo build --workspace`.
2. **Format** — `cargo fmt --all --check`.
3. **Lint** — `cargo clippy --workspace --all-targets -- -D warnings` (carries the no-panic lints, §3).
4. **Test** — `cargo test --workspace` (all four tiers, [testing-strategy.md](./testing-strategy.md)).
5. **Feature builds** — `cargo build --workspace --no-default-features` **and** `--all-features`, so the [§7](#7-cargo-features-table) matrix never rots.
6. **Supply chain** — `cargo audit` (RUSTSEC advisories) and `cargo deny` (licenses + bans + duplicate-version policy).
7. **Frontend job** — a separate node/vite job for `frontend/` (Tauri 2 + SolidJS, PRD 08): `npm ci`, lint, `vite build`. Runs only when `frontend/**` changes or the `frontend` feature is exercised.
8. **Release** — tag-triggered: build the `app` binary per target, attach artifacts; crate publishing is a later seam (libraries are not published in v1).

Coverage is a future seam (not a v1 gate). The MSRV job (1) is what keeps the [§1](#1-msrv--toolchain) pin honest.

## 9. `KernelEvent` — the unified lifecycle event ([ADR-0034](../adr/0034-kernelevent-unified-observability-stream.md))

> **v1 intent.** Pins the event surface; the exact variant list lands with the `observe` crate (PRD 04/05). Pattern lifted from CloudWeGo Eino's callback aspects — adopted, not its Go code (see [research/references.md](../research/references.md#round-2-references)).

The `on_event` hook emits **exactly one type: `KernelEvent`** — a fixed enum naming the lifecycle points of the turn cycle (e.g. turn start/end, tool call start/return, error). It is the **single event surface** two consumers subscribe to, so neither owns the wire format:

- **`Tracer`** ([ARCHITECTURE §4](../ARCHITECTURE.md#4-logical-view--components), the `observe` crate) — builds the structured `Episode` from the stream; `!Send` and `Session`-owned ([§7 concurrency](../ARCHITECTURE.md#7-concurrency-model)).
- **A pull-based OTLP exporter** — the observability wiring, bound to the same `KernelEvent` stream and carrying token/cost/latency attributes ([`RUSTYKEYS_OTLP_ENDPOINT`](../reference/configuration.md), v1 intent). **Pull-based** so a future `sandboxed` isolation profile ([ADR-0030](../adr/0030-capability-isolation-toolexecutor.md)) cannot blind operators.

Convention: new lifecycle hooks add a `KernelEvent` variant rather than introducing a second event type or a consumer-specific callback. This keeps `Tracer` and exporter in lockstep over one schema.
