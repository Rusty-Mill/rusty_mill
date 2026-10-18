---
category: Added
changelog: Add read-only Rusty Orch repository intelligence with deterministic maintenance findings and dependency impact.
---
## 2026-10-18 - Rusty Orch repository intelligence

- **Added:** `rusty_orch inspect [--repo <dir>]` discovers Cargo workspace members and architectural relationships, reuses CI dependency and layer validators, and emits versioned JSON findings with evidence, recommendations and conservative reverse-dependency impact. Includes policy unit tests and representative Cargo-workspace integration tests.
- **Known limitations:** inspection requires Python 3.11+, Cargo, a current lockfile and cached dependencies; static checks do not run builds, tests or infer live CI status. No automatic task creation or code changes. Report output is bounded to 1 MiB.
