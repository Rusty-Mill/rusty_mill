# Release Notes

Tracks notable changes to this repo, reverse chronological. As of PR #1, every
change lands through a PR against `main` (merge commit, green CI required);
entries from before that point predate the PR workflow and are keyed by
commit instead.

---

## Add docs/USAGE.md: practical guide to running the loop well
**2026-07-29**

- **Added:** `docs/USAGE.md`, linked from the README's "Running it" section.
  The README says what the tool is and how to invoke it; this covers how to
  get a useful result out of it — the three model roles and how to pick them,
  the call-count formula for budgeting a run, difficulty/split-size tuning,
  reading `report.json` rationales, the edit engine's unique-anchor constraint
  as it applies to authoring a starting skill, and implementing `Environment`
  for a real task.
- The tuning guidance is drawn from this repo's own logged run history rather
  than invented: the four consecutive experiments below (`full_claude` →
  `full_claude_bigtrain` → `smoke_claude_hard_bigval` →
  `smoke_claude_hard_bigtrain`) are summarized as a table, since together they
  demonstrate the single thing that determines whether a run produces anything
  — benchmark difficulty and training-set diversity, not epochs.
- Docs only; no code changes.

## Support Qwen via DashScope's OpenAI-compatible mode (parity-loop issue #8)
**2026-07-23** · [PR #14](https://github.com/baileyrd/rusty_SkillOpt/pull/14)

- **Added:** `configs/qwen_example.yaml`. Alibaba Cloud DashScope's
  OpenAI-compatible mode speaks standard Bearer auth + OpenAI-shaped
  chat-completions JSON at `/chat/completions` - exactly what
  `openai_compatible` already implements, the same way
  `configs/ollama_example.yaml` covers Ollama. **No code changes.**
- Verification is code-level only (DashScope's documented contract matches
  the existing request shape), not a live call: no DashScope API key was
  available, and `dashscope.aliyuncs.com` returned the same egress-policy
  403 that blocked `ollama.com` earlier in this session. Noted explicitly
  in the config's comments so a divergence reads as "update this example,"
  not "something's silently broken."
- Second issue closed by the `/parity-loop` run (see `gap-analysis.md`,
  issue #8) - confirms the gap analysis's prediction that this gap would
  likely close without touching Rust code at all.

## Add Azure OpenAI backend (parity-loop issue #7)
**2026-07-23** · [PR #7](https://github.com/baileyrd/rusty_SkillOpt/pull/7)

- **Added:** `Provider::AzureOpenAi` + `AzureOpenAiBackend`. Azure OpenAI's
  API differs from plain `openai_compatible` in two ways that provider
  doesn't accommodate: auth is an `api-key` header (not `Authorization:
  Bearer`), and the URL encodes the resource endpoint + deployment name
  rather than taking `model` in the request body
  (`{endpoint}/openai/deployments/{deployment}/chat/completions?api-version=...`).
- New `BackendConfig.api_version` field (optional, Azure-only, defaults to a
  recent stable GA version). `base_url` is the resource endpoint, `model` is
  the deployment name, `api_key_env` defaults to `AZURE_OPENAI_API_KEY`.
- Also fixed the same latent snake_case-rename pitfall caught on
  `openai_compatible` earlier: `AzureOpenAi` would have derived
  `azure_open_ai`, not the documented `azure_openai` — added the
  `#[serde(rename = ...)]` and a regression test before it could ship broken.
- First issue closed by a `/parity-loop` run against Microsoft SkillOpt's
  feature set (see `gap-analysis.md`) — filed as issue #7, small/additive/
  no-new-dependency, so implemented autonomously per the loop's rules.
- New tests: socket-level (real `TcpListener`, no mocking crate) asserting
  the `api-key` header, URL shape, and default `api_version`, plus a
  factory-level test of the base_url/key requirement.

## Support Ollama (and other no-auth local servers) via openai_compatible
**2026-07-23** · [PR #5](https://github.com/baileyrd/rusty_SkillOpt/pull/5)

- **Added:** `configs/ollama_example.yaml` — the `openai_compatible`
  provider already worked against Ollama's OpenAI-compatible endpoint in
  principle (`base_url: http://localhost:11434/v1`), it just required a
  dummy API key env var for a server that doesn't check auth at all.
- **Changed:** `openai_compatible`'s API key is now optional. If
  `api_key_env` is explicitly set in config, that variable must still be
  present (erroring otherwise - the user named it on purpose); if it's
  unset, `OPENAI_API_KEY` is used when present, and no `Authorization`
  header is sent at all when neither is set.
- **Fixed a real, previously-latent bug found while wiring this up:**
  `Provider`'s `#[serde(rename_all = "snake_case")]` derives
  `open_ai_compatible` for the `OpenAiCompatible` variant, not
  `openai_compatible` - every doc and example config in this repo has
  always written the latter. It never surfaced because no real run had
  ever actually used `provider: openai_compatible` from a YAML file until
  this config. Added `#[serde(rename = "openai_compatible")]` plus a
  regression test asserting all three provider strings parse as documented.
- New tests: a real socket-level test (`openai_compat_auth.rs`, no mocking
  crate) asserting no `Authorization` header goes out when no key is
  configured and one does when a key is set, plus a factory-level test of
  the api_key_env resolution order.

## Add smoke_claude_hard_bigtrain.yaml: does a bigger training pool find the gap?
**2026-07-23** · [PR #4](https://github.com/baileyrd/rusty_SkillOpt/pull/4)

- **Added:** `configs/smoke_claude_hard_bigtrain.yaml` — same difficulty knobs
  and val/test size as `smoke_claude_hard_bigval.yaml`, but `train_size`
  bumped from 8 to 32 (`epochs` dropped 2 -> 1 to avoid compounding the size
  increase with a second epoch's calls).
- Run result: **1/8 steps accepted, val 0.938 -> 1.0, test 1.0** (up from
  0.875 in the 8-example version). Confirms the earlier diagnosis: a bigger,
  more representative training pool surfaced a real failure and the loop
  produced a genuinely generalizing fix — the accepted edit came from a
  batch that itself scored a perfect 1.0 in training, yet still measurably
  improved val, and test went from 14/16 to a clean 16/16 afterward. The
  accepted skill adds explicit sequential step-by-step + double-check
  guidance, exactly what a 4-op chained problem needs.
- Also confirms an edge case: once at ceiling, the optimizer correctly
  proposed *zero-op* edits for 4 consecutive batches ("already at ceiling,
  no changes") instead of inventing busywork, and the engine correctly
  treats an empty edit as a rejection rather than erroring.

## Add smoke_claude_hard_bigval.yaml: bigger val/test to cut measurement noise
**2026-07-22** · [PR #3](https://github.com/baileyrd/rusty_SkillOpt/pull/3)

- **Added:** `configs/smoke_claude_hard_bigval.yaml` — same difficulty knobs
  as `smoke_claude_hard.yaml` (multi-step chaining, heavier distractors) but
  val/test bumped from 4 to 16 examples each. Running the 4-example version 6
  times showed val flipping between 0.75 and 1.0 run to run — at that size a
  single wrong answer swings the score by 0.25, making "does it top out" hard
  to distinguish from noise.
- Run result: val 1.0 -> 1.0 (0/4 accepted, all 16 val examples correct every
  step), but **test score 0.875** (2/16 wrong). Verified both failures
  (`test-36`, `test-38`) are legitimate 4-op chained problems with correct
  expected values, not another generator bug. Real finding: with only 8
  training examples, the loop never happened to see a chain hard enough to
  trigger a training failure and give the optimizer something to react to,
  even though the failure mode exists in the broader distribution — a
  training-set-diversity gap, not a "too easy" or "already solved" ceiling.

## Fix distractor sentences colliding with the protagonist's name
**2026-07-22**

- **Fixed:** `synthetic_arithmetic`'s distractor generator could pick the
  protagonist's own name, producing self-contradictory problems (e.g. "Bob
  has 18 stickers... Bob has 1 stickers."). Found via a real training run
  (`full_claude_bigtrain.yaml`, 64 train examples): the one test failure out
  of 16 turned out to be exactly this case, not a genuine Haiku reasoning
  gap. Distractor name selection now excludes the protagonist.
- New regression test generates 200 examples with `distractor_rate: 1.0`,
  `max_distractors: 2` and asserts the protagonist is never restated.

## Add full_claude_bigtrain.yaml: does the val/test ceiling hold at scale?
**2026-07-22** · [PR #1](https://github.com/baileyrd/rusty_SkillOpt/pull/1)

- **Added:** `configs/full_claude_bigtrain.yaml` — same difficulty knobs as
  `full_claude.yaml` but `train_size` bumped from 24 to 64 (`epochs` dropped
  1 -> 1 to avoid compounding the size increase with a second epoch's calls).
- Run result: still 0/16 steps accepted, val 1.0 -> 1.0 (Haiku scored every
  single training example correctly too) — the ceiling from `full_claude.yaml`
  holds regardless of training-set size; it's the difficulty level, not an
  artifact of a small, easily-saturated set. Test score came in at 0.938
  (15/16), and the one failure turned out to be the distractor-collision bug
  fixed above, not a real generalization gap.
- First PR merged through the new PR-against-`main` workflow.

## Apply repo-config governance scaffolding; fix formatting
**2026-07-22**

- **Added:** standard governance files (SECURITY.md, CONTRIBUTING.md,
  CODE_OF_CONDUCT.md, CHANGELOG.md, RELEASE_NOTES.md, ARCHITECTURE.md,
  `docs/adr/0001-template.md`, PR/issue templates, `.github/workflows/ci-rust.yml`
  running `cargo fmt --check` / `clippy -D warnings` / `cargo test`) via the
  repo-config skill. README was left as-is (already existed).
- **Fixed:** ran `cargo fmt --all` across the workspace — it wasn't previously
  formatted to rustfmt defaults, which would have made the new CI workflow
  red on its first run.
- **Known limitation:** `ARCHITECTURE.md`'s boundary table and overview were
  hand-written for real; the ADR log is still just the seed template — no
  individual decisions have been logged yet. The CI workflow isn't wired up as
  a required branch-protection check yet (needs to happen on GitHub directly).

## Add full_claude.yaml: example.yaml-sized run against real Anthropic API
**2026-07-22** · [062e82f](https://github.com/baileyrd/rusty_SkillOpt/commit/062e82f)

- **Added:** `configs/full_claude.yaml`, mirroring `example.yaml`'s train/env
  sizing (24/8/16 examples, 2 epochs, batch_size 4, val_batch_size 8) but
  wired to the live Anthropic API instead of the mock backends.
- Run result: 0/12 steps accepted, val score 1.0 -> 1.0, test score 1.0 — the
  benchmark at this difficulty/scale was already too easy for the Haiku
  executor, so there was nothing for the gate to accept. Confirmed the full
  12-step loop executes correctly against the live API, including graceful
  recovery from one batch where the optimizer's JSON response was missing a
  required field.

## Add multi-step chains + more distractors to synthetic_arithmetic
**2026-07-22** · [974e77c](https://github.com/baileyrd/rusty_SkillOpt/commit/974e77c)

- **Added:** `multi_step_rate` (chains 2-3 sequential gain/lose/double/halve
  operations) and `max_distractors` (more than one irrelevant sentence per
  problem) on `SyntheticArithmeticParams`, plus `configs/smoke_claude_hard.yaml`
  exercising them.
- Defaults unchanged (`multi_step_rate: 0.0`, `max_distractors: 1`), so prior
  behavior is preserved unless a config opts in to the harder difficulty.
- Run result against the live API (Haiku executor/reflector, Sonnet
  optimizer): initial val score 0.75, optimizer proposed an edit telling the
  agent to filter irrelevant entities and apply multi-step operations in
  order, validation gate accepted it (val 0.75 -> 1.0), test score 1.0 —
  first real accepted-edit demonstration of the loop end to end.

## Switch reqwest to native root store; add real-Claude smoke config
**2026-07-22** · [3c8ec99](https://github.com/baileyrd/rusty_SkillOpt/commit/3c8ec99)

- **Fixed:** the default `rustls-tls` reqwest feature bundles a fixed
  webpki-roots trust store, which didn't include this environment's
  TLS-terminating egress proxy CA and made every outbound request fail with
  `UnknownIssuer`. Switched to `rustls-tls-native-roots` (reads the OS trust
  store, which already carries the proxy's CA here) — TLS verification was
  never disabled.
- **Added:** `configs/smoke_claude.yaml`, a small real-Anthropic-backed config
  used to confirm rollout/reflect/optimize calls succeed end to end against
  the live API.

## Implement rusty_skillopt: hand-rolled Rust reimplementation of SkillOpt's core loop
**2026-07-22** · [d7c078f](https://github.com/baileyrd/rusty_SkillOpt/commit/d7c078f)

- **Added:** initial Cargo workspace (`skillopt-core`, `skillopt-model`,
  `skillopt-envs`, `skillopt-cli`) implementing the rollout -> reflect ->
  aggregate/select -> optimize -> validation-gate training loop for markdown
  skill documents. Anchor-based skill-edit engine, Anthropic + OpenAI-compatible
  `ChatBackend` adapters plus a network-free Mock, a deterministic synthetic
  arithmetic `Environment`, and the `skillopt train`/`eval` CLI.
- **Known limitation, stated explicitly:** this is an independent design, not
  a line-by-line port of SkillOpt's Python source (not available to
  transcribe from). WebUI, additional providers, and the offline "Sleep"
  engine are out of scope for this pass — see README's Scope section.
- 26 tests, including an end-to-end scripted-backend test proving the loop
  accepts an edit that measurably improves validation score and rejects ones
  that don't.
