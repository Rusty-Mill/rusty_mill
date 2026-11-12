---
category: Added
changelog: millwright-expert answers Millwright programme questions (phase, gate, sequencing, milestone) from docs/projects/millwright, with a handoff pack and a CLAUDE.md routing row.
---
## 2026-11-12 - Millwright expert and doc drift fixes

- **Added:** `.claude/agents/millwright-expert.md` and a handoff pack under
  `docs/projects/millwright/handoff/` that points at the Millwright docs instead of
  copying them. A CLAUDE.md routing row, below the existing experts, sends
  `docs/projects/millwright/**` and cards tagged `millwright` to it for programme
  questions only; system design stays with each system's expert.
- **Changed:** Millwright docs corrected against remind-me: the merge rule waits for
  evidence checks and per-agent credentials, not review rules alone; remind-me and
  Portfolio Intelligence are named as separate repositories; the Hermes pilot question no
  longer cites a reopened rule as settled. Nano's 2026-11-12 Let Em Loose design freeze is
  recorded (Let Em Loose D127, Millwright `lel-freeze`), which answers L9.
- **Known limitations:** docs only. Nothing on Mill Board was created or moved.
