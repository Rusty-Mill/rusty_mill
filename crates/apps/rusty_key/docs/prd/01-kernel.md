# PRD 01 — Kernel

## Responsibility

The kernel is the model's agent loop: **Decide + Act** in the OODA framing.
It takes a conversation history and a set of tools, runs the aisdk
`LanguageModelRequest` loop until the model produces a final answer or hits
the step limit, and returns the reply. It knows nothing about memory, policy,
verification, or the UI — those are harness concerns.

## Design

### aisdk as the loop engine

In Keystone, the kernel was a hand-written loop calling `litellm.completion()`
and parsing tool calls manually. In Rusty Keys, aisdk owns the loop:

```rust
let result = LanguageModelRequest::builder()
    .model(config.model())
    .system(system_prompt)          // produced by feed (PRD 03); consumed here
    .messages(history)
    .with_tools(dispatch.tools())   // tool schemas exposed via the ToolDispatch trait
    .stop_when(step_count_is(config.max_steps))
    .build()
    .generate_text()
    .await?;
```

The kernel's job is to configure that request correctly and hand the result
to the harness. `dispatch.tools()` returns the aisdk tool set from the
`ToolDispatch` trait, so the kernel never depends on the concrete `feed::ToolRegistry`.

### Tool dispatch and policy

The kernel does not dispatch tools directly, and it **does not import `feed`**.
It receives an abstract dispatcher as a **`&dyn ToolDispatch`** and the policy as
a **`&dyn Policy`** — both traits defined in `constrain` (see
[`ARCHITECTURE.md` §5](../ARCHITECTURE.md#5-crate-dependency-dag)). The concrete
`ToolRegistry` lives in `feed` and *implements* `ToolDispatch`; `app` injects it
into the kernel as a trait object. The dispatcher consults the policy before
executing; if policy blocks a call, dispatch returns a `BLOCKED` result and the
aisdk loop feeds it back to the model as a tool result. The model can observe the
block and recover.

```
aisdk loop → tool_call → ToolDispatch::dispatch()   (impl: feed::ToolRegistry)
                              ↓
                         Policy::before_tool().await → PolicyError → "BLOCKED …"
                              ↓ (ok)
                         tool_fn()                    → ToolOutcome → result string
```

`before_tool` is `async` (ADR-0016 — the `ApprovalGate` human-in-the-loop made it
concrete); the dispatch path therefore `await`s the policy check. The result
string is rendered from a structured `ToolOutcome` rather than ad-hoc prefixes
(ADR-0022); `constrain` (PRD 02) owns the `Policy`/`ToolDispatch` contracts.

### Observation hook

The kernel exposes an `on_event` callback (or `mpsc` sender) so the harness can
observe each tool call and turn as it happens — feeding observations into the
short-term memory stream in real time rather than as a batch at turn end. The
`Tracer` in the observe layer implements this hook.

### Streaming

aisdk provides `stream_text()` as a drop-in alternative to `generate_text()`.
The kernel API will expose a `stream` flag; the `Session` propagates the stream
handle to the caller so the CLI (or web gateway) can render tokens as they
arrive. Implemented in Phase 5.

## Interface

```rust
pub struct Kernel {
    model: String,
    system_prompt: String,
    max_steps: usize,
}

impl Kernel {
    pub async fn run(
        &self,
        history: &[Message],
        dispatch: &dyn ToolDispatch,   // impl lives in feed (ToolRegistry); kernel sees the trait
        policy: &dyn Policy,           // both traits defined in constrain
        extra_context: Option<&str>,
        tracer: &mut Tracer,
    ) -> Result<String, KernelError>;
}
```

`ToolDispatch` and `Policy` are abstract traits from `constrain`; the kernel
takes them as `&dyn …` and never names the concrete `feed::ToolRegistry`,
preserving the "kernel knows nothing about feed" invariant
([`ARCHITECTURE.md` §5](../ARCHITECTURE.md#5-crate-dependency-dag)).

`extra_context` is injected between the system prompt and the first user message —
the Orient layer (recall + task prompt) lands here without the kernel knowing
its origin. The `system_prompt` itself is **produced by the `feed` layer** (full
construction spec — layered template, task-state injection, composition with
`extra_context` — in [PRD 03](./03-feed.md)); the kernel only **consumes** the
finished string it is constructed with.

> **Signature-drift resolution.** This `(history, &dyn ToolDispatch, &dyn Policy,
> extra_context, tracer)` form is the canonical kernel signature, matching
> [`ARCHITECTURE.md` §5](../ARCHITECTURE.md#5-crate-dependency-dag) and reconciling
> the prior PRD 01 ↔ PRD 06 drift (PRD 01 previously took `&ToolRegistry` and no
> `policy`; PRD 06 step 8 already threads `policy` through). PRD 06 step 8 calls
> this with the concrete `&registry` (which *is* a `ToolDispatch`) and `&policy`;
> passing the registry as `&dyn ToolDispatch` is what keeps `kernel → feed` out of
> the dependency DAG. **The drift is now resolved in favour of this form.**

## Observability

Every tool call and turn is recorded by `Tracer` via the `on_event` hook:
- Tool call: `name`, `args`, `status` (ok / error / blocked), `result`
- Turn: `step`, `n_tool_calls`, `total_tokens`
- Final: `final_reached` bool

The episode is consumed by the compose layer's verifier after the run.

## Error handling

| Condition | Behaviour |
|---|---|
| Tool error | Tool returns an error `ToolOutcome`; dispatch renders `"ERROR: …"` (ADR-0022); model sees it |
| Policy block | Dispatch returns `"BLOCKED by policy: …"` from a structured `PolicyError` (ADR-0023); model sees it |
| `max_steps` reached | Loop exits; `Tracer.final_reached = false`; verifier catches it |
| Network / provider error | Retried by the shared aisdk-client per the policy below; a *terminal* failure surfaces as a typed `KernelError` to the `Session` (caller) |

### aisdk integration policy

Every LLM call in Rusty Keys — the kernel loop here, plus the `CriteriaJudge`,
memory consolidation, embeddings, and compaction summarisation — goes through a
**single shared aisdk-client wrapper** (it is *not* kernel-only; see
[`configuration.md` › Provider / aisdk integration policy](../reference/configuration.md#provider--aisdk-integration-policy-)
and [`ARCHITECTURE.md` §10](../ARCHITECTURE.md#10-failure-modes--resilience)).
The wrapper centralises:

- **Per-call timeout** — every request is bounded by `RUSTYKEYS_REQUEST_TIMEOUT_MS`
  (default `120000`). A timed-out call is treated as a retryable transport error.
- **Bounded exponential backoff + jitter** — up to `RUSTYKEYS_RETRY_MAX` retries
  (default `4`), with delay growing from `RUSTYKEYS_RETRY_BASE_MS` (default `500`)
  and randomised jitter to avoid thundering-herd retries.
- **`429` / `Retry-After`** — on HTTP `429` (and `503` carrying it), the wrapper
  honors the `Retry-After` header when present, overriding the computed backoff
  for that attempt.

**Retryable vs terminal `KernelError`.** The wrapper retries (within the budget),
then maps the outcome to a typed `KernelError` for the `Session`:

| Class | Examples | Disposition |
|---|---|---|
| Retryable | `429`, `5xx`, request timeout, transport/connection reset | retried up to `RETRY_MAX`; only surfaces if the budget is exhausted |
| Terminal | `4xx` auth/permission (`401`/`403`), malformed-request `400`, bad model id, unparseable response | **not retried**; surfaced immediately as a typed `KernelError` |

A retryable error that exhausts the retry budget is surfaced as a `KernelError`
too — terminal *after* retries. The mid-turn-failure semantics (side effects
already executed are **not** rolled back; the episode is recorded as aborted with
its partial tool trace) are owned by
[`ARCHITECTURE.md` §10](../ARCHITECTURE.md#10-failure-modes--resilience); the
streaming path (Phase 5) maps stream errors onto the same retryable/terminal
classes.

## Seams

- **Streaming**: `stream_text()` instead of `generate_text()` when Phase 5 lands.
- **Multi-agent**: `Session::send()` registered as a tool — one kernel calling
  another. The constrain layer controls cross-session permissions.
- **Step budget**: `stop_when` can be extended beyond `step_count_is` —
  time-based or token-budget-based stopping conditions.
