---
category: Changed
changelog: "CI: the rusty_tls live-internet check also runs on the native backend, counting Tier 1 item 1 for ADR-0002 Amendment 2."
---
## 2026-10-24 - CI: live internet check on the native backend

- **Added:** the native engine job (`rusty-tls-engine-native`) ends with the same non-blocking live step as the `ring` job. It runs `handrolled_live` against Google's sign-in hosts, with the rustls differential, through `live_internet_check.sh`. The 10 consecutive scheduled runs that Tier 1 item 1 requires are counted here for the native backend (ADR-0002 Amendment 2, condition 1).
- **Changed:** `live_internet_check.sh` reads an optional `LIVE_BACKEND`, so the two jobs' step summaries name their backend. The `ring` job sets `ring`, the native job `native`.
- **Tests:** `test_ci_workflow.py` checks both engine jobs with the same rules: limbo before live, live last, `continue-on-error`, through the script, and the backend label. `test_live_internet_check.py` covers the label.
- **Local check:** the native live test passed through this environment's TLS-intercepting proxy, which the test flags; such a run is not evidence about Google. CI runners provide the real runs.
- **Not changed:** no consumer is wired, and the step cannot gate a merge.
