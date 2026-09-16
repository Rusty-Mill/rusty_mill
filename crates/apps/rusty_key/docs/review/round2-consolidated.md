*Point-in-time working document — the consolidated synthesis of Round 2 (nine external references assessed by the five personas). It dedupes the five round-2 findings into one recommendation set and flags the decisions that need the owner. Constraint honoured throughout: **stay Rust + AI-first** (adopt patterns, not non-Rust code). This is a review artifact; **nothing here has been applied to the canonical docs yet** — applying is the follow-up step, pending owner approval.*

# Round 2 — consolidated recommendations

## Verdict matrix (per persona → overall)

| Source | Sys | SW | AI | Harness | Integ | **Overall** |
|---|---|---|---|---|---|---|
| Anthropic — *How we contain Claude* | ADOPT* | WATCH | note | ADAPT | ADAPT | **ADOPT (scope decision)** |
| opendocswork-mcp / **`rmcp`** | ADAPT | ADOPT | — | N/A | ADOPT | **ADOPT** |
| Microsoft SkillOpt | — | ADAPT | ADOPT | ADAPT | WATCH | **ADOPT** |
| ADHD (divergent ideation) | note | WATCH | ADOPT | WATCH | — | **ADAPT** |
| CloudWeGo Eino | ADAPT | ADAPT | — | WATCH | ADAPT | **ADAPT (patterns only)** |
| EvalMonkey | — | WATCH | note | ADAPT | WATCH | **ADAPT** |
| Rowboat | note | WATCH | ADAPT | WATCH | WATCH | **WATCH / small ADAPT** |
| NVIDIA AI-Q | ADAPT | WATCH | note | WATCH | ADAPT | **WATCH** |
| QMind | REJECT | REJECT | inspiration | REJECT | REJECT | **REJECT** |

\* systems-architect frames Anthropic as a warranted *scope expansion*, not a refinement.

## Recommendations

### ADOPT — high consensus, concrete, mostly doc-only refinements
1. **`rmcp` as the foundation of the `mcp` crate (PRD 07).** `rmcp` is the official `modelcontextprotocol/rust-sdk` — **MIT, tokio-native, v1.7.x** (verified). Re-spec PRD 07's hand-rolled JSON-RPC stdio/SSE transports as **thin adapters over `rmcp`**, keeping our namespacing, `McpPolicy`, `ApprovalGate`, and auth/TLS pins *above* it. opendocswork-mcp is **GPL-3.0 → reference layout only, never vendor**; add a `cargo deny` license gate. *(systems + software + integration consensus)* → new ADR + PRD 07.
2. **Validation-gated skills (SkillOpt).** A failure-born skill is minted as a **candidate** (no importance floor, not prune-exempt) and **promoted only after it validates** — online: a later matching turn goes VERIFIED (`validated: bool` on the memory row); offline: it survives a golden-episode regression gate. Skill grooming becomes a non-regression-gated optimizer; a human `direct_edit` un-validates a skill (Rowboat). So the self-improvement loop **cannot silently un-learn**. *(AI + software + harness consensus)* → PRD 03 (consolidation rubric, recall floor, loop) + eval-plan + data-model (`validated` column).

### ADAPT — take the pattern, reshaped for Rust + AI-first
3. **ADHD divergent→converge as a subagent strategy.** Map onto our existing `agent` tool + `SessionFactory` with **no new infra**: fan out N isolated child `Session`s (isolation = "no cross-branch context" for free) under different **cognitive-frame** identity preambles, then a mechanical **critic/converge** pass scores → clusters → deepens top-K. Offer it as a **plan-mode "explore" option**. Gate behind opt-in (≈5–10× cost). MIT — pattern only. → PRD 03/06 + BACKLOG.
4. **Anthropic threat-model additions (doc-only, low-risk).** Add to `threat-model.md`/PRD 07: **trust-boundary-before-config-parse** (defer parsing untrusted `.rustykeys/`, `checks.toml`, `mcp.toml`, `AGENT_GUIDE.md` until trust established); **egress allowlist = capability grant, not destination filter** (the approved-domain + attacker-key exfil case); **remote-vs-local MCP trust split** (remote SSE is mutable-after-approval → run against fake data; local stdio = audited host software); **tool-return inspection seam** (small-classifier hook on MCP/web returns *before* they enter context, mirroring `before_tool`); **symlink-resolution-before-path-validation** invariant; **persistent-memory-poisoning** → session-startup recall provenance; **multi-agent trust escalation** (subagent output not auto-elevated above raw tool-result trust). *(integration + harness + systems)*
5. **Chaos / resilience eval tier (EvalMonkey).** Add a fault-injection tier at the `ToolOutcome`/`FakeLanguageModel` seam (corrupt tool results, inject timeouts/schema corruption) and a **resilience companion to M-HIR** whose core assertion is **honest degradation, never verified-success-on-fault**; plus **failure-trace → golden-episode synthesis** from the 8-trace package (human-gated), tying into the Attribution→skill loop. → eval-plan + testing-strategy.
6. **Eino patterns (Go — patterns only).** Pin the `on_event` hook to a fixed **`KernelEvent` enum** shared by `Tracer` + the OTel seam (Eino callback aspects); lift **interrupt/resume state-persistence** as the model for plan-mode/subagent suspension. **Reject** a generic typed node-graph runtime for v1 (our turn cycle is a fixed pipeline; the verify/eval layer is our differentiator Eino lacks). → coding-standards + ARCHITECTURE; record as a multi-agent reference in BACKLOG.
7. **Observability wiring.** Realize the OTel seam as **one pull-based OTLP exporter** bound to the `KernelEvent` stream, carrying token/cost/latency attributes (AI-Q Phoenix); pull-based so a future `sandboxed` profile doesn't blind operators (the article's VM-blocked-EDR lesson). → ARCHITECTURE/observe (keep as a seam/backlog).

### WATCH — track, revisit when the triggering phase arrives
- **AI-Q** config-as-data / eval harnesses / Phoenix — revisit at the eval + observability work; note tension with our typed-trait wiring.
- **Rowboat** editable/inspectable memory — mostly already in our desktop memory browser; the only new hook (direct_edit un-validates a skill) folds into rec 2.
- **Eino node-graph** — revisit if/when multi-agent orchestration graduates from the seam.

### REJECT
- **QMind** — experimental personal repo (12★), quantum-metaphor reasoning. Only the *contradiction-handling* idea is borrowable as a consolidation prompt rule; ignore the 5-tier memory (our 3-tier is deliberate). Not a credible dependency or architecture source.

## The one scope-expansion that needs an explicit decision

**Capability isolation (Anthropic's central lesson: "supervise what the agent *can* do, not what it does").** Today our constrain layer is bash-pattern checkers + a canonicalized workspace boundary + redaction + egress rules — all *in-process* and bypassable if a checker misses (threat-model §residual-risk admits this). The systems-architect proposes promoting isolation to a first-class **`ToolExecutor` isolation seam** (sits below `feed`, above the OS; does **not** change the `constrain` vetting contract) selected by a **`RUSTYKEYS_ISOLATION=none|sandboxed` runtime profile**:
- `none` (default) — today's behaviour; keeps the local-first, sub-millisecond hot path untouched.
- `sandboxed` — run tool side-effects (esp. `bash`) inside an OS sandbox (Linux-first: landlock/namespaces, or a gVisor-class target) with **network-denied-by-default** and egress enforced at the sandbox boundary; "be wary of custom components" → wrap battle-tested primitives, don't hand-roll.

This matches Anthropic's "match isolation strength to the user's capacity for oversight." It is the highest-leverage idea this round and the one genuine scope expansion — hence flagged for your call rather than baked in.

## Open decisions for the owner
1. **Capability isolation** — adopt the `ToolExecutor` + `RUSTYKEYS_ISOLATION` seam? If so, in the v1 roadmap or the post-phase backlog, and is the default `none` acceptable?
2. **Beyond-the-paper eval integrity** — add an anti-gaming row to ARCHITECTURE §12's faithfulness map (keep answer keys/benchmarks out of agent context; harness-engineer's request)?
3. **Chaos/resilience eval** — in the v1 eval-plan, or backlog?
4. **Thresholds** (resilience weighting, divergent-explore branch count/cost cap) remain product calls.

## If approved, the concrete changes would be
New ADRs: `rmcp` adoption; capability-isolation seam + `RUSTYKEYS_ISOLATION`; validation-gated skills; ADHD subagent strategy; tool-return inspection; eval-integrity/anti-gaming. Edits: `threat-model.md` (+rec 4 items), PRD 07 (`rmcp`), PRD 03 (validation-gated skills + ADHD subagent), `eval-plan.md` (chaos/resilience + integrity), `data-model.md` (`validated` column), ARCHITECTURE (`KernelEvent`, ToolExecutor seam, §12 row), `coding-standards.md` (`KernelEvent`/OTLP), BACKLOG (isolation + chaos-eval + rmcp workstreams), configuration.md (`RUSTYKEYS_ISOLATION`).
