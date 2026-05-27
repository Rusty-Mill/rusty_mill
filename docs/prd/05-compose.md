# PRD 05 — Compose

## Responsibility

The compose layer takes the kernel's raw reply and packages it into an
**evidentiary output** — a verified, auditable record of what happened, not a
bare claim. It runs after the kernel returns and before the reply reaches the
caller.

Four responsibilities:
1. **Shape** the reply (whitespace trimming, format normalisation)
2. **Verify** the episode against deterministic and semantic checks
3. **Journal** the turn as a documented, transferable record
4. **Classify** the turn outcome under the five-label taxonomy (H3)

All wire enums in this layer — `EpisodeOutcome`, `FailureType`, and the embedded
`ToolOutcome`/`EntropyCategory` — carry `#[serde(rename_all = "snake_case")]`
(ADR-0025; e.g. `autonomous_verified_success`, `f_verify`). The authoritative
enum list and on-disk schemas live in
[`data-model.md`](../architecture/data-model.md) §5/§7.

## Design

### Check trait

A check inspects the final reply and the episode evidence, returns a verdict.

```rust
pub trait Check: Send + Sync {
    fn name(&self) -> &str;
    fn run(&self, reply: &str, episode: &Episode) -> CheckResult;
}

pub struct CheckResult {
    pub name: String,
    pub passed: bool,
    pub detail: String,
}
```

Checks are synchronous by default (deterministic, no I/O). The `CriteriaJudge`
is the exception — it is `async` and handled separately (see below).

### Default checks

**`NoToolErrors`**: fails if any tool event has `outcome.status = error | blocked`
(read structurally from `ToolOutcome`, ADR-0022 — not sniffed from the result
string). Guards against the agent asserting success on top of a failed action.

**`CleanTermination`**: fails if `episode.final_reached = false`. The loop hit
`max_steps` without producing a final answer.

### H3 checks (active when `RUSTYKEYS_HARNESS_LEVEL=h3`)

**`ReproduceBeforeEdit`**: fails if `edit_file` or `write_file` was called in
the episode without a prior `attribute_failure` tool call. Enforces the
reproduce → attribute → fix discipline.

**`VerificationReportRequired`**: fails if the episode ended without a
`verification_report` tool call. Enforces that the agent documents its evidence
before completing.

### Verifier

```rust
pub struct Verifier {
    pub checks: Vec<Box<dyn Check>>,
    pub limits: String,
}

impl Verifier {
    pub fn verify(&self, reply: &str, episode: &Episode) -> VerificationReport;
    pub async fn verify_with_judge(
        &self, reply: &str, episode: &Episode, judge: &CriteriaJudge,
    ) -> VerificationReport;
}
```

`limits` describes what the checks did *not* verify — always surfaced alongside
the verdict so "verified" is never over-read. Three constants:

```rust
pub const DETERMINISTIC_LIMITS: &str =
    "deterministic checks only; semantic correctness and task success not verified";

pub const SEMANTIC_LIMITS: &str =
    "LLM-judge on active task criteria included; \
     output quality beyond stated goals not evaluated";

pub const H3_LIMITS: &str =
    "H3 protocol: reproduction, attribution, deterministic checks, and \
     criteria judge included; full regression may be bounded by timeout";
```

### VerificationReport

```rust
pub struct VerificationReport {
    pub verified: bool,
    pub checks: Vec<CheckResult>,
    pub attributions: Vec<Attribution>,
    pub entropy: Option<EntropyAudit>,   // from EntropyAuditor (PRD 04)
    pub outcome: Option<EpisodeOutcome>, // H3 only
    pub judge_ran: bool,                 // false ⇒ judge_unavailable; bars autonomous-verified
    pub limits: &'static str,
}
```

`verify_with_judge()` threads the judge's `judge_ran` flag (see CriteriaJudge)
into the report so the outcome classifier can bar `AutonomousVerifiedSuccess`
when the judge was unavailable — an unavailable judge is never read as verified.

`render()` — human-readable multi-line output for `/verify`.
`as_observation()` — compact one-line for the memory stream learning signal.
`to_json()` — serialised for the evidence journal.

### Failure attribution

On an unverified turn each failed check is classified into a `(category, layer)`
pair **and** mapped onto the paper's fixed eight-member `FailureType` taxonomy
(ADR-0021). `category` and `layer` were free strings; they are now drawn from a
frozen matrix so attribution is aggregatable and comparable to the paper.

```rust
pub enum FailureType {       // snake_case on the wire (ADR-0025): f_context, …
    FContext,    // wrong/missing context fed to the model
    FTool,       // tool errored, was blocked, or behaved incorrectly
    FFeedback,   // tool result/observation not surfaced or misread
    FVerify,     // verification absent, skipped, or judged unavailable
    FRecovery,   // failed to recover after an error (loop, no retry)
    FEntropy,    // maintenance burden introduced (tests weakened, residue)
    FModel,      // model reasoning/output defect with adequate harness
    FUnknown,    // unattributable
}

pub struct Attribution {
    pub check: String,
    pub failure_type: FailureType,   // fixed enum, not a free string (ADR-0021)
    pub category: String,            // frozen vocabulary (matrix below)
    pub layer: String,               // frozen vocabulary (matrix below)
    pub evidence: String,
}
```

#### Frozen `(category, layer)` → `FailureType` matrix

Each failed check maps to exactly one row. Adding a row is an explicit,
exhaustively-checked change, not a new ad-hoc string. The `attribute_failure` H3
tool (PRD 03) takes `failure_type` / `layer` as enum-valued, not free strings.

| Check (trigger) | `category` | `layer` | `FailureType` |
|---|---|---|---|
| `no_tool_errors` (blocked) | `permission_block` | `constrain/policy` | `f_tool` |
| `no_tool_errors` (error) | `tool_error` | `feed/tools` | `f_tool` |
| `clean_termination` | `non_termination` | `kernel/loop` | `f_recovery` |
| `criteria_judge` (unmet) | `criteria_unmet` | `compose/semantic` | `f_model` |
| `criteria_judge` (unavailable) | `judge_unavailable` | `compose/semantic` | `f_verify` |
| `reproduce_before_edit` | `attribution_skipped` | `compose/h3` | `f_verify` |
| `verification_report_required` | `evidence_missing` | `compose/h3` | `f_verify` |
| entropy `severity >= 2` (TestWeakening / BoundaryViolation) | `entropy_unsafe` | `observe/entropy` | `f_entropy` |

`f_context` and `f_feedback` are reserved for attributions raised by the
`attribute_failure` tool during H3 reproduction (no deterministic check emits
them); `f_unknown` is the fallback when no row matches. The `FailureType` enum
and its serde encoding are owned by
[`data-model.md`](../architecture/data-model.md) §5/§7.

Attribution feeds back as the targeted learning signal: consolidation captures
*why* the turn failed, not just *that* it failed.

## CriteriaJudge (semantic check)

When the active `TaskState` has `success_criteria`, the criteria judge fires an
async aisdk call to evaluate the reply against each criterion independently.

```rust
pub struct CriteriaJudge {
    pub model: String,
    pub task_store: Arc<TaskStore>,
    pub self_consistency: u8,   // n=1 default; 2–3 enables majority vote (see below)
}

impl CriteriaJudge {
    /// On a clean call: passed = verdict, judge_ran = true.
    /// On call/parse failure: passed = false, judge_ran = false (NOT a silent pass).
    pub async fn run(&self, reply: &str) -> JudgeResult;
}

pub struct JudgeResult {
    pub result: CheckResult,   // name = "criteria_judge"
    pub judge_ran: bool,       // false ⇒ judge_unavailable; bars AutonomousVerifiedSuccess
}
```

### Prompt design

```
You are a success-criteria judge for an AI assistant.

Task goal: {goal}

Success criteria — all must be satisfied for the task to be complete:
1. {criterion_1}
2. {criterion_2}
…

Assistant reply:
{reply}

A criterion is met only if the reply clearly and explicitly addresses it —
do not infer what is not stated.

Return ONLY valid JSON:
{"verdict": "pass"|"fail", "criteria": [{"criterion": "…", "met": bool, "reason": "…"}]}
```

A criterion is only `met` if explicitly addressed — strictness is intentional.
The agent must state the outcome, not merely do the work silently.

### Self-consistency (optional)

For borderline criteria, the judge can be run `n = 2–3` times and the per-criterion
verdict decided by majority (`self_consistency` field). This trades latency/cost
for stability on ambiguous wording; it stays off the reply path (it runs inside
the post-turn `tokio::join!`). Default `n = 1`. A tie on an even `n` resolves to
`fail` (strictness preserved).

### Concurrency

In Keystone the criteria judge was a synchronous blocking call that added
visible latency between the reply appearing and the next prompt. In Rusty Keys,
the judge, idle consolidation, and entropy audit run concurrently via
`tokio::join!` after the reply is already in the caller's hands:

```rust
let (judge_result, consolidation_result, entropy_audit) = tokio::join!(
    criteria_judge.run(&reply),
    memory.consolidate(ConsolidationScope::Idle),
    entropy_auditor.audit(tracer.episode(), task.scope()),
);
```

All three complete while the user is reading the reply. The learning signals
from all three are observed together before the next turn's recall.

### Degradation — no silent pass-as-verified

If the aisdk call fails or returns unparseable JSON, `CriteriaJudge::run()` does
**not** return a passing result. It records a failed `CheckResult` named
`criteria_judge` with category `judge_unavailable`, sets `judge_ran = false`, and
attaches a diagnostic note. The judge is best-effort in that it does not *block*
the turn (the reply is already in the caller's hands), but an unavailable judge
must never be read as "criteria met":

- The turn cannot be classified `AutonomousVerifiedSuccess` when `judge_ran =
  false` (the `EpisodeOutcome` classifier bars it — see Outcome taxonomy); it
  degrades to `UnverifiedSuccess` (or `Failed` if other checks also fail).
- A `judge_unavailable` attribution (`f_verify`, see the matrix above) is
  recorded so consolidation learns the verification path was incomplete.
- `judge_unavailable` is counted by the eval plan's judge-unavailable rate so a
  flaky judge is observable rather than masquerading as success.

## H3 workflow — reproduce → attribute → fix → verify → report (with back-edge)

The H3 discipline is the paper's central loop. RK enforces its spine through the
`reproduce_before_edit` and `verification_report_required` checks (above) and the
`attribute_failure` tool (PRD 03):

```
reproduce ──▶ attribute ──▶ fix ──▶ verify ──▶ report
                  ▲                   │
                  └─── re-attribute ──┘   (back-edge: verify failed)
```

**The back-edge is load-bearing.** When `verify` fails — a deterministic check
or the criteria judge does not pass after a fix — the agent does **not** proceed
to `report`. It loops back to `attribute` to re-diagnose with the new evidence
(the failed verification is itself an observation), producing a fresh
`attribution_log` entry. The previous spec drew only the forward spine; without
the back-edge a failed verification had nowhere to go but a premature report.
Each pass through the loop appends to `attribution_log` and `verification_trace`,
so the episode package records the full reproduce/attribute/verify history, not
just the final state. The loop terminates at `report` only once `verify` passes
(or `max_steps`/timeout is hit, which classifies `Failed`).

## H3 Episode Package

When `RUSTYKEYS_HARNESS_LEVEL=h3`, each turn produces a full episode package
(the AI Harness Engineering paper's central output artifact): an auditable record
containing all eight trace types.

### DeterministicCheck registry

```rust
pub struct DeterministicCheck {
    pub name: String,
    pub command: String,            // bash command to execute
    pub expected_substring: String, // output must contain this string
    pub covers: Vec<String>,        // requirement IDs this check covers
}

pub struct CheckRegistry {
    checks: Vec<DeterministicCheck>,
}

impl CheckRegistry {
    pub fn load_from_file(path: &Path) -> Result<Self>;  // loads checks.toml
    pub async fn run_all(&self) -> Vec<CheckRunResult>;
}

pub struct CheckRunResult {
    pub check: String,
    pub observed: String,
    pub expected: String,
    pub passed: bool,
    pub duration_ms: u64,
}
```

Loaded from `.rustykeys/checks.toml` (or `harness/checks.toml` for
project-level checks). Agent-visible at H3; used by evaluator at all levels.

### Outcome taxonomy

Every H3 turn is classified under the five-label taxonomy from the AI Harness
Engineering paper:

```rust
pub enum EpisodeOutcome {       // snake_case on the wire (ADR-0025)
    /// All checks pass; verification report produced; judge ran; no interventions.
    AutonomousVerifiedSuccess,
    /// Checks pass but interventions were recorded during the turn.
    AssistedVerifiedSuccess,
    /// Task appears done but no verification report, OR the judge was unavailable
    /// (judge_ran = false) — verification could not be confirmed.
    UnverifiedSuccess,
    /// Required checks fail or no usable reply produced.
    Failed,
    /// Tests weakened, unrelated destructive edits, or task bypassed.
    UnsafeInvalid,
}
```

Classifier rules (rule-based; the back-edge and `judge_ran` gate are load-bearing):

- `AutonomousVerifiedSuccess` requires **all** checks pass, a verification report
  produced, `judge_ran = true` (a `judge_unavailable` turn can never be
  autonomous-verified — see CriteriaJudge degradation), and **no** non-`benign`
  interventions recorded during the turn.
- `UnsafeInvalid` is triggered by any entropy audit finding with `severity >= 2`
  on `TestWeakening` or `BoundaryViolation` (PRD 04). It takes precedence over a
  success label: a turn whose checks pass but which weakened a test is
  `UnsafeInvalid`, not verified-success.
- **Full-regression timeout exemption.** A `verification_trace` entry whose
  `method = full_regression` that is bounded by a timeout is not counted as a
  failed check (`H3_LIMITS` records the bound); it does not by itself force
  `Failed`. The classifier reads it as "covered within limits."

### EpisodePackage (versioned; all eight traces)

The episode package is the paper's central output artifact. It is **versioned**
and carries **all eight canonical traces** — including `context_trace`, which the
earlier JSON-only spec omitted. The on-disk JSON shape and serde encoding are
owned by [`data-model.md`](../architecture/data-model.md) §5; this is the
producing struct (the compose layer builds and `EvidenceJournal` writes it):

```rust
#[serde(rename_all = "snake_case")]   // ADR-0025
pub struct EpisodePackage {
    pub schema_version: u32,          // 1 (ADR-0027)
    pub episode_id: String,           // "ep_<task_id>" — groups all turns of ONE task (ADR-0018)
    pub turn_id: String,              // "turn_20260527_143022_abc123"
    pub task_id: String,
    pub harness_level: HarnessLevel,  // h3
    pub ts: f64,                      // epoch seconds
    pub initial_state: InitialState,  // { commit, workspace } — task baseline

    // ---- the eight paper traces ----
    pub action_trace: Vec<ActionEvent>,        // 1: read_file | edit_file | run_tool | write_report | update_task_state | declare_complete
    pub tool_trace: Vec<ToolEvent>,            // 2: name, ToolOutcome.status, exit_code, duration_ms, timeout, recovered, result
    pub context_trace: Vec<ContextEntry>,      // 3: { artifact, contribution, influenced_decision } — PAPER trace, previously MISSING
    pub verification_trace: Vec<VerifyEntry>,  // 4: { type, method, result, covers[], interpretation }
    pub attribution_log: Vec<Attribution>,     // 5: { observed, expected, failure_type, layer, evidence, alternatives, next_action }
    pub reproduction_log: Option<ReproductionLog>, // 6: { check, observed, expected }
    pub verification_report: ReportBlock,      // 7: { requirements[], limits }
    pub intervention_log: Vec<InterventionRecord>, // 8: { kind, avoidability, harness_gap, burden }

    pub entropy: EntropyAudit,        // { delta, findings } (PRD 04)
    pub outcome: EpisodeOutcome,
}

pub struct ContextEntry {            // the previously-missing context_trace element
    pub artifact: String,            // e.g. "src/auth.rs", a recalled memory, AGENT_GUIDE
    pub contribution: String,        // primary | supporting | unused
    pub influenced_decision: bool,   // did this artifact change what the agent did?
}
```

Notes:
- `attribution_log` is a **`Vec`** (was a single object): the verify→re-attribute
  back-edge appends one entry per loop pass, so multi-pass episodes are recorded.
- `tool_trace` reads `ToolOutcome.status` (ADR-0022) and is redacted before write
  (ADR-0026); `verification_trace[].method` is the controlled vocabulary
  (`bug_reproduction`, `deterministic_check`, `registered_test`, `targeted_test`,
  `full_regression`, `lint`, `patch_review`, `manual`) — data-model §5.
- `failure_type` is the fixed `FailureType` enum (above), not a free string.

`EvidenceJournal::record_episode()` appends a summary line to `evidence.jsonl`
(carrying `episode_id` so turns regroup into a task) and writes the full package
to `episodes/<turn_id>.json`. The example record is in data-model §5.

## EvidenceJournal

Append-only JSONL at `.rustykeys/evidence.jsonl`. Every turn's verification
package, every consolidation changelog, and every compaction event are recorded.

```rust
pub struct EvidenceJournal {
    path: PathBuf,
    episodes_dir: PathBuf,
    enabled: bool,
}

impl EvidenceJournal {
    pub fn record_turn(&self, reply: &str, episode: &Episode,
                       report: &VerificationReport) -> Result<()>;
    pub fn record_episode(&self, pkg: &EpisodePackage) -> Result<()>;  // H3
    pub fn record_improvement(&self, stats: &ConsolidationStats) -> Result<()>;
    pub fn record_compaction(&self, tier: CompactionTier, tokens_before: usize,
                             tokens_after: usize) -> Result<()>;
    pub fn recent(&self, n: usize) -> Result<Vec<serde_json::Value>>;
    pub fn count_turns(&self) -> Result<usize>;
}
```

The turn, consolidation-changelog, and compaction record schemas are owned by
[`data-model.md`](../architecture/data-model.md) §4.1 (each carries `v`,
`session_id`, `turn_id`, and — at H3 — the `outcome` and `episode_id` that
regroup turns into a task). They are not restated here to avoid drift; `outcome`
is `null` below H3 and an `EpisodeOutcome` at H3.

`count_turns()` scans the journal for well-formed `kind = "turn"` entries
(torn-line tolerant, data-model §10) — used by the M-HIR computation (PRD 04)
without coupling the observe layer to the compose layer's file path.

## Seams

- **Journal rotation**: the evidence log grows unbounded. A rotation/retention
  policy (by age or size) is a future seam.
- **Persisted trajectories**: the full episode (including tool args and results)
  could be persisted for replay — the journal currently stores only the summary
  at non-H3 levels.
- **Importance reinforcement**: on a verified turn, the compose layer could
  directly boost the importance of the specific skill memories that were recalled
  — targeted reinforcement rather than waiting for consolidation.
- **LLM-assisted outcome classification**: `EpisodeOutcome` is currently
  rule-based; a future seam adds an aisdk call as a second-opinion judge for
  ambiguous cases.
