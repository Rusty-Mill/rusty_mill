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
                   chat completions (OpenAI/local), Azure OpenAI, aisf_stage
                   (a real governed agent, driven as a subprocess), claude_cli
                   (shells out to the `claude` CLI's print mode), and a
                   network-free Mock backend for tests and dry runs
  skillopt-envs/   Environment impls: a deterministic, offline synthetic
                   arithmetic word-problem benchmark with programmatic scoring,
                   plus aisf_triage and aisf_validation, JSONL-backed
                   real-scenario benchmarks
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

### Optimizing an agent's prompt inside a real system: `aisf_stage`

`provider: aisf_stage` treats one stage of a real, tool-using, multi-turn
agent system as the executor — the "skill" being trained is that agent's
actual system prompt in production, not a synthetic benchmark stand-in.
It's built against [AISF](https://github.com/baileyrd/AISF), an AI
software factory with its own governance layer (every tool call gated
outside the model's control), by driving its `eval-stage` subcommand as a
subprocess per rollout: `chat()` writes the candidate skill to a scratch
`FACTORY_PROMPTS_DIR`, pipes the example's scenario JSON to `eval-stage`'s
stdin, and returns its JSON output (report + full audit log) unparsed for
the environment to score.

Nothing about this needed engine or trait changes — `ChatBackend::chat`'s
signature never promised "one API call," so a whole multi-turn, governed
tool-use loop running inside a single `chat()` call satisfies it exactly
the way a single Anthropic request does. See
`configs/aisf_triage_example.yaml` (`model` is reinterpreted as the AISF
stage name, e.g. `"triage"`; `aisf_binary_path` points at AISF's built
`software-factory` binary) and the paired `env: { name: aisf_triage }`,
which loads hand-labeled scenarios from a JSONL file
(`crates/skillopt-envs/src/aisf_triage.rs`) instead of generating a
synthetic distribution — the data-driven counterpart to
`synthetic_arithmetic`.

AISF's `validation` stage is wired up the same way — `configs/
aisf_validation_example.yaml` (`model: validation`) and `env: { name:
aisf_validation }` (`crates/skillopt-envs/src/aisf_validation.rs`), a
genuinely separate `Environment` from `aisf_triage` (a PR review isn't a
GitHub issue list — different scenario shape, different scorer field:
`verdict`, not `priority`). `AisfStageBackend` needed zero changes to
support it — the stage name was always a plain string. One real
asymmetry, stated honestly rather than papered over: AISF's own
`eval-stage` has no `claude_cli`/MCP-bridge driver for `validation` yet
(only `triage`), so running this executor role live still needs a real
`ANTHROPIC_API_KEY`.

That path was actually attempted, with a real key supplied by a human,
against this config's val split. It surfaced a real bug one level down,
in AISF itself, not in anything here: AISF's `reqwest` client trusted
only a bundled Mozilla root list, so it couldn't get past this sandbox's
TLS-intercepting egress proxy even though that proxy's CA was already
installed system-wide — every prior "reached the expected
TLS-certificate failure" note in AISF's own history turned out to be
this, not simply a missing key. Fixed upstream in AISF (switched to
`rustls-tls-native-roots`; see AISF's own `RELEASE_NOTES.md`). With that
fix, the same run got through the TLS handshake and reached the real
Anthropic API for the first time in either project's history — which
then correctly rejected the request with an insufficient-credit-balance
error: a genuine API-level response, not a network failure. So the
boundary has moved, precisely: parsing, wire format, both driver
dispatch paths, and now the network path to Anthropic itself are all
confirmed live and correct. A real model completion for `validation`
still hasn't happened in this project's history — but only because that
specific key's account had no available credit, not for any technical
reason.

No `ANTHROPIC_API_KEY` in your environment? `AisfStageBackend` sets
`EVAL_STAGE_DRIVER=claude_cli` on the spawned `eval-stage` process
unconditionally (harmless when a real key *is* set — AISF checks for one
first and only falls back to this) — AISF's own side then drives the same
governed tool loop through `claude -p` via an in-process MCP server
instead of a raw Anthropic call. **Verified end-to-end in this project's
own sandbox with zero API keys anywhere in the chain:** `skillopt-cli
eval` against `configs/aisf_triage_example.yaml` scored **0.75** (6/8
correct) on the real, hand-labeled val split — a genuinely meaningful
result from a real governed agent, not a mock. See AISF's own
`RELEASE_NOTES.md` for the two real (previously-latent) bugs this
uncovered along the way.

### No API key, but a working `claude` CLI session: `claude_cli`

`provider: claude_cli` shells out to the `claude` CLI's non-interactive
print mode (`claude -p`) instead of calling the Anthropic API directly.
Useful wherever a working `claude` CLI session already exists (e.g. an
OAuth-authenticated Claude Code sandbox) but no portable
`ANTHROPIC_API_KEY` is available for a raw HTTP client — confirmed in this
project's own development sandbox, where a plain `curl` to
`api.anthropic.com` 401s with no key, but `claude -p` already has a
working session. `model` passes straight through as `claude -p`'s
`--model` (e.g. `"sonnet"`). All the CLI's own built-in tools are disabled
(`--tools ""`) and sessions aren't persisted — this is a plain single-turn
text completion, matching every existing `ChatBackend` call site
(`Engine` never sends more than one system message plus one user message
per `chat()` call). That also means it's a fit for the `executor`,
`optimizer`, or `reflector` role in any config using a plain chat
environment (e.g. `synthetic_arithmetic`) — but this backend itself is
**not** a substitute for a governed tool-use loop, so it can't drive
`aisf_stage`'s executor role directly (that role needs `aisf_stage`
itself; see above for how *that* gets a no-key path of its own, via
AISF's own separate `claude -p` + MCP bridge, not this backend). See
`configs/claude_cli_example.yaml` — verified with a real, live, complete
training run in this project's own sandbox (`0/2 steps accepted, val
1.0 -> 1.0, test 1.0` at smoke scale — see `RELEASE_NOTES.md`), the first
non-mock run in this project's history that didn't need an
`ANTHROPIC_API_KEY`.

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
  runners like Ollama), Azure OpenAI, `aisf_stage` (a real multi-turn,
  governed agent stage, driven as a subprocess), and `claude_cli` (shells
  out to the `claude` CLI's print mode — no API key needed, just a working
  CLI session), behind a `ChatBackend` trait, plus a Mock backend for
  offline tests.
- **Benchmark**: one deterministic synthetic environment
  (`synthetic_arithmetic`) so the whole loop is testable without network
  access or API keys — see `crates/skillopt-envs/src/synthetic_arithmetic.rs`.
  Also `aisf_triage`/`aisf_validation`, JSONL-backed real-scenario
  benchmarks for `aisf_stage`'s triage/validation agents
  (`crates/skillopt-envs/src/aisf_{triage,validation}.rs`).
  Adding a real benchmark (e.g. a QA dataset) means implementing
  `Environment` and registering it in `skillopt-envs`'s factory.
- **Not implemented**: a WebUI/monitoring dashboard, a MiniMax backend, and
  an offline self-evolution ("Sleep") engine. The trait boundaries are there
  for these to be added without touching `skillopt-core`.
