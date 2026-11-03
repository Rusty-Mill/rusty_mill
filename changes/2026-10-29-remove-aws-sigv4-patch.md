---
category: Removed
changelog: Removed the vendored aws-sigv4 1.6.0 patch, unused since remind-me left the workspace.
---
## 2026-10-29 - Remove the unused aws-sigv4 patch

- **Removed:** `vendor/aws-sigv4`, its `[patch.crates-io]` entry, workspace exclusion, provenance row and `verify.py` branch. Its only consumer, remind-me's `aws-sdk-s3`, left with #1213, and Cargo recorded the patch as `[[patch.unused]]`; the lockfile drops that entry and resolves no `aws-sigv4`. Rusty-Mill/rusty-remind-me keeps its own copy.
- **Known limitations:** none. The jsonwebtoken patch and RUSTSEC-2026-0387 handling are unchanged.
