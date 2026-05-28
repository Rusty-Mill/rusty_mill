# Harness Assessment — Recommended Approach

Consolidated output of a four-lens review (systems, software, architecture, and
integration engineering) of the Rusty Key AI harness, benchmarked against three
published agent-harness writeups:

- VILA-Lab, *Dive into Claude Code* — <https://github.com/VILA-Lab/Dive-into-Claude-Code>
- Augment / Martin Fowler, *Harness engineering for coding agents* —
  <https://www.augmentcode.com/guides/harness-engineering-ai-coding-agents> /
  <https://martinfowler.com/articles/harness-engineering.html>
- Vrungta, *Claude Code Architecture (Reverse Engineered)* —
  <https://vrungta.substack.com/p/claude-code-architecture-reverse>

Visuals: `1_harness_anatomy.svg`, `2_turn_cycle.svg`, `3_scorecard.svg`,
`4_roadmap.svg` (this directory).

---

## 1. Corrections to the initial assessment

Two findings from the code review change the original scorecard and the priority order:

1. **Sandbox isolation is a *runtime* switch, not a build feature.** Both
   `LocalExecutor` and `SandboxedExecutor` compile in unconditionally; the
   selector is `RUSTYKEYS_ISOLATION=none|sandboxed` (`crates/feed/src/exec.rs`,
   `crates/config/src/lib.rs`), defaulting to `none`. "Sandbox by default" is
   therefore a config-flip + rollout decision (see ADR-0030), not new build work.
2. **`RUSTYKEYS_MAX_STEPS` is documented but never wired.** `run_turn` /
   `stream_turn` call the aisdk loop with no `stop_when`
   (`crates/kernel/src/lib.rs`), so the agent loop has no step cap, while the
   `CleanTermination` check assumes one. This is a latent runaway-loop hole and
   is promoted to **P0**.

Also notable: `/init` writes `AGENT_GUIDE.md` (`crates/app/src/main.rs`) but
nothing ever reads it — the "guides" gap is half-built.

## 2. Cross-reviewer consensus

- **Guides:** a `GuideLoader` in `feed` discovering the `AGENT_GUIDE.md`
  hierarchy (managed → user → project → local), reusing the existing
  `CheckRegistry` precedence idiom, emitting a `ContextEntry` for audit.
- **Declarative extensibility = reuse existing seams.** Skills are `.md` +
  frontmatter files loaded in `feed` (matching Claude Code's format for
  ecosystem compatibility). A hook that can **block a tool is a policy** →
  realize it as a `HookPolicy` in the existing `PolicyChain` (`constrain`);
  observe-only hooks subscribe to the existing `KernelEvent` stream (ADR-0034).
  Plugins are bundle manifests wired in `app`. The kernel stays off the
  dependency path; the acyclic DAG is preserved.
- **Loop ownership:** keep delegating to aisdk for now. Own the loop only later,
  behind a new `ContextController` / `between_steps` trait seam in `constrain`,
  so the kernel still imports neither `feed` nor `compose`.
- **Subagents:** generalize `SessionFactory` with `IsolationMode { shared |
  worktree | remote }`, and honor the per-agent `tools` subset (currently
  ignored, `crates/app/src/session.rs`).

## 3. Resolved disagreement — where guides live in the prompt

The software-engineering review argued guides belong in the **static, cached
`system` prefix** (they are session-stable, so putting them in the per-turn
block needlessly busts the prompt cache every turn). The architecture and
integration reviews argued for injecting them as per-turn `extra_context` like
recalled memory.

**Decision (ADR-0037): guides go in the cached system prefix.** Session-stable
content should sit above the prompt-cache breakpoint. We still emit a
`ContextEntry` so the episode trace records that the guide was consulted.
Machine-generated recalled memory stays in the per-turn block — that content is
correctly dynamic.

## 4. Roadmap

See `4_roadmap.svg` for the dependency view.

| Phase | Item | Effort | Rationale |
|---|---|---|---|
| **P0 — Safety floor** | Wire `max_steps`/`stop_when`; phased sandbox-by-default (warn when launcher present, flip default when available, fall back gracefully) | S + M | Prerequisite for any timeout / resource bound to mean anything |
| **P1 — Foundation** | Project config file `.rustykeys/config.toml` (env stays highest-precedence override; fold in `mcp.toml`) | S/M | Declarative extensions need lists/nesting env vars can't express |
| **P2 — Feedforward** | `GuideLoader` (read the `AGENT_GUIDE.md` hierarchy `/init` already writes) → cached system prefix | S/M | Closes the largest gap; half-built already |
| **P3 — Feedback (the ratchet)** | Aggregate attributions by `(failure_type, category)` via the existing `upsert`/skill loop; `RatchetLog` + `/ratchet` command that *proposes* (human-approved) `checks.toml` stanzas. Enforce "zero aspirational rules" in code: no log entry → no proposable check | S/M | Reuses the `FailureType` matrix + `CheckRegistry`; pairs with P2 |
| **P4 — Accuracy** | Real provider token usage feeding compaction decisions; then finer compaction tiers (data change in `tier_for`) | S + S | Tiers should fire on real tokens, not `len/4` |
| **P5 — Extensibility** | Skills (`.md` in `feed`) → Hooks (`HookPolicy` in `constrain`) → Plugins (bundles in `app`); MCP maturity (SSE transport, schema-drift re-approval, auth enforcement) | L | Built on the P1 config + policy/trust foundations |
| **P6 — Subagents** | `IsolationMode` (worktree/remote), honor `tools` subset, declarative agent types | M/L | Composes with the sandbox seam (orthogonal axes); needs the P5 file format |
| **P7 — Deferred** | Own the loop behind a `ContextController` seam — only when a real between-steps requirement lands | L | Highest risk (re-implements aisdk retry); no trigger yet |

**Sequencing notes**

- P0 is genuinely first — the unwired step cap is a real hole, and it is a
  prerequisite for P3/P4 timeouts and subagent bounds.
- P2 and P3 ship together: they are the feedforward/feedback pair the
  "harness engineering" thesis is built around, and the cheapest high-leverage
  work.
- Everything from P5 onward depends on the P1 config file.
- Subagent resource bounding (a shared concurrency semaphore, per-child
  wall-clock timeout, tree-wide token ceiling, cancellation propagation) rides
  with P0/P6; today `explore` fans children into an unbounded `JoinSet`.

## 5. New ADRs

| ADR | Decision | Status |
|---|---|---|
| [0037](../adr/0037-guides-loaded-into-cached-system-prefix.md) | Layered guides loaded at session start into the cached system prefix | Proposed |
| [0038](../adr/0038-stratified-declarative-extension-model.md) | Stratified declarative extension model — skills, hooks-as-policy, plugins | Proposed |
| [0039](../adr/0039-defer-loop-ownership-behind-contextcontroller.md) | Defer loop ownership; revisit behind a `ContextController` seam | Proposed |
