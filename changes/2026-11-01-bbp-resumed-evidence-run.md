---
category: Fixed
changelog: `bbp pairs` refuses a resumed evidence run: every capped `bbp mod` records its segment, and a run with more than one, or one whose build differs from its saved output, is not evidence (#1326).
---
## 2026-11-01 - bbp pairs refuses a resumed evidence run

- **Fixed:** an evidence run stopped and resumed under its cap was judged by its last `bbp mod` output alone, so a first segment's wall or cap stop, its build and its state timings were lost and a resumed run could load as a clean one (#1326). Each capped `bbp mod` now appends its segment (the build it ran) to `evidence/<task>.segments` under its lock and syncs it to disk before it acts on the task, so a moderator killed mid-run still leaves its segment, and `bbp pairs` refuses a run with no segment, a cut-off or unreadable segment record, more than one segment, or a segment built from another commit than its saved output reports. A resume under the same cap is still allowed and still capped; it just never counts.
- **Changed:** the evidence marker is now schema 2 (`"schema":2`). A marker written by an older `bbp` is refused, and an older `bbp` refuses to resume or count a run started under the new one.
- **Known limitations:** resumed runs are refused rather than folded, so a run interrupted for any reason must be repeated as a new task.
