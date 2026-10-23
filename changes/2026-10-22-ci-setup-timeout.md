---
category: Fixed
changelog: Cap CI build environment setup at 15 minutes to prevent stuck setup from occupying runners for hours.
---
## 2026-10-22 - Bound CI environment setup

- **Fixed:** apply a 15-minute step timeout to all four setup-build-env invocations in CI. Setup timeouts fail normally; compilation, tests, required gates and merge-queue requirements are preserved.
- **Known limitations:** applies to future jobs and cannot release already-running jobs.
