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

**`NoToolErrors`**: fails if any tool event has `status = Error | Blocked`.
Guards against the agent asserting success on top of a failed action.

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
    pub limits: &'static str,
}
```

`render()` — human-readable multi-line output for `/verify`.
`as_observation()` — compact one-line for the memory stream learning signal.
`to_json()` — serialised for the evidence journal.

### Failure attribution

On an unverified turn each failed check is classified into `(category, layer)`:

| Check | Category | Layer |
|---|---|---|
| `no_tool_errors` (blocked) | `permission_block` | `constrain/policy` |
| `no_tool_errors` (error) | `tool_error` | `feed/tools` |
| `clean_termination` | `non_termination` | `kernel/loop` |
| `criteria_judge` | `criteria_unmet` | `compose/semantic` |
| `reproduce_before_edit` | `attribution_skipped` | `compose/h3` |
| `verification_report_required` | `evidence_missing` | `compose/h3` |

```rust
pub struct Attribution {
    pub check: String,
    pub category: String,
    pub layer: String,
    pub evidence: String,
}
```

Attribution feeds back as the targeted learning signal: consolidation captures
*why* the turn failed, not just *that* it failed.

## CriteriaJudge (semantic check)

When the active `TaskState` has `success_criteria`, the criteria judge fires an
async aisdk call to evaluate the reply against each criterion independently.

```rust
pub struct CriteriaJudge {
    pub model: String,
    pub task_store: Arc<TaskStore>,
}

impl CriteriaJudge {
    pub async fn run(&self, reply: &str) -> CheckResult;
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

### Concurrency

In Keystone the criteria judge was a synchronous blocking call that added
visible latency between the reply appearing and the next prompt. In Rusty Keys,
the judge, idle consolidation, and entropy audit run concurrently via
`tokio::join!` after the reply is already in the caller's hands:

```rust
let (judge_result, consolidation_result, entropy_audit) = tokio::join!(
    criteria_judge.run(&reply),
    memory.consolidate(ConsolidationScope::Idle),
    entropy_auditor.audit(tracer.episode()),
);
```

All three complete while the user is reading the reply. The learning signals
from all three are observed together before the next turn's recall.

### Graceful degradation

If the aisdk call fails or returns unparseable JSON, `CriteriaJudge::run()`
returns a passing `CheckResult` with a diagnostic note. The judge is best-effort;
it does not block the turn.

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
pub enum EpisodeOutcome {
    /// All checks pass; verification report produced; no interventions.
    AutonomousVerifiedSuccess,
    /// Checks pass but interventions were recorded during the turn.
    AssistedVerifiedSuccess,
    /// Task appears done but agent produced no verification report.
    UnverifiedSuccess,
    /// Required checks fail or no usable reply produced.
    Failed,
    /// Tests weakened, unrelated destructive edits, or task bypassed.
    UnsafeInvalid,
}
```

`UnsafeInvalid` is triggered by entropy audit findings with `severity >= 2`
on `TestWeakening` or `BoundaryViolation` categories.

### Episode package schema

Written to `.rustykeys/episodes/<turn_id>.json`:

```json
{
  "id": "turn_20260527_143022_abc123",
  "harness_level": "h3",
  "ts": 1748346622.5,
  "action_trace": [...],
  "tool_trace": [
    {"name": "read_file", "status": "ok", "duration_ms": 42, "result": "…"}
  ],
  "reproduction_log": {
    "check": "empty_password_probe",
    "observed": "{\"ok\":false,\"errors\":[\"Invalid credentials.\"]}",
    "expected": "{\"ok\":false,\"errors\":[\"Password is required.\"]}"
  },
  "attribution_log": {
    "failure_type": "validation_missing",
    "layer": "validator",
    "evidence": "empty string reaches credential matching"
  },
  "verification_trace": [
    {"check": "empty_password_probe", "passed": true, "covers": ["req-1"]},
    {"check": "valid_login_probe", "passed": true, "covers": ["req-2"]}
  ],
  "verification_report": {
    "requirements": [
      {"requirement": "req-1", "met": true, "evidence": "empty_password_probe passed"}
    ],
    "limits": "H3 protocol; full regression bounded by timeout"
  },
  "entropy": {"delta": 0, "findings": []},
  "intervention_log": [],
  "outcome": "autonomous_verified_success"
}
```

`EvidenceJournal::record_episode()` appends a summary to `evidence.jsonl` and
writes the full package to `episodes/`.

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

Turn record schema (non-H3):
```json
{
  "ts": 1234567890.5,
  "kind": "turn",
  "verified": true,
  "checks": [{"name": "…", "passed": true, "detail": "…"}],
  "attributions": [],
  "entropy": {"delta": 0, "findings": []},
  "limits": "…",
  "evidence": [{"name": "read_file", "status": "ok"}],
  "reply": "…"
}
```

Compaction record:
```json
{
  "ts": 1234567890.5,
  "kind": "compaction",
  "tier": "session_summary",
  "tokens_before": 148000,
  "tokens_after": 12000
}
```

`count_turns()` scans the journal for `kind = "turn"` entries — used by the
`InterventionLogger` to compute M-HIR without coupling the observe layer to
the compose layer's file path.

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
