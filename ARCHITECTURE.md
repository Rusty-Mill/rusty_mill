# Architecture

## Overview
An independent Rust reimplementation of the core idea behind Microsoft's
SkillOpt: a markdown "skill" document is the trainable state of a frozen LLM
agent, optimized in text space (add/delete/replace line edits) with a
training-loop discipline — epochs, batches, a strict validation gate — and
no weight updates. Not a source port; see the README's Scope section for
what's deliberately out.

## Boundaries
`skillopt-core` owns the orchestration (`Engine`) and only depends on two
traits it defines itself. Everything that talks to the outside world —
an LLM API, a benchmark dataset — is an adapter implemented in a separate
crate, so new providers/benchmarks are additive and never touch the engine.

| Port | Adapter(s) | Notes |
| ---- | ---------- | ----- |
| `ChatBackend` (`skillopt-core::traits`) | `AnthropicBackend`, `OpenAiCompatBackend`, `AzureOpenAiBackend`, `AisfStageBackend`, `ClaudeCliBackend`, `MockBackend` (`skillopt-model`) | engine only ever holds `Arc<dyn ChatBackend>`; Mock is network-free, used in tests and CLI dry runs. `AisfStageBackend` runs a whole multi-turn, governed tool-use loop (a real AISF factory stage, via its `eval-stage` subcommand) inside one `chat()` call — the trait never promised "one API call," so this needed no signature change. `ClaudeCliBackend` shells out to the `claude` CLI's print mode (a plain single-turn completion, tools disabled) instead of a raw Anthropic HTTP call, for environments with a working CLI session but no portable API key. |
| `Environment` (`skillopt-core::traits`) | `SyntheticArithmeticEnv`, `AisfTriageEnv` (`skillopt-envs`) | programmatic `score()` keeps the validation gate independent of any LLM call. `AisfTriageEnv` loads hand-labeled scenarios from a JSONL file instead of generating a synthetic distribution. |

## Structure
A Cargo workspace of four crates, which is the ports-and-adapters split made
literal at the crate boundary rather than a module-level convention:

- `skillopt-core` — types, the `ChatBackend`/`Environment` ports, the
  anchor-based skill-edit engine, YAML config, batch scheduler, and `Engine`
  (the training loop)
- `skillopt-model` — `ChatBackend` adapters (Anthropic, OpenAI-compatible,
  Azure OpenAI, AISF stage, Claude CLI, Mock)
- `skillopt-envs` — `Environment` adapters (a synthetic benchmark, plus a
  JSONL-backed one for real AISF scenarios)
- `skillopt-cli` — the `skillopt` binary wiring config → adapters → `Engine`

This is a modular monolith (a single deployable binary) and should stay one
unless a concrete forcing function shows up — e.g. a benchmark adapter that
needs a different language/runtime, or a provider adapter that needs to
scale independently of the CLI.

## Data flow
One training step, run by `Engine::run_step` (`crates/skillopt-core/src/engine.rs`):

1. **Rollout** — `executor` `ChatBackend` runs each example in the batch
   with the current skill injected as its system prompt.
2. **Reflect** — `Environment::score` grades each output programmatically;
   `reflector` `ChatBackend` adds a short qualitative critique.
3. **Aggregate/select** — worst-scoring examples' critiques become the
   optimizer's feedback; rejected-edit rationales from `RejectionBuffer`
   are appended so the optimizer doesn't retry them.
4. **Optimize** — `optimizer` `ChatBackend` proposes a bounded `SkillEdit`
   (JSON: `add`/`delete`/`replace` ops anchored to exact existing lines).
5. **Apply + validation gate** — `skill_edit::apply_edit` produces a
   candidate skill; it's evaluated on the held-out val split and accepted
   only if it strictly improves the mean val score over the current best.

`Engine::train` repeats this for `epochs × batches`, then evaluates the
final best skill on the test split.

## Key decisions
See [docs/adr/](./docs/adr/) for the record of individual decisions and their tradeoffs.

## Non-goals
- A literal port of SkillOpt's Python source (not available to transcribe from).
- WebUI/monitoring dashboard, Azure-specific auth, additional providers
  (Qwen, MiniMax), an offline self-evolution ("Sleep") engine — the
  `ChatBackend`/`Environment` traits are designed so these are additive later.
