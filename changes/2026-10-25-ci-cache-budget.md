---
category: Changed
changelog: "CI keeps only the workspace build caches, so they survive until the next queue run."
---
## 2026-10-25 - CI saves only the workspace clippy/test caches

- **Changed:** the daily sweep on `main` saved a Rust target cache for every component job as well as the workspace `clippy-<os>`/`test-<os>` ones. Against the 10 GB repository quota, a queue run two hours after the 10-25 sweep found `v0-rust-clippy-ubuntu-latest-…-a49c5372` missing (the sweep had uploaded it: 912 MB), and clippy took 11-14 minutes cold. The 22 component-job cache steps are gone, and `setup-build-env` caches only when given a key (clippy and test). The registry-only `plan` and `dependency-policy` caches stay. A test pins this.
