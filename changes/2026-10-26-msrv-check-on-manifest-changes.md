---
category: Changed
changelog: PR and merge-queue CI runs the declared-rust-version check only when a Cargo.toml or Cargo.lock changed; the scheduled sweep still checks every crate.
---
## 2026-10-26 - Declared-rust-version check only on manifest or lockfile changes

- **Changed:** the `compile at declared rust-version` job now runs on a PR or merge-queue entry only when the diff touches a `Cargo.toml` or `Cargo.lock` (new `msrv` output of `ci_plan.py`), and then for the affected crates as before. Scheduled and manual full sweeps still check every crate. The job cost about 1,100 runner-minutes a day, mostly on code-only changes that cannot move a floor.
- **Known limitations:** a code-only change that uses an API newer than its crate's declared `rust-version` now surfaces on the next scheduled sweep rather than in its own PR.
