---
category: Added
changelog: "`rusty_orch consolidate` opens a D44 erosion cleanup task after 5 accepted claims on a repo, or records a skip when both shares are already zero."
---
## 2026-11-05 - Erosion gate: scheduled cleanups

- **Added:** `rusty_orch consolidate --dir BBP --repo NAME --repo-path DIR` (card #704 slice 4b, D44, D110, D113). It counts accepted claims on NAME's claims tasks since the last cleanup, across tasks. After 5 it plans a cleanup. With `--execute` it opens a claims task through `bbp open --mode claims`, using the operator's profile set with the erosion profile's `--base` set to the measured HEAD. A set whose erosion profile isn't `--cleanup` is refused. When both shares are already zero, it records a skip instead (D44). Only the count leg is built here; the half-limit leg (D114) follows in its own PR.
- **Docs:** orch ADR-0023.
