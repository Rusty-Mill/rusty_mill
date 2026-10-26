---
category: Changed
changelog: BBP co-edited specs refuse a stale edit only when it touches a line edited since its base, not a whole section.
---
## 2026-10-25 - BBP line-level edit check

- **Changed:** `stale_base` now refuses a co-edited spec edit only when one of its ops is anchored on a line (or the start of the spec) that an edit accepted since its base also anchored on. Edits of other lines in the same section are applied to the head. Step 4b of `docs/bbp-event-based-plan.md`, chosen by the decision rule fixed before the step 4a run.
- **Changed:** the load harness reports `merged/edit` in place of the same-line estimate. At 4 editors on a 4-section spec, refusals fall from 0.21 to 0.03 per edit and wasted work from 18% to 3%.
- **Known limitations:** two edits placing lines after the same line are still refused, though a merge would keep both. No host or MCP surface for live agents yet.
