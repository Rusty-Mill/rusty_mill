# `aisf_triage_hard_labels.jsonl` — labeling notes

24 hand-labeled scenarios for `env: { name: aisf_triage }`
(`crates/skillopt-envs/src/aisf_triage.rs` — same `Environment` impl,
just pointed at a different `labels_path`; no code changes needed).
Same format as `aisf_triage_labels.jsonl`, same priority rubric (see
that file's own notes). This set exists for one reason: both
`aisf_triage`'s and `aisf_validation`'s skills reached ceiling
(1.0/perfect scores) on the original 40-scenario datasets after the
P2/P3 fix found by this project's `train` loop (see `RELEASE_NOTES.md`),
so a harder benchmark is the only way to tell "no further improvement
exists" apart from "this benchmark can't see it anymore" — the same
lesson the coarse-vs-wide validation-gate experiment already taught
once.

## What makes these harder

Not harder in the sense of more obscure trivia — harder in the sense of
deliberately probing the exact reasoning shortcuts a triage agent (or a
too-literal reading of its own skill document) could take instead of
real judgment:

- **`hard-01`–`04` (reproduces reliably, but genuinely P3).** The
  skill's own new rule says a UI bug that "reliably reproduces during a
  common, ordinary usage flow ... is P2, even if it looks minor."
  These four scenarios *do* reproduce reliably in a common flow (a
  tooltip typo you see on every hover, a footer date you see on every
  page load) — they're a direct test of whether that rule over-fires
  into genuine trivia rather than staying about real, if minor,
  functional impact.
- **`hard-05`–`08` (misleading labels).** GitHub's own `labels` field is
  exactly the kind of surface signal a shortcut-taking triage agent
  might defer to. Each of these has a label that points one direction
  and content that points the other — an `enhancement`-labeled request
  that's actually a live credential leak, a `bug`-labeled cosmetic
  animation glitch.
- **`hard-09`–`12` (severity hidden inside a cosmetic-looking wrapper,
  or the reverse).** A stray debug object in a chart tooltip *reads*
  like a rendering bug; it's actually a cross-customer PII leak. A
  duplicate toast notification *reads* like it could be anything; it's
  actually nothing. Both directions are tested, not just "assume
  everything hides something."
- **`hard-13`–`14` (tone is not a priority signal).** All-caps urgency
  wrapping a trivial issue; a calm, apologetic tone wrapping a real
  payment-correctness bug.
- **`hard-15`–`16` (blast-radius reasoning).** A total block for a real
  but bounded user segment (one browser/OS combination; users with 2FA
  enabled) — genuinely P1 by the existing rubric's own definition
  ("badly degraded for a real but bounded subset ... no reasonable
  workaround"), not P0 (reserved for "all or most users") and not
  automatically downgraded just because the segment is narrower than
  "everyone."
- **`hard-17`–`18` (silent data correctness vs. transient cosmetic
  flicker).** No crash, no error message, in both cases — one is a
  real, business-impacting silent data-integrity bug (P0 by the
  existing rubric); the other is a one-second UI flicker with zero
  lasting effect (P3).
- **`hard-19`–`20` (multi-issue tie-breaking)**, **`hard-21`–`24`
  (straightforward controls)** — same convention as the original
  dataset: confirm the skill still gets the unambiguous cases right,
  so a lower score here reads as "the hard cases are hard," not "the
  skill regressed on the easy ones too."

## Split sizes and distribution

12 train / 6 val / 6 test. Priority distribution across all 24 is
intentionally skewed toward `P0` (7) and `P3` (9) over `P1` (6) and `P2`
(2) — a direct consequence of most categories above being built as
"looks trivial, is severe" or "looks reproducible, is trivial" pairs,
which land at the extremes almost by construction. This is a deliberate
property of a benchmark built to stress specific reasoning boundaries,
not an attempt at the same broad, even coverage the original 40-scenario
set aimed for.
