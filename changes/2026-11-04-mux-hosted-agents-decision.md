---
category: Changed
changelog: Let Em Loose records D79–D81 (gateway and jail, orch/gateway split, mux-hosted agent ptys) and adds the approved mux-hosted agents design (D82).
---
## 2026-11-04 - Let Em Loose: mux-hosted agent ptys design

- **Changed:** `docs/projects/let-em-loose/DECISIONS.md` adds D78 (superseded), D79 and D80 (#726 D-a and D-b) and D81 (agents get mux-hosted ptys; D20 and D66 marked reopened). `docs/projects/let-em-loose/mux-hosted-agents-design.md` is the design for Nano's approval: orch decides, the mux hosts the jail launcher's pty, the control socket stays out of the jail, takeover is recorded on the board, and the mux API orch needs.
- **Known limitations:** docs only. Nano approved the design and the takeover schema change (D82, 2026-11-05); the build lands in later PRs.
