# PRD 05 — Compose

## Responsibility

The compose layer takes the kernel's raw reply and packages it into an
**evidentiary output** — a verified, auditable record of what happened, not a
bare claim. It runs after the kernel returns and before the reply reaches the
caller.

Three responsibilities:
1. **Shape** the reply (whitespace trimming, format normalisation)
2. **Verify** the episode against deterministic and semantic checks
3. **Journal** the turn as a documented, transferable record

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

### Verifier

```rust
pub struct Verifier {
    pub checks: Vec<Box<dyn Check>>,
    pub limits: String,
}

impl Verifier {
    pub fn verify(&self, reply: &str, episode: &Episode) -> VerificationReport;
}
```

`limits` describes what the checks did *not* verify — always surfaced alongside
the verdict so "verified" is never over-read. Two constants:

```rust
pub const DETERMINISTIC_LIMITS: &str =
    "deterministic checks only; semantic correctness and task success not verified";

pub const SEMANTIC_LIMITS: &str =
    "LLM-judge on active task criteria included; \
     output quality beyond stated goals not evaluated";
```

### VerificationReport

```rust
pub struct VerificationReport {
    pub verified: bool,
    pub checks: Vec<CheckResult>,
    pub attributions: Vec<Attribution>,
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
the judge and idle consolidation run concurrently via `tokio::join!` after the
reply is already in the caller's hands:

```rust
let (judge_result, consolidation_result) = tokio::join!(
    criteria_judge.run(&reply),
    memory.consolidate(ConsolidationScope::Idle),
);
```

Both complete while the user is reading the reply. The learning signals from
both are observed together before the next turn's recall.

### Graceful degradation

If the aisdk call fails or returns unparseable JSON, `CriteriaJudge::run()`
returns a passing `CheckResult` with a diagnostic note. The judge is best-effort;
it does not block the turn.

## EvidenceJournal

Append-only JSONL at `.rustykeys/evidence.jsonl`. Every turn's verification
package and every consolidation changelog are recorded here.

```rust
pub struct EvidenceJournal {
    path: PathBuf,
    enabled: bool,
}

impl EvidenceJournal {
    pub fn record_turn(&self, reply: &str, episode: &Episode,
                       report: &VerificationReport) -> Result<()>;
    pub fn record_improvement(&self, stats: &ConsolidationStats) -> Result<()>;
    pub fn recent(&self, n: usize) -> Result<Vec<serde_json::Value>>;
    pub fn count_turns(&self) -> Result<usize>;  // M-HIR denominator
}
```

Turn record schema:
```json
{
  "ts": 1234567890.5,
  "kind": "turn",
  "verified": true,
  "checks": [{"name": "…", "passed": true, "detail": "…"}],
  "attributions": [],
  "limits": "…",
  "evidence": [{"name": "read_file", "status": "ok"}],
  "reply": "…"
}
```

Improvement record schema:
```json
{
  "ts": 1234567890.5,
  "kind": "improvement",
  "scope": "idle",
  "changes": [{"action": "added", "type": "skill", "title": "…"}]
}
```

`count_turns()` scans the journal for `kind = "turn"` entries — used by the
`InterventionLogger` to compute M-HIR without coupling the observe layer to
the compose layer's file path.

## Seams

- **Semantic checks**: `CriteriaJudge` is already implemented (Phase 4). The
  `Check` trait's sync signature stays; async checks go through a dedicated
  async path in `Verifier` to avoid making all checks async.
- **Journal rotation**: the evidence log grows unbounded. A rotation/retention
  policy (by age or size) is a future seam.
- **Persisted trajectories**: the full episode (including tool args and results)
  could be persisted for replay — the journal currently stores only the summary.
- **Importance reinforcement**: on a verified turn, the compose layer could
  directly boost the importance of the specific skill memories that were recalled
  — targeted reinforcement rather than waiting for consolidation.
