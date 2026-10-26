---
category: Changed
changelog: Contributors merge `main` into a PR branch only for a conflict or a needed fix, not to update it.
---
## 2026-10-26 - Merge main into PR branches only when needed

- **Changed:** CONTRIBUTING and AGENTS now say not to merge the default branch into a PR branch just to bring it up to date. The merge queue already tests each entry against current `main`, and each such merge re-runs the PR's CI. From 2026-10-25 08:55 to 2026-10-26 08:55 UTC, 24 PR runs were on bare merge-from-main commits, about 2,350 runner-minutes.
- **Known limitations:** guidance only; nothing enforces it.
