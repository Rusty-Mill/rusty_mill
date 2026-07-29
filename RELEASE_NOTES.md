# Release Notes

Tracks notable changes to this repo, reverse chronological. As of PR #1, every
change lands through a PR against `main` (merge commit, green CI required);
entries from before that point predate the PR workflow and are keyed by
commit instead.

---

## Add `aisf_validation` env + 40 real labeled scenarios for AISF's validation stage
**2026-07-29**

- **Added:** `AisfValidationEnv` + `AisfValidationParams`
  (`crates/skillopt-envs/src/aisf_validation.rs`), `aisf_triage`'s
  sibling for AISF's `validation` stage — a JSONL-backed benchmark, not a
  copy of `aisf_triage` with renamed fields: `ValidationScenario`'s shape
  (`pr_number`/`diff`/`tests_passed`/`test_summary`) is genuinely
  different from a GitHub issue list, and `score` reads `verdict`
  (`Pass`/`Fail`/`NeedsHuman`), not `priority`, off `eval-stage`'s JSON
  output. Same two programmatic signals as triage's scorer (correct
  answer, no audit-log denials), same known limitation (AISF's
  `validate()` silently defaults to `NeedsHuman` when `report_validation`
  is never called, indistinguishable here from a genuine `NeedsHuman`
  label).
- **Required wiring `eval-stage validation` on AISF's side first** — it
  didn't exist before this (only `triage` was wired up). See AISF's own
  `RELEASE_NOTES.md` for the full detail: `ValidationScenario`,
  scenario-driven `read_pr_diff`/`run_tests` mocks, and a placeholder
  `Implemented`/`Triaged`/`Signal` construction to call
  `pipeline::validate` (which takes the typed pipeline handoff, not a
  bare `Signal`, unlike `pipeline::triage`). `AisfStageBackend` needed
  **zero changes** — the AISF stage name was always a plain string
  parameter.
- **Added:** `configs/aisf_validation_example.yaml`,
  `skills/aisf_validation_initial.md` (a copy of AISF's real
  `prompts/validation.md`), `data/aisf_validation_labels.jsonl` (40
  scenarios: 24 train / 8 val / 8 test, 16 Pass / 12 Fail / 12
  NeedsHuman, every split representing all three verdicts) and
  `data/aisf_validation_labels.md` (the labeling rubric — see there for
  what actually distinguishes the three verdicts; the interesting class
  is `NeedsHuman`, scenarios where `tests_passed: true` alone is *not* a
  safe signal: committed secrets, tests disabled/deleted rather than
  fixed, scope creep into security-sensitive code, a weakened permission
  check, a migration or force-push the test suite can't cover).
- **One real asymmetry with `aisf_triage`, stated honestly rather than
  hidden:** AISF's `eval-stage` has no `claude_cli`/MCP-bridge driver for
  `validation` yet, only `triage` — so unlike triage, this executor role
  can't run fully live without a real `ANTHROPIC_API_KEY` yet. Verified
  everything short of that: `AisfValidationEnv` parsed all 40 real rows
  through `build_env`; `eval --split train` against the real config
  reached AISF's subprocess, delivered the scenario, and correctly hit
  the "claude_cli not wired up for validation" error when no key was
  set; setting even a fake `ANTHROPIC_API_KEY` confirmed the *other*
  dispatch path is also reached correctly (a real HTTPS connection
  attempt to `api.anthropic.com` for `validate {pr_number=801}`, failing
  only on this sandbox's known TLS/cert gap, not a wiring bug).
- 9 new unit tests in `aisf_validation.rs` (parsing/partitioning,
  scenario round-trip, malformed-line reporting, and the same
  correct/denied/wrong/unparseable/missing-audit scoring matrix
  `aisf_triage`'s tests cover). 68 tests total across the workspace, all
  passing; `cargo fmt`/`clippy` clean.

## `aisf_stage` runs fully live too: AISF's own claude_cli + MCP bridge
**2026-07-29**

- **Changed:** `AisfStageBackend::chat` now sets `EVAL_STAGE_DRIVER=
  claude_cli` on the spawned `eval-stage` process unconditionally
  (`crates/skillopt-model/src/aisf_stage.rs`). Harmless when a real
  `ANTHROPIC_API_KEY` is already set — AISF checks for a key first and
  only consults this as a fallback — but it means a sandbox with a
  working `claude` CLI session and no key just works, without the caller
  remembering to export anything themselves.
- **What changed on AISF's side** (see its own `RELEASE_NOTES.md` for the
  full detail): `eval-stage triage` now has a second driver alongside its
  original Anthropic-API one. `EVAL_STAGE_DRIVER=claude_cli` routes it
  through `claude -p` instead, talking to a new in-process MCP server
  (AISF's `src/mcp_bridge.rs`) that exposes `fetch_github_issues`/
  `comment_on_issue`/`report_triage` directly — every call still gated
  through AISF's own `governance::authorize()` first, so which model is
  driving never changes what's allowed to happen. Prints the exact same
  JSON outcome shape either way, so nothing on this side (`AisfStageBackend`,
  `AisfTriageEnv`) needed to change to consume it.
- **This is the first fully live, zero-API-key run of the `aisf_triage`
  executor role — not just the optimizer/reflector `claude_cli` already
  unlocked last entry.** `skillopt-cli eval --config
  configs/aisf_triage_example.yaml --split val` scored **0.75** (6/8
  correct) against the real, hand-labeled 8-example val split: a
  genuinely meaningful signal from a real governed triage agent, no API
  key anywhere in the entire chain (rusty_skillopt → AISF's `eval-stage` →
  the MCP bridge → `claude -p` → real tool calls → a real classification).
- Two real, previously-latent bugs surfaced and got fixed on AISF's side
  while getting this to actually complete (both would have affected the
  original Anthropic-driven path too — no prior run ever had a real key
  to get far enough to hit either one): `tracing_subscriber::fmt()`'s
  stdout-by-default writer was silently corrupting `eval-stage`'s
  single-JSON-line-on-stdout contract whenever `FACTORY_PROMPTS_DIR` was
  set (which this backend always sets) — the very first attempt at this
  end-to-end run scored a flat 0.0 across every val example before that
  fix landed, 0.75 after. See AISF's `RELEASE_NOTES.md` for the second
  bug (the MCP bridge's output-file write timing) and both fixes in full.
- No code changes needed in `skillopt-core`, `skillopt-envs`, or
  `AisfTriageEnv`'s scoring logic — the one-line env var addition in
  `AisfStageBackend` is the entire integration surface on this side.

## Add `claude_cli` backend: real runs without an `ANTHROPIC_API_KEY`
**2026-07-29**

- **Added:** `Provider::ClaudeCli` + `ClaudeCliBackend`
  (`crates/skillopt-model/src/claude_cli.rs`). Shells out to the `claude`
  CLI's non-interactive print mode (`claude -p`) instead of a raw Anthropic
  HTTP request — useful wherever a working `claude` CLI session exists
  (e.g. an OAuth-authenticated Claude Code sandbox) but no portable
  `ANTHROPIC_API_KEY` is available. Confirmed the actual gap this closes,
  in this project's own development sandbox: a plain `curl` to
  `api.anthropic.com` 401s with no key, while `claude -p "hi"` already
  answers with no further setup.
- All the CLI's built-in tools are disabled (`--tools ""`) and sessions
  aren't persisted (`--no-session-persistence`) — a plain single-turn
  text completion (system prompt via `--system-prompt-file`, one user
  turn on stdin), matching every existing `ChatBackend` call site
  (`Engine` never sends more than one system message plus one user
  message per `chat()` call). **Not** a substitute for `aisf_stage`'s
  executor role, which genuinely needs a governed tool-use loop this
  backend deliberately doesn't provide.
- `config.rs` gets one new `Provider` variant, no new `BackendConfig`
  field — `model` passes straight through as `--model`. No key
  resolution: the CLI's own session is the auth, not something this
  process holds.
- **First genuinely live, full end-to-end run in this project's
  history.** Every prior "real" run in this repo's own log needed a live
  `ANTHROPIC_API_KEY` this environment never had; every `aisf_stage`
  verification so far stopped at `MissingApiKey`. Ran
  `configs/claude_cli_example.yaml` (smoke-scale: 4 train / 2 val / 2
  test, 1 epoch, batch 2, all three roles on `claude_cli`) to completion
  against `synthetic_arithmetic`: **0/2 steps accepted, val 1.0 → 1.0,
  test 1.0** in ~1m43s — correct behavior at this tiny/easy scale (nothing
  to fix), with genuinely contextual optimizer rationales in
  `report.json` ("Reinforces isolating the relevant operation and
  ignoring distractor details...", "Add guidance for handling multi-step
  or worded arithmetic problems...") rather than canned text, confirming
  real model calls drove every rollout/reflect/optimize step, not a mock.
- New tests: message-partitioning (system+user, user-only, multiple user
  messages joined, empty/system-only errors), a spawn-failure path
  (`PATH` overridden to exclude `claude`, restored afterward since it's a
  process-global env var), and an `#[ignore]`d live smoke test
  (`tests/claude_cli_smoke.rs`, no sibling checkout needed unlike
  `aisf_stage`'s) — run and passing in this same sandbox.
- 59 tests total across the workspace, all passing (6 new); `cargo
  fmt`/`clippy` clean.

## Author 40 real hand-labeled triage scenarios for `aisf_triage`
**2026-07-29**

- **Replaced** `data/aisf_triage_labels.jsonl`'s 8 illustrative placeholder
  rows with 40 real hand-labeled scenarios (24 train / 8 val / 8 test),
  each judged against a stated rubric rather than by feel — see the new
  `data/aisf_triage_labels.md` for the P0-P3 definitions, the rationale
  for including multi-issue scenarios (AISF's triage prompt says "fetch
  issues, classify the highest-priority one" — plural in, one priority
  out — so a few scenarios list 2 issues and `expected_priority` is the
  worse one, to actually exercise that judgment instead of just grading
  single-issue classification), and the known small-split-noise
  limitation at val/test size 8 (same lesson `docs/USAGE.md` §5 already
  documents for `synthetic_arithmetic`).
- Every split (train/val/test) includes all four priority levels, so the
  validation gate is never scoring against a split silently missing a
  class. Distribution: 12 P0 / 11 P1 / 10 P2 / 7 P3 across the 40 — a
  deliberate skew toward P0/P1 (more support at the highest-stakes
  boundary) rather than a flat split.
- Categories spread across outages/availability, security (auth bypass,
  stored XSS, tenant-isolation leak, privilege escalation, session-token
  invalidation), payments/billing, data loss/integrity, auth, mobile,
  browser-specific, performance, notifications, search, UI/UX, docs, and
  low-priority enhancement requests — titles written to read like real
  issue titles, not ones that announce their own priority.
- Verified against the real parser, not just inline test strings: `cargo
  run -p skillopt-cli -- eval --split train` against
  `configs/aisf_triage_example.yaml` successfully parsed all 40 rows via
  `AisfTriageEnv`/`build_env` and proceeded into an actual rollout attempt
  against a real, locally built AISF binary (correctly stopping only at
  `MissingApiKey` — no live Anthropic key in this session).
- Data + docs only; no code changes. Existing `aisf_triage` unit tests
  (which exercise the parser/scorer against inline fixture strings, not
  this file) are unaffected and still pass.

## Add `aisf_stage` backend + `aisf_triage` env: optimize a real agent's prompt
**2026-07-29**

- **Added:** `Provider::AisfStage` + `AisfStageBackend`
  (`crates/skillopt-model/src/aisf_stage.rs`). Treats one stage of a real,
  tool-using, multi-turn agent as the executor by driving
  [AISF](https://github.com/baileyrd/AISF)'s new `eval-stage` subcommand
  as a subprocess per rollout — writes the candidate skill to a scratch
  `FACTORY_PROMPTS_DIR`, pipes the example's scenario JSON to stdin,
  returns the JSON output unparsed. **No `skillopt-core` engine/trait
  changes** — `ChatBackend::chat`'s signature never promised "one API
  call," so a whole governed tool-use loop running inside a single
  `chat()` satisfies it exactly like a single Anthropic request would.
  `config.rs` gets one new `Provider` variant and one new `BackendConfig`
  field (`aisf_binary_path`), the same kind of additive config-only change
  Azure OpenAI added — `model` is reinterpreted as the AISF stage name
  (e.g. `"triage"`) for this provider, the same reinterpretation Azure
  already does for `model` (deployment name).
- **Added:** `AisfTriageEnv` + `AisfTriageParams`
  (`crates/skillopt-envs/src/aisf_triage.rs`), the data-driven counterpart
  to `synthetic_arithmetic`: loads hand-labeled scenarios from a JSONL
  file (one `LabeledRow` per line, a `split` field instead of separate
  train/val/test files) instead of generating a distribution. `score`
  reads two programmatic signals straight off `eval-stage`'s JSON output
  — priority match, and whether the audit log contains any `deny`
  decision — no LLM judge. **Known limitation, stated explicitly:** AISF's
  own triage stage silently defaults to `P2` when `report_triage` is never
  called at all, so that failure mode is indistinguishable from a genuine
  `P2` here unless the expected priority also happens to be `P2` — a
  property of `eval-stage`'s current output shape, not something this
  scorer can see past.
- **Added:** `configs/aisf_triage_example.yaml`, `skills/aisf_triage_initial.md`
  (a copy of AISF's real `prompts/triage.md`), and
  `data/aisf_triage_labels.jsonl` (8 illustrative hand-labeled scenarios —
  a demonstration of the format, not the full labeled set a real training
  run would want). Verified end-to-end against a real, locally built AISF
  binary: `cargo run -p skillopt-cli -- eval` with this config correctly
  reaches the subprocess, delivers the scenario, and surfaces AISF's own
  `MissingApiKey` error cleanly (no live Anthropic key available in this
  session, so a real model-driven rollout is unverified — the same
  limitation both projects already disclose).
- New tests: `aisf_stage`'s marker-extraction (round-tripped against the
  real `skillopt_core::prompts::executor_system_prompt`, not a copy of its
  format string), factory wiring (missing `aisf_binary_path` errors), and
  an `#[ignore]`d subprocess-boundary test (`tests/aisf_stage_smoke.rs`,
  needs a sibling AISF checkout, no API key) proving the wire format is
  correct up to AISF's own missing-key check. `aisf_triage`'s JSONL
  parsing/split-partitioning and scoring (correct+clean, correct+denied,
  wrong, unparseable, missing-audit) are all plain unit tests. 53 tests
  total across the workspace, all passing; `cargo fmt`/`clippy` clean.

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
