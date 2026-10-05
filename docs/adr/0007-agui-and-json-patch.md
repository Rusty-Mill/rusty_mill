# ADR-0007: `rusty_agui` and `rusty_json_patch`, sovereign agent-to-UI protocol crates

**Status:** Accepted (implemented in the same change)
**Date:** 2026-10-05
**Deciders:** Repository owner

## Context

A survey of [CopilotKit](https://www.copilotkit.ai/) against this workspace
(2026-10-05) found that its one load-bearing idea is
[AG-UI](https://docs.ag-ui.com/), an open, MIT-licensed event protocol
between an agent and a user interface: a run is opened by `RUN_STARTED`,
streams text, tool calls, state snapshots and RFC 6902 state deltas, and is
closed by `RUN_FINISHED` or `RUN_ERROR`. The client sends a `RunAgentInput`
(thread, run, messages, client-side tools, context, state) and reads
`text/event-stream` back. CopilotKit's open React SDK and its OpenBot
platform both consume AG-UI, so any server that speaks it gets a working
chat and generative UI without writing a frontend. Everything else
CopilotKit sells (a React component kit, a hosted thread store, a
"threads to reviewable skills" learning loop, Slack and Teams channels,
per-bot containers) is a product on top of the protocol.

The workspace already has the other three agent protocols (`rusty-mcp`,
`rusty_a2a`, `rusty_acp`), three agent runtimes with human-in-the-loop
(`rusty_adk`, `nexus-agent`, `rusty_key`), LLM routing, a CEL policy
gateway, memory (`rusty_remind_me`) and skill optimisation
(`rusty_skillopt`). It had three gaps:

1. **No AG-UI.** Instead, three incompatible agent-to-UI event vocabularies
   had grown: `rusty_key`'s `contract.rs` (9 events), `nexus-ai-runtime`'s
   `AiEvent` (12 variants) and `adk-core`'s `Event`. Each product's UI is
   bespoke to its own.
2. **No JSON Patch.** Nothing in the workspace implements RFC 6901/6902,
   which AG-UI's `STATE_DELTA` and `ACTIVITY_DELTA` carry.
3. **No streaming in the shared web server.** `rusty_serve` (the server
   behind `rusty_tick` and `rusty_fair_play`) answered buffered JSON only.
   The workspace's one sovereign SSE write path lived inside
   `rusty-meshed-registry`, an app, where no other family may depend on it
   (ADR-0003).

Community Rust SDKs for AG-UI exist (`agui-rs`, `ag-ui-rust`,
`agui-protocol`). They bring `serde`, `serde_json`, `tokio` and `axum`.

## Decision

Build the protocol, not the product: two Tier S crates (ADR-0002) and one
change to an existing library.

### 1. `rusty_json_patch` (`crates/foundation/rusty_json_patch`, layer `foundation`)

JSON Pointer (RFC 6901), JSON Patch (RFC 6902) and JSON Merge Patch
(RFC 7386) over `rusty_json::Value`. `no_std` + `alloc`; `std` on by
default only for `std::error::Error`.

| Item | Decision |
|---|---|
| `Pointer` | Parsed once into unescaped tokens; `resolve`, `resolve_mut`, `child`, `is_proper_prefix_of`; `Display` re-escapes. Index rules per RFC 6901 §4 (no leading zeros; `-` only for `add`). |
| `Op`, `Patch` | The six operations as an enum; `Patch::from_value`/`to_value` for the wire form. |
| `Patch::apply` | **Atomic**: operations run on a clone and the document is replaced only on success. A client applying a `STATE_DELTA` must never be left half-patched. |
| `diff` | Member-wise for objects, index-wise plus tail add/remove for arrays, `replace` on type change. Deliberately not LCS: the state deltas it serves are small objects. |
| `merge_patch` | RFC 7386, infallible. |
| Dependency | `rusty_json` by **path** with `default-features = false`. Cargo ignores `default-features = false` on a `workspace = true` dependency whose workspace entry has defaults on, and the workspace entry does (`serde` on), so a path dependency is the only way the manifest can refuse `serde`. `rsi-runtime` already does this for the same reason. |
| Tests | The RFCs' own appendix vectors (6901 §5, 6902 Appendix A, 7386 Appendix A) plus atomicity, root and index edge cases and diff round trips. No external test oracle: the RFC vectors are the oracle. |

### 2. `rusty_agui` (`crates/libs/protocol/rusty_agui`, layer `libs`)

| Module | Decision |
|---|---|
| `types` | `RunAgentInput`, `Message` (one enum variant per role, so a tool message always has its `tool_call_id`), `Content` (text or multimodal parts), `Tool`, `Context`. |
| `event` | `Event { kind: EventKind, meta: EventMeta }`: the 31 event types as an enum, the shared optional fields (`timestamp`, `subagentRunId`, `metadata`, `rawEvent`) beside it rather than repeated in every variant. Wire `type` strings follow the TypeScript SDK (`RUN_STARTED`). |
| `codec` | Hand-written `Value` in and out. Encoding emits exactly what the TypeScript encoder emits (optional and empty fields omitted). Decoding **ignores unknown members** so a newer peer still parses; a missing required field is an error naming it. |
| `sse` | `encode` (one `data:` frame per event) and an incremental `Decoder` that handles split frames, `\n`/`\r\n`/`\r`, multi-line `data:`, comments, and ignores `event:`/`id:`/`retry:`. |
| `verify` | `Verifier::push(Event) -> Vec<Event>`: the ordering rules (`RUN_STARTED` first and once; nothing after the run ends; text message, tool call and reasoning message opened before content and closed with the same id, never two at once; steps balanced; non-empty deltas) and **chunk expansion**, turning `TEXT_MESSAGE_CHUNK`, `TOOL_CALL_CHUNK` and `REASONING_MESSAGE_CHUNK` into their start/content/end forms exactly as the TypeScript SDK's transform does. One entry point, so a consumer cannot verify without expanding or expand without verifying. |
| `reduce` | `Reducer { messages, state }`: folds canonical events into what a client shows. State deltas go through `rusty_json_patch`; a failing delta is an error and leaves state untouched. |
| `serve` (feature) | `Agent` trait (`run(&input, &mut Emitter) -> Result<Option<Value>>`) and `AgentHandler`, a `rusty_serve::Handler`. The handler frames the run (`RUN_STARTED`, then `RUN_FINISHED` or `RUN_ERROR`), verifies every emitted event, and streams frames from a bounded channel while the agent runs on its own thread. An agent that emits a bad sequence gets `RUN_ERROR` naming the rule; the bad event never leaves the server. One agent serves one run at a time, matching `rusty_serve`'s one-lock model. |
| Dependencies | `rusty_json` (path, no `serde`), `rusty_json_patch`; `rusty_serve` and `rusty_http` behind `serve`. No external dependency in any configuration. |
| Tests | One wire sample per event type round-tripped; decode errors; every ordering rule; chunk expansion; the reducer's text, tool-call, state, activity and reasoning paths; and an end-to-end run over a real socket, decoded with the SSE decoder and folded with the reducer. |

### 3. `rusty_serve` gains a streaming body

`Response.body` becomes `Body::Json(Vec<u8>) | Body::Stream { content_type,
chunks }`, with `Response::json` and `Response::stream` constructors. A
stream is written as `Transfer-Encoding: chunked`, one chunk per iterator
item, pulled **after** the handler has returned and its lock is released,
so a long-running agent does not block other requests. Keep-alive survives
a stream. `rusty_tick` and `rusty_fair_play` move to `Response::json`; no
behaviour change for them. This is the hoist of `rusty-meshed-registry`'s
SSE path into a library, in sync form, so a `rusty_serve` app can serve an
agent without an async runtime or `axum`.

### What is deliberately not built

- **Adapters from the three existing vocabularies** (`adk-core::Event`,
  `nexus-ai-runtime::AiEvent`, `rusty_key`'s contract) to AG-UI. Each is a
  small, separate change in its own family, and each family decides when
  its UI moves. The protocol crate must exist first; this ADR lands it.
- **A React chat kit, a hosted thread store, channels, per-bot containers,
  A2UI.** No consumer in the workspace asks for them; CopilotKit's open SDK
  consumes a `rusty_agui` endpoint directly.
- **A WebSocket transport.** AG-UI's standard transport is SSE.
  `rusty_term`'s RFC 6455 code can be hoisted when a second consumer
  appears.
- **CORS on the agent endpoint.** `rusty_serve`'s model is same-origin (the
  UI is served from `with_web_dir`). A cross-origin frontend is a
  `rusty_serve` concern, not this crate's.

## Alternatives considered

**Depend on a community Rust SDK (`agui-rs`) as Tier A.** Faster, and
ADR-0002 allows it behind a first-party seam. Rejected: the types are a few
hundred lines, and it would make `rusty_agui` the only `libs/protocol`
crate whose JSON is not `rusty_json`, pulling `serde`, `serde_json`,
`tokio` and `axum` into a layer the workspace is otherwise keeping
first-party. The verifier and reducer, which are the real work, would still
have to be written against that crate's types. Using `agui-rs` as a
dev-only conformance oracle remains open under ADR-0002's S-tier rule and
is a follow-up, not a blocker: the wire samples in `codec`'s tests are
taken from the TypeScript SDK's documented shapes.

**Put the event model in `rusty_adk` and have the others adopt it.**
Rejected by ADR-0003: `rusty_adk` is a library, but tying the wire protocol
to one agent framework's crate makes the other two families depend on a
framework they do not use.

**Extend `rusty_json` with pointer and patch.** `rusty_json::Value` already
has `pointer()`. Rejected: patch semantics (atomicity, `-`, move-into-self,
diff) are a layer above a JSON value and `rusty_json` has 24 dependents
that want none of it.

**Make `rusty_serve` async for streaming.** Rejected; `rusty_serve`'s own
ADR reasoning holds: a personal server has a handful of connections and one
store behind one lock. A blocking iterator pulled on the connection thread
streams just as well.

## Consequences

- Every agent in the workspace can be put behind a CopilotKit or OpenBot
  frontend by implementing one trait, once its family writes the adapter.
- `rusty_serve`'s `Response` is a breaking change for its two consumers;
  both are updated in the same commit.
- The workspace map, README crate table, `CHANGELOG.md` and
  `RELEASE_NOTES.md` carry the two new members.
- Follow-ups, each its own change: the three adapters above; an `agui-rs`
  dev-dependency conformance test; a `rusty_agui` client (an `HttpAgent`
  over `rusty_request`) when a Rust consumer of a remote AG-UI agent
  appears; the learning loop (`rusty_agui` thread store → `remind_me`
  capture and promotion → `rusty_skillopt`) only with a forcing function.
