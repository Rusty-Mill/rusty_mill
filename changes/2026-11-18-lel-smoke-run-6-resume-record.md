---
category: Added
changelog: Let Em Loose smoke run 6 resume recorded (claim 6 reopened and chosen under D159 step 5); not yet the clean pass, criterion 2 is unmet.
---
## 2026-11-18 - Let Em Loose smoke run 6 resume record

- **Added:** in `docs/projects/let-em-loose/1166-smoke-run.md`, the smoke run 6 resume result: claim 6 reopened once, dropped again in epoch 2 on an invented test-run citation (msg 14) and a verbatim repeat that cites itself and its own earlier vote (msg 15), so under D159 step 5 Nano's `choose 1 6` counts toward criterion 3 with the grounding audit as the reason; the board confirms it, in Nano's own `bbp why --claim 6` output (claim 6 dropped again at rev 133, Accepted at rev 145, rule Human accept, epoch 2; Claude did not read the store itself). Criterion 2 stays unmet (2 candidates; the reasoning budget loaded only after the run). In `handoff/OPEN_QUESTIONS.md`, Q61's evidence (the adversary cited the A2 fixture claim as a test run in smoke-3, smoke-4 and twice in smoke-6) and a status line.
- **Known limitations:** docs only; smoke run 6 is not the clean pass, so the D127 freeze holds; the next run is smoke-7 with the budget loaded.
