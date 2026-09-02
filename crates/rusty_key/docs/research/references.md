# Research & references

> **Authoritative source** for the external material Rusty Keys draws on — the
> grounding paper, its lineage, the comparative implementations it learns from, the
> protocols and concepts it implements, and the technology stack it is built on.
> Where a link came from the repo it is cited verbatim; comparative-implementation
> identifiers are reproduced as cited in [`../../BACKLOG.md`](../../BACKLOG.md).

---

## 1. Primary research

**Zhong, H. & Zhu, S. — *AI Harness Engineering*.** arXiv preprint `2605.13357v1`, CC-BY 4.0.
- Local copy: [`2605.13357v1.pdf`](./2605.13357v1.pdf)
- arXiv: https://arxiv.org/abs/2605.13357 · DOI: https://doi.org/10.48550/arXiv.2605.13357

The foundational reference for the whole project. It supplies the **H0–H3 maturity
ladder**, the **episode package** (8 traces) as the unit of evaluation, the
**M-HIR** metric, the **entropy audit** concept, the **five-label outcome taxonomy**,
the fixed **failure-attribution taxonomy**, and the *reproduce → attribute → fix →
verify → report* workflow. The faithfulness map in
[`../ARCHITECTURE.md` §12](../ARCHITECTURE.md#12-faithfulness-to-the-research-paper)
tracks where each concept is realized and where Rusty Keys deliberately diverges
(ADR-0018/0019/0020/0028).

> **Source now machine-readable (Round 3).** The original build environment lacked PDF
> tooling, so the first-pass faithfulness assessment relied on a degraded zlib
> `FlateDecode` recovery (stripped spaces/ligatures). The paper now extracts cleanly with
> PyMuPDF to [`2605.13357v1.txt`](./2605.13357v1.txt), and the three previously-unconfirmed
> details are **confirmed verbatim**: the 7 entropy categories + 0–3 severity (p.10),
> `M-HIR = missing-harness interventions / total episodes` (p.4), and the intervention-log
> fields avoidability / burden / harness-gap (p.10). ADR-0018/0019/0020 are accordingly
> **Accepted** (0028 kept Proposed but broadened to the ladder's R1/R5 question); see the
> Round-3 audit in [`../review/round3-consolidated.md`](../review/round3-consolidated.md).

## 2. Lineage & LLM provider

| Reference | Link | Role |
|---|---|---|
| **Keystone** | https://github.com/baileyrd/Keystone | The Python predecessor. Rusty Keys carries forward its harness philosophy (constrain/feed/observe/compose), OODA framing, and H0–H3 model; the *Relationship to Keystone* table in [PRD 00](../prd/00-overview.md#relationship-to-keystone) records what changed. |
| **aisdk** | https://aisdk.rs | The Rust LLM provider abstraction (73+ providers, async, streaming, `#[tool]` proc macro). Replaces LiteLLM + Keystone's manual `Tool` dataclass. See [PRD 01](../prd/01-kernel.md) and ADR-0002. |
| **LiteLLM** | (named comparison) | The Python provider library aisdk replaces; cited only as the point of comparison in PRD 00 / ADR-0002. |

## 3. Comparative implementations (cited in BACKLOG)

These are the harness/agent projects named in the BACKLOG reference table that
informed specific design choices. Identifiers are GitHub shorthand as cited; some
may be private or illustrative.

| Reference | Link | What it informs |
|---|---|---|
| `baileyrd/claude-code` | https://github.com/baileyrd/claude-code | Tool suite (53 tools), permission modes, 3-tier compaction, 5-tier memory |
| `nousresearch/hermes-agent` | https://github.com/nousresearch/hermes-agent | Skill/memory consolidation, background-fork pattern, tool guardrails |
| `crynta/terax-ai` | https://github.com/crynta/terax-ai | Frontend UI: xterm.js, CodeMirror 6 diff, Tauri 2, plan mode, approval gates |
| `harness/harness-ai` | https://github.com/harness/harness-ai | MCP-client use case (CI/CD tools via an MCP server) |

## 4. Protocols & standards

| Standard | Link | Used by |
|---|---|---|
| **Model Context Protocol (MCP), v1** | https://modelcontextprotocol.io | The `mcp` crate's client + server (JSON-RPC 2.0 over stdio/SSE). See [PRD 07](../prd/07-mcp.md). |
| **JSON-RPC 2.0** | https://www.jsonrpc.org/specification | MCP wire protocol. |
| **Server-Sent Events (SSE)** | https://html.spec.whatwg.org/multipage/server-sent-events.html | Gateway `/stream` and MCP SSE transport (PRD 06/07). |
| **SQLite FTS5** | https://www.sqlite.org/fts5.html | Lexical recall in the long-term store ([data-model §3](../architecture/data-model.md#3-long-term-store--storedb-sqlite--storeduckdb-duckdb)). |

**Example MCP servers** referenced in the PRD 07 `mcp.toml` sample (illustrative):
`@modelcontextprotocol/server-filesystem` (npm), `mcp-server-sqlite` (uvx/PyPI), and
an SSE example endpoint `https://mcp.harness.io/sse`.

## 5. Foundational concepts

| Concept | Source | Where used |
|---|---|---|
| **OODA loop** (Observe–Orient–Decide–Act) | John Boyd (military strategy) | The harness-verb ↔ OODA mapping ([ARCHITECTURE §2](../ARCHITECTURE.md#2-conceptual-foundation), ADR-0008). |
| **Ashby's Law of Requisite Variety** | W. Ross Ashby, *An Introduction to Cybernetics* (1956) | Justifies the constrain layer as variety-reduction ([PRD 02](../prd/02-constrain.md), [threat-model](../architecture/threat-model.md)). |
| **`C_system = F(C_model, C_harness, C_environment, T)`** | The grounding paper (§1) | The capability-as-emergent-property premise ([PRD 00](../prd/00-overview.md#conceptual-grounding)). |

## 6. Technology references (implementation stack)

Named across the PRDs and the engineering-substrate docs. Rust crates link to their
crates.io registry page; frameworks link to their project site.

**Rust runtime & libraries** —
[tokio](https://tokio.rs) ·
[aisdk](https://aisdk.rs) ·
[rusqlite](https://crates.io/crates/rusqlite) ·
`duckdb` ([duckdb-rs](https://crates.io/crates/duckdb), Phase 5) ·
[reqwest](https://crates.io/crates/reqwest) ·
[axum](https://crates.io/crates/axum) ·
[serde](https://serde.rs) / [serde_json](https://crates.io/crates/serde_json) ·
[thiserror](https://crates.io/crates/thiserror) ·
[anyhow](https://crates.io/crates/anyhow) ·
[proptest](https://crates.io/crates/proptest) ·
[insta](https://crates.io/crates/insta) ·
[trait-variant](https://crates.io/crates/trait-variant).

**Desktop frontend** (PRD 08) —
[Tauri 2](https://tauri.app) ·
[SolidJS](https://www.solidjs.com) ·
[CodeMirror 6](https://codemirror.net) ·
[xterm.js](https://xtermjs.org) ·
[Tailwind CSS v4](https://tailwindcss.com) ·
[Vite](https://vitejs.dev).

**Tooling & CI** ([coding-standards](../dev/coding-standards.md)) —
clippy & rustfmt ([rust-lang.org](https://www.rust-lang.org)) ·
[cargo-audit](https://crates.io/crates/cargo-audit) ·
[cargo-deny](https://crates.io/crates/cargo-deny).

**Web-search backends** (`RUSTYKEYS_SEARCH_PROVIDER`, [configuration.md](../reference/configuration.md#tools)) —
Brave Search API · Serper · DuckDuckGo.

**Voice (seam)** — OpenAI Whisper (`RUSTYKEYS_VOICE`, PRD 08 seam).

## 7. Round-2 references

The nine external sources the owner asked us to assess in Round 2 (constraint: stay
Rust + AI-first — adopt patterns, not non-Rust code), plus the one crate that round
adopted. Per-persona verdicts and the dedup'd recommendation set live in
[`../review/round2-consolidated.md`](../review/round2-consolidated.md); the source
briefs (stack/license/maturity) in
[`../review/round2-sources.md`](../review/round2-sources.md).

| # | Source | License | What we took |
|---|---|---|---|
| 1 | [NVIDIA AI-Q](https://github.com/NVIDIA-AI-Blueprints/aiq) | Apache-2.0 | **WATCH.** Config-as-data, eval harnesses, and Phoenix-style token/cost/latency profiling — informs the pull-based OTLP exporter ([ADR-0034](../adr/0034-kernelevent-unified-observability-stream.md)); revisit at the eval/observability work. |
| 2 | [EvalMonkey](https://github.com/Corbell-AI/evalmonkey) | Apache-2.0 | **ADAPT.** A chaos/resilience eval tier at the `ToolOutcome`/`FakeLanguageModel` seam + failure-trace → golden-episode synthesis; the resilience metric asserts honest degradation, never verified-success-on-fault. |
| 3 | [Microsoft SkillOpt](https://github.com/microsoft/SkillOpt) | MIT | **ADOPT.** Validation-gated skills: a failure-born skill is a candidate, promoted only after it validates (online VERIFIED turn or offline regression gate); a human `direct_edit` un-validates it. Markdown skill artifacts mirror our `skill` memory type. |
| 4 | [QMind](https://github.com/Neo-Unknown/QMind-Project-Folder) | MIT | **REJECT** (experimental personal repo). Only its contradiction-handling idea is borrowable as a consolidation-prompt rule; the 5-tier memory is declined (our 3-tier is deliberate). Inspiration, not a dependency. |
| 5 | [opendocswork-mcp](https://github.com/Aimino-Tech/opendocswork-mcp) | **GPL-3.0** | **Reference layout only — never vendor** (copyleft; enforced by a `cargo deny` license gate). Demonstrates a clean Rust MCP-server tool layout and surfaced **`rmcp`** (below). |
| 6 | [Rowboat](https://github.com/rowboatlabs/rowboat) | Apache-2.0 | **WATCH / small ADAPT.** Validates local-first + inspectable memory (mostly already in the desktop memory browser); the one new hook — `direct_edit` un-validates a skill — folds into the validation-gated-skills work (row 3). |
| 7 | [ADHD](https://github.com/UditAkhourii/adhd) ([site](https://adhdstack.github.io/)) | MIT | **ADAPT** (pattern only). Divergent→converge as a plan-mode "explore" subagent strategy on the existing `agent` tool + `SessionFactory` (isolated child `Session`s give no-cross-branch context for free); opt-in given ≈5–10× cost. |
| 8 | [CloudWeGo Eino](https://github.com/cloudwego/eino) | Apache-2.0 | **ADAPT** (Go — patterns only). Callback-aspects → the fixed `KernelEvent` enum on `on_event` ([ADR-0034](../adr/0034-kernelevent-unified-observability-stream.md), [coding-standards §9](../dev/coding-standards.md#9-kernelevent--the-unified-lifecycle-event-adr-0034)); interrupt/resume state-persistence → plan-mode/subagent suspension. A generic typed node-graph runtime is **rejected** for v1. |
| 9 | [Anthropic — *How we contain Claude*](https://www.anthropic.com/engineering/how-we-contain-claude) | n/a (article) | **ADOPT** (scope decision). Capability isolation — "supervise what the agent *can* do" — motivates the `ToolExecutor` seam + `RUSTYKEYS_ISOLATION` ([ADR-0030](../adr/0030-capability-isolation-toolexecutor.md)); plus threat-model additions (trust-boundary-before-config-parse, egress-as-capability-grant, symlink-before-path-validation, memory-poisoning provenance, multi-agent trust escalation). |

**Adopted crate:** [`rmcp`](https://github.com/modelcontextprotocol/rust-sdk) — the
official `modelcontextprotocol/rust-sdk` (**MIT**, tokio-native, v1.7.x). Re-spec PRD
07's hand-rolled JSON-RPC stdio/SSE transports as **thin adapters over `rmcp`**,
keeping our namespacing, `McpPolicy`, `ApprovalGate`, and auth/TLS pins above it.

## 8. Where references appear in the corpus

| Document | Primary references it relies on |
|---|---|
| [PRD 00](../prd/00-overview.md) | the paper, Keystone, aisdk, Ashby's Law |
| [PRD 01](../prd/01-kernel.md) | aisdk, tokio |
| [PRD 02](../prd/02-constrain.md) | Ashby's Law |
| [PRD 03](../prd/03-feed.md) | aisdk (`#[tool]`, embeddings), SQLite FTS5, hermes-agent |
| [PRD 04/05](../prd/04-observe.md) | the paper (M-HIR, entropy, episode package, outcome taxonomy) |
| [PRD 06](../prd/06-app.md) | axum, tokio, Tauri |
| [PRD 07](../prd/07-mcp.md) | MCP, JSON-RPC 2.0, SSE, harness-ai |
| [PRD 08](../prd/08-frontend.md) | Tauri 2, SolidJS, CodeMirror 6, xterm.js, Tailwind, Vite, terax-ai |
| [BACKLOG.md](../../BACKLOG.md) | the paper + all four comparative implementations |
| [dev/*](../dev/) | the Rust crate & tooling stack |
