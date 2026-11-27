---
category: Added
changelog: Let Em Loose #1382 slice plan for orch-minted, lens- and claim-scoped gateway tokens (D209, D212).
---
## 2026-11-27 - Let Em Loose #1382 token slice plan

- **Added:** `docs/projects/let-em-loose/1382-token-slices.md`, cutting the approved #726 token design into five slices (sans-IO `orch-token` mint, orch-bbp token write, revocation, end-to-end, witness on the claim), D209 placing the mint in a new `orch-token` crate, and D212 (Nano): no #1382 code until #1375–#1379 land. Docs only.
- **Known limitations:** no code yet. The design it cuts (`726-sandbox-design.md`, D180) is approved but still on PR #1873, not on main. The revocation slice needs an ask-first gateway config field.
