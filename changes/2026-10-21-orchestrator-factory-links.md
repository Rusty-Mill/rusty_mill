---
category: Added
changelog: Add provider-neutral, generation-fenced card execution links for the autonomous factory.
---
## 2026-10-21 - Orchestrator factory execution links

- **Added:** a pure, provider-neutral factory aggregate that gives each Millboard card one recoverable Orch execution link, tracks all cross-component identifiers, and fences host commands with assignment generations.
- **Added:** bounded project/task/worker delegation with parent-retirement and stale-generation rejection, while retaining existing BBP/Sandbox adapters as the execution boundary.
- **Fixed:** make execution-link adoption persistence-idempotent and enforce aggregate delegation/call budgets, unique assignment identities, replacement grant bounds, and ancestor-generation fencing.
- **Fixed:** delegation and call reservations use fully checked arithmetic (a budget at `u32::MAX` is refused, never wrapped or panicked), and retiring an assignment at the last generation is refused without changing it.
- **Added:** `Factory::regrant`, the explicit way for a parent's current generation to re-authorize a child delegated by its replaced generation; a plain child replacement stays stale.
- **Added:** Millboard `card_execution_recover` / `Board::recover_execution`, a compare-and-set on the stored execution generation that persists a recovered execution; a stale actor can neither recover again nor rebind. Covered at the service boundary, including store reopen and a two-way race.
