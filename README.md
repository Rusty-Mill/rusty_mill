# rusty_skillopt

A from-scratch Rust take on the core idea behind Microsoft's
[SkillOpt](https://github.com/microsoft/SkillOpt): treat a skill markdown
document as the trainable state of a frozen LLM agent, and optimize it with
neural-network-style training discipline — epochs, batches, a validation
gate — entirely in text space, with no weight updates.

This is **not** a port of SkillOpt's source (which wasn't available to
transcribe faithfully). It's an independent implementation of the same
concept, designed idiomatically for Rust. Provider coverage, benchmark
adapters, the WebUI, and the offline "Sleep" engine from the original are
out of scope for this pass; see [Scope](#scope) below.

## The loop

```
rollout -> reflect -> aggregate/select -> optimize (propose edit) -> validation gate -> accept/reject
```

1. **Rollout** — run the executor model on a batch of training examples,
   with the current skill document injected as its system prompt.
2. **Reflect** — score each trajectory programmatically via the
   environment (deterministic, no model call needed for scoring), and ask a
   reflector model for a short qualitative critique.
3. **Aggregate/select** — summarize the batch's mean score and surface the
   worst-scoring examples' critiques as the feedback the optimizer sees.
4. **Optimize** — ask an optimizer model to propose a bounded set of
   `add` / `delete` / `replace` line edits against the skill document,
   anchored to exact existing lines (not line numbers, which models can't
   reliably count).
5. **Validation gate** — apply the candidate edit, evaluate it on a
   held-out validation split, and accept only if it *strictly* improves the
   mean validation score over the current best. Rejected edits' rationales
   are kept in a bounded buffer and shown to the optimizer so it doesn't
   retry the same change.
6. Repeat for `epochs x batches`, then evaluate the final best skill on the
   test split and write `best_skill.md` + `report.json`.

## Layout

```
crates/
  skillopt-core/   types, traits (ChatBackend, Environment), the anchor-based
                   edit engine, YAML config, batch scheduler, the engine
                   (training loop orchestration)
  skillopt-model/  ChatBackend impls: Anthropic Messages API, OpenAI-compatible
                   chat completions (OpenAI/local), Azure OpenAI, and a
                   network-free Mock backend for tests and dry runs
  skillopt-envs/   Environment impls: a deterministic, offline synthetic
                   arithmetic word-problem benchmark with programmatic scoring
  skillopt-cli/    the `skillopt` binary (`train`, `eval` subcommands)
configs/example.yaml   example run configuration
skills/initial.md      example starting skill document
```

`skillopt-core` only depends on its own traits — `ChatBackend` and
`Environment` — so new LLM providers and new benchmarks are additive: drop
an implementation in `skillopt-model` or `skillopt-envs` and register it in
that crate's factory, without touching the engine.

## Running it

For a practical guide to running this well — choosing the three models, sizing
a run and its call budget, making the benchmark hard enough to produce signal,
and reading `report.json` — see [docs/USAGE.md](docs/USAGE.md).

```bash
cargo test --workspace

# Dry run with the network-free mock backends (won't actually improve the
# skill, just exercises the CLI/config/engine wiring end to end):
cargo run -p skillopt-cli -- train --config configs/example.yaml

# Evaluate an existing skill against a split without optimizing:
cargo run -p skillopt-cli -- eval --config configs/example.yaml \
    --skill skills/initial.md --split test
```

To actually train against a real model, edit `configs/example.yaml`: set
`executor` / `optimizer` / `reflector` to `provider: anthropic` (with
`ANTHROPIC_API_KEY` set) or `provider: openai_compatible` (with
`OPENAI_API_KEY` set, and `base_url` for non-OpenAI-compatible endpoints).

`openai_compatible` also covers local runners like [Ollama](https://ollama.com)
out of the box — it exposes an OpenAI-compatible endpoint and doesn't check
auth, so no API key env var is needed at all. See `configs/ollama_example.yaml`
(`base_url: http://localhost:11434/v1`, no `api_key_env` set).

Qwen models are covered the same way, via Alibaba Cloud DashScope's
OpenAI-compatible mode — see `configs/qwen_example.yaml`
(`base_url: https://dashscope.aliyuncs.com/compatible-mode/v1`). Verified at
the code level against DashScope's documented contract, not with a live
call (no API key available, and the domain is blocked by this session's
egress policy) — if DashScope's actual behavior diverges, it's a one-line
config fix, not a code change.

Azure OpenAI doesn't fit `openai_compatible`'s request shape (it uses an
`api-key` header instead of `Authorization: Bearer`, and encodes a
deployment name in the URL instead of `model` in the body), so it's its own
`provider: azure_openai` — see `configs/azure_openai_example.yaml`
(`base_url` is the resource endpoint, `model` is the deployment name,
`api_key_env` defaults to `AZURE_OPENAI_API_KEY`, `api_version` is optional).

## Config

See `configs/example.yaml` for the full shape. Key `train` knobs, and their
rough SkillOpt-training analogy:

| field | analogy |
|---|---|
| `epochs`, `batch_size` | training epochs / batch size |
| `max_ops_per_edit` | learning-rate cap (bounds edit size per step) |
| `rejection_buffer_size` | avoid re-proposing recently-rejected edits |
| `min_improvement` | validation-gate strictness |
| `val_batch_size` | held-out set size used per gate check |

## Scope

Built as a broad-but-bounded first pass:

- **Backends**: Anthropic, any OpenAI-compatible endpoint (including local
  runners like Ollama), and Azure OpenAI, behind a `ChatBackend` trait, plus
  a Mock backend for offline tests.
- **Benchmark**: one deterministic synthetic environment
  (`synthetic_arithmetic`) so the whole loop is testable without network
  access or API keys — see `crates/skillopt-envs/src/synthetic_arithmetic.rs`.
  Adding a real benchmark (e.g. a QA dataset) means implementing
  `Environment` and registering it in `skillopt-envs`'s factory.
- **Not implemented**: a WebUI/monitoring dashboard, a MiniMax backend, and
  an offline self-evolution ("Sleep") engine. The trait boundaries are there
  for these to be added without touching `skillopt-core`.
