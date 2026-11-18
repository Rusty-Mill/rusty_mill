---
category: Added
changelog: Let Em Loose smoke run 3 recorded (D150, Q53 additions, Q54); not yet the clean pass, the D127 freeze holds.
---
## 2026-11-17 - Let Em Loose smoke run 3 record

- **Added:** D150 in `docs/projects/let-em-loose/DECISIONS.md` (architect prompt: one claim post per reply, no evidence post; a bug fix) and a note on D147 that its condition is met; Q53 additions (invented runs, self-citations and citations of the question, with message ids), Q54 (the reviewers attack every claim and none supports, so nothing can reach accept) and a reply-failure note in `handoff/OPEN_QUESTIONS.md`; a "Run 3 result" section in `1166-smoke-run.md`.
- **Added:** D151 (Nano: the first-run bar is met through reviewer prompt wording: support only when the reviewer can point to text in the brief, nothing when unsure) and D152 (no host change; rerun smoke-4 on the prompt fixes alone), Q55 (should orch render the brief's artifact id in the lens prefix) and Q54 marked resolved for now.
- **Known limitations:** smoke run 3 is not the clean pass (Q stayed open with every candidate dropped), so the D127 freeze holds; D150 is decided, not built.
