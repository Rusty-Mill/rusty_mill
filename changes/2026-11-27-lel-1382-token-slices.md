---
category: Added
changelog: Let Em Loose #1382 slice plan for orch-minted, lens- and claim-scoped gateway tokens (D209).
---
## 2026-11-27 - Let Em Loose #1382 token slice plan

- **Added:** `docs/projects/let-em-loose/1382-token-slices.md`, cutting the approved #726 token design into five slices (sans-IO `orch-token` mint, orch-bbp token write, revocation, end-to-end, witness on the claim), and D209 placing the mint in a new `orch-token` crate. Docs only.
- **Known limitations:** no code yet; build order waits on Nano (the design puts #1382 last), and the revocation slice needs an ask-first gateway config field.
