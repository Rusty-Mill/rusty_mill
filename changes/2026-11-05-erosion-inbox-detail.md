---
category: Added
changelog: `bbp inbox` shows erosion detail on an escalated claim, including per-metric deltas, the worst-affected functions and unmeasured code.
---
## 2026-11-05 - Erosion gate: detail in the inbox

- **Added:** each `bbp inbox` escalation has an `erosion` field (card #704 slice 3, D43, D45, D91). For an escalated claim, it holds the `erosion` bin's line from one report. That report is the one that put `erosion` in the claim's failing set (`failing: true`) or, when erosion isn't failing, the latest report on the claim's current candidate that ran the profile, so `unmeasured` code shows even while report-only. The line has per-metric base, candidate and delta in basis points, `within_limits`, total CC, LOC and function deltas, up to three worst-affected functions, `unmeasured` and `parse_errors`. A bin error line is shown as `error`. A log that can't be read or has no version-1 line gives `null`, and the inbox still answers.
- **Known limitations:** the Mill Board inbox doesn't render the field yet; that is the next PR. Whether the board also shows deltas while erosion passes is OPEN_QUESTIONS Q23.
