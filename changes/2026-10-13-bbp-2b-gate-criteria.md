---
category: Added
changelog: **BBP 2b gate criteria.** The concurrency note proposes, before any measured run, the metrics and thresholds that decide whether to build the shared `rusty_crdt` (Phase 2b) or stop at 2a.
---
## 2026-10-13 - BBP 2b gate criteria

- **Added:** a section in `docs/bbp-concurrency-design.md`: paired N=1 and N=2 runs with real agents on spec authoring (the artifact 2b co-edits), a frozen rubric and failed-pair rule, changed-line conflicts on a common base, attributed loss, six go conditions and a decision table. Revised after the owner's review. Thresholds are proposals for the owner to approve or change before any run.
- **Known limitations:** docs only; the metrics tooling is a separate change. Ten tasks is a coarse screen; token cost is a proxy until harnesses report usage.
