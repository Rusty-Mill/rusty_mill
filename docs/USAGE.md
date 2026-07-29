# Using rusty_skillopt

A practical guide to getting value out of this tool. The [README](../README.md)
covers what it is and how to install it; this covers *how to run it well* —
picking models, sizing a run, reading the results, and the failure modes that
look like bugs but aren't.

---

## 1. The mental model

The skill document is the weights. Everything else is the training harness.

| Neural training | Here |
| --- | --- |
| model weights | `skills/initial.md` → `out/<run>/best_skill.md` |
| forward pass | executor rollout on a batch |
| loss | `Environment::score` (programmatic, no LLM) |
| gradient | reflector critiques of the worst-scoring examples |
| optimizer step | optimizer proposes anchored `add`/`delete`/`replace` line edits |
| learning-rate cap | `max_ops_per_edit` |
| early stopping / gate | strict improvement on the held-out val split |

The executor model is **frozen**. The only thing that ever changes is text in
the markdown file. That has one big consequence for how you use this: **a run
only produces something if the executor is capable of doing better than it
currently is, and the benchmark is hard enough to show the difference.**

---

## 2. Three runs to do first, in order

### a. Mock dry run — verifies wiring, costs nothing

```bash
cargo test --workspace
cargo run -p skillopt-cli -- train --config configs/example.yaml
```

Expected output: `0/12 steps accepted, val score 0.000 -> 0.000`.

That is **success**, not failure. The mock executor echoes its input back, so
it scores ~0 no matter what the skill says and nothing can ever be accepted.
This run proves config parsing, the batch loop, the edit engine, the gate, and
the report writer all work end to end. Don't try to interpret its scores.

### b. Baseline eval — measure before you optimize

```bash
cargo run -p skillopt-cli -- eval --config configs/full_claude.yaml \
    --skill skills/initial.md --split test
```

`eval` runs the executor over one split with a fixed skill and no optimization.
Do this *before* training. If the baseline is already 1.0, training has nothing
to find and you should make the benchmark harder (§5) before spending calls.

### c. A real training run

```bash
export ANTHROPIC_API_KEY=sk-ant-...
cargo run -p skillopt-cli -- train --config configs/smoke_claude_hard.yaml
```

Then diff the result against where you started:

```bash
diff skills/initial.md out/smoke_claude_hard/best_skill.md
cargo run -p skillopt-cli -- eval --config configs/smoke_claude_hard.yaml \
    --skill out/smoke_claude_hard/best_skill.md --split test
```

---

## 3. Picking the three models

The config takes three independent backends, and they should generally *not*
be the same model:

| Role | What it does | Pick |
| --- | --- | --- |
| `executor` | runs the task with the skill as its system prompt | **The model you actually intend to deploy behind this skill.** Optimizing against a model you won't ship is optimizing the wrong thing. Usually the small/cheap one. |
| `optimizer` | reads critiques, emits a JSON edit | **The strongest model you're willing to pay for.** It runs once per step (cheap in volume) and it's the only component doing real reasoning. Weak optimizers produce malformed JSON and bad anchors. |
| `reflector` | one-two sentence critique per rollout | Cheap/fast. It only explains failures the scorer already detected — it never decides the score. |

The configs in this repo use Haiku executor + Sonnet optimizer + Haiku
reflector, which is the shape to copy.

Give the optimizer real headroom on `max_tokens` (1024 in the shipped configs).
It has to emit a full JSON object containing verbatim skill lines; truncation
shows up as a parse failure and a wasted step.

---

## 4. Sizing a run (and what it costs)

Steps per run = `epochs × ceil(train_size / batch_size)`.

Calls per run, by role:

```
executor  = val_batch_size            (initial gate baseline)
          + steps × batch_size        (rollouts)
          + steps × val_batch_size    (one gate check per step)
          + test_size                 (final eval)
reflector = steps × batch_size
optimizer = steps
```

For `smoke_claude_hard_bigtrain.yaml` (32 train, batch 4, 1 epoch, val 16,
test 16): 8 steps → **192 executor + 32 reflector + 8 optimizer = 232 calls.**

Two things follow from that formula:

- **`val_batch_size` dominates cost.** It's paid on *every* step, accepted or
  not. It is also the single biggest lever on result quality (§5), so this is
  the real tradeoff to think about — not `epochs`.
- **Everything is sequential.** There is no concurrency in the engine; wall
  clock is roughly `total_calls × per-call latency`. A 232-call run against a
  hosted API takes minutes, not seconds. Budget accordingly, and prefer
  raising `train_size` with `epochs: 1` over adding a second epoch.

---

## 5. The thing that actually determines whether a run works

**Your benchmark has to be hard enough to fail, and your training set has to be
diverse enough to fail in a representative way.** This repo's own run history
(`RELEASE_NOTES.md`) is four consecutive experiments learning exactly this:

| Config | Result | Lesson |
| --- | --- | --- |
| `full_claude.yaml` | 0/12 accepted, val 1.0 → 1.0 | Benchmark too easy — the executor was already perfect, so the gate had nothing to accept. |
| `full_claude_bigtrain.yaml` | 0/16 accepted, val 1.0 → 1.0 | More training data doesn't help when the *difficulty* is the ceiling. |
| `smoke_claude_hard.yaml` | val 0.75 → 1.0, test 1.0 | Turning on `multi_step_rate` + more distractors created real failures → first accepted edit. |
| `smoke_claude_hard_bigval.yaml` | val 1.0 → 1.0 but **test 0.875** | Only 8 training examples: the failure mode existed in the distribution but never appeared in a training batch. |
| `smoke_claude_hard_bigtrain.yaml` | 1/8 accepted, val 0.938 → 1.0, **test 1.0** | 32 training examples surfaced it; the fix generalized. |

Practical rules that fall out of this:

- **If nothing is ever accepted, look at the initial val score first.** At 1.0
  the run was pointless — raise difficulty. Below 1.0 with no accepts, the
  optimizer genuinely isn't finding a fix; check `report.json` rationales.
- **Small val splits are mostly noise.** At `val_size: 4`, one wrong answer
  moves the score by 0.25 and the gate accepts or rejects on coin flips —
  running the 4-example config six times gave scores flipping between 0.75 and
  1.0. 16 is the minimum that made results legible here.
- **The val subset is the *first* `val_batch_size` examples, in fixed order**,
  not a random sample (`Engine::train` → `subset`). Setting `val_batch_size`
  below `val_size` doesn't sample the split, it truncates it. Keep them equal
  unless you specifically want a fixed cheaper sub-slice.
- **Train/val/test never overlap** and generation is deterministic given
  `seed`, so runs are reproducible. Change `seed` to get a different draw from
  the same distribution — useful for checking that a result isn't seed luck.

For `synthetic_arithmetic`, the difficulty knobs are `multi_step_rate` (chains
2–3 sequential ops — the main one), `max_distractors` and `distractor_rate`
(irrelevant numbers to filter out), and the split sizes.

---

## 6. Reading the output

Every run writes two files to `output_dir`:

- **`best_skill.md`** — the winning skill. Note it is written *even when
  nothing was accepted*, in which case it's byte-identical to your input. Always
  `diff` it against `skill_path` rather than assuming it changed.
- **`report.json`** — the full `TrainOutcome`: `initial_val_score`,
  `best_val_score`, `test_result`, and a `steps[]` array with one record per
  step: `epoch`, `batch`, `accepted`, `rationale`, `train_mean_score`,
  `val_mean_score`, `best_val_mean_score`.

The `rationale` field is where the diagnosis lives. It carries the optimizer's
own one-line explanation on a normal step, and on a failed step it carries the
reason prefixed for you:

| `rationale` starts with | What happened |
| --- | --- |
| `optimizer call/parse failed:` | The optimizer's reply wasn't valid `SkillEdit` JSON, or the API call failed. Raise its `max_tokens` or use a stronger model. |
| `edit rejected (anchor not found:` | The optimizer invented a line that isn't in the skill. Usually self-corrects next step. |
| `edit rejected (anchor matched N lines` | Two identical lines in the skill doc — see §7. |
| `edit rejected (edit has no ops)` | The optimizer decided nothing needed changing (common once you're at ceiling). Correctly treated as a rejection, not an error. |
| `edit rejected (edit exceeds max ops` | Optimizer ignored `max_ops_per_edit`. |

A healthy run has *some* rejections — the gate is supposed to reject. A run
that is *all* parse failures is a model/`max_tokens` problem, not a tuning one.

Set `RUST_LOG=info` (or `debug`) for live tracing during the run.

---

## 7. Writing a good starting skill

The shipped `skills/initial.md` is deliberately minimal — three lines — because
you want the optimizer to discover the guidance, not to be handed it. Start
weak on purpose.

The constraint that matters is the edit engine's: **every op anchors to an
exact, verbatim, unique line** (compared after trimming). So:

- **Keep every non-blank line unique.** Two identical bullets anywhere in the
  document make that line un-anchorable — any op targeting it fails with
  `AmbiguousAnchor` and burns the step.
- **One idea per line.** Ops replace whole lines, so a paragraph packed onto one
  line can only be rewritten wholesale.
- **Short, declarative, imperative lines** anchor more reliably than long prose
  the optimizer has to reproduce character-for-character.

Ops within a single edit apply in order against a running buffer, so later ops
see earlier ones' effects.

---

## 8. Backends

| Provider | Config | Notes |
| --- | --- | --- |
| `anthropic` | `model`, `api_key_env` (default `ANTHROPIC_API_KEY`) | See `configs/full_claude.yaml`. |
| `openai_compatible` | `model`, `base_url`, optional `api_key_env` | OpenAI proper, plus any compatible endpoint. |
| `openai_compatible` (local) | `base_url: http://localhost:11434/v1`, **no** `api_key_env` | Ollama; auth header is omitted entirely when no key is configured. `configs/ollama_example.yaml`. |
| `openai_compatible` (Qwen) | `base_url: https://dashscope.aliyuncs.com/compatible-mode/v1` | DashScope compat mode. `configs/qwen_example.yaml` — verified against the documented contract, not a live call. |
| `azure_openai` | `base_url` = resource endpoint, `model` = **deployment name**, `api_key_env` (default `AZURE_OPENAI_API_KEY`), optional `api_version` | Needs its own provider: `api-key` header, deployment in the URL. `configs/azure_openai_example.yaml`. |
| `mock` | — | Network-free. Dry runs and tests only. |

Write these provider strings exactly as spelled above — `openai_compatible`,
not `open_ai_compatible` (there's a regression test pinning this).

Local models are the cheapest way to iterate on *config and difficulty tuning*;
switch to a hosted optimizer once you care about the resulting skill, since
edit quality is where the strong model earns its cost.

---

## 9. Using this on your own task

The synthetic benchmark exists so the loop is testable offline. To optimize a
skill for something you care about, implement `Environment` for it:

```rust
// crates/skillopt-envs/src/my_task.rs
impl Environment for MyEnv {
    fn name(&self) -> &str { "my_task" }
    fn train_examples(&self) -> &[Example] { &self.train }
    fn val_examples(&self) -> &[Example] { &self.val }
    fn test_examples(&self) -> &[Example] { &self.test }
    fn score(&self, example: &Example, output: &str) -> f64 { /* 0.0..=1.0 */ }
}
```

Register it in `crates/skillopt-envs/src/factory.rs` and name it in your
config's `env.name`. Nothing in `skillopt-core` changes — that's the point of
the trait boundary.

The one design rule: **`score` must be programmatic and deterministic.** The
validation gate is only trustworthy because scoring never involves a model
call. An LLM-judge scorer would make the gate as noisy as the thing it's
gating. If your task genuinely needs fuzzy scoring, make it a deterministic
function of the model output (exact match, regex extraction, unit-test pass
rate, containment) rather than a second opinion.

New LLM providers work the same way: implement `ChatBackend` in
`skillopt-model`, register it in that crate's factory.

---

## 10. Troubleshooting

| Symptom | Cause |
| --- | --- |
| `UnknownIssuer` / TLS errors | Fixed in-tree by using `rustls-tls-native-roots` (reads the OS trust store). If it recurs behind a TLS-terminating proxy, add the proxy CA to the OS store — never disable verification. |
| `unknown environment: "..."` | Env not registered in `skillopt-envs`'s factory, or a typo in `env.name`. |
| `invalid params for ... env` | `env.params` keys don't match the env's params struct; unknown keys are rejected. |
| Every step is a parse failure | Optimizer `max_tokens` too low, or the optimizer model is too weak for strict JSON. |
| `best_skill.md` identical to input | Zero accepted steps — check `initial_val_score` in `report.json` first (§5). |
| Run takes far longer than expected | Calls are sequential; recompute with the §4 formula. |
