---
category: Fixed
changelog: The bbp pairs pin reports git's own error when it refuses, and no longer rewrites the index it inspects.
---
## 2026-11-01 - bbp pairs pin: git's reason, and a read-only status

- **Fixed:** when `git status`, `git rev-parse` or `git merge-base` failed, the `bbp pairs` pin refused with no reason, because it kept only git's stdout. It now includes git's stderr (for example "index file smaller than expected").
- **Fixed:** the pin's `git status` could refresh and rewrite `.git/index` while checking that the tree is clean. It now runs with `GIT_OPTIONAL_LOCKS=0`, as `build.rs` already did.
