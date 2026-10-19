---
category: Added
changelog: "A plan for sovereignty item 12 (`docs/research/TRACING-FACADE-PLAN.md`): measures `tracing` use across the workspace and recommends a re-export chokepoint crate over an in-house implementation. No code changes."
---
## 2026-10-18 - Plan for the `tracing` facade (sovereignty item 12)

- **Added:** `docs/research/TRACING-FACADE-PLAN.md`, the plan item 12 requires before any code. It measures current use (30 workspace crates on `tracing`, 9 on `tracing-subscriber`, about 380 macro call sites outside the frozen Nexus), notes that third-party crates in the build graph (`axum`, `h2`, `sqlx`, `aws-*`, `tonic`) keep `tracing` in the graph and would lose their events under an in-house subscriber, and compares three options: keep, a re-export chokepoint crate, or a full in-house implementation. It recommends the chokepoint, which the owner approved (option B, 2026-10-19).
- **Not changed:** any code, manifest or policy check.
