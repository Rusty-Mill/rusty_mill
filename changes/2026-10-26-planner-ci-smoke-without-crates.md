---
category: Fixed
changelog: A CI change shipped with a change fragment or docs, and no crate change, now runs the CI-only smoke jobs instead of no build at all.
---
## 2026-10-26 - CI edits with no affected crate run the smoke jobs

- **Fixed:** the planner chose CI-only smoke only when every changed path was CI. Every PR also adds a change fragment, so a real CI PR never qualified; with no affected crate it ran no Cargo job, and edits to `setup-build-env` or a build step went unexercised until the merge queue (#928, #1010, #1073, #1081 took this route). `ci_plan.py --needs-ci-smoke` now selects the smoke jobs whenever a CI path changed and no crate is affected. A PR with an affected crate is unchanged: its lanes already run the shared build setup.
- **Known limitations:** the smoke job tests one crate (`rusty_baseline`); it shows the build environment comes up, not full workspace coverage.
