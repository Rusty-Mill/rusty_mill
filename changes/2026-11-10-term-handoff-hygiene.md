---
category: Changed
changelog: docs: rusty_term HANDOFF points at AGENTS.md for merges and at the bench files for floors and ceilings.
---
## 2026-11-10 - docs: rusty_term HANDOFF queueing rule and floors brought up to date

- **Changed:** `docs/projects/term/HANDOFF.md` §4 told agents to queue ready PRs; it now defers to AGENTS.md and CONTRIBUTING.md, and points at the open question of who queues and merges. §3.1 said the throughput floors were not met; they are met on the 4 MB workloads (#1019), and the section now links `bench/floors.py` and `bench/ir_ceilings.txt` instead of copying numbers. §5's stale ceilings table is replaced by the same link. Docs only.
- **Changed:** the rusty_term pack closes Q2 (D16) and Q1 (D157: Nano confirmed 2026-11-10 that the PR queue watcher also queues and merges; AGENTS.md and CONTRIBUTING.md now say so); SPEC.md and ROADMAP "Next" row 3 updated to match.
