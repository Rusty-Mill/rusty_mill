# Spike 01 — the aisdk ↔ harness tool seam

**Status:** complete · **Date:** 2026-05-27 · **Crate:** `crates/spike` (`rk-spike`, throwaway)
**Risk addressed:** BACKLOG Phase 1 — *"aisdk `#[tool]`→`ToolFn` adapter is the riskiest seam → spike it first."*

Verified against **aisdk 0.5.2** / **aisdk-macros 0.3.0** (toolchain: stable 1.94.1).
The spike builds, passes `clippy -D warnings`, 5 tests, and an offline demo.

## Question

Can the harness's design — an async `ToolFn` registry, `async before_tool` policy
vetting **before** dispatch (ADR-0007/0016), a structural `ToolOutcome` (ADR-0022),
and a kernel that loops over `&dyn ToolDispatch` (PRD 01) — be built on aisdk's
`#[tool]` macro and agent loop? Or do we need to **fork** aisdk?

## Answer: don't fork. Wrap.

The harness design holds. A fork is not justified — the only real friction is a
pair of `pub(crate)` types, fixable by a narrow upstream PR or sidestepped today.

## What the source actually shows

1. **`#[tool]` produces a *synchronous*, zero-arg descriptor.**
   `fn read_file(path: String) -> Tool` is rewritten into `fn read_file() -> Tool`
   returning a `Tool { name, description, input_schema, execute }` whose `execute`
   is `Box<dyn Fn(Value) -> Result<String, String>>` — **not async**, and it
   re-stringifies errors. The PRD's assumed `read_file::NAME` / `read_file::schema()`
   associated items and async body **do not exist**.
   → We use the descriptor as a **schema carrier only** and run async execution in
   our own `ToolFn::call`. (`crates/spike/src/tool.rs`)

2. **aisdk's high-level loop dispatches tools with no interception point.**
   `LanguageModelRequest::generate_text()` calls `options.handle_tool_call(..)`,
   which is **`pub(crate)`** and invokes `Tool.execute` directly. There is no
   per-call hook with veto power (only `on_step_start`/`on_step_finish`/`stop_when`).
   → Policy vetting cannot live in aisdk's loop. It lives in
   `ToolRegistry::dispatch` and the **kernel drives its own loop**.

3. **The kernel-owns-loop path is blocked by `pub(crate)` types (the load-bearing finding).**
   The low-level `LanguageModel::generate_text(options)` *does* advertise tools and
   return tool calls **without** executing them — the right single-step primitive.
   But `LanguageModelOptions.messages: Vec<TaggedMessage>` and **`TaggedMessage`
   itself are `pub(crate)`** (so are `tools`, `current_step_id`, `stop_reason`).
   External code cannot name `TaggedMessage`, so it cannot build the request. The
   derived builder does not help — its `.messages()` setter takes the private type.
   *(Confirmed by compile error E0603.)*

## Two strategies (the fork decision rests here)

| | **Strategy A — reuse aisdk's loop** | **Strategy B — kernel owns the loop** (PRD 01) |
|---|---|---|
| How | Register tools whose sync `execute` closure bridges to our async `ToolDispatch` (`block_in_place` + `Handle::block_on`); return `Ok(outcome.render())` always | Call low-level `generate_text(options)` per step; vet + dispatch + feed results ourselves |
| Policy vetting | ✓ (inside the closure) | ✓ (clean seam) |
| Structural `ToolOutcome` | partial — must encode status in the Ok string (aisdk re-stringifies `Err`) | ✓ full |
| Async tool bodies | ✓ (via the bridge) | ✓ native |
| Uses public API only | ✓ **works today** | ✗ blocked by `pub(crate)` `TaggedMessage` |
| Loop ownership | aisdk | harness (matches PRD 01) |

**Recommendation:** target **Strategy B** (it matches PRD 01 and keeps `ToolOutcome`
structural) and unblock it with a **narrow upstream PR** to aisdk — make
`TaggedMessage` public *or* add a public `LanguageModelOptions` constructor taking
`Vec<Message>` (public). Use **Strategy A** as the interim/live path; it needs no
upstream change. A fork is unwarranted: it would hand us maintenance of 70+ provider
integrations (the very "aisdk is young" risk in the BACKLOG register) for no gain
the wrap doesn't already deliver.

## What the spike proves (green)

- `AiSdkTool` adapts a `#[tool]` descriptor + an async body into a `ToolFn` (name +
  JSON schema from the descriptor; sync `execute` bypassed).
- `ToolRegistry` (`impl ToolDispatch`) runs `Policy::before_tool` **before**
  `ToolFn::call`; a block returns `ToolOutcome { status: Blocked }` and the body
  never runs. Unknown tool → `Error`, not a panic.
- `ToolOutcome` carries status structurally; one `render()` produces the
  model-facing string (never parsed back).
- `kernel::run_turn` loops `ChatModel → vet+dispatch → ChatModel` to completion;
  `FakeChatModel` drives a full multi-step tool turn offline (CI-testable, no provider).
- A live aisdk model adapts to the `ChatModel` port via public conversions
  (`to_messages`, `schema_only_tool_list`) — `OpenAICompatible` + `base_url` points
  at local ollama for a Strategy-A live run.

## Secondary notes for Phase 1

- **Path resolution must be shared** between policy and tool. The spike's policy
  resolves workspace-relative paths against the root, but the builtin reads the raw
  path against CWD — they must use one resolver (the workspace root) in the real build.
- `derive_builder` treats `Option<_>` fields as optional automatically, so only
  non-Option fields (`messages`, `current_step_id`) are "required" — not the friction;
  the `pub(crate)` visibility is.
- schemars **1.x** `Schema` ⇄ `serde_json::Value` via `serde_json::to_value` /
  `from_value` works for schema transport.

## Disposition

`crates/spike` was the throwaway. It has been **deleted** now that Phase 1 landed
the real crates (`config`, `observe`, `constrain`, `feed`, `kernel`, `app`), which
carry forward its lessons: the `ToolFn`/`AiSdkTool` shape (`feed`), the
`ToolDispatch` seam (`constrain`), the structural `ToolOutcome` (`observe`), and
the **Strategy A** kernel bridge (`kernel`) — aisdk's high-level loop with policy
enforced inside the tool closure. The optional upstream PR (public `TaggedMessage`
/ `Vec<Message>` options ctor) remains the path to Strategy B if we later want the
kernel to own the loop; it blocks nothing today.
