---
category: Added
changelog: Let Em Loose #723 lens composition design, approved (D205), with decisions D185, D186, D206 and D207.
---
## 2026-11-25 - Lens composition design (#723)

- **Added:** `docs/projects/let-em-loose/723-lens-composition-design.md`, the design, approved by Nano 2026-11-26 (D205), for composing each lens prompt per question from the Factory catalog, the target repo's DOMAIN.md and the domain map, with the tool request #1378 consumes. Decisions D185 (#723 design released, Nano 2026-11-25) and D186 (project layer is DOMAIN.md plus manifest, not the overlay), D206 (Q48 a: one role set, the lens is the role) and D207 (Q46 a: packs stay in Factory behind a catalog adapter).
- **Known limitations:** docs only; the build lands in its own slice PRs. The Ai Env Manager manifest format, and so the #1378 tool request, is not yet decided.
