# Millwright — spec index
Design chat: unknown (no single chat; harvested) · remind-me tag `millwright`
Harvested: 2026-11-12 @ 667b79ba2
Owner: Nano (`baileyrd`) — the person whose words count as `[stated]`
Tags: [stated] [default] [open].

Millwright is a programme label (DECISIONS.md, name), not a crate, repo or board project.
This pack only points; the expert opens the files below at call time.

| Subject | Source of truth | Expert |
|---|---|---|
| What it is, rules of engagement, what waits | `../README.md` | this one |
| Phases, milestone codes (G, B, R, L, P, S), gates, critical path, board plan | `../ROADMAP.md` | this one |
| Nano's decisions | `../DECISIONS.md` (keys, remind-me ids) | this one |
| Project priority tiers (current view of project-priorities) | `../PRIORITIES.md` | this one |
| Questions waiting on Nano, with recommendations | `../QUESTIONS.md` | this one |
| Agents, systems, what is off the main path | `../architecture.svg` | this one |
| Let Em Loose design (L-codes' content) | `docs/projects/let-em-loose/` | `let-em-loose-expert` |
| rusty_term, rusty_mux, conhost | `docs/projects/rusty_term/handoff/` | `rusty-term-expert` |
| Portfolio Intelligence design (P-codes' content) | its own repo, `docs/handoff/` | `portfolio-intelligence-expert` (that repo) |
| remind-me design (R-codes' content) | `rusty-remind-me` repo | none yet |
| Merge, review and contribution rules | `AGENTS.md`, `CONTRIBUTING.md` | none: they are the rules |

## Scope of `millwright-expert`
- In: which phase or gate a piece of work belongs to, which milestone code owns it,
  sequencing and dependencies, what waits until a later phase, whether a card fits the
  "committed work only" board, the one-change-in-flight rule.
- Out: any system's design. It routes those to the expert in the table and never answers
  for them.
