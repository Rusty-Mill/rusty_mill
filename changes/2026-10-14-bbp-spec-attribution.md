---
category: Added
changelog: **BBP spec attribution metric.** `rusty_bbp::spec_metrics::spec_attribution` marks each non-approved spec version retained or unused by whether the approved spec kept its added lines, and `bbp metrics` reports it with `unused_conflicts`; this backs 2b gate criterion 4.
---
## 2026-10-14 - BBP spec attribution metric

- **Added:** `spec_metrics::spec_attribution(events, blob_reader)`: each non-approved spec version is measured against the version stored just before it. `Retained` if at least half of the non-blank lines it added are in the approved spec, `Unused` if fewer, `Unchanged` if it changed nothing, `Unmeasured` if unreadable, beyond the exact diff limit, or stored after the approval. A version that only deleted lines is judged by whether the approved spec also lacks them. `unused_conflicts` counts versions that are both `Conflict` and `Unused`; `bbp metrics` prints `spec_attribution` and `unused_conflicts`.
- **Backs:** 2b gate criterion 4 (contention is real: an unused spec version has a conflict).
- **Known limitations:** the base is the previous stored version, not the one the author read (the log does not record it). Line identity is exact text without trailing whitespace, so a reflowed line counts as unused. Not an evidence run.
