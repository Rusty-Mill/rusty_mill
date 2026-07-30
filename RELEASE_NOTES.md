# Release Notes

Tracks notable changes to this repo, reverse chronological. As of PR #1, every
change lands through a PR against `main` (merge commit, green CI required);
entries from before that point predate the PR workflow and are keyed by
commit instead.

---

## Author a harder benchmark for both skills — one holds, one doesn't
**2026-07-30**

- **Added:** `data/aisf_triage_hard_labels.jsonl` and
  `data/aisf_validation_hard_labels.jsonl`, 24 new hand-labeled
  scenarios each (12 train / 6 val / 6 test), plus their `.md` labeling
  notes and `configs/aisf_{triage,validation}_hard_example.yaml`. No
  code changes anywhere — `AisfTriageEnv`/`AisfValidationEnv` are
  already parameterized by `labels_path`, so a harder benchmark is
  purely new data. Each scenario is built around a specific reasoning
  seam: for triage, cases that reproduce reliably in a common flow but
  are genuinely trivial (stress-testing the applied P2/P3 fix's own
  boundary), misleading GitHub labels, severity hidden behind a
  cosmetic-looking wrapper, tone-as-a-false-signal, and bounded-segment
  blast-radius judgment calls; for validation, diffs engineered to look
  "simple" while quietly weakening a security control (the exact seam
  the applied Pass-default fix opened up), `test_summary` text that
  does or doesn't actually contradict `tests_passed`, and irreversible
  changes a test suite structurally can't validate.
- **Verified live, and the result is a genuine asymmetry:**
  `aisf_validation` scored a perfect **1.000** on both the hard val and
  test splits (6/6 each) — it held under direct adversarial pressure on
  its own fix's exact seam. `aisf_triage` did not: **0.333** val (2/6),
  **0.500** test (3/6).
- **Went one level deeper than the aggregate score, on purpose:**
  invoked `eval-stage triage` directly for each of the 12 held-out hard
  examples to see which ones missed and why, not just the mean. 3 of 7
  misses were exactly the predicted failure mode — a tooltip typo, a
  stale copyright year, and a momentary UI flicker, all three genuinely
  `P3`, all three reported as `P2` — the applied fix's "reproduces
  reliably in a common flow" rule over-generalizing past where it
  should stop. The other 4 misses were harder blast-radius calls (a
  bounded-segment total block; a calmly-worded real payment bug) that
  went in both directions, not one consistent bias — noted honestly as
  genuinely harder judgment, not all cleanly attributable to a model
  error.
- **Conclusion:** ceiling on a benchmark means the benchmark's
  exhausted, not that there's nothing left — confirmed two different
  ways now (this, and the earlier coarse-vs-wide validation-gate
  finding). The P2/P3 rule has a real, specific, actionable next
  target; `aisf_validation`'s fix held up and doesn't need one yet.

## Run a fresh baseline eval against the updated validation skill
**2026-07-30**

- **Verified live:** `skillopt-cli eval` against
  `configs/aisf_validation_example.yaml`, using the now-synced
  `skills/aisf_validation_initial.md` — val split **1.000** (8/8), test
  split **1.000** (8/8). Both match the full-dataset train run's own
  internally-recorded numbers for the accepted candidate exactly, same
  cross-check as triage's fresh baseline.
- This skill now scores at ceiling on `aisf_validation`'s own dataset,
  same conclusion as `aisf_triage`'s equivalent entry.

## Close the loop on `aisf_validation` too: apply and sync
**2026-07-30**

- **Changed (in AISF, upstream):** the previous entry's accepted edit
  is now real — AISF's actual `prompts/validation.md` includes both
  new lines in production. See AISF's own `RELEASE_NOTES.md`/`CLAUDE.md`
  for the full write-up, including the deliberate update to
  `original::VALIDATION`.
- **Changed (here):** `skills/aisf_validation_initial.md` synced to
  match byte-for-byte.
- **Documented explicitly:** every `aisf_validation` eval/train score
  in this repo's history so far (`0.75` eval, `0/12`/`1/12` accepted)
  was measured against the pre-fix wording — accurate history, but
  rerunning these configs today starts from the already-improved
  skill and won't reproduce those numbers.

## Widen the validation gate on `aisf_validation` too — another real find
**2026-07-30**

- **Added:** `configs/aisf_validation_claude_cli_full_deep_example.yaml`
  — the `aisf_validation` sibling of
  `aisf_triage_claude_cli_full_deep_example.yaml`: `val_batch_size: 8`
  (the entire val split) and `epochs: 2` (twelve optimizer attempts),
  every role live via `claude -p`, zero API keys anywhere in the chain,
  against the full 40-scenario `data/aisf_validation_labels.jsonl`.
- **Verified live:** `1/12 steps accepted, val score 0.750 -> 1.000,
  test score 1.000` — a clean 8/8 on the held-out test split. The
  accepted edit added two lines: one hardening that a failing test
  always means `Fail` regardless of how the diff looks, and one
  narrowly scoped to "simple, self-contained diffs where `tests_passed`
  is true" defaulting to `Pass` without escalating scrutiny beyond what
  the change warrants. Confirmed this didn't trade away the
  benchmark's central property: the test split's two genuine
  `NeedsHuman` scenarios (a weakened auth check, a disabled test
  reported as a clean pass — see `data/aisf_validation_labels.md`'s
  rubric) both still scored correctly. Eleven other proposals across
  both epochs were tried and correctly rejected, mostly because val was
  already at its 1.0 ceiling.
- Same shape as the `aisf_triage` result, same conclusion: a real,
  model-found, model-confirmed improvement to a real agent's real
  production prompt, with no human in the loop and no API key anywhere
  in the chain.

## Run a fresh baseline eval against the updated skill
**2026-07-30**

- **Verified live:** `skillopt-cli eval` against
  `configs/aisf_triage_example.yaml`, using the now-synced
  `skills/aisf_triage_initial.md` — val split **1.000** (8/8), test
  split **0.875** (7/8). Both numbers match the full-dataset train
  run's own internally-recorded `best_val_mean_score`/
  `test_result.mean_score` for the accepted candidate exactly, which is
  a real correctness check: a standalone `eval` and the training loop's
  internal evaluation agree on the same skill.
- **A discrepancy noted honestly, not hidden:** the very first `0.75`
  val score recorded for the pre-fix skill (when this integration was
  first verified live) doesn't match the `0.875` the full-dataset train
  run's own `initial_val_score` later recorded for that same unedited
  skill text on the same 8 val examples. Both measurements are real;
  the likely explanation is ordinary run-to-run variance in a live
  model's judgment on genuinely ambiguous scenarios, not a bug in
  either one — worth knowing about when comparing scores across
  separate live runs of this benchmark.
- This skill now scores at or near ceiling on `aisf_triage`'s own
  dataset; further headroom from here would need a harder benchmark or
  scenarios this dataset doesn't already cover.

## Close the loop: apply the found improvement to AISF, sync the local copy
**2026-07-30**

- **Changed (in AISF, upstream):** the previous entry's accepted edit
  is now real, not just a training artifact — AISF's actual
  `prompts/triage.md` includes the new P2/P3 line in production. See
  AISF's own `RELEASE_NOTES.md`/`CLAUDE.md` for the full write-up on
  that side, including the deliberate update to `original::TRIAGE`
  (the constant that exists specifically to force a conscious update
  on an intentional reword, not silent drift).
- **Changed (here):** `skills/aisf_triage_initial.md` synced to match
  byte-for-byte, so it stays a genuinely current copy of AISF's real
  prompt rather than a frozen pre-fix snapshot.
- **Documented explicitly, not left implicit:** every eval/train score
  in this repo's history so far (`0.75` eval, `0/1`/`0/6`/`1/12`
  accepted) was measured against the *pre-fix* wording. Those numbers
  are accurate as history; rerunning the existing example configs
  today starts from the already-improved skill, so they won't
  reproduce those exact numbers. A fresh baseline eval against the
  updated skill hasn't been run yet — the next natural step if further
  headroom is worth looking for.

## Widen the validation gate: the prompt wasn't actually already optimal
**2026-07-30**

- **Added:** `configs/aisf_triage_claude_cli_full_deep_example.yaml` —
  the full-dataset config with two changes aimed at giving a real
  improvement an actual chance to be found and confirmed:
  `val_batch_size: 8` (the entire val split, not a 2-example sample —
  sixteen possible score values instead of three) and `epochs: 2`
  (twelve optimizer attempts instead of six, so a rejected direction's
  rationale gets a real second round via the rejection buffer).
- **Verified live:** `1/12 steps accepted, val score 0.875 -> 1.000,
  test score 0.875` — roughly 220 real `claude -p` calls, about an
  hour of wall-clock time. The accepted edit was one well-targeted
  line clarifying that a reproducible-but-minor UI bug is `P2`, not
  `P3` — it directly fixed a real failure mode and took val to a
  perfect 1.0. Test score rose from the previous entry's `0.500`
  (the original, unedited prompt) to **0.875** (7/8) with the edit
  applied. Every other proposal, including all 6 tried in the second
  epoch, was correctly rejected — mostly because val was already at
  its 1.0 ceiling by then, leaving no further room to detect an
  improvement against.
- **This overturns the previous entry's framing, not just adds to
  it.** `0/6 steps accepted` on the coarse 2-example gate looked like
  "the prompt is already as good as it gets"; it was actually "this
  gate's resolution couldn't detect the real improvement that was
  there." A real, model-found, model-confirmed improvement to a real
  agent's real production prompt, with no human in the loop and no API
  key anywhere in the chain, is exactly the result this whole
  `aisf_stage` integration was built to make possible.

## Run the zero-API-key train loop again against the full 40-scenario dataset
**2026-07-29**

- **Added:** `configs/aisf_triage_claude_cli_full_example.yaml` — the
  previous entry's config, unchanged except `labels_path` pointing at
  the complete `data/aisf_triage_labels.jsonl` (24 train / 8 val / 8
  test) instead of the 8-row smoke subset.
- **Verified live:** `0/6 steps accepted, val score 0.500 -> 0.500,
  test score 0.500` — six real batches (`batch_size: 4` over 24 train
  examples), each a genuine rollout/reflect/optimize/validate round via
  `claude -p`, roughly 5x the smoke run's call volume. Every one of the
  six optimizer proposals was distinct and plausible (tightening P0/P1
  criteria, adding anchor examples, shifting toward impact-based
  reasoning); every one was correctly rejected for not actually
  improving the held-out score.
- **A real caveat, not swept under the rug:** `val_batch_size: 2` carried
  over unchanged from the smoke config, so the validation gate itself
  only ever checked 2 of the 8 real val examples per decision — a noisy
  signal, not the full split. The final test score is not subject to
  this: `train()` always evaluates the complete test set regardless of
  batch settings, so `0.500` there is a real number over all 8 held-out
  test scenarios — AISF's actual, unedited `triage` prompt gets exactly
  half of them right. The new config's own header comment documents this
  trade-off and how to widen the gate (`val_batch_size: 8`) on a future,
  longer run.

## Run a real `train` loop end to end with zero API keys
**2026-07-29**

- **Added:** `configs/aisf_triage_claude_cli_example.yaml` — every
  role live, no `ANTHROPIC_API_KEY` anywhere: `executor: aisf_stage`
  (AISF's real `triage` agent, driven via `claude_cli` + the MCP
  bridge) and `optimizer`/`reflector: claude_cli`. Paired with
  `data/aisf_triage_labels_smoke.jsonl`, a genuine 8-row subset (4
  train / 2 val / 2 test, one of each priority in the train rows) of
  the real 40-scenario `aisf_triage_labels.jsonl` — not fabricated
  data, just fewer of the same real rows, sized down because `train()`
  visits every training example each epoch and evaluates the entire
  test split at the end regardless of `batch_size`/`val_batch_size`,
  and this needed to finish in a few real `claude -p` calls' worth of
  time, not dozens.
- **Verified live:** a complete `train` run — `0/1 steps accepted, val
  score 1.000 -> 1.000, test score 1.000`. Every call was real: 4
  governed `triage` rollouts, 4 reflector critiques, one optimizer
  call that proposed a real, well-formed edit ("codify explicit
  priority-triggering criteria with concrete anchor examples"). The
  edit applied cleanly but scored 0.5 on the val subset against the
  unedited skill's 1.0, so the validation gate correctly rejected it
  and kept the original prompt as best — the loop's safety property
  working exactly as designed, with a real model on every side of it
  and nobody watching.
- This is the first `train` (not just `eval`) run in this project's
  history against AISF's real governed agent rather than a mock or the
  synthetic benchmark.

## Close the last aisf_stage asymmetry: validation gets a claude_cli path too
**2026-07-29**

- **Closed:** the one real asymmetry the previous entry left open. AISF
  grew a `claude_cli`/MCP-bridge driver for its `validation` stage
  (`mcp_bridge.rs`'s dispatch/governance/snapshot logic now serves
  either stage, parameterized rather than duplicated) — no code changed
  on this side of the fence at all, since `AisfStageBackend` already set
  `EVAL_STAGE_DRIVER=claude_cli` unconditionally on every spawned
  `eval-stage` process; AISF's own side just started honoring it for
  `validation` too.
- **Verified live, zero API keys anywhere in the chain:**
  `skillopt-cli eval` against `configs/aisf_validation_example.yaml`'s
  val split scored **0.75** (6/8 correct), matching `aisf_triage`'s own
  live 0.75 from earlier in this project's history. One of the eight
  scenarios directly exercises the rubric's central claim in
  `data/aisf_validation_labels.md` — `tests_passed: true` with an
  authorization check quietly commented out of the diff — and the real
  governed agent, driven by `claude -p` with no API key at all, did not
  reflexively report `Pass`.
- README and `docs/USAGE.md` updated to describe both stages
  symmetrically now, rather than flagging `validation` as the one that
  still needs a real key.

## Attempt a real live `aisf_validation` run with a human-provided API key
**2026-07-29**

- **Attempted:** a genuinely live `aisf_stage` executor run against
  `configs/aisf_validation_example.yaml`'s val split, using a real
  `ANTHROPIC_API_KEY` a human supplied directly in conversation (used
  only as an in-memory environment variable for the duration of the
  commands that needed it — never written to any file, config, or
  commit in either repo).
- **Found and fixed a real bug, but in AISF, not here:** the run failed
  with a TLS handshake error (`invalid peer certificate: UnknownIssuer`)
  against this sandbox's TLS-intercepting egress proxy, even though that
  proxy's CA was already installed system-wide — AISF's `reqwest` client
  only trusted a bundled Mozilla root list, never the OS store. Every
  prior "reached the expected TLS-certificate failure at the first real
  Anthropic API call" note in AISF's own `CLAUDE.md` history turned out
  to be this exact bug, not simply an absent key — the two are
  indistinguishable from the caller's side without a real key on hand.
  Fixed upstream in AISF (`rustls-tls` → `rustls-tls-native-roots`,
  still pure-Rust rustls, no openssl-sys); see AISF's own
  `RELEASE_NOTES.md`.
- **Verified live, past that fix:** the run reached the real Anthropic
  API for the first time in either project's history and got a genuine,
  distinct API-level response back — an insufficient-credit-balance
  rejection, not a network error. The boundary from the previous entry's
  "the actual live model call is the one piece not yet possible here"
  has moved precisely to that: everything up to and including the
  network path to Anthropic is now confirmed live and correct; a real
  model *completion* for `validation` still hasn't happened, purely
  because that specific key's account had no available credit.
- README updated to describe this precisely rather than repeat the
  now-stale "not yet possible" framing.

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
