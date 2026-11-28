---
category: Added
changelog: Let Em Loose #1236 design for the dedup stage-1 holdout suite and protected test paths, with D228 and D229.
---
## 2026-11-28 - Let Em Loose #1236 holdout suite design

- **Added:** `docs/projects/let-em-loose/1236-holdout-suite-design.md`, the design for the dedup stage-1 holdout suite (test seam, toolchain, holdout model, profiles, leak review, freezing, protected globs), and DECISIONS D228 (leak review by a local non-builder model) and D229 (the suite freezes at `bbp open`).
- **Known limitations:** design only. The test seam, toolchain and holdout model wait on Nano; nothing is generated or frozen.
