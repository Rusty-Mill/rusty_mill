# Data model — on-disk state

> **Authoritative source.** This document is the single source of truth for everything Rusty Keys persists: the `.rustykeys/` layout, both SQLite schemas, every JSONL/TOML/JSON record, the serde wire conventions, and the on-disk versioning and durability rules. Other documents (the PRDs, `ARCHITECTURE.md`) **link here** rather than restating schemas. If a schema appears anywhere else, this file wins.
>
> Concrete column names, DB filenames, and field sets below are **v1 intent** — they are the design to build against and revisit after the Phase 1 spike, not a frozen ABI. Forward-compatibility is handled by the versioning rules in [§9](#9-versioning--forward-compatibility).

Related: [`ARCHITECTURE.md`](../ARCHITECTURE.md) (structure, concurrency, faithfulness) · [`reference/configuration.md`](../reference/configuration.md) (the env vars that point at these paths) · [`architecture/threat-model.md`](./threat-model.md) (redaction) · ADR-0025 (serde), ADR-0027 (versioning).

---

## 1. `.rustykeys/` directory layout

All state is local to the workspace root, under `.rustykeys/`. Every path is overridable via a `RUSTYKEYS_*` env var (see configuration reference); defaults shown.

```
<workspace>/.rustykeys/
├── stream.db              # SQLite — short-term observation stream (RUSTYKEYS_SHORT_TERM_BACKEND=sqlite)
├── store.db               # SQLite — long-term memory graph (RUSTYKEYS_LONG_TERM_BACKEND=sqlite)
│   └── store.duckdb        #   …or DuckDB when RUSTYKEYS_LONG_TERM_BACKEND=duckdb (Phase 5)
├── evidence.jsonl         # append-only — verification packages, consolidation + compaction events
├── interventions.jsonl    # append-only — human interventions (drives M-HIR)
├── security.jsonl         # append-only — blocked tool calls (security checkers)
├── entropy.jsonl          # append-only — per-turn entropy audits
├── ratchet.jsonl          # append-only — failed-turn attributions feeding /ratchet (P3); see §8
├── task.json              # current TaskState (working memory: goal + criteria + scope)
├── config.toml            # project settings ([settings]) + MCP servers ([mcp]); env overrides it
├── checks.toml            # local deterministic-check registry (H3); see §8
├── mcp.toml               # legacy standalone MCP declarations (fallback); see §8
├── episodes/              # H3 episode packages, one JSON file per turn
│   └── <turn_id>.json
└── sessions/              # persisted/named sessions for /resume and gateway session_id
    └── <session_id>.json
```

**DB filename note (resolves a naming collision).** The harness's own stores are **`stream.db`** and **`store.db`**. The `memory.db` that appears in the PRD 07 `mcp.toml` example is the database of an *external* `mcp-server-sqlite` process — unrelated to the harness's memory. That example should be renamed (e.g. `external.db`) to avoid confusion; the harness never reads it.

**Two distinct "task" concepts — do not conflate:**
- **`TaskState`** (this file, `task.json`) — the agent's *working-memory goal + success criteria*. Persisted, single, cross-turn. Set via `set_task` / `/task`.
- **`task_create` registry** — a *background-operation* registry (subagent runs, long bash jobs). In-session only, **not persisted**, never written to `task.json`.

---

## 2. Short-term stream — `stream.db` (SQLite)

Append-optimized OLTP log of every observation. Trait: `Stream` (PRD 03). Backend selected by `RUSTYKEYS_SHORT_TERM_BACKEND` (only `sqlite` in v1).

```sql
PRAGMA user_version = 1;          -- schema version (ADR-0027)
PRAGMA journal_mode = WAL;        -- concurrent readers + one writer (multi-session safety, §10)
PRAGMA busy_timeout = 5000;       -- ms; back off instead of erroring on lock contention

CREATE TABLE observations (
    id          INTEGER PRIMARY KEY AUTOINCREMENT,
    session_id  TEXT    NOT NULL,                  -- owning session (see §6)
    ts          REAL    NOT NULL,                  -- epoch seconds, f64
    role        TEXT    NOT NULL,                  -- user | assistant | system | tool
    kind        TEXT    NOT NULL,                  -- message | tool_event | verification | task_change | consolidation
    content     TEXT    NOT NULL
);
CREATE INDEX idx_obs_session_ts ON observations(session_id, ts);
CREATE INDEX idx_obs_kind       ON observations(kind);
```

Maps to `Observation { ts, role, kind, content }` (PRD 03) plus `session_id` for session identity (§6). `recent(n)` / `since(ts)` query by `session_id` ordered by `ts`.

---

## 3. Long-term store — `store.db` (SQLite) / `store.duckdb` (DuckDB)

Consolidated memory graph: facts, summaries, skills, entities, with typed edges. Trait: `Store` (PRD 03). The **recall scoring** (relevance + recency + importance weights, decay) and **consolidation** semantics live in **PRD 03** — this section defines only the *storage shape* those algorithms read/write.

```sql
PRAGMA user_version = 1;
PRAGMA journal_mode = WAL;
PRAGMA busy_timeout = 5000;

CREATE TABLE memories (
    id           INTEGER PRIMARY KEY AUTOINCREMENT,
    title        TEXT    NOT NULL UNIQUE,          -- stable identity; edges reference this
    body         TEXT    NOT NULL,
    mem_type     TEXT    NOT NULL,                 -- fact | summary | skill | entity  (snake_case)
    importance   REAL    NOT NULL DEFAULT 0.5,     -- 0.0..1.0 (skills floored only once validated=1, see PRD 03 / ADR-0011)
    validated    INTEGER NOT NULL DEFAULT 0,       -- 0/1 bool; skill candidate→promoted lifecycle (ADR-0031)
    created_ts   REAL    NOT NULL,
    last_used_ts REAL    NOT NULL,                 -- recall recency input
    use_count    INTEGER NOT NULL DEFAULT 0,
    embedding    BLOB,                             -- packed little-endian f32[]; NULL ⇒ lexical-only (mixed corpus OK)
    source_ts_lo REAL,                             -- provenance: observation window this was distilled from
    source_ts_hi REAL
);
CREATE INDEX idx_mem_type       ON memories(mem_type);
CREATE INDEX idx_mem_importance ON memories(importance);

CREATE TABLE memory_edges (
    src_title TEXT NOT NULL,                       -- references memories.title
    dst_title TEXT NOT NULL,
    rel       TEXT NOT NULL,                       -- relates | causes | supersedes | part_of
    PRIMARY KEY (src_title, dst_title, rel)
);

-- Lexical recall when no embed model is configured (RUSTYKEYS_EMBED_MODEL unset).
CREATE VIRTUAL TABLE memories_fts USING fts5(title, body, content='memories', content_rowid='id');
-- (insert/update/delete triggers keep memories_fts in sync; omitted for brevity)
```

- `candidates(query, embed, k)` → top-k by FTS5 `bm25` (lexical) **or** `embedding` cosine (semantic); the returned raw relevance score is **normalized within the batch** before blending (PRD 03 — FTS5 and cosine live in different domains).
- `validated` is the skill **candidate→promoted** lifecycle (ADR-0031): a failure-born skill is minted with `validated=0` (a *candidate* — recall-eligible but neither importance-floored nor prune-exempt) and flipped to `validated=1` (*promoted*) only after it validates (online: a later matching turn goes VERIFIED; offline: it survives golden-episode replay — PRD 03 / [`eval-plan`](../dev/eval-plan.md)). A human `direct_edit` to a skill-derived body resets it to `0`. `validated` is `v1 intent`.
- `prune(older_than, importance_below)` **must exclude `mem_type='skill'`** — but **only skills with `validated=1`** are prune-exempt + importance-floored (ADR-0011, ADR-0031); a `validated=0` candidate skill decays and prunes like an ordinary memory until it earns promotion.
- **DuckDB backend** (`store.duckdb`, Phase 5): same logical columns; vector search via `list_cosine_similarity`; `embedding` stored as a `FLOAT[]` column. Embedding dims/threshold are pinned in PRD 03.

---

## 4. Append-only JSONL logs

All four logs share rules: one JSON object per line, `\n`-terminated; every record carries **`"v": 1`** (schema version) and **`"ts"`** (epoch seconds). Durability and torn-line recovery: [§10](#10-append-only-durability). Secret redaction is applied to any tool args/results **before** they reach these files ([§11](#11-secret-redaction), ADR-0026).

### 4.1 `evidence.jsonl` — verification, consolidation, compaction (PRD 05)

Turn record (non-H3):
```json
{"v":1,"ts":1748346622.5,"kind":"turn","session_id":"s_abc","turn_id":"turn_20260527_143022_abc123",
 "parent_turn_id":null,
 "verified":true,
 "checks":[{"name":"no_tool_errors","passed":true,"detail":""}],
 "attributions":[],
 "entropy":{"delta":0,"findings":[]},
 "outcome":null,
 "limits":"deterministic checks only; semantic correctness and task success not verified",
 "evidence":[{"name":"read_file","status":"ok"}],
 "reply":"…"}
```
- `parent_turn_id` (nullable) links a **subagent** turn to its spawning turn (resolves the nested-episode gap; subagents share `.rustykeys/`).
- `outcome` is `null` below H3; at H3 it carries an `EpisodeOutcome` (§5) and the full package is written to `episodes/`.

Consolidation changelog:
```json
{"v":1,"ts":…,"kind":"improvement","session_id":"s_abc","scope":"idle",
 "created":3,"updated":1,"pruned":0,"groomed":0}
```
Compaction event:
```json
{"v":1,"ts":…,"kind":"compaction","session_id":"s_abc","tier":"session_summary",
 "tokens_before":148000,"tokens_after":12000}
```
`count_turns()` counts well-formed lines with `kind=="turn"` (torn-line tolerant, §10) and is used by the M-HIR computation (PRD 04) without coupling observe to compose.

### 4.2 `interventions.jsonl` — human interventions, drives M-HIR (PRD 04, ADR-0019)

```json
{"v":1,"ts":…,"session_id":"s_abc","kind":"task_override","note":"fix the parser not the formatter",
 "avoidability":"avoidable","harness_gap":"task_interface","burden":1,
 "source_message_id":"m_42"}
```
- `kind` ∈ the seven RK kinds (`task_override`, `manual_reflect`, `manual_groom`, `manual_verify`, `unverified_followup`, `tool_block`, `direct_edit`).
- **`avoidability`** ∈ `avoidable | unavoidable | benign`, **`harness_gap`** (which of the responsibilities), **`burden`** (0–3): these three (from the paper) are what make the metric *M*-HIR (missing-harness) rather than raw HIR — only non-`benign` records enter the numerator.
- `source_message_id` enables one-action-one-record dedup (PRD 04).

### 4.3 `security.jsonl` — blocked calls (PRD 02)

```json
{"v":1,"ts":…,"session_id":"s_abc","tool":"bash","checker":"CommandInjectionCheck",
 "pattern":"curl … | sh","args":{"command":"<redacted>"}}
```
`args` is redacted (§11). `checker` is the structured `PolicyError` variant name (ADR-0023), not free prose.

### 4.4 `entropy.jsonl` — per-turn entropy audit (PRD 04)

```jsonc
{"v":1,"ts":…,"session_id":"s_abc","kind":"entropy","turn_id":"turn_…","delta":-2,
 "findings":[{"category":"test_weakening",   // RK category; paper = test (snake_case RK enum, not a paper category name — ADR-0020 map, PRD 04)
              "severity":2,
              "description":"Removed assertion in auth_test.rs","evidence":"src/auth_test.rs:47"}]}
```
`category` is snake_case (§7); `severity` 0–3; `delta = -Σ severity`. Heuristics + the paper's 7-category map: PRD 04.

---

## 5. Episode package — `episodes/<turn_id>.json` (H3, PRD 05)

The paper's central output artifact. **Versioned** and carrying **all eight traces** (the prior spec omitted `context_trace`). Written when `RUSTYKEYS_HARNESS_LEVEL=h3`; a summary line also goes to `evidence.jsonl`. The package is assembled by the **episode-package assembly projector** (ADR-0036), which projects raw evidence into the typed trace elements defined in [§5.1](#51-trace-element-schemas-produced-by-the-assembly-projector--adr-0036).

```jsonc
{
  "schema_version": 1,
  "episode_id": "ep_<task_id>",          // groups all turns of ONE task (ADR-0018)
  "turn_id": "turn_20260527_143022_abc123",
  "task_id": "…",
  "harness_level": "h3",
  "ts": 1748346622.5,
  "initial_state": { "commit": "…", "workspace": "…" },

  "action_trace":   [ /* read_file | edit_file | run_tool | write_report | update_task_state | declare_complete */ ],
  "tool_trace":     [ {"name":"read_file","status":"ok","exit_code":0,"duration_ms":42,"timeout":false,"recovered":false,"result":"…"} ],
  "context_trace":  [ {"artifact":"src/auth.rs","contribution":"primary","influenced_decision":true} ],  // PAPER trace, previously MISSING
  "verification_trace": [ {"type":"deterministic_check","method":"registered_test","result":"pass","covers":["req-1"],"interpretation":"…"} ],
  "attribution_log":[ {"observed":"…","expected":"…","failure_type":"f_verify","layer":"compose","evidence":"…","alternatives":"…","next_action":"…"} ],
  "reproduction_log": { "check":"empty_password_probe","observed":"…","expected":"…" },
  "verification_report": { "requirements":[{"requirement":"req-1","met":true,"evidence":"…"}], "limits":"…" },
  "intervention_log":[ {"kind":"…","avoidability":"…","harness_gap":"…","burden":0} ],
  "entropy": { "delta": 0, "findings": [] },
  "outcome": "autonomous_verified_success"
}
```

- `failure_type` ∈ the fixed 8-member `FailureType` enum (`f_context`, `f_tool`, `f_feedback`, `f_verify`, `f_recovery`, `f_entropy`, `f_model`, `f_unknown`) — ADR-0021, PRD 05. No longer a free string.
- `verification_trace[].method` draws from a controlled vocabulary (bug_reproduction, deterministic_check, registered_test, targeted_test, full_regression, lint, patch_review, manual) — PRD 05.
- `outcome` ∈ `EpisodeOutcome` (§7).

### 5.1 Trace-element schemas (produced by the assembly projector — ADR-0036)

The eight-trace package is built by the **episode-package assembly projector** (ADR-0036): the named builder that projects raw evidence (the stream, `ToolOutcome`s, recall output, `CheckResult`s) into the typed traces below. These are the minimal on-disk shapes; the producing Rust structs live in PRD 05 (`EpisodePackage`) and PRD 04 (`Tracer`). All are **versioned with their parent package** (`schema_version`, ADR-0027) — they are not separately versioned records but elements of the §5 package, and unknown fields are ignored on read (§7).

**`ToolEvent` — one `tool_trace[]` element (F13).** Carries the structural `ToolStatus` plus the recovery-metric fields the prior 4-field shape lacked (`exit_code`, `timeout`, `recovered`), so the tool-recovery-rate metric has data:

```jsonc
{
  "name":        "read_file",   // tool name surfaced to aisdk
  "status":      "ok",          // ToolStatus (§7) — structural, not sniffed (ADR-0022)
  "exit_code":   0,             // process exit for shell-backed tools; null/omitted otherwise
  "timeout":     false,         // did the call hit its deadline? (pairs with status=timeout)
  "recovered":   false,         // did a later turn-step succeed after this one errored/timed out? — feeds tool-recovery-rate (ADR-0036)
  "duration_ms": 42,
  "result":      "…"            // redacted before write (§11, ADR-0026)
}
```

`recovered` is projected by the assembly projector (ADR-0036), not the tracer: it is knowable only across the episode (a retry/alternative after a failed call), so the projector sets it when assembling `tool_trace` rather than at `record_tool()` time.

**`ActionEvent` — one `action_trace[]` element (F11).** Distinct from `tool_trace` (the paper lists the action trace as a *separate* trace tied to episode coherence): an action is an externally-meaningful operation, not a 1:1 tool call.

```jsonc
{
  "op":     "edit_file",        // read_file | edit_file | run_tool | write_report | update_task_state | declare_complete
  "target": "src/auth.rs",      // file/report/task the op acted on (workspace-relative)
  "ts":     1748346622.5        // when the action was taken
}
```

The `op` set mirrors the §5 package example and PRD 05's `EpisodePackage`; ADR-0036/F11 also names the paper's `inspect_diff` op — reconciling the 6-op list here/§5/PRD-05 to the paper's full set is a follow-up owned by the §5 example / PRD 05.

**`ContextEntry` — one `context_trace[]` element (F12).** Projected from the `Vec<ContextEntry>` that `recall()`/`orient()` now emits (PRD 03), giving `context_trace` a real producer so the H2 cross-session-recall gate (defined on `influenced_decision`) is measurable:

```jsonc
{
  "artifact":            "src/auth.rs",   // a recalled memory title, a read file, or a static artifact (e.g. AGENT_GUIDE)
  "contribution":        "primary",       // primary | supporting | unused
  "influenced_decision": true             // did this artifact change what the agent did? (v1 heuristic in PRD 03)
}
```

**`VerifyEntry` — one `verification_trace[]` element (F14).** Carries the requirement-coverage link (`covers[]`) on every check, not only the deterministic path, so the report stays requirement-linked:

```jsonc
{
  "type":           "deterministic_check", // class of check
  "method":         "registered_test",     // controlled vocab (see note above)
  "result":         "pass",                // pass | fail | skipped
  "covers":         ["req-1"],             // requirement IDs this check evidences
  "interpretation": "…"                    // what the result means for the requirement
}
```

---

## 6. Sessions — `sessions/<session_id>.json`

Defines what `/resume [id]` restores and what a gateway/MCP `session_id` maps to (resolves the "session identity is implied but undefined" gap).

```json
{"v":1,"session_id":"s_abc","created_ts":…,"last_active_ts":…,
 "model":"anthropic/claude-opus-4-7","harness_level":"h1",
 "history":[{"role":"user","content":"…"},{"role":"assistant","content":"…"}],
 "task_id":null}
```
- `session_id` is the foreign key used by `observations`, all JSONL `session_id` fields, and the episode `task_id` lineage.
- History may be snapshotted here on `shutdown()`/idle and rehydrated on `/resume`; the observation stream (`stream.db`) remains the append-only source of record.

---

## 7. Serde wire conventions (ADR-0025)

- **All on-disk/wire enums use `#[serde(rename_all = "snake_case")]`.** This makes `EpisodeOutcome` (`autonomous_verified_success`), `ToolStatus` (`ok`/`error`/`blocked`/`timeout`/`truncated`), `InterventionKind`, `EntropyCategory` (`test_weakening`), `CompactionTier` (`session_summary`), and `FailureType` (`f_verify`) consistent. The prior spec mixed PascalCase, snake_case, and lowercase — this rule supersedes every inline example.

#### `ToolStatus` — the 5-member SSOT (resolves the 3-vs-5 contradiction, F13)

This file is the single source of truth for `ToolStatus`. The enum has **five** members; the earlier 3-member listing (`ok`/`error`/`blocked`) was incomplete and is superseded here so PRD 03 and PRD 04 agree:

```rust
#[serde(rename_all = "snake_case")]   // ADR-0025
pub enum ToolStatus {
    Ok,         // ok        — call succeeded
    Error,      // error     — call ran but failed (non-zero exit, ToolError)
    Blocked,    // blocked   — policy vetoed before dispatch (call() never ran)
    Timeout,    // timeout   — call exceeded its deadline
    Truncated,  // truncated — output was capped (e.g. grep 200-line / web 50k-char limit)
}
```

`ToolStatus` is the `status` field carried structurally by `ToolOutcome` (ADR-0022) and projected into `tool_trace[].status` (§5, below). The single formatter/parser in [`error-handling.md`](../dev/error-handling.md) is the only place this maps to/from the model-facing string.
- **Timestamps** are `ts: f64` epoch seconds everywhere (not RFC3339), matching existing examples.
- **Paths** are stored workspace-relative where possible; absolute paths only when they escape the workspace (and such writes are themselves policy-gated).
- **Unknown fields** are ignored on read (`#[serde(default)]` for added fields; never `deny_unknown_fields` on persisted types) so older binaries tolerate newer records (§9).

---

## 8. TOML / config-shaped files

### `task.json` (TaskState — PRD 03, with the added `scope` field)
```json
{"v":1,"goal":"Add empty-password validation","success_criteria":["rejects empty password","unit test added"],
 "scope":["crates/feed/","src/auth.rs"],"status":"active","updated_ts":…}
```
`scope: Vec<String>` is **new** — required by the entropy `BoundaryViolation` heuristic (PRD 04) which previously referenced a field that did not exist. `status` ∈ `idle | active | done`.

### `checks.toml` — deterministic check registry (H3, PRD 05)
Two locations with defined precedence: **project-level `harness/checks.toml`** (committed, shared baseline) loads first; **`.rustykeys/checks.toml`** (local, gitignored) overrides entries **by `name`** (local wins). Each entry:
```toml
[[checks]]
name = "empty_password_probe"
command = "cargo test auth::empty_password -- --nocapture"
expected_substring = "Password is required"
covers = ["req-1"]
```

### `ratchet.jsonl` — failed-turn attributions (P3)
One record per attribution on a failed turn: `{v, kind: "ratchet", ts, turn_id, failure_type, category, layer, check, evidence}`. `/ratchet` aggregates these by `(failure_type, category)` and *proposes* `checks.toml` stanzas for pairs that recur ≥ `RATCHET_MIN_OCCURRENCES` (2) times. Proposals are advisory: the harness never writes `checks.toml` itself (ADR-0040, "zero aspirational rules" — a check is only proposable from a logged attribution).

### MCP servers (PRD 07)
Servers are declared under the `[mcp]` table of the unified `config.toml` (`[[mcp.servers]]`); per-server fields are unchanged from PRD 07. The legacy standalone `mcp.toml` (top-level `[[servers]]`) is still read as a fallback when `config.toml` declares none. (Reminder: rename the example's `memory.db` to avoid colliding with the harness's `store.db`/`stream.db`.)

---

## 9. Versioning & forward-compatibility (ADR-0027)

- **Every JSONL record, episode package, and `task.json`/`session` file carries `v` / `schema_version`** (currently `1`).
- **Each SQLite/DuckDB database carries `PRAGMA user_version`** (currently `1`).
- **Read rule:** additive fields are backward/forward compatible (unknown fields ignored, §7). A bump of the integer version signals a **breaking** change; readers either run a registered migration (for DBs) or skip+log records they cannot parse (for JSONL). Migrations are forward-only.
- **No silent reinterpretation:** a record without `v` is treated as `v=0` (pre-versioning) and read with best-effort defaults.

---

## 10. Append-only durability

- **One record = one `write_all` of a single `\n`-terminated line**, then `flush`. An optional `fsync`-per-record mode is gated by config for durability-critical deployments (default off; the OS page cache is acceptable for local-first use).
- **Torn-line recovery:** on read, a trailing line lacking a terminating `\n`, or any line that fails to parse, is **skipped and logged** — never fatal. A crash mid-append therefore costs at most the last partial record.
- **`count_turns()` and all scanners are torn-line tolerant** by construction (skip-on-parse-error), so the M-HIR denominator never chokes on a partial final line.
- Log rotation/retention remains a future seam (ADR-0015); it does not affect the record schema.

---

## 11. Secret redaction (ADR-0026 — summary; full rule in threat-model.md)

Tool `args` and results may contain secrets (MCP/SSE auth tokens, web API keys, anything the model passes as an argument). **Before** a `ToolEvent` is written to `evidence.jsonl`/`security.jsonl`, included in an episode package, or emitted over `/evidence` or `rk://tool_event`, a redaction pass scrubs:
- argument keys matching `*token*`, `*key*`, `*secret*`, `auth*`, `password*` → value replaced with `"<redacted>"`;
- high-entropy value patterns (long base64/hex) in string fields.

Redaction must **not** remove evidence that attribution/verification traces depend on (it scrubs values, not structure). The exact deny-list and value patterns are owned by [`threat-model.md`](./threat-model.md).
