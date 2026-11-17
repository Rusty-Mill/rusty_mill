---
category: Added
changelog: `rusty_orch lens run` runs the #598 cost breaker, pauses a task on a ceiling or an enforced trip, and taints each post with what its provider could see (Let Em Loose #758).
---
## 2026-11-17 - Cost breaker, taint and data boundaries for orch

- **Added:** `lens run` judges each lens's fresh cards against the cost breaker's window (40 calls or $5, a score move of 4, from the lens config). `observe`, the default, logs `breaker would trip`; `enforce` runs `bbp pause --reason breaker` and ends the run. The window is kept in memory and starts again with each run (598 PLAN S8).
- **Added:** a task ceiling that halts a round now runs `bbp pause --reason spend_ceiling` or `call_ceiling`. Pause ops are `orch-<run>-<reason>-<calls>`, so a new run after a resume pauses again.
- **Added:** every post that lands is tainted with the highest class its provider's context admitted (`round::Report::taints`). The table is saved per board before the scheduler state and applied to every card and settled line each pass, and to the alien lens's cards, so a tainted claim is shown only to providers cleared for its class.
- **Added:** `rusty_orch lens breaker <config> --bbp --dir --task --state --experiment`, one read-only JSON line: the config's breaker, the task's calls and spend, its taint table by class, and whether BBP holds it paused, with the pause's reason and detail (D137). `orch-store` gains `Store::read_meter` and `Store::read_taints`, which read without writing.
- **Added:** orch-bbp `sensitive_paths` are added to every pool member's sandbox `hide` at launch, so each confined harness works on BBP's filtered copy; with them set, an unconfined pool member is refused (D138). `real_data` roots feed the new `rusty_orch dry-run --bbp CONFIG --name N --work DIR -- PROGRAM ARGS`, which runs `bbp dry-run` and prints only its summary.
- **Added:** the breaker marks every lens's claims together, taking each claim's newest view, so lenses that follow different domains don't read each other's claims, or a stale score, as progress. A lens post with no taint entry (its answer lost, or a crash before the table was saved) reads as `sensitive` (D139).
- **Added:** an `--alien` sweep stopped by a task ceiling, before or during a hunt, now runs `bbp pause` like `lens run` does; a hunt stopped midway is still recorded (`Hunted::stopped`). `lens breaker` counts a reservation held at a crash, as `lens run` will on restore.
