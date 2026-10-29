---
category: Changed
changelog: CI jobs now time out on a hang (60 min heavy, 30 other, 5 for required-gate) instead of the 360-minute default.
---
## 2026-10-29 - CI job timeouts and merge-queue settings

- **Changed:** every `ci.yml` job sets `timeout-minutes`: 60 for the workspace clippy, test and msrv-compile jobs, 5 for `required-gate`, 30 for the rest. Before, only one job had a limit and the rest defaulted to 360 minutes, so the merge queue's check timeout was the only guard against a hang.
- **Changed:** ADR-0009's ruleset checklist now says status check timeout 180 minutes (120 dropped an entry that spent most of 123 minutes waiting for hosted runners) and build concurrency 2. The ruleset itself is changed in repository settings.
- **Known limitations:** limits come from durations seen in late October; a job that legitimately grows past its limit needs its own value raised.
