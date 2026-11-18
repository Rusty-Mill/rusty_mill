---
category: Added
changelog: Let Em Loose smoke run 5 recorded (D156, Q53 additions, a note on D153); not yet the clean pass, the D127 freeze holds.
---
## 2026-11-18 - Let Em Loose smoke run 5 record

- **Added:** D156 (architect prompt: set `answers` to the question's id on every claim; a bug fix) in `docs/projects/let-em-loose/DECISIONS.md`, a note on D153 that the Qwen reasoning budget worked (0 empty replies, against 3 and 3 in smoke-3 and smoke-4); Q53 additions (a vote crediting the brief with `lstat`, a wrong claim about `stat`, votes citing other votes, a speculative vote) and the grounded counters in `handoff/OPEN_QUESTIONS.md`; a "Run 5 result" section in `1166-smoke-run.md`.
- **Added:** D157 in `DECISIONS.md` (Nano: keep the D143 bar, build D156, rerun as smoke-6; the BBP rule for an all-dropped question returns to him only if candidates keep dropping legitimately).
- **Added:** D158 in `DECISIONS.md` (Nano: the grounding audit also covers dropped claims, as a read-only report) and Q59 closed.
- **Known limitations:** smoke run 5 is not the clean pass (2 candidates from 2 providers, both legitimately dropped, Q open), so the D127 freeze holds; what "the first run" means once every candidate is legitimately dropped is escalated to Nano; D156 is decided, not built.
