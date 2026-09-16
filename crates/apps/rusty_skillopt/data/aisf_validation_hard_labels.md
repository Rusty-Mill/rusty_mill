# `aisf_validation_hard_labels.jsonl` — labeling notes

24 hand-labeled scenarios for `env: { name: aisf_validation }`
(same `AisfValidationEnv`, just a different `labels_path` — no code
changes needed). Same format and verdict rubric as
`aisf_validation_labels.jsonl` (see that file's own notes). Exists for
the same reason as `aisf_triage_hard_labels.jsonl`: this skill scored a
perfect 1.0 val/test on the original 40-scenario set after the fix this
project's `train` loop found (see `RELEASE_NOTES.md`), and ceiling on
one benchmark doesn't mean there's nothing left to find — it means that
benchmark can no longer tell the difference.

## What this set is really stress-testing

The applied fix added: *"For simple, self-contained diffs where
tests_passed is true, default to Pass ... without escalating scrutiny
beyond what the change warrants."* That's a real, useful rule — but any
rule phrased around "simple-looking" is exactly the kind of rule an
adversarial diff can exploit by looking simple while not being simple.
Every category below targets that specific seam:

- **`hard-01`–`04` ("simple" diffs that quietly weaken a security
  control).** A one-line change: an admin check commented out, a
  visibility default flipped, a rate limit raised by 1000x, a
  parameterized query swapped for string interpolation. Every one is
  small, single-purpose, and `tests_passed: true` — because the
  existing tests don't cover the weakened path, not because the change
  is safe. This is the rule's most direct failure mode if it over-fires.
- **`hard-05`–`08` (`test_summary` text agreeing or disagreeing with
  the boolean).** Two of these say "skipped" in the summary; only one
  of them is actually a red flag (tests specifically covering the
  changed behavior were disabled in the same PR) — the other is
  ordinary, disclosed, unrelated flakiness. The point isn't "any skip
  is suspicious," it's reading which tests were skipped and why.
- **`hard-09`–`12` (scope/blast-radius hidden behind a single-file
  diff).** Small line count and "one file changed" are not the same
  thing as low-risk — a one-line change to a shared auth middleware
  file, tested only by the one caller that happened to submit the PR,
  is a different animal from a one-line change to a file with exactly
  one caller and full coverage of it. Both shapes are represented,
  including one Pass control (`hard-10`) that looks structurally
  identical to the NeedsHuman case next to it (`hard-09`).
- **`hard-13`–`16` (genuinely simple, safe diffs — the control
  group).** If widening this benchmark just pushes every score toward
  `NeedsHuman` by reflex, that's not an improvement, it's a different
  failure mode with the opposite bias. These four are unambiguously
  safe and covered, and should stay `Pass`.
- **`hard-17`–`18` (a confident narrative next to a failing test).**
  The PR description insists the change is safe or extensively
  reviewed; the test result says otherwise. Ground truth is the test
  result, never the prose around it — this is the mirror image of the
  `NeedsHuman` trap: don't let language talk you out of Fail, either.
- **`hard-19`–`20`, `24` (irreversible or high-consequence changes a
  test suite structurally can't validate).** A dropped database column,
  a force-push over shared history, a cron schedule change for a
  customer-facing email job — each described as small in its PR text,
  none of which "tests_passed" can actually vouch for.
- **`hard-21`–`23` (additional controls)** — a test-only change, an
  unambiguous Fail, and a silently-swallowed exception with no test
  asserting the old, safer behavior still holds.

## Split sizes and distribution

12 train / 6 val / 6 test. Verdict distribution across all 24 is
intentionally `NeedsHuman`-heavy (12 of 24, vs. 8 `Pass` and 4 `Fail`)
— a direct consequence of the applied fix's new `Pass` default being
exactly the behavior this set is built to stress-test. This is a
deliberate property of a benchmark aimed at one specific rule, not an
attempt at the same broad, even coverage the original 40-scenario set
aimed for.

## Known scoring limitation (inherited from `aisf_validation_labels.md`)

Same inherited limitation: AISF's own `validate()` silently defaults to
`NeedsHuman` when `report_validation` is never called at all, which is
indistinguishable here from a genuine `NeedsHuman` label — and this set
happens to have more genuine `NeedsHuman` labels than the original,
which somewhat *narrows* how much that blind spot could inflate a
score here, but doesn't remove it.
