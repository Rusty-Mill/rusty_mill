---
category: Added
changelog: Let Em Loose smoke run 2 recorded (D148, D149, Q52 update, Q53); the first-run freeze (D127) still holds.
---
## 2026-11-17 - Let Em Loose smoke run 2 record

- **Added:** D148 (candidates never reviewed: the lens seat re-reads a question with `claim_card` when a delta carries a claim that answers it) and D149 (the architect prompt says a claim has no `evidence` field) in `docs/projects/let-em-loose/DECISIONS.md`, both bug fixes; Q53 (a mislabelled evidence direction and a self-citing counter decided claim A) and a Q52 update in `handoff/OPEN_QUESTIONS.md`; a "Run 2 result" section in `1166-smoke-run.md`.
- **Known limitations:** smoke run 2 is not the clean pass D143 asks for (2 candidates from 2 providers, Q stayed open), so the D127 freeze holds; D148 and D149 are decided, not built.
