---
category: Fixed
changelog: rusty_orch lens runner rejects executed evidence in a lens reply at parse, so one such post no longer halts the run.
---
## 2026-11-17 - Lens replies cannot post executed evidence

- **Fixed:** `orch-lens` `post::parse` now refuses `evidence_kind: "executed"` from a lens ("executed evidence is written by the runner"), and the reply protocol text no longer offers it. Before, the reply parsed, `bbp` refused the post (`EvidenceRequired`: no runner report) and `lens run` halted with `Halt::Board`; in the first smoke run an adversary on gpt-oss-120b ended the run this way. A refused reply is now a malformed reply: the job fails for that provider and moves on (D144).
- **Known limitations:** a refused board post of any other kind still halts the run (Q51, unchanged).
