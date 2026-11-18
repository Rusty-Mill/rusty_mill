---
category: Added
changelog: Let Em Loose records the claim 6 grounding audit (Q53), the floor deviation (Q58), Nano's drop of claim 6 (D154) and the standing grounding audit in every read-back (D155).
---
## 2026-11-18 - Let Em Loose claim 6 audit, D154 and D155

- **Added:** in `docs/projects/let-em-loose/`: D154 (Nano drops claim 6 on smoke-4: grounded score 0 and it deviates from the brief) and D155 (every run's read-back includes the grounding audit before Nano acts on an escalated claim) in `DECISIONS.md`; the audit of claim 6's five votes in Q53 and Q58 (the claim floors the age in whole days, so it keeps files aged between DAYS and DAYS+1 days that the brief says to delete) in `handoff/OPEN_QUESTIONS.md`; a line in the Run 4 result of `1166-smoke-run.md`.
- **Known limitations:** docs only; the audit script is a separate change.
