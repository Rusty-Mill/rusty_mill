---
category: Security
changelog: Disable and bound the interactive Claude writer; correct subscription authentication and PR routing.
---
## 2026-10-22 - Hold interactive Claude worker activation

- **Fixed:** use the official subscription-token input, explicit GitHub-token authentication, human-only mentions and same-repository exact PR-head routing.
- **Security:** add a literal disabled gate, immutable action pins, runtime/turn bounds and queued per-target concurrency.
- **Known limitations:** activation, subscription eligibility, spending limits and persistent writing access require a separate explicit owner decision; no credentials installed or providers invoked.
