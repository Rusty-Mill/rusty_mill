---
category: Changed
changelog: cargo-deny ignores RUSTSEC-2026-0543 (chrono unmaintained), which blocked every merge-queue run; replacing chrono is tracked as separate work.
---
## 2026-11-17 - cargo-deny: ignore the chrono unmaintained advisory

- **Changed:** `deny.toml` ignores RUSTSEC-2026-0543 with its reason. The advisory, published 2026-11-17, marks every chrono version unmaintained with no fixed release. 31 workspace crates use chrono directly, so the merge queue's `cargo-deny` job failed for every PR.
- **Known limitations:** chrono is still unmaintained. Moving to `jiff` or `time` is a separate migration.
