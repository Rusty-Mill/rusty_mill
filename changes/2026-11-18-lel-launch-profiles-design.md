---
category: Added
changelog: Design for Let Em Loose agent launch profiles (card #1571) and the mux grants-and-revocation principle.
---
## 2026-11-18 - Agent launch profiles design (#1571)

- **Added:** `docs/projects/let-em-loose/1571-launch-profiles-design.md`, a draft design Nano gates: an agent file (trust cap) and a project file (needs) resolve purely into one C75 `Launch`; credentials minted per launch and revoked by each service; isolation levels `same_user`, `agent_user`, `jail`.
- **Added:** `## Grants and revocation` in `docs/projects/mux/AGENT-HOSTING.md`, stating Nano's principle (mux grants at launch, each service revokes, orch holds both keys).
- **Added:** LEL `DECISIONS.md` D160 (design approved), D161 (v0 isolation `same_user`) and D162 (profile files in Factory), recording Nano's 2026-11-18 answers; the design's status is now approved and its section 7 lists what was decided.
- **Known limitations:** docs only; no code yet.
