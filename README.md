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
support it — the stage name was always a plain string.

For a while there was one real asymmetry here, stated honestly rather
than papered over: AISF's own `eval-stage` had no `claude_cli`/
MCP-bridge driver for `validation` (only `triage`), so running this
executor role live needed a real `ANTHROPIC_API_KEY`. That path was
actually attempted, with a real key supplied by a human, and surfaced a
real bug one level down in AISF itself: its `reqwest` client trusted
only a bundled Mozilla root list, so it couldn't get past this sandbox's
TLS-intercepting egress proxy even though that proxy's CA was already
installed system-wide. Fixed upstream in AISF
(`rustls-tls-native-roots`), which got the run through to a genuine,
distinct API-level rejection (insufficient credit on that key's
account) rather than a network failure — confirming everything up to
the model call itself was correct, with only that one key's billing
still in the way.

**The asymmetry itself is now closed.** AISF grew a `claude_cli`/
MCP-bridge driver for `validation` too — `mcp_bridge.rs`'s dispatch/
governance/snapshot logic now serves either stage, parameterized by
which one it's asked for rather than duplicated. No `ANTHROPIC_API_KEY`
in your environment? `AisfStageBackend` sets `EVAL_STAGE_DRIVER=
claude_cli` on the spawned `eval-stage` process unconditionally
(harmless when a real key *is* set — AISF checks for one first and only
falls back to this) — AISF's own side then drives the same governed
tool loop through `claude -p` via an in-process MCP server instead of a
raw Anthropic call, for `triage` and `validation` alike now. **Verified
end-to-end in this project's own sandbox with zero API keys anywhere in
the chain, for both stages:** `skillopt-cli eval` scored **0.75** (6/8
correct) against `configs/aisf_triage_example.yaml`'s val split, and
**0.75** again against `configs/aisf_validation_example.yaml`'s —
genuinely meaningful results from a real governed agent, not a mock,
including a scenario where `tests_passed: true` but the diff quietly
weakened an authorization check, which the agent correctly did not
rubber-stamp as `Pass`. See AISF's own `RELEASE_NOTES.md` for the real
(previously-latent) bugs this project's live-verification work
uncovered along the way, in both directions.

Eval only exercises the executor role, though — a real `train` run needs
`optimizer`/`reflector` live too, and with `aisf_stage` restricted to
`executor` (see the config comments above), the natural zero-API-key
choice for those two roles is `claude_cli` itself, the plain
`ChatBackend` described next. `configs/aisf_triage_claude_cli_example.yaml`
puts all three together: `executor: aisf_stage` (driving AISF's real
`triage` agent via `claude_cli`+the MCP bridge, as above), `optimizer`/
`reflector: claude_cli`. Since `train()` visits every training example
each epoch and evaluates the *entire* test split at the end regardless
of `batch_size`/`val_batch_size`, keeping a real run's call budget small
enough to finish in a few minutes meant shrinking the dataset itself:
`data/aisf_triage_labels_smoke.jsonl` is a genuine 8-row *subset* of the
real 40-scenario set (4 train covering all four priorities, 2 val, 2
test), not fabricated data. **Verified live, a complete `train` run with
zero API keys anywhere in the chain:** `0/1 steps accepted, val score
1.000 -> 1.000, test score 1.000`. Every piece ran for real — 4 real
governed `triage` rollouts via `claude -p`, 4 real reflector critiques,
one real optimizer call that proposed a genuine, well-formed edit
("codify explicit priority-triggering criteria with concrete anchor
examples"), which applied cleanly but then scored 0.5 on the val subset
— worse than the unmodified skill's 1.0 — so the validation gate
correctly rejected it and kept the original prompt as best. That's the
loop's safety property doing exactly its job: a real optimizer proposal
that sounded reasonable but didn't actually pan out got caught before it
could regress a real agent's real production prompt, without a human
watching the run at all.

Also verified against the **complete** 40-scenario dataset, not the
8-row subset — `configs/aisf_triage_claude_cli_full_example.yaml`, same
setup, `labels_path: data/aisf_triage_labels.jsonl` (24 train / 8 val / 8
test). All 24 train examples split into 6 real batches (`batch_size: 4`),
so this is six full rollout/reflect/optimize/validate rounds, not one —
roughly 5x the call volume and about 20-30 minutes of wall-clock time.
Result: `0/6 steps accepted, val score 0.500 -> 0.500, test score
0.500`. Every one of the six optimizer proposals was a real, distinct,
plausible-sounding edit (tightening P0/P1 severity criteria, adding
anchor examples, shifting toward impact-based reasoning) — and every one
of them was correctly rejected, because none of them actually improved
the held-out score. Read that val number for what it is, though:
`val_batch_size: 2` was carried over unchanged from the smoke config, so
the gate was only ever checking 2 of the 8 real val examples per
decision — a noisy signal (worth 0.5 per example), not the full split.
The final test score, by contrast, *is* over the complete 8-example test
split (`train()` always evaluates the entire test set, uncapped by any
batch setting) — 0.5 there is a real, if unflattering, number: AISF's
actual `triage` prompt, unedited, gets exactly half of these particular
held-out scenarios right. Bump `val_batch_size` to `8` for a less noisy
gate signal on a future run, at the cost of a meaningfully longer one
(the config file's own comment covers this trade-off).

That bump was itself worth doing, and it changed the answer.
`configs/aisf_triage_claude_cli_full_deep_example.yaml` reruns the same
full dataset with `val_batch_size: 8` (the entire val split — sixteen
possible score values instead of three) and `epochs: 2` (twelve
optimizer attempts instead of six, so a rejected direction's rationale
gets a real second round via the rejection buffer). Roughly 220 real
`claude -p` calls, about an hour of wall-clock time. Result: **`1/12
steps accepted, val score 0.875 -> 1.000, test score 0.875`** — up from
the coarse-gate run's `0.500`. The accepted edit was a single,
well-targeted line: *"A user-facing display or UI bug that reliably
reproduces during a common, ordinary usage flow ... is P2, even if it
looks minor; reserve P3 only for cosmetic issues that require rare,
unsupported, or contrived conditions to trigger"* — it directly fixed a
real failure mode (reproducible-but-minor UI bugs getting misfiled as
P3 instead of P2) and took val to a perfect 1.0. Every one of the
remaining 11 proposals (including all 6 tried in the second epoch) was
correctly rejected — mostly because val was already at its ceiling of
1.0 by then, so there was no more room left to detect an improvement
against. Test score went from 0.500 (the original, unedited prompt) to
**0.875** (7/8) with the accepted edit applied — a real, meaningful,
model-found-and-confirmed improvement to a real agent's real production
prompt, entirely without a human in the loop or an API key anywhere in
the chain. So: no, the original skill wasn't already perfect — the
first full-dataset run's `0/6 accepted` said more about that run's
validation gate than about the skill. Widening the gate is what
actually answered the question.

That accepted edit was applied for real: AISF's actual
`prompts/triage.md` now includes this line in production (see AISF's
own `RELEASE_NOTES.md`), and `skills/aisf_triage_initial.md` here was
synced to match, so it stays a genuinely *current* copy of AISF's real
prompt rather than a frozen pre-fix snapshot. One consequence worth
being explicit about: every eval/train score documented above and in
this repo's `RELEASE_NOTES.md` (the `0.75` eval, `0/1`/`0/6`/`1/12`
accepted) was measured against the *pre-fix* wording — those numbers
are accurate history, not something to reproduce verbatim by rerunning
these configs today.

That fresh baseline has now been run: `skillopt-cli eval` against
`configs/aisf_triage_example.yaml`'s val split scored a perfect
**1.000** (8/8), and against its test split, **0.875** (7/8) —
matching, exactly, the `best_val_mean_score`/`test_result.mean_score`
the training run itself recorded for the accepted candidate. That
agreement is a real correctness check in its own right: a standalone
`eval` run and the number the training loop computed internally for
the same skill line up. (One honest loose end: the very first `0.75`
val score recorded for the *pre-fix* skill, back when this integration
was first verified live, doesn't match the `0.875` the full-dataset
train run's own `initial_val_score` later recorded for that same
unedited skill on the same 8 val examples — both are real, both were
against identical skill text; the most likely explanation is ordinary
run-to-run variance in a real model's live judgment calls on the
genuinely ambiguous scenarios, not a bug in either measurement.) Since
this skill already scores at or near ceiling on this exact dataset,
finding further headroom from here needs either a harder benchmark or
scenarios this dataset doesn't already cover.

The same widened-gate question was asked of `aisf_validation` too —
`configs/aisf_validation_claude_cli_full_deep_example.yaml`, identical
shape (`val_batch_size: 8`, `epochs: 2`), against the full 40-scenario
validation dataset. Same conclusion: **`1/12 steps accepted, val score
0.750 -> 1.000, test score 1.000`** — a clean 8/8 on held-out test. The
accepted edit added two lines (test outcomes always override how a
diff "looks"; simple, self-contained diffs with `tests_passed: true`
default to `Pass` without extra scrutiny) — and, checked explicitly
rather than assumed, this didn't trade away the benchmark's central
property: the test split's two genuine `NeedsHuman` scenarios both
still scored correctly. See `RELEASE_NOTES.md` for the full step-by-step.

That edit was applied for real too: AISF's actual
`prompts/validation.md` now includes both lines in production, and
`skills/aisf_validation_initial.md` here was synced to match. Same
caveat as triage's: the `0.75` eval and `0/12`/`1/12` train scores
documented above for `aisf_validation` were measured against the
pre-fix wording — accurate history, not something rerunning these
configs today will reproduce, since they now start from the
already-improved skill.

That fresh baseline has been run too: `skillopt-cli eval` against
`configs/aisf_validation_example.yaml` scored a perfect **1.000** on
both the val split (8/8) and the test split (8/8) — matching,
exactly, the training run's own recorded numbers for the accepted
candidate. This skill now scores at ceiling on its own dataset, same
conclusion as triage's.

**Both skills at ceiling doesn't mean "done improving" — it means
"this benchmark's exhausted."** `configs/aisf_triage_hard_example.yaml`
and `configs/aisf_validation_hard_example.yaml` (paired with
`data/aisf_{triage,validation}_hard_labels.jsonl`, 24 new hand-labeled
scenarios each, no code changes needed — same `Environment` impls, just
a different `labels_path`) exist to answer that question properly:
deliberately adversarial cases engineered around the exact seams each
applied fix introduced. See each dataset's own `.md` labeling notes for
the full category breakdown and reasoning.

**The result is a genuine asymmetry, not a uniform outcome either
way:** `aisf_validation` held — a perfect **1.000** on both the hard
val and test splits (6/6 each), including diffs specifically built to
look "simple" while quietly weakening a security control, exactly the
seam its own applied fix opened up. `aisf_triage` did not: **0.333**
val (2/6), **0.500** test (3/6). Digging into which of the 12
held-out examples missed (not just the aggregate score) shows a real
pattern, not noise: 3 of the 7 misses (all three "reproduces reliably
in a common flow, but is genuinely trivial" test cases — a tooltip
typo, a stale copyright year, a momentary cart-counter flicker) were
all reported as `P2` when they should be `P3` — the applied fix's own
new rule over-generalizing "reproduces reliably in a common flow" past
where it should stop. The other 4 misses were blast-radius judgment
calls (a bounded-user-segment total block; a calmly-worded but real
payment bug) that went in *both* directions (2 over-classified, 2
under-classified) rather than one consistent bias — genuinely harder
judgment, and in at least one case (a checkout bug labeled `P1` here)
a call reasonable people could make differently, not simply a model
error to correct.

That's a real, actionable next target: the P2/P3 rule needs a second
pass, not from scratch. Training against `aisf_triage_hard_example.yaml`
(same shape as the earlier `_full_deep_` configs — `val_batch_size: 8`,
`epochs: 2`) is the natural next step; `aisf_validation` doesn't need
one yet, since it held.

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
