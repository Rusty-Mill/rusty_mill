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
    .system(system_prompt)
    .messages(history)
    .with_tools(registry.tools())
    .stop_when(step_count_is(config.max_steps))
    .build()
    .generate_text()
    .await?;
```

The kernel's job is to configure that request correctly and hand the result
to the harness.

### Tool dispatch and policy

The kernel does not dispatch tools directly. It delegates to the `feed` layer's
tool registry, which consults the `constrain` layer's policy before executing.
If policy blocks a call, the registry returns a `BLOCKED` string; the aisdk
loop feeds it back to the model as a tool result. The model can observe the
block and recover.

```
aisdk loop → tool_call → ToolRegistry::dispatch()
                              ↓
                         Policy::before_tool()   → PolicyError → "BLOCKED …"
                              ↓ (ok)
                         tool_fn()               → result string
```

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
        registry: &ToolRegistry,
        extra_context: Option<&str>,
        tracer: &mut Tracer,
    ) -> Result<String, KernelError>;
}
```

`extra_context` is injected between the system prompt and the first user message —
the Orient layer (recall + task prompt) lands here without the kernel knowing
its origin.

## Observability

Every tool call and turn is recorded by `Tracer` via the `on_event` hook:
- Tool call: `name`, `args`, `status` (ok / error / blocked), `result`
- Turn: `step`, `n_tool_calls`, `total_tokens`
- Final: `final_reached` bool

The episode is consumed by the compose layer's verifier after the run.

## Error handling

| Condition | Behaviour |
|---|---|
| Tool error | Tool returns `Err`; registry converts to `"ERROR: …"` string; model sees it |
| Policy block | Registry returns `"BLOCKED by policy: …"`; model sees it |
| `max_steps` reached | Loop exits; `Tracer.final_reached = false`; verifier catches it |
| Network / provider error | Propagated as `KernelError`; `Session` handles retry or surfaces to caller |

## Seams

- **Streaming**: `stream_text()` instead of `generate_text()` when Phase 5 lands.
- **Multi-agent**: `Session::send()` registered as a tool — one kernel calling
  another. The constrain layer controls cross-session permissions.
- **Step budget**: `stop_when` can be extended beyond `step_count_is` —
  time-based or token-budget-based stopping conditions.
