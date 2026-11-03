---
category: Changed
changelog: The merge-main rule names its three exceptions: a merge conflict, a required-check failure caused by `main`, or an explicit owner request.
---
## 2026-10-26 - Name the exceptions to the merge-main rule

- **Changed:** AGENTS and CONTRIBUTING now list exactly when to merge `main` into a PR branch: a merge conflict, a required-check failure caused by `main`, or an explicit request from the repository owner. "A fix the PR needs to go green" was open to reading as any red check, and an owner request was not covered.
- **Known limitations:** guidance only; nothing enforces it.
