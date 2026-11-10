---
category: Changed
changelog: docs: rusty_term HANDOFF points at AGENTS.md for merges and at the bench files for floors and ceilings.
---
## 2026-11-10 - docs: rusty_term HANDOFF queueing rule and floors brought up to date

- **Changed:** `docs/projects/term/HANDOFF.md` §4 told agents to queue ready PRs; it now defers to AGENTS.md and CONTRIBUTING.md (agents never queue or merge; DottiusMaximus does). §3.1 said the throughput floors were not met; they are met on the 4 MB workloads (#1019), and the section now links `bench/floors.py` and `bench/ir_ceilings.txt` instead of copying numbers. §5's stale ceilings table is replaced by the same link. Docs only.
- **Changed:** the rusty_term pack closes Q1 (new row D154: agents never queue or merge) and Q2 (D16); SPEC.md and ROADMAP "Next" row 3 updated to match.
