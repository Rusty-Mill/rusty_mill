---
category: Added
changelog: docs/projects/mux/DELTA.md, the design (not code) for incremental attach deltas on top of RTCP v3.
---
## 2026-10-25 - Incremental attach design note

- **Added:** `docs/projects/mux/DELTA.md` designs incremental attach for C05 and C13. It covers the history sequence and epoch counters the grid still lacks, a base token sent with Ready, the server's delta-or-full rule, a proposed RTCD v1 layout (non-line state, appended history, changed rows), expected sizes, the test plan and open questions. REPLAY.md links to it.
- **Known limitations:** design only; no code, wire format or feature bit changes.
