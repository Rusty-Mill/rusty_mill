---
category: Added
changelog: rusty_bbp_host freezes and runs holdout suites (#597 slice 4) - bbp open --holdout-profiles/--holdout-root, bbp holdout for new versions, summarised holdout logs.
---
## 2026-11-03 - Host holdout suites (Let Em Loose #597, slice 4)

- **Added:** `rusty_bbp_host::holdout`: a holdout set is a profile set of `holdout:`/`golden:` profiles, an absolute suite directory and the SHA-256 of each file in it, stored content-addressed per task. `bbp open --mode claims ... --holdout-profiles FILE --holdout-root DIR` freezes it and opens the task with its `holdout_digest`.
- **Added:** the runner serves holdout candidates: it checks the suite directory against the manifest first (a changed, missing or planted file or a symlink makes the run `error`), runs the suite's profiles with the directory readable and named in `BBP_HOLDOUT`, and reports under the holdout digest. The log artifact is a summary; the full output stays host-only under `<dir>/holdout-logs/` (#597 2.2).
- **Added:** `bbp holdout --dir D --task T --holdout-profiles FILE --holdout-root DIR` adopts a new version: a pure addition is written as `holdout_extended`; anything else is staged and refused `human_required` naming its digest, which the human adopts with `bbp human holdout-updated DIGEST` (owner answer G3).
- **Added:** the MCP `put_artifact` candidate takes `suite: holdout` for the holdout lens.
- **Added:** host tests for the run, the summary, a planted suite file and both version paths (trace CL-80).
- **Known limitations:** the holdout lens's model separation and context exclusions are orch's (#596). Golden sets (SIGNOFF check, case runner) are the next slice.
