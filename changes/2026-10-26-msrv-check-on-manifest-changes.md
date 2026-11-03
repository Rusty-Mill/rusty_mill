---
category: Changed
changelog: PR and merge-queue CI checks the declared rust-version only for crates whose own Cargo.toml changed; the scheduled sweep still checks every crate.
---
## 2026-10-26 - Declared-rust-version check only for crates whose manifest changed

- **Changed:** on a PR or merge-queue entry, the `compile at declared rust-version` job now checks only the workspace crates whose own `Cargo.toml` is in the diff (new `msrv_manifests` output of `ci_plan.py`, mapped to members by `check_msrv_compile.py --manifests`), and is skipped when there are none. Scheduled and manual full sweeps still check every crate. The job cost about 1,100 runner-minutes a day, mostly on code-only changes that cannot move a floor.
- **Known limitations:** a code-only change that uses an API newer than its crate's declared `rust-version`, a lockfile-only dependency bump that raises a floor, and a dependent built at a lower floor than a changed crate now surface on the next scheduled sweep rather than in their own PR. `check_rust_versions.py` still compares declared floors on every PR.
