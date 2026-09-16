# Rusty Keys — architecture & plan, at a glance

> **What this is.** A diagram-first entry point for implementers: the system in pictures, tied to the
> development plan. It **synthesizes and links** the authoritative docs — it does not restate them.
> If anything here disagrees with the source-of-truth docs, **they win**:
> [`ARCHITECTURE.md`](./ARCHITECTURE.md) (system structure) ·
> [`architecture/data-model.md`](./architecture/data-model.md) (on-disk schemas) ·
> [`../BACKLOG.md`](../BACKLOG.md) (the phased plan) ·
> [`prd/`](./prd) (per-component depth) · [`adr/`](./adr) (decisions).

---

## 1. System context

One Rust binary (plus an optional Tauri desktop frontend) sits between its users and four external
dependencies: the LLM provider (reached through `aisdk`), external MCP servers, the local `.rustykeys/`
state, and — opt-in, egress-guarded — the web.

```mermaid
flowchart TB
    dev["Developer<br/>CLI / terminal"]
    ide["IDE / editor<br/>MCP client"]
    deskUser["Desktop user"]
    web["Web / HTTP client"]

    subgraph RK["Rusty Keys — one binary + frontend"]
        app["app · Session::send()<br/>CLI · gateway · MCP server · Tauri bridge"]
        harness["harness crates<br/>constrain · feed · observe · compose · kernel · mcp"]
        fe["frontend/ · Tauri 2 + SolidJS"]
        app --- harness
        fe -. Tauri IPC .- app
    end

    llm["LLM provider<br/>via aisdk"]
    mcpext["External MCP servers<br/>stdio / SSE"]
    store[("Local state · .rustykeys/<br/>SQLite/DuckDB · JSONL")]
    websites["Web · web_fetch / web_search<br/>egress-guarded"]

    dev -->|stdin/stdout| app
    ide -->|JSON-RPC| app
    deskUser --> fe
    web -->|HTTP + SSE| app
    app -->|model calls| llm
    harness -->|consume tools| mcpext
    harness -->|read / write| store
    harness -.->|opt-in| websites
```

**SSOT:** [`ARCHITECTURE.md` §1–§2](./ARCHITECTURE.md), [§9 deployment](./ARCHITECTURE.md).

---

## 2. The four verbs on the OODA loop

The harness is decomposed into four verbs mapped onto OODA; the kernel is deliberately thin (it knows
nothing about memory, policy, or verification). `constrain` is the gate that vets **every** tool call
before it acts.

```mermaid
flowchart LR
    subgraph loop["Turn cycle = OODA"]
      direction LR
      O1["Observe<br/>observe + feed"] --> Or["Orient<br/>feed · recall + TaskState"]
      Or --> D["Decide<br/>kernel · aisdk loop"]
      D --> A["Act<br/>kernel → tool dispatch"]
      A --> V["verify<br/>compose"]
      V -.-> O1
    end
    G["constrain — the gate<br/>Policy::before_tool vets before Act"]
    A -.->|before_tool| G
    G -.->|allow / block| A
```

| Verb | OODA | Crate | Owns |
|---|---|---|---|
| **Constrain** | gate | `constrain` | tool-call vetting, permission modes, security checkers, approval |
| **Feed** | Observe+Orient | `feed` | tools, memory, TaskState, context assembly |
| **Observe** | Observe | `observe` | episode trace, interventions (M-HIR), entropy |
| **(Kernel)** | Decide+Act | `kernel` | the aisdk agent loop |
| **Compose** | verify | `compose` | verification, attribution, evidence, episode packages |

**SSOT:** [`ARCHITECTURE.md` §2](./ARCHITECTURE.md), [`adr/0005`](./adr/0005-harness-decomposed-into-four-verbs.md), [`reference/glossary.md`](./reference/glossary.md).

---

## 3. Components — eight crates + a frontend

Acyclic by construction. `app` is the only binary and wires everything; `kernel` receives the tool
registry and policy as **trait objects** (`&dyn ToolDispatch` / `&dyn Policy`) so it never imports
`feed` or `compose`. The full edge set and import contract is the authoritative DAG in `ARCHITECTURE.md` §5.

```mermaid
flowchart TD
    config["config · leaf"]
    observe["observe"]
    constrain["constrain"]
    feed["feed"]
    kernel["kernel"]
    mcp["mcp · client + server"]
    compose["compose"]
    app["app · Session (the centre)"]

    app --> kernel & compose & mcp & feed & constrain & observe & config
    kernel --> constrain & observe & config
    compose --> feed & observe & config
    mcp --> feed & constrain & config
    feed --> constrain & observe & config
    constrain --> observe & config
    observe --> config
```

> Cross-crate cycle avoidance: the `agent` subagent tool lives in `feed` but must build a child
> `Session` (in `app`) — resolved by the `SessionFactory` trait `app` injects ([`adr/0017`](./adr/0017-subagent-spawning-via-sessionfactory-trait.md)).

**SSOT:** [`ARCHITECTURE.md` §4–§5](./ARCHITECTURE.md).

---

## 4. Runtime — the turn cycle as data flow

`Session::send()` owns the cycle; adapters are thin transports. This is the data-flow companion to the
sequence diagram in `ARCHITECTURE.md` §6.

```mermaid
flowchart TD
    msg["user message"] --> detect["detect interventions<br/>unverified_followup · tool_block"]
    detect --> obs1["observe(user msg) + push history → stream.db"]
    obs1 --> budget["token_budget.check_and_compact()"]
    budget --> orient["orient() → recall + TaskState render = context"]
    orient --> start["tracer.start_episode()"]
    start --> krun["kernel.run(history, &dyn ToolDispatch,<br/>&dyn Policy, context, &mut Tracer)"]
    krun -->|"per tool call"| gate{"Policy::before_tool<br/>async"}
    gate -->|allow| dispatch["dispatch → ToolOutcome"]
    gate -->|block| blocked["BLOCKED → ToolOutcome<br/>+ security.jsonl"]
    dispatch --> krun
    blocked --> krun
    krun --> reply["reply"]
    reply --> obs2["observe(reply) + push history"]
    obs2 --> join["post-turn tokio::join!"]
    join --> judge["compose · verify_with_judge"]
    join --> consol["feed · consolidate(Idle)"]
    join --> entropy["observe · entropy.audit(episode, scope)"]
    judge --> jr["journal.record_turn<br/>/ record_episode at H3"]
    consol --> jr
    entropy --> jr
    jr --> out["(reply, VerificationReport)<br/>+ emit rk://turn_complete"]
```

**SSOT:** [`ARCHITECTURE.md` §6–§7](./ARCHITECTURE.md), [`prd/06-app.md`](./prd/06-app.md), [`adr/0012`](./adr/0012-post-turn-compose-runs-concurrently.md).

### 4a. Memory lifecycle

Short-term observations consolidate (idle/sleep/explicit) into the long-term graph; `recall()` scores
and returns structured entries that feed `orient()`. Failure-born skills are **candidates** until
validated (ADR-0031) before they become prune-exempt.

```mermaid
flowchart LR
    obsv["observations<br/>stream.db · short-term"] -->|"idle / sleep / explicit"| consol["consolidation<br/>ADR-0009"]
    consol --> store["memory graph<br/>store.db / store.duckdb"]
    store --> recall["recall() → Vec&lt;ContextEntry&gt;<br/>relevance · recency · importance"]
    recall --> ctx["context · orient()"]
    ctx --> turn["turn"]
    turn --> obsv
    fail["UNVERIFIED turn + Attribution"] -->|"failure-born"| cand["skill candidate<br/>validated=0 · ADR-0031"]
    cand -->|"VERIFIED match / golden replay"| promo["promoted skill<br/>validated=1 · prune-exempt"]
    cand --> store
    promo --> store
```

**SSOT:** [`prd/03-feed.md`](./prd/03-feed.md), [`adr/0008`](./adr/0008-memory-is-observe-orient-half-of-ooda.md)/[`0009`](./adr/0009-tiered-consolidation-idle-sleep-explicit.md)/[`0011`](./adr/0011-skills-exempt-from-pruning.md)/[`0031`](./adr/0031-validation-gated-skill-promotion.md).

### 4b. Episode-package assembly (H3)

At H3, the **assembly projector** (ADR-0036) is the named builder that projects raw evidence into the
eight typed traces — so no trace ships without a producer.

```mermaid
flowchart LR
    subgraph rawev["raw evidence"]
      stream["stream.db observations"]
      outcomes["ToolOutcomes"]
      recallout["recall() output"]
      checkres["CheckResults"]
      interv["interventions.jsonl"]
    end
    asm["EpisodeAssembler<br/>compose-time projector · ADR-0036"]
    rawev --> asm
    subgraph traces["8 typed traces"]
      t1["action_trace · ActionEvent"]
      t2["tool_trace · +exit_code/timeout/recovered"]
      t3["context_trace · ContextEntry"]
      t4["verification_trace · VerifyEntry · covers[]"]
      t5["attribution_log · FailureType"]
      t6["reproduction_log"]
      t7["verification_report"]
      t8["intervention_log · per-turn filtered"]
    end
    asm --> traces
    traces --> pkg["EpisodePackage<br/>episodes/&lt;turn_id&gt;.json + EpisodeOutcome"]
```

**SSOT:** [`data-model.md` §5–§5.1](./architecture/data-model.md), [`adr/0036`](./adr/0036-episode-package-assembly-projector.md), [`prd/05-compose.md`](./prd/05-compose.md).

---

## 5. Data & on-disk state

All state is workspace-local under `.rustykeys/`; every path is `RUSTYKEYS_*`-overridable. The diagram
below shows **which crate owns each artifact** (`checks.toml`/`mcp.toml` are read-only config inputs).

```mermaid
flowchart TD
    subgraph disks[".rustykeys/ — workspace-local"]
      subgraph dbs["SQLite / DuckDB"]
        s1["stream.db · observations"]
        s2["store.db / store.duckdb<br/>memories · edges · fts"]
      end
      subgraph logs["append-only JSONL · v + ts · redacted"]
        l1["evidence.jsonl"]
        l2["interventions.jsonl"]
        l3["security.jsonl"]
        l4["entropy.jsonl"]
      end
      subgraph files["JSON / TOML"]
        j1["task.json · TaskState"]
        j2["episodes/&lt;turn_id&gt;.json"]
        j3["sessions/&lt;session_id&gt;.json"]
        j4["checks.toml · mcp.toml · config in"]
      end
    end
    feedC["feed"] --> s1 & s2 & j1
    observeC["observe"] --> l2 & l4
    composeC["compose"] --> l1 & j2
    constrainC["constrain"] --> l3
    appC["app"] --> j3
```

Logical view of the stores (separate DBs — no cross-DB FKs; `sessions/` is JSON, shown for lineage):

```mermaid
erDiagram
    OBSERVATIONS {
      int id PK
      string session_id
      float ts
      string role
      string kind
      string content
    }
    MEMORIES {
      int id PK
      string title UK
      string body
      string mem_type
      float importance
      int validated
      blob embedding
    }
    MEMORY_EDGES {
      string src_title FK
      string dst_title FK
      string rel
    }
    SESSIONS {
      string session_id PK
      string harness_level
      string task_id
    }
    MEMORIES ||--o{ MEMORY_EDGES : "src_title"
    MEMORIES ||--o{ MEMORY_EDGES : "dst_title"
    SESSIONS ||--o{ OBSERVATIONS : "session_id"
```

**SSOT:** [`data-model.md`](./architecture/data-model.md) (every schema, serde, versioning, durability), [`reference/configuration.md`](./reference/configuration.md), [`adr/0027`](./adr/0027-on-disk-schema-versioning.md).

---

## 6. Deployment topologies

One binary; `RUSTYKEYS_MODE` selects the transport. Every vetted side-effect runs through a
`ToolExecutor` (ADR-0030) whose profile (`none` default, or `sandboxed`) is orthogonal to the mode.

```mermaid
flowchart TD
    bin["rusty-keys · one binary<br/>RUSTYKEYS_MODE selects topology"]
    bin --> cli["CLI REPL · default<br/>stdin/stdout"]
    bin --> gw["Web gateway · --gateway<br/>axum HTTP + SSE · Bearer + CORS"]
    bin --> mcps["MCP server · --mcp<br/>JSON-RPC stdio/SSE · chat tool"]
    gw --> single["single · Arc&lt;Mutex&lt;Session&gt;&gt;"]
    gw --> multi["multi · session_id → N sessions"]
    desktop["Desktop · Tauri 2<br/>22 IPC commands · rk:// events"] -->|over the gateway| gw
    subgraph exec["ToolExecutor · ADR-0030 · RUSTYKEYS_ISOLATION"]
      ex1["none · in-process · default"]
      ex2["sandboxed · OS sandbox<br/>network-deny-by-default"]
    end
    cli & single & multi & mcps --> exec
```

**SSOT:** [`ARCHITECTURE.md` §9–§10](./ARCHITECTURE.md), [`adr/0030`](./adr/0030-capability-isolation-toolexecutor.md), [`prd/07-mcp.md`](./prd/07-mcp.md), [`prd/08-frontend.md`](./prd/08-frontend.md).

---

## 7. H0–H3 maturity & the controlled-visibility ablation

The ladder is **additive in capability** but must become a **controlled-visibility ablation** before any
cross-level claim is trustworthy: **R1** hides higher-level artifacts at the feed/context-read seam
(existence, not authority), and **R5** adjudicates every level evaluator-side.

```mermaid
flowchart TB
    subgraph ladder["Capability ladder"]
      direction TB
      H0["H0 · task + repo files<br/>no tool registry · ablation floor"]
      H1["H1 · tool registry + tool-use<br/>Phase 1"]
      H2["H2 · project memory · TaskState · context<br/>Phases 3–4"]
      H3["H3 · deterministic checks · attribution · verification<br/>Phases 2, 4, 10"]
      H0 --> H1 --> H2 --> H3
    end
    R1["R1 · controlled visibility<br/>hide higher-level artifacts<br/>at the feed/context-read seam"]
    R5["R5 · evaluator-side adjudication<br/>CheckRegistry::run_all() at ALL levels<br/>→ comparable EpisodeOutcome per level"]
    gate["Gate · do NOT report any Hn-vs-Hm lift<br/>until R1 + R5 land · ADR-0035"]
    ladder --- R1 --- gate
    ladder --- R5 --- gate
```

**SSOT:** [`ARCHITECTURE.md` §3](./ARCHITECTURE.md), [`adr/0035`](./adr/0035-controlled-visibility-ablation-eval-substrate.md), [`adr/0028`](./adr/0028-h0-selectable-harness-level-or-eval-only.md), [`dev/eval-plan.md`](./dev/eval-plan.md).

---

## 8. Development plan — phases, crates, issues

Each phase is a **working, runnable system**, dependency-sequenced. The full dependency graph,
Definition-of-Done, acceptance criteria, test gates, and risks live in
[`../BACKLOG.md`](../BACKLOG.md); the GitHub issue for each item carries the working state. The
**engineering substrate** (error model, `ToolOutcome`, `async before_tool`, `FakeLanguageModel`, CI,
`KernelEvent`, on-disk versioning) lands **with Phase 1** and is intentionally issue-less.

| Phase | Goal (maturity) | Primary crate(s) | Size | Issues |
|---|---|---|---|---|
| 1 | Runnable skeleton (H1) | config · observe · constrain · feed · kernel · app | L | #1–#7 |
| 2 | Observe + Compose (H3-deterministic) | observe · compose | M | #36 |
| 3 | Memory (H2) | feed | L | #37 |
| 4 | Task State + semantic verification (H2/H3) | feed · compose | M | #38 |
| 5 | DuckDB + embeddings | feed | M | #39 |
| 6 | Full tool suite | feed | L | #8–#11 |
| 7 | Permission system | constrain | M | #14 |
| 7B | Capability isolation (ToolExecutor) | constrain · feed | L | #40 |
| 8 | Token & context management | kernel · app | M | #15 |
| 9 | Plan mode | app · feed | S | #16 |
| 10 | H3 episode packages | observe · compose | L | #21 · **#42** (projector) · **#41** (ablation) |
| 11 | Entropy auditor | observe | M | #19 |
| 12 | MCP integration | mcp | L | #12 · #13 |
| 13 | Extended CLI | app | M | #20 |
| 14 | Web gateway | app | M | #18 |
| 15 | Desktop frontend | frontend/ | XL | #22–#27 |
| cross-cutting | Eval-integrity guard | compose · observe | M | #43 |
| post-phase | Rich terminal UI (ratatui) | app | — | #17 |

**Critical path:** Phase 1 → {2, 3, 6} → 4 → {10, 11}; 7 → 7B and 7 → 9 → ...; 14 → 15. See the
[dependency graph in `BACKLOG.md`](../BACKLOG.md#phase-dependency-graph).

---

## 9. Where to go next

| You want… | Read |
|---|---|
| System structure, concurrency, faithfulness map | [`ARCHITECTURE.md`](./ARCHITECTURE.md) |
| Exact on-disk schemas, serde, versioning | [`architecture/data-model.md`](./architecture/data-model.md) |
| Threat model & redaction | [`architecture/threat-model.md`](./architecture/threat-model.md) |
| The phased plan (DoD/AC/gates/risks) | [`../BACKLOG.md`](../BACKLOG.md) |
| Per-component design | [`prd/`](./prd) (00 overview → 08 frontend) |
| Why a decision was made | [`adr/`](./adr) (0001–0036) |
| Errors, testing, eval, standards | [`dev/`](./dev) |
| Env vars & config | [`reference/configuration.md`](./reference/configuration.md) |
| Vocabulary | [`reference/glossary.md`](./reference/glossary.md) |
