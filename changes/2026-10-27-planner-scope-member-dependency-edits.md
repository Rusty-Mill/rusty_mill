---
category: Changed
changelog: PR and merge-queue CI scope a crate's dependency edits to that crate and its dependents instead of running the full workspace sweep; the scheduled sweep still builds everything.
---
## 2026-10-27 - Member dependency edits are scoped, not swept

- **Changed:** a member `Cargo.toml` that changes its dependency tables or `[features]` no longer forces the full sweep. The planner selects that member and its reverse dependencies by path, as for any other edit in the crate, and a `Cargo.lock` change beside a manifest edit adds the lock's consumers (`lockfile_diff.py`) instead of falling back. An unresolvable lock change, a root manifest change beyond additions and lanes spanning most components still run the full sweep. `cargo_toml_diff.py --members` is removed. An edit inside a `[patch]`ed in-repo package (`vendor/jsonwebtoken`, `vendor/aws-sigv4`) now selects every workspace crate depending on it through the full graph; before, it selected nothing unless the lock also changed. ADR-0009 records the exception.
- **Why:** on 2026-10-26/27 every run that checked the declared rust-version was a full sweep, and the sampled plans fell back on this rule (one `[target.'cfg(windows)'.dependencies]` line in `rusty_mux_ctl` swept 294 crates). Replayed over the 25 merges since 2026-10-24 that hit either fallback, 16 would have been scoped (1 to 28 crates) and 9 still swept.
- **Known limitation:** Cargo unifies features across a workspace-wide build, so a dependency edit can change how a crate that never depends on the edited one builds there (for example, a crate relying on a feature only another member enabled). That now surfaces on the daily scheduled sweep rather than in its own PR.
