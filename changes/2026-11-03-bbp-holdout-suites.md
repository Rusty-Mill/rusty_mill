---
category: Added
changelog: rusty_bbp claims mode gains holdout suites (#597 slice 1) - holdout candidates, a holdout digest, and an accept line that waits for a holdout pass.
---
## 2026-11-03 - BBP holdout suites (Let Em Loose #597, slice 1)

- **Added:** a claims task may open with `holdout_digest`, the SHA-256 of a frozen holdout suite whose tests live outside every sandbox. A role with the new `holdout` capability stores a holdout candidate (`suite: holdout`) that must build exactly the claim's current code (`holdout_mismatch` otherwise); it is debited like any run and never becomes current.
- **Added:** reports bind to their candidate's suite: R16 checks a holdout report against `holdout_digest`, and holdout profile names (`holdout:`, `golden:`) never appear in build reports or the reverse. Holdout failures enter `failing(c)` like build failures, so only a holdout pass that lists them clears them.
- **Changed:** with a holdout suite, the accept line also needs a passed holdout report on the current candidate's code, stored after it became current (owner answer G1). Claims without candidates are unaffected and a human accept still overrides. `claim_card` shows `holdout_due`.
- **Added:** trace tests CL-71 to CL-76 in `tests/claims.rs`, and the rules in `docs/spec/bbp-claims-addendum.md`.
- **Known limitations:** no host path sets `holdout_digest` yet, so no task can have a holdout suite until the host slice that freezes the holdout set file; the host runner refuses a holdout run until then. Suite versions (`holdout_extended`, `holdout_updated`), protected test paths and golden sets are later slices. All new fields are optional and not written at their defaults, so existing logs replay byte for byte.
