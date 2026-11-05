---
category: Added
changelog: rusty_mux: proposed capability rows C75-C79 and a scoped C46 for hosting Let Em Loose agents (design only).
---
## 2026-11-05 - Proposed mux rows for mux-hosted agent ptys

- **Added:** `docs/projects/mux/AGENT-HOSTING.md`, the mux side of Let Em Loose D81: agent launch with an exact environment and passed descriptors (C75), signal and release without a mux kill (C76), an owner-only input gate with granted leases (C77), hold on exit (C78), block ownership scoped from C46, and Windows `--ssh` attach (C79). Rows are Proposed in CAPABILITIES.md and DECISIONS.md, and two pack QA entries record what the mux has today.
- **Known limitations:** design only; no code until Nano accepts each row. Surviving a mux restart, a program-less feed block and write-ahead input stay deferred.
