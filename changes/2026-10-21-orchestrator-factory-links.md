---
category: Added
changelog: Add provider-neutral, generation-fenced card execution links for the autonomous factory.
---
## 2026-10-21 - Orchestrator factory execution links

- **Added:** a pure, provider-neutral factory aggregate that gives each Millboard card one recoverable Orch execution link, tracks all cross-component identifiers, and fences host commands with assignment generations.
- **Added:** bounded project/task/worker delegation with parent-retirement and stale-generation rejection, while retaining existing BBP/Sandbox adapters as the execution boundary.
- **Fixed:** make execution-link adoption persistence-idempotent and enforce aggregate delegation/call budgets, unique assignment identities, replacement grant bounds, and ancestor-generation fencing.
