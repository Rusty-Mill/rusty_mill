---
category: Changed
changelog: **BBP verdicts bind to what the judge read (2a gap closed).** Above one live turn a verdict is rejected `stale_subject` while content another principal appended during the judge's turn has not been returned to it by a read; the turn stays live and no log format changes.
---
## 2026-10-15 - BBP verdict binding

- **Changed:** with `max_live_turns > 1`, `TaskState` tracks per live turn the messages and artifacts others appended that its role may read and no read response has shown it (`Turn.unseen`, derived from the log, not logged). `check_verdict` rejects a verdict while it is not empty (`stale_subject`). The rejection keeps the turn, mints none (the turns budget in `grant` is untouched), and a read of the content lets the same verdict through. At one live turn nothing is tracked; schema 2 and 3 logs and the N=1 golden are byte-identical.
- **Tests:** the known-gap test flips to `a_verdict_is_refused_until_the_judge_has_read_what_was_appended`; new property test `verdicts_never_bind_to_unread_content` (two testers, refused-verdict retry loops, aborts, clocks; oracle is the independent `metrics::stale_verdicts`, which must stay zero; a refusal must mint no turn; budget and fold invariants as in the build-phase test). With the refusal disabled both tests fail.
- **Compatibility:** `stale_verdicts` now reads zero on current logs and still counts them on logs written before this change.
- **Not done:** `at_rev` on `TurnGranted` (an observed read base for the spec metrics) is a separate, optional log change; the refusal does not need it.
