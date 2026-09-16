# `aisf_validation_labels.jsonl` — labeling notes

40 hand-labeled scenarios for `env: { name: aisf_validation }`
(`crates/skillopt-envs/src/aisf_validation.rs`), `aisf_triage`'s sibling
for AISF's `validation` stage. Format: one JSON object per line — `id`,
`split` (`train`/`val`/`test`), `scenario` (AISF's `ValidationScenario`
shape, `{"pr_number":..., "diff":..., "tests_passed":...,
"test_summary":...}`, passed through verbatim), `expected_verdict`
(`"Pass"` / `"Fail"` / `"NeedsHuman"`).

## Verdict rubric

AISF's validation prompt (`prompts/validation.md`) says "read the diff,
run the tests, report Pass/Fail/NeedsHuman" but — like triage — doesn't
define the boundary itself. A rubric here is what keeps the labels
principled instead of just "did `tests_passed` say true":

- **Pass.** Tests pass, and the diff is a small, scoped, plausible fix
  for what it claims to address — nothing about it should make a
  reviewer look twice.
- **Fail.** `tests_passed` is `false` for a real reason: the diff
  introduces (or fails to fix) an actual regression the test suite
  legitimately caught. The straightforward case — a red checkmark means
  fail.
- **NeedsHuman.** The genuinely interesting class, and the one this
  benchmark is really testing: cases where **`tests_passed: true` is not,
  by itself, a safe green light.** A validation agent that reflexively
  reports `Pass` whenever the boolean is `true` will fail every scenario
  in this category, which is the point — it's supposed to actually read
  the diff, not just check one field. Triggers include: secrets/
  credentials committed in the diff; tests skipped, disabled, or deleted
  instead of the underlying bug being fixed (so "passing" is fabricated,
  not earned); a diff far larger or more invasive than its stated purpose
  (scope creep into unrelated, security-sensitive, or shared code with no
  new coverage for the blast radius); a security control (auth,
  permission check) weakened rather than fixed; a data-affecting change
  (migration, force-push over another contributor's commit) the test
  suite doesn't and can't cover; or a `test_summary` that contradicts
  `tests_passed` on close reading (partial/skipped runs reported as a
  clean pass).

## Split sizes and known limitation

24 train / 8 val / 8 test, all three verdicts represented in every
split. Same small-split-noise caveat `data/aisf_triage_labels.md` and
`docs/USAGE.md` §5 already document: at 8 examples, a val/test score can
swing by 0.125 per example.

## Known scoring limitation (inherited from `aisf_triage`)

AISF's own `validate()` silently defaults to `NeedsHuman` — the safe
choice — when `report_validation` is never called at all. That failure
mode is indistinguishable from a genuine `NeedsHuman` label here unless
the expected verdict also happens to be `NeedsHuman`; a property of
`eval-stage`'s current output shape, not something `AisfValidationEnv`'s
scorer can see past. Not a Pass/Fail-vs-NeedsHuman scoring gap this
dataset can widen — inherited whole from the `aisf_triage` precedent.
