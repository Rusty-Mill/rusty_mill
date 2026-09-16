# ADR-0038: Stratified declarative extension model — skills, hooks-as-policy, plugins

- Status: Proposed
- Date: 2026-05-28
- Tags: feed, constrain, config, extensibility, integration, roadmap

## Context

Today the only recompile-free extension mechanism is MCP (ADR-0029, the `mcp`
crate). The benchmarked Claude Code writeups describe a *stratified* extension
model — Hooks → Skills → Plugins → MCP — defined declaratively in `.md` / `.json`
rather than code, stratified by context cost. The harness-assessment review
(`docs/assessment/RECOMMENDATIONS.md`) flags the missing declarative layer as a
top gap, with the strong constraint that new mechanisms must not violate the
acyclic crate DAG or the kernel's "knows nothing about feed/compose" invariant
(ARCHITECTURE §5), and must route capability grants through `constrain`.

Note: RK already has *generative* skills (memory-promoted, ADR-0011/0031). Those
are distinct from CC's *authored* skills and must not be conflated.

## Decision

Map each stratum onto an **existing seam** rather than inventing a parallel
system:

- **Skills (authored):** `.md` files with YAML frontmatter
  (`name`, `description`, optional `allowed-tools`), matching Claude Code's
  Agent-Skills format for ecosystem compatibility, discovered under
  `.rustykeys/skills/`. Loaded in **`feed`** (same orient/budget seam as guides,
  ADR-0037); a skill becomes injected context or a synthetic `ToolFn` registered
  via `ToolRegistry`. A skill's `allowed-tools` may only **intersect**, never
  widen, the active `PermissionMode`.
- **Hooks:** declared in `.rustykeys/hooks.toml`. Observe-only hooks subscribe to
  the existing `KernelEvent` stream (ADR-0034) — no new contract. A hook that can
  **block or mutate a tool call is a policy** and is compiled into a
  **`HookPolicy` implementing `constrain::Policy`**, appended to the
  `PolicyChain` and fail-closed (a non-zero exit becomes a `PolicyError`). Hooks
  must **not** acquire an execution side-channel that bypasses `before_tool`, and
  hook commands run through the `ToolExecutor` / isolation seam (ADR-0030), not
  around it.
- **Plugins:** a bundle manifest (skills + hooks + MCP refs) — a packaging
  concern owned by **`app`** wiring; no new low-crate seam.

Declarative specs are parsed in **`config`** (the leaf crate) and realized into
their target crate (`feed` for skills, `constrain` for `HookPolicy`). Third-party
(non-local) extensions require explicit opt-in via a `[trust]` allowlist,
mirroring MCP first-use approval. Frontmatter carries a `schema_version`; unknown
major versions are rejected.

DAG flow: `config` (parse) → `constrain` (`HookPolicy`) / `feed` (skills) → `app`
(assembly). The kernel is untouched.

## Consequences

- Adds the missing declarative extension layer while preserving acyclicity and
  the kernel invariant; no new crate is strictly required.
- The critical boundary call is explicit: capability-granting hooks are policies
  behind the `constrain` contract — this is the acyclicity/safety threat to guard.
- `.md`+frontmatter compatibility lets users reuse community skill/agent
  definitions — high leverage for low cost.
- Hooks execute arbitrary shell; risk is contained by routing through the
  isolation seam and the trust allowlist. Skills-first sequencing (cheap,
  high-value) lets hooks land behind a hardened boundary.
- Depends on the project config file (`.rustykeys/config.toml`) for declaration.
