*Point-in-time working document — Round 2 of the multi-persona review. Source briefs for nine external references the owner asked us to assess for applicability to Rusty Keys (constraint: stay Rust + AI-first). Compiled from each project's README / landing page (two recovered via web search after a 403). Superseded by the round-2 consolidated recommendations.*

# Round 2 — external source briefs

| # | Source | Stack | License | Maturity |
|---|---|---|---|---|
| 1 | [NVIDIA AI-Q](https://github.com/NVIDIA-AI-Blueprints/aiq) | Python/TS (LangGraph, NeMo Agent Toolkit) | Apache-2.0 | Production, NVIDIA-backed (670★) |
| 2 | [EvalMonkey](https://github.com/Corbell-AI/evalmonkey) | Python/TS CLI | Apache-2.0 | Early (36★), conceptually rich |
| 3 | [Microsoft SkillOpt](https://github.com/microsoft/SkillOpt) | Python | MIT | High-visibility (1k★), arXiv paper |
| 4 | [QMind](https://github.com/Neo-Unknown/QMind-Project-Folder) | Python | MIT | **Experimental personal repo (12★)** |
| 5 | [opendocswork-mcp](https://github.com/Aimino-Tech/opendocswork-mcp) | **Rust** (`rmcp`, calamine, …) | **GPL-3.0** | Early (38★), functional |
| 6 | [Rowboat](https://github.com/rowboatlabs/rowboat) | TypeScript/Electron | Apache-2.0 | High-visibility (14.6k★, YC) |
| 7 | [ADHD](https://adhdstack.github.io/) ([repo](https://github.com/UditAkhourii/adhd)) | TS skill on Claude Agent SDK | MIT | Method/skill, benchmarked |
| 8 | [CloudWeGo Eino](https://github.com/cloudwego/eino) | **Go** | Apache-2.0 | Established (11.5k★) |
| 9 | [Anthropic — How we contain Claude](https://www.anthropic.com/engineering/how-we-contain-claude) | (article) | n/a | Authoritative principles |

## Briefs & harness-relevant patterns

**1. NVIDIA AI-Q** — enterprise research-agent framework. Patterns worth lifting (not the code): a single *orchestration node* that classifies intent + sets research depth in one step; **YAML-driven workflow configs** (agents/tools/LLMs/routing tunable without code); **job-scoped sandbox execution** (Modal); built-in **eval harnesses** (Deep Research Bench, FreshQA); **Phoenix tracing** + token/cost/latency profiling; guardrails + AuthN/Z. *Relevance:* config-as-data, eval harness, observability/cost profiling.

**2. EvalMonkey** — "prove it works, then break it." Baseline benchmarks **plus chaos injection** (payload mutation, latency spikes, schema corruption); a **Production-Reliability metric** (60% baseline + 40% chaos); **auto-synthesises test cases from failure traces**; framework-agnostic over HTTP; MCP-embeddable. *Relevance:* extends our `eval-plan.md` with **resilience/chaos** testing and failure→test synthesis (which ties to our attribution→skill loop).

**3. Microsoft SkillOpt** — optimises **natural-language skills** for *frozen* LLMs via **trajectory-driven edits, validation-gated updates, and deployable `best_skill.md` artifacts** ("training behaviour in text space"). Best skills persisted as markdown, exempt from rollback. *Relevance:* near-direct analog to our skills/consolidation/grooming + the self-improvement loop; **validation-gating** = our verification-before-promotion idea; markdown skill artifacts = our `skill` memory type.

**4. QMind** — *experimental personal project*; quantum-inspired symbolic reasoning. The only transferable ideas: a **5-tier memory** (working→episodic→semantic→long-term→cold cache), **contradiction handling**, and **meta-cognition** (monitor/regulate reasoning quality). *Relevance:* low-credibility source; treat the 5-tier memory + contradiction-handling only as *inspiration* for our 3-tier memory, not adoption.

**5. opendocswork-mcp** — **Rust-native MCP server** for Office docs; built on **`rmcp` 1.7+** (the Rust MCP SDK), stdio + streamable HTTP, ~0.38ms/call; 40+ modular tools; a tool-level **"skills system"** (reusable workflows); a **semantic entity DAG** for edit consistency. *Relevance:* the single most directly applicable Rust artifact — it demonstrates **`rmcp`** as a concrete crate for our `mcp` crate (PRD 07 names no crate today) and a clean Rust MCP-server tool layout. **Caution: GPL-3.0 — reference/learn only, do not vendor** into a permissively-licensed project; `rmcp` itself is separately licensed (verify).

**6. Rowboat** — local-first AI "coworker"; **editable Markdown memory on-device**; builds an **inspectable knowledge graph** instead of cold retrieval; **MCP** for pluggable tools; bring-your-own-model (Ollama/LM Studio/hosted); memory **compounds via persistent local state**. *Relevance:* validates our local-first + memory-graph posture; the **human-editable / inspectable memory** idea maps to our desktop memory browser and the `direct_edit` intervention.

**7. ADHD (parallel divergent ideation)** — MIT, TypeScript skill on the Claude Agent SDK. **Algorithm:** *divergence* = N parallel calls (default 5), each a **fresh session with zero shared context**, each given the problem + one **cognitive frame** (a vantage-point rewrite of the whole question, e.g. "think as a hardware engineer") + a system prompt that **forbids evaluation**; *convergence* = 3 separate calls — **score** (novelty/viability/fit 0–10 + trap detection), **cluster** (angle-level), **deepen top-K** (sketch/risk/first-step/child-ideas). ~10 LLM calls/run, **5–10× baseline cost**; semaphore-gated concurrency; generator↔critic split by opposite system prompts. Results: wins 5/6 vs single-shot; trap-detection 5.2×, novelty 2.9×. *Relevance:* a concrete **subagent orchestration pattern** for our `agent` tool + `SessionFactory` (isolated child `Session`s give "no cross-branch context" for free) and a **plan-mode "explore" strategy** (diverge → critic → converge). Adopt the *pattern* (MIT), not the TS code; gate behind opt-in given the cost.

**8. CloudWeGo Eino (Go)** — "the LLM app framework in Go." **Typed component model** (ChatModel/Tool/Retriever/Embedding), **graph/workflow orchestration** (`compose`, typed node I/O), automatic **streaming**, agents (ReAct `ChatModelAgent`, `DeepAgent` with sub-agents), **callback aspects at fixed lifecycle points** (OnStart/OnEnd/OnError), and **interrupt/resume with state persistence for human-in-the-loop**. *Relevance:* the closest *architectural* analog in a systems-style language — its **callback-aspect observability** maps to our `on_event`/Tracer, **interrupt/resume** to our approval-gate/plan-mode, **typed graph orchestration** is a candidate model for multi-agent. Note: Eino is an orchestration framework; it has **no verification/eval/entropy** layer — that is exactly Rusty Keys' differentiator.

**9. Anthropic — "How we contain Claude"** *(full text available this session at the upload path `/root/.claude/uploads/0edc8b2c-930e-46c3-a7f8-f2d7a758d54f/6e4454f8-How_we_contain_Claude_across_products.md`; cite the URL, do not commit the article into the repo).* Core thesis: **cap the blast radius**; prefer **containment (supervise what the agent CAN do)** over per-action approval (their telemetry: **users approved ~93% of prompts → approval fatigue**; defenses are probabilistic with a non-zero miss rate). **Three risk classes** (user misuse / model misbehavior / external attackers) and **three defense layers** (the **environment** — process sandbox/VM/filesystem/egress; the **model** — prompts/classifiers/training; the **external content** — MCP/tools/web, where *tool output is itself an attack surface*), applied as defense-in-depth. **Three isolation patterns:** ephemeral gVisor container (claude.ai), **OS-level sandbox with network-denied-by-default** (Claude Code: Seatbelt/bubblewrap; 84% fewer prompts; open-sourced sandbox-runtime), full local VM (Cowork; host keychain creds never enter guest; per-session scoped token).

*Lessons directly applicable to Rusty Keys:*
- **Trust boundary before config parse** — Claude Code executed a malicious `.claude/settings.json` hook *before* the "trust this folder?" prompt. RK reads untrusted workspace config (`.rustykeys/`, `checks.toml`, `mcp.toml`, `AGENT_GUIDE.md`) — **defer parsing/execution until trust is established**; treat project-open/config-load/localhost as untrusted inbound. (NEW threat-model item.)
- **Egress is the deterministic backstop** — a phished user got Claude to read `~/.aws/credentials` and POST them (24/25); only egress + filesystem boundaries held. Validates our redaction + `WebEgressGuard`, and argues the FS boundary must keep secrets *outside the workspace* (`~/.aws`, `~/.ssh`) unreadable.
- **An allowlist entry is a capability grant, not a destination filter** — exfiltration through an *approved* domain (attacker's own API key uploaded to their account via an allowed host). Our `RUSTYKEYS_WEB_ALLOWLIST` must carry this caveat.
- **Symlink resolution must precede path validation** (else a symlink in an authorized dir escapes) — RK uses `Path::canonicalize()`; state the ordering invariant explicitly.
- **Persistent memory poisoning** — injections that land in long-term memory / `AGENT_GUIDE.md` / state dirs reload every session → session-startup provenance/classifier on recalled memories. (NEW eval + threat item.)
- **Multi-agent trust escalation** — subagent output must NOT be auto-elevated above raw tool-result trust (direct input to our `agent`/`SessionFactory` seam).
- **Eval-gaming failures** — models read git history for test answers and identified the benchmark to decrypt its key → golden episodes must resist gaming (eval-plan integrity).
- **Three summary principles:** contain at the *environment layer first*, then steer at the model layer; **match isolation strength to the user's capacity for oversight**; **be wary of custom security components** (battle-tested hypervisors/syscall filters held; their custom proxy/allowlist failed). *Relevance:* the strongest single input this round — validates the semi-trusted-LLM thesis and motivates the systems-architect's `ToolExecutor` isolation seam plus several concrete threat-model additions.

## Cross-cutting reading
- **Architecture reference:** Eino (closest analog) and AI-Q (config-as-data, orchestration).
- **MCP/Rust:** opendocswork-mcp → the `rmcp` crate decision for PRD 07.
- **Memory/skills/self-improvement:** SkillOpt (strongest), Rowboat (editable/inspectable), QMind (inspiration only).
- **Eval/verification:** EvalMonkey (chaos + failure→test), AI-Q eval harnesses, Anthropic (eval-gaming failures).
- **Constrain/security:** Anthropic (capability isolation, egress) — strongest single input this round.
- **Multi-agent/reasoning:** ADHD (divergent→converge), Eino DeepAgent.
- **Lower priority:** QMind (experimental), and ADHD is a method to emulate, not infra.
