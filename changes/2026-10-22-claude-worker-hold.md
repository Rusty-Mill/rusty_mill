---
category: Security
changelog: Disable and bound the interactive Claude writer; correct subscription authentication and PR routing.
---
## 2026-10-22 - Hold interactive Claude worker activation

- **Fixed:** use the official subscription-token input, explicit GitHub-token authentication, human-only mentions and same-repository exact PR-head routing.
- **Security:** add a literal disabled gate, immutable action pins, runtime/turn bounds and queued per-target concurrency.
- **Known limitations:** activation, subscription eligibility, spending limits and persistent writing access require a separate explicit owner decision; no credentials installed or providers invoked.

- **Activation hold:** keep the complete design in an inert template and a disabled live hold stub because current repository action policy blocks provider actions even in skipped jobs. A separate allowlist decision is required; no rules were changed.
