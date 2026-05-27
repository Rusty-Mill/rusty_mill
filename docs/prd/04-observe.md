# PRD 04 — Observe

## Responsibility

The observe layer provides **structured visibility** into every kernel turn. It
has two components:

- **`Tracer`**: captures the episode (tool events, token counts, stop reason)
  as evidence for the compose layer's verifier.
- **`InterventionLogger`**: records human interventions and surfaces the M-HIR
  metric (Missing-Harness Human Intervention Rate).

Observe is read-only with respect to the kernel — it watches without directing.

## Tracer

### What it captures

Every tool call is recorded as it happens via the `on_event` callback the kernel
exposes. At turn end the tracer holds a complete episode:

```rust
pub struct Episode {
    pub tool_events: Vec<ToolEvent>,
    pub final_reached: bool,
    pub total_tokens: u64,
}

pub struct ToolEvent {
    pub name: String,
    pub args: serde_json::Value,
    pub status: ToolStatus,   // Ok | Error | Blocked
    pub result: String,
}
```

`ToolStatus` is inferred from the result string: `BLOCKED …` → `Blocked`,
`ERROR …` → `Error`, anything else → `Ok`.

### Episode lifecycle

```rust
impl Tracer {
    pub fn start_episode(&mut self);   // reset per-run state; tokens stay cumulative
    pub fn record_tool(&mut self, event: ToolEvent);
    pub fn record_turn(&mut self, step: usize, n_tools: usize, tokens: u64);
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

- `ToolEvent` is a value type; no heap allocation per field beyond the strings.
- `start_episode()` replaces the `Vec` in place — `Vec::clear()` retains
  allocated capacity, so steady-state operation does not allocate.
- The tracer is `!Send` by design (owned by the `Session`, never shared) —
  no lock needed.

## InterventionLogger

### What is an intervention

An intervention is any human action that compensates for a missing or insufficient
harness capability. The M-HIR metric measures how often this happens:

```
M-HIR = interventions / total_turns
```

A rising rate signals harness gaps; a falling rate signals improvement over time.
The raw data is persisted across sessions so the trend is visible.

### Intervention kinds

| Kind | Trigger | What it signals |
|---|---|---|
| `task_override` | User sets `/task` when one is already active | Agent drifted or misunderstood |
| `manual_reflect` | User runs `/reflect` or `/sleep` | Idle consolidation didn't fire |
| `manual_groom` | User runs `/groom` | Skills accumulated without auto-grooming |
| `manual_verify` | User inspects `/verify` | User didn't trust the agent's reply |
| `unverified_followup` | User sends a message after an unverified turn | Agent produced a bad answer |

### Storage

Append-only JSONL at `.rustykeys/interventions.jsonl`:

```json
{"ts": 1234567890.5, "kind": "task_override", "note": "fix the parser not the formatter"}
```

```rust
pub struct InterventionLogger {
    path: PathBuf,
}

impl InterventionLogger {
    pub fn record(&self, kind: InterventionKind, note: &str) -> Result<()>;
    pub fn recent(&self, n: usize) -> Result<Vec<InterventionRecord>>;
    pub fn mhir(&self, total_turns: usize) -> MhirReport;
}

pub struct MhirReport {
    pub n_interventions: usize,
    pub n_turns: usize,
    pub rate: f64,
    pub breakdown: HashMap<InterventionKind, usize>,
}
```

`total_turns` comes from `EvidenceJournal::count_turns()` — the observe layer
has no coupling to the compose layer's journal; `Session` passes the count in.

### Detecting `unverified_followup`

`Session` tracks `last_report: Option<VerificationReport>`. Before processing
a regular user message, if `last_report.is_some_and(|r| !r.verified)`, the
logger records `unverified_followup`. This is a one-field state check — no LLM
call, no file I/O in the hot path.

## Seams

- **OpenTelemetry**: aisdk's roadmap includes OTel support. When available,
  wire span-per-tool-call and span-per-turn to the tracer — the `ToolEvent`
  struct maps cleanly to OTel span attributes.
- **Sampling**: for high-volume deployments, the tracer could sample episodes
  rather than capturing all of them.
- **Structured log sink**: today stderr; a future seam routes to a structured
  log collector (e.g. `tracing` crate subscribers).
