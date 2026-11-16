---
category: Fixed
changelog: rusty_bbp_host's filtered-build acceptance test no longer fails intermittently when its stand-in author forfeits the turns after the run.
---
## 2026-11-16 - Filtered-build acceptance test flake

- **Fixed:** the stand-in Codex author in `confined_harness`'s filtered-build test exited at once on any turn after turn 2. Once the run passed, the moderator gave the maker turns 3, 4 and 5, each forfeited within milliseconds, so the moderator could pause on three forfeits before the test's 100 ms poll cancelled the task (1 failure in 60 local runs). The stand-in now idles on later turns until the cancel stops it: 0 failures in 120 runs, four at a time.
- **Known limitations:** the test now takes about 5 seconds, the moderator's end grace before it stops the idle harness.
