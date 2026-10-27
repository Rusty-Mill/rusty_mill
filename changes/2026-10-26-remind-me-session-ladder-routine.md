---
category: Added
changelog: rusty_remind_me: decomposition can wait for a session to settle (idle_minutes), a recaptured session is due for decomposition again, and a scenario needs facts from two origins; a daily routine prompt drives the ladder (ADR-0029).
---
## 2026-10-26 - remind-me: drive session captures up the ladder

- **Added:** `idle_minutes` on `remind_me_decompose_batch`, defaulting to 0. A scheduled pass leaves a live session until it has been unchanged for that long.
- **Changed:** a capture changed after the revision its facts were drawn from is due for decomposition again, so what a resumed session adds is not lost, even when it lands while facts are being written. The queue returns each capture's `capture_revision` and `remind_me_decompose` accepts it. Previously any one fact retired a capture for good.
- **Changed:** decomposed facts carry their capture's session provenance (project, session, git branch and commit, working directory).
- **Fixed:** `remind_me_decompose_batch` rejects an invalid supplied `batch_size` or `idle_minutes` instead of falling back to defaults. This applies to `remind_me_decompose_batch`, the `capture_to_fact` candidates and the maintenance backlog.
- **Changed:** a `fact_to_scenario` candidate now needs facts from at least two origins (`MIN_SCENARIO_SOURCES`) as well as three facts. A fact's origin is the capture it came from, or the fact itself when it was written directly, so one long session can't promote itself and hand-written facts still count one each.
- **Added:** `docs/routines/session-ladder.md`, the prompt a daily Claude routine runs to decompose settled sessions into decisions, references, learnings and preferences and promote topic summaries.
- **Known limitations:** nothing is decomposed until the routine is scheduled.
