*Point-in-time working document — Round 2 of the multi-persona review, **systems-architect lens**. Assesses the nine external references (briefs + cautions in [`round2-sources.md`](./round2-sources.md)) for what to expand or refine in Rusty Keys' **system structure, composition seams, deployment topology, and threat model** — under the hard constraint: stay Rust + AI-first, learn patterns not code. Superseded by the round-2 consolidated recommendations.*

# Round 2 — Systems Architect

## 1. Scope & lens
System integrity through composition: the multi-agent seam (`SessionFactory`, plan-mode interrupt/resume), config-as-data, deployment topology, and — the headline — whether **capability-isolation** is a warranted scope expansion to [`threat-model.md`](../architecture/threat-model.md) §9. I read ARCHITECTURE, data-model, threat-model; I do **not** re-litigate Round 1 (DAG, concurrency, SQLite WAL, versioning are settled).

## 2. Per-source verdict

| source | verdict | what to take through my lens | target RK doc | priority |
|---|---|---|---|---|
| **Eino** (Go) | **ADAPT** | Typed-graph `compose` validates our linear seam; lift **interrupt/resume + state-persistence** as the model for plan-mode and subagent suspension, and **callback-aspects at fixed lifecycle points** as the contract our `Tracer`/`on_event` already implies. Do **not** adopt a generic node-graph runtime — RK's turn cycle is a fixed pipeline, not a user-authored DAG; its verify/eval layer is the differentiator Eino lacks. | ARCHITECTURE §6/§7; ADR-0017 (note); new ADR (interrupt/resume) | P2 |
| **NVIDIA AI-Q** | **ADAPT** | **Config-as-data**: `checks.toml`/`mcp.toml` already do this; generalize to a single declared **`harness profile`** (level + isolation + tool/MCP allowlists) so topology is tunable without code. **Job-scoped sandbox** reinforces the Anthropic recommendation below. The "single orchestration node" (classify intent + set depth in one step) is an `app`-layer pattern, not infra. | ARCHITECTURE §9 (profile); configuration.md | P2 |
| **Anthropic — contain Claude** | **ADOPT (as documented scope expansion / v1 intent)** | "Supervise what the agent CAN do, not what it does." Promote OS-level **capability-isolation** from threat-model §9 *non-goal prose* to a **named, optional isolation seam + runtime profile** with explicit egress enforcement. This is the strongest single systems input this round. (Rec 1.) | threat-model §9 → new §; ARCHITECTURE §9; new ADR | **P1** |
| **opendocswork-mcp** (Rust) | **ADAPT** | A concrete Rust data point that **`rmcp` 1.7+** is the production-credible crate for our `mcp` crate (PRD 07 names none) — stdio + streamable HTTP, sub-ms/call. Adopt the *crate decision* and its clean modular tool layout; **GPL-3.0 → reference only, never vendor**; verify `rmcp`'s own (MIT/Apache) license before depending. (Rec 2.) | PRD 07; new ADR | **P1** |
| EvalMonkey | WATCH | Chaos/resilience + failure→test synthesis — verification persona's call; systems-relevant only as a future eval *topology* (HTTP-embeddable harness). | eval-plan.md (not mine) | P3 |
| Microsoft SkillOpt | WATCH | Markdown-skill optimization + validation-gating — memory/self-improvement persona; no structural change. | PRD 03 (not mine) | P3 |
| QMind | REJECT | Experimental 12★ personal repo; 5-tier memory is inspiration at most. No systems impact. | — | — |
| Rowboat | WATCH | Local-first + inspectable memory graph validates our posture (already our design); no new seam. | — (validation) | P3 |
| ADHD | WATCH | Divergent→converge is a *method* atop the existing `agent`/`SessionFactory` seam — needs no new infra, just confirms the seam must support N parallel context-isolated children. | ARCHITECTURE §7 (note) | P3 |

## 3. Recommendations

### Rec 1 — Capability-isolation as an optional seam + runtime profile *(proposal / v1 intent)*
**What.** Today's controls are application-level: the canonicalized `WorkspacePolicy`, five heuristic bash checkers, SSRF block-set, redaction (threat-model §1–§8). §9 already, honestly, calls OS isolation a **non-goal** and names "run inside an externally-provided sandbox" as the mitigation. Anthropic's containment work pushes one step further than "leave it to the operator": make capability-isolation a **first-class, optional seam RK owns the contract for**, selectable as a deployment profile rather than an undocumented external wrapping.

Concretely, propose a `ToolExecutor` seam (where `bash`/`edit_file` side effects are actually performed) with two implementations: the v1 **in-process** executor (today's behavior) and a **sandboxed** executor (subprocess under an OS confinement mechanism — gVisor/landlock/namespaces on Linux, as available) selected by a new `RUSTYKEYS_ISOLATION` profile (`none` (default) | `sandboxed`). Egress enforcement (the SSRF block-set in §4) becomes a **network policy at the sandbox boundary** in `sandboxed` mode, not just an in-process URL check the model could route around via `bash`. The seam *lives below* `feed`'s tool suite and *above* the OS — it does not touch the `constrain` policy contract (vetting still happens first); it changes only *where the vetted call runs*.

**Why.** The threat-model's own §9 residual risk — "a checker bypass is a full-privilege bypass" — is exactly the failure class Anthropic documents (models that escaped a sandbox to finish a task, read git history for test answers, identified the benchmark to decrypt its key). Those are "agentic capabilities operating without adequate goal constraints," not bugs. RK's `bash` checkers are requisite-variety reduction over a steerable model, but a single obfuscation defeats them; an OS boundary degrades gracefully where a regex cannot. Making it a *seam* keeps local-first ergonomics (default `none`, sub-ms hot path untouched) while giving untrusted/`--gateway`-exposed deployments a real boundary.

**What it changes.** New ADR ("ToolExecutor isolation seam; `RUSTYKEYS_ISOLATION` profile; egress enforced at the sandbox boundary in `sandboxed` mode", Status: Proposed); threat-model §9 rewritten from flat non-goal to **"non-goal in `none`; available in `sandboxed`"** with the trade-off table; ARCHITECTURE §9 gains an isolation column in the topology matrix and a feature flag (`sandbox`, off by default); configuration.md adds `RUSTYKEYS_ISOLATION`. Pairs naturally with `PermissionMode::Bypass` (§7) — bypass-inside-a-sandbox is the coherent CI story the §9 mitigation already gestures at.

**Rust + AI-first note.** Rust has credible building blocks (`landlock`, `cap-std`/`cap-std`-style capability handles, `nix` for namespaces, subprocess confinement); gVisor is an *external* runtime RK targets, not vendors. The seam stays a trait (`ToolExecutor`) consistent with ADR-0024 (trait objects at plugin seams). AI-first: the point is precisely that the *agent* is the semi-trusted principal — isolation supervises capability, not output.

**Caution.** Real OS sandboxing is platform-specific (Linux-first; macOS/Windows are partial or out-of-scope for v1) and adds a subprocess hop + IPC cost on every confined tool call — hence opt-in, off by default, and explicitly v1-*intent* not v1-*commitment*. Anthropic's article is authoritative *principles* (no license/code to lift); we adopt the framing, not an implementation.

### Rec 2 — Pin `rmcp` as the `mcp`-crate dependency *(refine; resolves an open PRD-07 gap)*
**What.** PRD 07 specifies the `McpClient`/`McpToolFn` traits, transports (stdio + SSE), auth, and reconnect — but **names no crate**. opendocswork-mcp is a functional Rust MCP server built on **`rmcp` 1.7+** (the Rust MCP SDK) doing exactly our two roles (stdio + streamable HTTP) at ~0.38ms/call. Adopt `rmcp` as the named dependency for both the client and the `--mcp` server, and mirror its modular per-tool layout.

**Why.** Removes a load-bearing unknown before Phase 12 and de-risks the MCP transport with a production data point in our own language; the perf number confirms MCP dispatch won't dominate the turn cycle.

**What it changes.** PRD 07 (name `rmcp` in the client/server sections; reconcile our hand-rolled `StdioMcpClient`/`SseMcpClient` structs against what `rmcp` provides — likely thinner wrappers); new ADR ("`rmcp` as the MCP SDK", Status: Proposed); ARCHITECTURE §9 feature-flag matrix (`mcp` feature already lists "jsonrpc, reqwest" → replace with `rmcp`).

**Rust + AI-first note.** Directly Rust-native; no stack drift. **Licensing caution (load-bearing):** opendocswork-mcp itself is **GPL-3.0 — reference and learn its layout, do not copy/vendor** into permissively-licensed RK. `rmcp` is *separately* licensed — **verify it is MIT/Apache** before adding the dependency; if it is not, this drops to WATCH.

### Rec 3 — Generalize config-as-data into a "harness profile" *(proposal / v1 intent; small)*
**What.** AI-Q's YAML-driven workflow config and Eino's typed composition both argue the same thing: deployment shape should be **declared data**, not scattered env vars. RK already has the ingredients (`RUSTYKEYS_HARNESS_LEVEL`, `checks.toml`, `mcp.toml`, the per-mode topology). Propose a single optional **profile** binding `{ harness_level, isolation (Rec 1), tool allowlist, mcp servers, web allow/deny }` so a topology is one reviewable artifact — and so the H0–H3 controlled-visibility ablation (ADR-0028) and an isolation level are co-declared.

**Why.** Reduces the config surface area, makes the H0–H3 ablation reproducible (eval needs a single switch), and gives Rec 1's isolation level a natural home. It is consolidation, not new mechanism.

**What it changes.** ARCHITECTURE §9 (a "harness profile" note); configuration.md (the profile maps onto existing `RUSTYKEYS_*` vars — it is sugar, env vars remain the SSOT). No new crate. Defer the file format; sketch only.

**Rust + AI-first note.** Pure config/serde; no dependency change. AI-first because the profile is exactly the "variety" (Ashby) the regulator is configured with for a given agent + environment.

## 4. Cross-persona dependencies
- **Security/safety persona:** Rec 1 (capability-isolation) is jointly owned — I own the *seam placement* (`ToolExecutor` below `feed`, above OS; topology/profile) and the trade-off framing; they own the *enforcement detail* (which syscalls/egress rules, gVisor vs landlock, how it composes with the bash checkers and `Bypass`). The threat-model §9 rewrite needs both signatures.
- **Verification/observability persona:** AI-Q's Phoenix tracing/cost-profiling and Eino's callback-aspects both land on the **`Tracer`/`on_event`** contract and per-turn cost/latency — theirs to confirm; I only note the lifecycle-hook points already exist in §6.
- **Memory/self-improvement persona:** owns SkillOpt/QMind/Rowboat verdicts; my only structural ask is that the `agent`/`SessionFactory` seam (ADR-0017) support N **context-isolated** parallel children (ADHD pattern) and child suspension (Eino interrupt/resume) — a capability note, not a memory decision.
- **Integration persona:** Rec 2 (`rmcp`) overlaps PRD 07 transport/auth ownership — I assert the *crate*, they own the wire/reconnect detail; the GPL-3.0-vs-`rmcp`-license check is a shared gate.
- **Product/roadmap persona:** sequence Rec 1 as a post-MVP isolation workstream (after Phase 12 MCP, alongside/after Phase 14 gateway, since it most matters when network-exposed); Rec 2 lands *in* Phase 12.
