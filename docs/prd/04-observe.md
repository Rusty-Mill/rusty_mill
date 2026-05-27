# PRD 04 — Observe

## Responsibility

The observe layer provides **structured visibility** into every kernel turn. It
has three components:

- **`Tracer`**: captures the episode (tool events, token counts, stop reason)
  as evidence for the compose layer's verifier.
- **`InterventionLogger`**: records human interventions and surfaces the M-HIR
  metric (Missing-Harness Human Intervention Rate).
- **`EntropyAuditor`**: detects maintenance burden introduced by the agent and
  records it as a per-turn entropy audit.

Observe is read-only with respect to the kernel — it watches without directing.

## Tracer

### What it captures

Every tool call is recorded as it happens via the `on_event` callback the kernel
exposes. At turn end the tracer holds a complete episode:

```rust
pub struct Episode {
    pub tool_events: Vec<ToolEvent>,       // raw dispatch-level record (every tool call)
    pub action_events: Vec<ActionEvent>,   // externally-meaningful operations (NOT a copy of tool_events)
    pub final_reached: bool,
    pub total_tokens: u64,
}

pub struct ToolEvent {
    pub name: String,
    pub args: serde_json::Value,   // redacted before logging (see below)
    pub outcome: ToolOutcome,      // structured status + result (ADR-0022)
    pub exit_code: Option<i32>,    // process exit code for run_tool/bash; None for in-proc tools
    pub timeout: bool,             // the call hit its deadline
    pub recovered: bool,           // a later turn step succeeded after this call failed (tool-recovery signal)
    pub duration_ms: u64,
}
```

**Status is structural, not sniffed.** The tracer reads `ToolOutcome.status`
directly — it does *not* re-parse the result string. The earlier design inferred
`ToolStatus` from magic prefixes (`BLOCKED …` → `Blocked`, `ERROR …`/`TIMEOUT …`
→ `Error`, else `Ok`); any tool whose legitimate output began with one of those
words was misclassified. Per **ADR-0022**, `ToolOutcome` carries the status as a
field, produced once by the dispatch boundary and consumed unchanged by observe
and compose. **`ToolStatus` is the reconciled five-variant set
`{ ok, error, blocked, timeout, truncated }`** (snake_case per ADR-0025) — the
authoritative enum lives in [`data-model.md`](../architecture/data-model.md) §7
(SSOT); `timeout` and `truncated` are first-class statuses, not inferred from a
boolean flag. The single formatter/parser that renders `ToolOutcome` to/from the
model-facing string is the authoritative `ToolResultClassifier`; its contract
lives in [`docs/dev/error-handling.md`](../dev/error-handling.md) and the type's
serde encoding in data-model §7. `NoToolErrors` (PRD 05) reads `outcome.status`
rather than scanning text.

**`tool_trace` records exit/timeout/recovery (F13).** Beyond `status`, each
`ToolEvent` carries `exit_code` (the process exit code for `run_tool`/`bash`
shellouts; `None` for in-process tools), `timeout` (the call hit its deadline),
and `recovered` (a later step in the same episode succeeded after this call
failed — the substrate for the eval plan's tool-recovery-rate metric). These
populate the episode package's `tool_trace` (PRD 05, data-model §5) directly.

**Tool args are redacted before logging (ADR-0026).** `ToolEvent.args` and the
result may contain secrets (auth tokens, API keys, anything the model passes as
an argument). A redaction pass scrubs deny-listed argument keys and high-entropy
values **before** a `ToolEvent` reaches `evidence.jsonl`, an episode package, or
any `/evidence` / `rk://tool_event` emission. Redaction scrubs *values, not
structure*, so the attribution and verification traces that depend on tool
events stay intact. The deny-list and value patterns are owned by
[`threat-model.md`](../architecture/threat-model.md); see data-model §11.

### `ActionEvent` — the `action_trace` producer (ADR-0036, F11)

`action_trace` and `tool_trace` are **distinct traces, not two views of the same
list.** `tool_trace` is the raw dispatch-level record — every `[tool]` call the
kernel made, with its `ToolOutcome.status`. `action_trace` is the higher-level
record of **externally-meaningful operations**: the things the agent *did to the
world or the task* that matter for audit and attribution, regardless of how many
tool calls realised them.

The tracer emits an `ActionEvent` (alongside the `ToolEvent`) when the kernel
performs one of these operation kinds:

- `read_file`, `edit_file`, `run_tool` — file and tool side-effects
- `write_report` — the agent produced a verification report
- `update_task_state` — the agent set/changed the active `TaskState`
- `inspect_diff` — the agent reviewed a diff
- `declare_complete` — the agent asserted the task is done

```rust
pub struct ActionEvent {
    pub kind: ActionKind,            // the operation, not the underlying tool name
    pub target: Option<String>,      // file path / report id / task id (redacted, ADR-0026)
    pub tool_event_idx: Option<usize>, // back-pointer into Episode.tool_events when one tool realised it
    pub ts: f64,
}

pub enum ActionKind {                // snake_case on the wire (ADR-0025)
    ReadFile, EditFile, RunTool, WriteReport, UpdateTaskState, InspectDiff, DeclareComplete,
}
```

Why not a copy of `tool_trace`: not every tool call is an externally-meaningful
action (a recall lookup or a context-assembly probe is a tool call but not an
*action*), and not every action is one tool call (an `edit_file` action may be
preceded by a read; a `declare_complete` may be a control-flow signal with no
dedicated tool). The `compose`-time **assembly projector** (PRD 05, ADR-0036)
builds the package's typed `action_trace` from `Episode.action_events`; it does
**not** re-derive it by relabelling `tool_trace`.

### Episode lifecycle

```rust
impl Tracer {
    pub fn start_episode(&mut self);   // reset tool_events + action_events; tokens stay cumulative
    pub fn record_tool(&mut self, event: ToolEvent);
    pub fn record_action(&mut self, action: ActionEvent);  // externally-meaningful op (F11)
    pub fn record_turn(&mut self, step: usize, n_tools: usize, tokens: u64);
    pub fn set_final_reached(&mut self, reached: bool);
    pub fn episode(&self) -> &Episode;
}
```

`start_episode()` is called at the start of each `Session::send()`. The episode
is consumed by `Verifier::verify()` after the kernel run completes.

### Structured logging

When `RUSTYKEYS_TRACE=1`, the tracer emits structured lines to stderr:

```
[trace] turn 1: tool_calls=2 | tokens so far=1423
[trace]   -> tool read_file({"path":"src/main.rs"}) => // ... content ...
[trace] turn 2: final response | tokens so far=2187
```

Evidence is accumulated regardless of the trace flag — the verifier always has
a complete episode.

### Rust advantages

- `ToolEvent` is a value type; status lives in `ToolOutcome` as data, so there is
  no per-event string re-parse — no heap allocation per field beyond the strings.
- `start_episode()` clears the `tool_events` and `action_events` `Vec`s in place
  — `Vec::clear()` retains allocated capacity, so steady-state operation does not
  allocate.
- The tracer is `!Send` by design (owned by the `Session`, never shared) —
  no lock needed.

## InterventionLogger

### What is an intervention

An intervention is any human action that compensates for a missing or insufficient
harness capability. The metric is **M**-HIR — *Missing-Harness* Human Intervention
Rate — not raw HIR: only interventions that reflect a harness gap enter the
numerator. The paper characterises an intervention by its **avoidability**, the
**harness gap** it corresponds to, and the **burden** it imposed; RK's seven
UI-observable kinds are mapped onto those three attributes (ADR-0019).

#### M-HIR computation (v1 intent)

```
M-HIR(window) = count(interventions where avoidability == "avoidable") / denom
denom         = count(turns)        # RK unit = turn (one Session::send())
```

- **Numerator — only `avoidable` records count (D2/F23).** An intervention enters
  the numerator *only* when it represents runtime support a maturing harness could
  have closed — the help "the human would otherwise have to provide" (paper p.4).
  Both other classes are **excluded**: a `benign` intervention (e.g. the user types
  a normal follow-up the agent had already handled correctly, or a default
  `manual_verify`) is healthy, not a gap; and an `unavoidable` intervention (an
  `tool_block` where the permission boundary correctly stopped a disallowed action)
  is *the policy working as intended*, not a missing harness. Counting only
  `avoidable` is exactly what makes the metric *M*-HIR (missing-harness) rather than
  raw HIR; without `avoidability`, the log would count every human action. (A
  *recurring* `tool_block` on the same legitimate action is a policy gap and may be
  reclassified `avoidable`, at which point it *does* count — see the kinds table.)
- **Denominator = turns, not episodes (DIVERGENCE).** The paper defines
  M-HIR per *episode* (one full task attempt). RK's denominator is *turns*
  (`count(turns)` from `EvidenceJournal::count_turns()`), because RK's unit of
  evaluation is the turn, not the task. This is a deliberate, documented
  faithfulness divergence — see **ADR-0018** (episode = turn, with `episode_id`
  grouping); task-level M-HIR is recovered by aggregating over `episode_id` in
  the eval plan, not in the hot path.
- **Session-boundary rule.** `count_turns()` spans every session under
  `.rustykeys/`, so the all-time rate is cross-session. The per-session rate
  uses the `session_id` carried on each `turn` and `intervention` record
  (data-model §4.1/§4.2); `trend` (below) is rate *per session* for the
  sparkline, while the headline `rate` is all-time cumulative. Both are surfaced
  explicitly so neither is over-read.
- **Double-count rule — one user action → at most one record.** If a single
  message would trigger more than one kind (e.g. a `/task` override *and* an
  `unverified_followup`), record only the most specific (`task_override`). Dedup
  is by `source_message_id` (data-model §4.2): a record is dropped if one with
  the same `source_message_id` already exists for the message.
- **Reset rule.** The denominator **never auto-resets**; the log is append-only
  and persisted across sessions, so the cumulative trend is always visible. There
  is no decay or windowing applied to the stored records — windowing (if any) is
  a read-time concern of the consumer.

The record schema (with `avoidability` / `harness_gap` / `burden` /
`source_message_id`) is owned by [`data-model.md`](../architecture/data-model.md)
§4.2.

### Intervention kinds → avoidability / harness_gap / burden

The seven kinds are RK's UI-facing taxonomy; each carries the three paper-aligned
attributes that drive the M-HIR numerator (ADR-0019). `avoidability` and `burden`
below are the **v1 intent** defaults — they are the recorded-at-capture
classification and may be re-tuned after a spike; a kind marked `benign` here can
still be recorded as `avoidable` when context warrants (e.g. a `manual_verify`
that catches a real miss). `harness_gap` names which of the eleven harness
responsibilities the intervention points at.

| Kind | Trigger | Signals | `avoidability` | `harness_gap` | `burden` |
|---|---|---|---|---|---|
| `task_override` | User sets `/task` when one is already active | Agent drifted or misunderstood | `avoidable` | `task_interface` | 1 |
| `manual_reflect` | User runs `/reflect` or `/sleep` | Idle consolidation didn't fire | `avoidable` | `memory` | 1 |
| `manual_groom` | User runs `/groom` | Skills accumulated without auto-grooming | `avoidable` | `memory` | 1 |
| `manual_verify` | User inspects `/verify` | User didn't trust the agent's reply | `benign`¹ | `verification` | 0 |
| `unverified_followup` | User sends a message after an unverified turn | Agent produced a bad answer | `avoidable` | `verification` | 2 |
| `tool_block` | User blocks a tool approval request | Agent tried a disallowed action | `unavoidable`² | `permissions` | 1 |
| `direct_edit` | User edits a file directly in the editor (desktop only) | Agent output not trusted | `avoidable` | `tools` | 3 |

¹ `manual_verify` is `benign` by default (inspecting evidence is healthy, not a
harness failure) and so does **not** enter the numerator; promote to `avoidable`
only if the inspection surfaces a missed defect. ² `tool_block` is
`unavoidable` (the policy working as intended is *not* a missing-harness signal),
but a *recurring* block on the same legitimate action indicates a policy gap and
may be reclassified `avoidable`. **Only `avoidable` records count toward M-HIR
(D2/F23);** both `unavoidable` (a correct `tool_block`) and `benign` are excluded.
An `unavoidable` `tool_block` is the policy doing its job and is therefore *not* a
missing-harness signal — it counts only after reclassification to `avoidable`.

### Storage

Append-only JSONL at `.rustykeys/interventions.jsonl`. The record schema is owned
by [`data-model.md`](../architecture/data-model.md) §4.2 — shown here for context,
not re-specified:

```json
{"v":1,"ts":1234567890.5,"session_id":"s_abc","kind":"task_override",
 "note":"fix the parser not the formatter",
 "avoidability":"avoidable","harness_gap":"task_interface","burden":1,
 "source_message_id":"m_42"}
```

```rust
pub struct InterventionLogger {
    path: PathBuf,
}

pub struct InterventionRecord {
    pub kind: InterventionKind,
    pub note: String,
    pub avoidability: Avoidability,   // Avoidable | Unavoidable | Benign (ADR-0019)
    pub harness_gap: String,          // which of the 11 responsibilities
    pub burden: u8,                   // 0–3
    pub source_message_id: String,    // dedup key (one action → one record)
}

pub enum Avoidability { Avoidable, Unavoidable, Benign }  // snake_case on the wire

impl InterventionLogger {
    /// De-dupes by `source_message_id`; classifies kind → avoidability/harness_gap/burden.
    pub fn record(&self, kind: InterventionKind, note: &str,
                  source_message_id: &str) -> Result<()>;
    pub fn recent(&self, n: usize) -> Result<Vec<InterventionRecord>>;
    /// Numerator counts only records where `avoidability == Avoidable` (M-HIR, not raw HIR; D2/F23).
    pub fn mhir(&self, total_turns: usize) -> MhirReport;
}

pub struct MhirReport {
    pub n_interventions: usize,   // avoidable only (the M-HIR numerator — D2/F23)
    pub n_unavoidable: usize,     // recorded but excluded (e.g. correct tool_block) — surfaced for transparency
    pub n_benign: usize,          // recorded but excluded — surfaced for transparency
    pub n_turns: usize,           // denominator = turns (ADR-0018 divergence)
    pub rate: f64,                // all-time cumulative
    pub breakdown: HashMap<InterventionKind, usize>,
    pub trend: Vec<f64>,          // rate per session for sparkline (not cumulative)
}
```

`total_turns` comes from `EvidenceJournal::count_turns()` — the observe layer
has no coupling to the compose layer's journal; `Session` passes the count in.
`count_turns()` is torn-line tolerant (data-model §10), so the denominator never
chokes on a partial final line.

### Detecting `unverified_followup`

`Session` tracks `last_report: Option<VerificationReport>`. Before processing
a regular user message, if `last_report.is_some_and(|r| !r.verified)`, the
logger records `unverified_followup` against that message's `source_message_id`.
This is a one-field state check — no LLM call, no file I/O in the hot path. Per
the double-count rule above, if the same message also triggers a `task_override`,
only the more specific `task_override` is kept (deduped by `source_message_id`).

## EntropyAuditor

### Motivation

Autonomous agents do not only produce solutions — they also introduce maintenance
burden: residue files, weakened tests, stale documentation, unnecessary
dependencies, architectural violations. These do not break the immediate task
but degrade the project over time. The `EntropyAuditor` makes this burden
observable and auditable. (See AI Harness Engineering paper §Implications:
"Entropy is part of autonomous engineering.")

No equivalent exists in Claude Code or hermes-agent. This is a genuine
capability improvement.

### Data structures

```rust
pub struct EntropyAudit {
    pub findings: Vec<EntropyFinding>,
    pub delta: i32,   // net entropy score: negative = burden introduced
}

pub struct EntropyFinding {
    pub category: EntropyCategory,
    pub severity: u8,          // 0–3 (0 = informational, 3 = significant burden)
    pub description: String,
    pub evidence: String,      // file path + line or tool call reference
}

pub enum EntropyCategory {       // snake_case on the wire (ADR-0025)
    Residue,           // debug scripts, temp files, dead/redundant code left behind
    TestWeakening,     // test removed, assertion loosened, #[ignore] added
    StaleDocs,         // doc comment removed or contradicted by code change
    DependencyChurn,   // dep added then removed same turn, or unused dep added
    BoundaryViolation, // file written outside declared architecture layer
    TaskContradiction, // comment contradicts active TaskState goal
}
```

#### Paper → RK category map (ADR-0020)

The AI Harness Engineering paper enumerates **seven** entropy categories; RK has
**six**. The two sets do not line up one-to-one, so entropy-delta comparisons
against the paper go through this reconciliation. RK merges the paper's *code*
(redundant/dead code) into `Residue` alongside *file-residue*, and renames the
paper's *workflow* category to `TaskContradiction`. The RK enum is unchanged; a
finding is translated to the paper's vocabulary only for cross-paper comparison.

| Paper category | RK `EntropyCategory` | Note |
|---|---|---|
| code | `Residue` | dead/redundant code folded in with file-residue |
| file-residue | `Residue` | debug scripts, temp files, `.bak`/`.orig` |
| test | `TestWeakening` | 1:1 |
| documentation | `StaleDocs` | 1:1 |
| dependency | `DependencyChurn` | 1:1 |
| architecture | `BoundaryViolation` | 1:1 |
| workflow | `TaskContradiction` | renamed |

(The exact seven paper categories and the 0–3 severity scale are pending human
confirmation against the rendered PDF — see the PDF verification caveat in
ARCHITECTURE.md §12.)

### Detection heuristics + severity thresholds (v1 intent)

`EntropyAuditor::audit(episode, task_scope)` inspects `ToolEvent` records
synchronously after the kernel run — **no LLM call**. Severity is **0–3**
(0 = informational, 1 = minor, 2 = notable, 3 = significant burden). These are
*syntactic* heuristics over `edit_file`/`write_file` args; the semantic cases
(`StaleDocs`, `TaskContradiction`) are best-effort until the LLM-assisted seam
(see Seams) lands. Globs and thresholds are **v1 intent** — the design to build
against, revisit after a spike.

| Category (paper map) | Heuristic | Severity |
|---|---|---|
| `Residue` (code + file-residue) | `write_file` to glob `{debug_*, tmp_*, *.bak, *.orig, scratch*, test_scratch.*}` → **2**; file written but never re-read/edited/referenced by a later `tool_event` in the same turn → **1**; commented-out block ≥10 lines added via `edit_file` → **1** | 1–2 |
| `TestWeakening` (test) | `edit_file` on path matching `{*_test.*, *spec*, test_*, tests/*}` whose new content removes ≥1 `assert*` / `expect(` / `#[test]`, **or** adds `#[ignore]` / `.skip(` / `xit(` / `@pytest.mark.skip` → **3**; net assertion-token count decreases (count `assert`/`expect` tokens old vs new) → **2** | 2–3 |
| `StaleDocs` (documentation) | `edit_file` whose new content changes a `fn` / `def` / `function` signature line but leaves the immediately-preceding doc block (`///`, `/**`, `"""`, `#`) unchanged → **1**; doc comment deleted with no replacement → **2** | 1–2 |
| `DependencyChurn` (dependency) | within one turn, a dep added then removed across `{Cargo.toml, package.json, pyproject.toml}` edits → **2**; dep added but no source file in the turn references it (import/`use` scan) → **1** | 1–2 |
| `BoundaryViolation` (architecture) | `write_file` / `edit_file` to a path outside `TaskState.scope` (the `scope: Vec<String>` field — data-model §8) → **3**; write crossing a declared crate/layer boundary not named in the task → **2** | 2–3 |
| `TaskContradiction` (workflow) | an added comment/string literal contains a negation of an active `TaskState.goal` keyword (lexical overlap + negation token) → **1** (raised to **2** only under the LLM-assisted seam) | 1 |

**Score and gate.** The net entropy score is

```
delta = -Σ severity        # over all findings; negative = burden introduced
```

Entropy is **non-blocking/informational** — findings are surfaced but do not flip
`verified` (see Lifecycle). They do, however, feed the H3 outcome classifier:
`UnsafeInvalid` is triggered by any `TestWeakening` **or** `BoundaryViolation`
finding with `severity >= 2` (consistent with the paper's `unsafe_invalid`
definition — "tests are weakened, unrelated destructive edits occur, or the task
is bypassed"). This trigger is owned by the `EpisodeOutcome` classifier in PRD 05.

### Lifecycle

`EntropyAuditor::audit()` runs in the post-turn `tokio::join!` alongside the
`CriteriaJudge` and idle consolidation:

```rust
let (judge_result, consolidation_result, entropy_audit) = tokio::join!(
    criteria_judge.run(&reply),
    memory.consolidate(ConsolidationScope::Idle),
    entropy_auditor.audit(tracer.episode(), task.scope()),  // scope for BoundaryViolation
);
```

The audit result is:
- Appended to `.rustykeys/entropy.jsonl` (append-only JSONL)
- Included in `VerificationReport` as a non-blocking field (findings shown but
  don't fail `verified` — entropy is informational, not a gate)
- Available via `/entropy` CLI command and the desktop harness dashboard

### Storage

Schema owned by [`data-model.md`](../architecture/data-model.md) §4.4; `category`
is snake_case (ADR-0025), `delta = -Σ severity`:

```json
{"v":1,"ts":1234567890.5,"session_id":"s_abc","kind":"entropy","turn_id":"turn_…",
 "delta":-2,
 "findings":[{"category":"test_weakening","severity":2,
              "description":"Removed assertion in auth_test.rs",
              "evidence":"src/auth_test.rs:47"}]}
```

```rust
pub struct EntropyAuditor {
    log_path: PathBuf,
}

impl EntropyAuditor {
    /// `task_scope` is `TaskState.scope` (data-model §8); empty ⇒ BoundaryViolation skipped.
    pub async fn audit(&self, episode: &Episode, task_scope: &[String])
        -> Result<EntropyAudit>;
    pub fn recent(&self, n: usize) -> Result<Vec<EntropyAudit>>;
    pub fn cumulative_delta(&self) -> Result<i32>;
}
```

## Seams

- **OpenTelemetry**: aisdk's roadmap includes OTel support. When available,
  wire span-per-tool-call and span-per-turn to the tracer — the `ToolEvent`
  struct maps cleanly to OTel span attributes.
- **Sampling**: for high-volume deployments, the tracer could sample episodes
  rather than capturing all of them.
- **Structured log sink**: today stderr; a future seam routes to a structured
  log collector (e.g. `tracing` crate subscribers).
- **LLM-assisted entropy detection**: today heuristic-only; a future seam runs
  a lightweight aisdk call to detect semantic contradictions (e.g. TaskContradiction
  with richer context than a string match).
- **Entropy dashboard**: `/entropy history` and cumulative score surfaced in the
  CLI alongside `/mhir`; the desktop harness dashboard (PRD 08) renders a
  per-turn bar chart.
