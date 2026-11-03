---
category: Added
changelog: rusty_term handoff pack harvested from the repo, PRs, remind-me and Mill Board, and the rusty-term-expert subagent that answers rusty_term and rusty_mux design questions before they reach Nano.
---
## 2026-11-03 - rusty_term project expert

- **Added:** `docs/projects/rusty_term/handoff/` (SPEC index, DECISIONS D1–D62 with a Source column (D62 and one QA entry come from the pre-merge smoke test), OPEN_QUESTIONS with nine source conflicts and seven carried owner calls), harvested with project-handoff v1.3.0 at `1417c404`. `.claude/agents/rusty-term-expert.md`, the design authority for the terminal core, mux server and protocols, transport, rusty_conhost and GUI code behaviour. A "Project experts" table in `CLAUDE.md` routes paths to it, routes GUI visual design to `rusty-term-design-expert`, and gives the cloud fallback.
- **Known limitations:** `rusty-term-design-expert` and its `gui-design/` pack are not on main, so GUI design questions escalate until they land (OPEN_QUESTIONS Q4). Most owner decisions in the existing logs carry no quote of Nano and are tagged `[default]`; 14 of the 29 `[stated]` rows rest on a quote an agent recorded in a repo doc or memory rather than on a linked message.
