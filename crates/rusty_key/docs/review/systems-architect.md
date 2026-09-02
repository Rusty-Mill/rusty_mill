*Point-in-time working document from the five-persona expert review; superseded once the canonical docs (ARCHITECTURE.md, architecture/data-model.md, refined roadmap) are written.*

# Systems Architect review — Rusty Keys spec

## 1. Scope & lens

Overall system structure and integrity: the missing standalone architecture doc, the crate dependency DAG, the on-disk data/storage model (the central deliverable), the concurrency model, deployment topologies, failure/resilience, NFRs, and on-disk format versioning. I read PRDs 00–08 in full plus README and BACKLOG, and cite where the corpus already covers a concern rather than re-flagging it.

## 2. Validated gaps

- **G1 — No standalone architecture document.** The system view lives inside `prd/00-overview.md` (component map, 15 ADRs, H0–H3 ladder). There is no single doc a newcomer reads first; the README ASCII diagram is the only top-level structural artifact and it omits `kernel`/`config`/`mcp`. Justification: confirmed — nothing above the PRD layer.
- **G2 — Crate count conflict: 7 vs 8.** `00-overview.md §Component map` lists 7 crates (kernel, constrain, feed, observe, compose, app, config — no `mcp`). `06-app.md §Cargo workspace layout` lists 8 (adds `crates/mcp/`). README and the review brief both say "7-crate." This must be reconciled (mcp is a real crate per PRD 07 → answer is 8 + frontend).
- **G3 — DAG asserted but not enumerated, and its stated rules are incomplete.** `06-app.md` (end) gives prose rules: "kernel cannot import feed or compose; observe cannot import compose; app imports everything … is a DAG." But: (a) no node/edge listing or diagram; (b) `config`, `constrain`, `mcp` are never placed in the order; (c) **compose→observe edge is unstated yet real** — `VerificationReport` (compose, `05 §VerificationReport`) embeds `Option<EntropyAudit>`, a type defined in observe (`04 §Data structures`); (d) **compose→feed edge is unstated yet real** — `CriteriaJudge` holds `Arc<TaskStore>` (`05 §CriteriaJudge`). Need a formal DAG that includes these and proves acyclicity (it is acyclic: observe/feed sit below compose; app on top).
- **G4 — `.rustykeys/` directory layout is never enumerated in one place.** Paths are scattered across config defaults (`06 §Config`) and individual PRDs. No single tree shows evidence.jsonl, interventions.jsonl, security.jsonl, entropy.jsonl, task.json, checks.toml, mcp.toml, episodes/, and the SQLite DB file(s) together. The only `.db` filename anywhere is `memory.db` in an mcp.toml example (`07 §Config file`), and that is the DB of an *external* `mcp-server-sqlite` process — not the harness's own stores. The harness's short-term-stream and long-term-store DB filenames are unspecified entirely, even though `RUSTYKEYS_SHORT_TERM_BACKEND` and `RUSTYKEYS_LONG_TERM_BACKEND` (`06 §Config`) can select different backends and imply distinct files.
- **G5 — SQLite schemas are undefined.** `03 §Short-term stream` gives `Observation{ts,role,kind,content}` and `03 §Long-term graph` gives a `Memory` trait API with "typed edges," FTS5, importance/recency — but **no CREATE TABLE / column types / index / edge-table design** exists anywhere. The single source-of-truth data-model must define both schemas concretely.
- **G6 — On-disk format versioning is entirely absent.** No JSONL record, episode package, task.json, or SQLite schema carries a `schema_version`/`v` field. ADR-015 calls rotation a "future seam" but says nothing about forward-compat. For append-only logs that outlive the binary, this is a real durability gap.
- **G7 — Partial-write / torn-line resilience for append-only JSONL is unspecified.** PRDs describe append-only JSONL but never how a crash mid-append is handled (atomic line write? `fsync`? recovery skips trailing partial line?). `count_turns()` (`05`) line-scans the journal and would choke on a torn final line.
- **G8 — Mid-turn LLM failure with partial side effects is not addressed.** `01 §Error handling` says network/provider error is "propagated as `KernelError`; `Session` handles retry or surfaces to caller" — but the retry policy is undefined, and more importantly tool side effects (file writes, bash) already executed before the failing model call are not rolled back or recorded. No transactional/episode-abort story.
- **G9 — SQLite lock contention across sessions is unhandled.** ADR-003 mandates `spawn_blocking` for SQLite. But multi-session gateway (`06 §Session model`) and MCP `multi` mode (`07 §Session lifecycle`) run N Sessions, each with its own `Memory`, all pointing at the *same* `.rustykeys/` DB files. No WAL-mode statement, no busy-timeout, no single-writer arbitration. `task.json` has the same problem — N sessions write the same file path.
- **G10 — No NFR / quality-attributes section.** Goals in `00` are qualitative ("minimal," "local-first"). There is no statement of latency budgets (hot-path policy check, post-turn join), durability guarantees, throughput targets for the gateway, or memory bounds. The "zero-overhead hot path" claim (ADR-001, ADR-003) is unquantified.
- **G11 — `policy.before_tool` sync-vs-async is contradictory.** `02 §Policy trait` and the `dispatch()` sample call it synchronously; ADR-007 says "runs synchronously." But `02 §ApprovalGate` states it "requires `before_tool` to become `async fn` … accepted here as the use case is now concrete." The canonical signature is unresolved; it cascades into kernel/registry signatures.

## 3. Already-covered / pruned

- **Concurrency model — largely covered.** Session-on-a-tokio-task with `mpsc` channel pair sized 1 for turn ordering (`06 §Session on its own task`); `spawn_blocking` for SQLite (ADR-003); post-turn `tokio::join!` of judge+consolidation+entropy (`05 §Concurrency`, `06 §send`, `04 §Lifecycle`). Tracer is `!Send` by design, no lock (`04 §Rust advantages`). Subagent cancellation via `CancellationToken` (`03 §Task management tools`). This is well-specified — do **not** flag as a gap; the data-model/architecture docs should reference it, not redo it.
- **Deployment topologies — covered, needs consolidation not invention.** Single binary with `RUSTYKEYS_MODE = cli|gateway|mcp` (`06 §Config`), gateway single/multi (`06 §Session model`), MCP stdio/sse server (`07 §Transports`), Tauri desktop (`08`). Scattered but present; ARCHITECTURE.md should collect them into one topology section + feature-flag matrix.
- **Feature-flag matrix — partially covered.** `duckdb` optional feature (ADR-010), web tools opt-in (`RUSTYKEYS_ALLOW_WEB`), harness level h1/h2/h3 gating tools/checks. Not assembled as a matrix anywhere — that assembly is the work, but the inputs exist.
- **Post-turn ordering hazard — already acknowledged.** ADR-012 explicitly notes consolidation may miss the judge's signal and mitigates by joining all three before observing. Don't re-flag.
- **Graceful degradation of CriteriaJudge — covered.** `05 §Graceful degradation`: unparseable JSON → passing result with diagnostic. Good.
- **MCP failure modes — covered.** `07 §Error handling` table handles server-fails-to-start, call error, mid-session crash (reconnect), schema-validation failure.
- **Log rotation — acknowledged as a seam** in `02`, `04`, `05 §Seams`. Not a gap, but versioning (G6) is distinct from rotation and is *not* covered.
- **CLI ↔ gateway ↔ Tauri share one `Session`** — the adapter-over-Session principle is stated repeatedly (`00 ADR-004`, `06`, `07`, `08`). Sound; no gap.

## 4. New gaps (not in the original focus list)

- **N1 — Field-level type drift between PRDs.** (a) `TaskState` struct (`03 §Task State`) has no `scope` field, but `04 §Detection heuristics` (BoundaryViolation) and `EntropyCategory` reasoning rely on "the active `TaskState`'s declared scope (if `scope` field set)." (b) `kernel.run()` signature differs: `01 §Interface` = `(history, registry, extra_context, tracer)`; `06 §send` step 8 = `(history, registry, policy, context, tracer)`. (c) `EpisodePackage` is used as a Rust type (`05` `record_episode(pkg: &EpisodePackage)`) but only ever shown as JSON, never as a struct. The data-model doc must pick canonical definitions.
- **N2 — Two distinct "task" concepts collide in storage naming.** `TaskState` (goal+criteria working memory, persisted to `task.json`) vs the `task_create/...` background-operation registry (`03 §Task management tools`, in-session only, not persisted). The `/task` command, `RUSTYKEYS_TASK_FILE`, and `task.json` all refer to the former; readers will conflate them. Needs an explicit disambiguation note in the data-model.
- **N3 — Subagent (`agent` tool) storage/observability is underspecified.** `03` says "the child's episode is recorded as a nested entry in `EvidenceJournal`," but the evidence.jsonl turn schema (`05`) has no nesting/parent-id field, and child sessions presumably share the same `.rustykeys/` (same stream/store/logs) as the parent — concurrency (G9) and attribution implications unaddressed.
- **N4 — `EpisodeOutcome` enum has no defined wire encoding.** The enum is PascalCase in Rust (`05`) but appears as `"autonomous_verified_success"` (snake_case) in the episode JSON. Serde rename convention must be pinned in the data-model (applies to `ToolStatus`, `InterventionKind`, `EntropyCategory`, `CompactionTier` too).
- **N5 — `/resume [id]` and `/export` imply session identity & persistence that storage doesn't define.** `06 §Full command set` lists `/resume [id]` ("resume a named previous session") and MCP keys sessions by `session_id` (`07`), but nothing in the data model defines what a persisted/named session *is* on disk, or how history is restored. This is a load-bearing gap for the data-model SSOT.
- **N6 — `harness/checks.toml` (project-level) vs `.rustykeys/checks.toml` precedence is unstated.** `05 §DeterministicCheck registry` mentions both paths; load order/override semantics undefined.

## 5. Recommended edits

For the data-model SSOT, the schemas/formats to consolidate and their source PRDs:

| Format | Source PRD(s) |
|---|---|
| `.rustykeys/` directory tree (all files + SQLite DB names) | assembled from 06 §Config + all PRDs |
| short-term stream SQLite schema (`Observation`) | 03 §Short-term stream |
| long-term store SQLite schema (`Memory` + edges, FTS5) | 03 §Long-term graph |
| `evidence.jsonl` (turn / improvement / compaction records) | 05 §EvidenceJournal + ADR-015 |
| `episodes/<turn_id>.json` package | 05 §Episode package schema |
| `interventions.jsonl` | 04 §Storage |
| `security.jsonl` | 02 §Security checkers |
| `entropy.jsonl` | 04 §Storage |
| `task.json` (`TaskState`) | 03 §Task State |
| `checks.toml` (+ project `harness/checks.toml`) | 05 §DeterministicCheck registry |
| `mcp.toml` | 07 §Config file |

| target file | change | priority | depends-on |
|---|---|---|---|
| docs/ARCHITECTURE.md (new) | Author standalone top-level architecture: component map, formal crate DAG (with node/edge list + diagram), concurrency model summary, topology section, feature-flag matrix. Lift from `00`/`06`; do not duplicate ADRs (link them). | P0 | G1,G2,G3 |
| docs/architecture/data-model.md (new) | SSOT for on-disk state: the `.rustykeys/` tree + all 11 formats in the table above, with concrete SQLite DDL and per-format `schema_version` field. | P0 | G4,G5,G6 |
| docs/architecture/data-model.md | Add a "Versioning & forward-compat" section: version field on every JSONL/episode/task record and `PRAGMA user_version` for both DBs; define migration/skip-unknown rules. | P0 | G6 |
| docs/architecture/data-model.md | Define torn-line/partial-write policy for append-only JSONL (atomic single-line append, recovery skips trailing partial) and make `count_turns()` tolerant. | P1 | G7 |
| docs/architecture/data-model.md | Pin serde wire encoding (rename_all = snake_case) for `EpisodeOutcome`, `ToolStatus`, `InterventionKind`, `EntropyCategory`, `CompactionTier`. | P1 | N4 |
| docs/architecture/data-model.md | Disambiguate `TaskState`/`task.json` vs `task_create` background registry; define persisted/named session shape for `/resume` & `session_id`. | P1 | N2,N5 |
| docs/ARCHITECTURE.md | Add concurrency/resilience subsection covering SQLite WAL + busy_timeout + single-writer story for multi-session gateway/MCP and shared `task.json`. | P1 | G9 |
| docs/ARCHITECTURE.md | Add "Failure modes" subsection: mid-turn LLM failure with partial side effects, retry policy, episode-abort handling. | P1 | G8 |
| docs/ARCHITECTURE.md (or new nfr.md) | Add NFR / quality-attributes section with quantified hot-path/post-turn latency budgets and durability guarantees. | P2 | G10 |
| prd/00-overview.md | Fix crate count to 8 (+frontend); add `mcp` and `config` to component map; reconcile with README diagram. | P0 | G2 |
| prd/00-overview.md / prd/02 | Resolve `before_tool` sync-vs-async contradiction (ADR-007 vs ApprovalGate); state canonical signature, propagate to kernel/registry. | P1 | G11 |
| prd/03-feed.md / prd/01-kernel.md | Reconcile `TaskState.scope` field and `kernel.run()` signature drift; define `EpisodePackage` struct. | P2 | N1 |
| prd/05-compose.md | Add parent/child fields to evidence turn schema for subagent episodes; state checks.toml precedence. | P2 | N3,N6 |
| BACKLOG.md | Refined roadmap: add explicit data-model/versioning workstream; note Phase 7 makes `before_tool` async (breaking) — sequence it before/with Phase 12 MCP & Phase 14 gateway. | P1 | G11,G6 |

## 6. Cross-persona dependencies

- **Memory/cognition persona:** owns the *content/semantics* of the long-term store (memory types, edges, recall scoring, consolidation). I own its *on-disk schema*. The data-model DDL (G5) must be co-authored — I define columns/indexes, they define the recall/importance/decay semantics and the edge taxonomy.
- **Security/safety persona:** `security.jsonl` schema (G4/data-model) and the `Bypass` mode / `RUSTYKEYS_ALLOW_BYPASS` gate; the `before_tool` async resolution (G11) is also a security-surface decision (ApprovalGate is the trigger).
- **Verification/observability persona:** episode package + evidence.jsonl + entropy.jsonl schemas and the `EpisodeOutcome` taxonomy encoding (N4) are shared SSOT; the compose→observe DAG edge (G3) is theirs to confirm.
- **Product/roadmap persona:** the refined-roadmap edits (versioning workstream, async-`before_tool` sequencing, crate-count fix) need their ownership of BACKLOG phasing.
