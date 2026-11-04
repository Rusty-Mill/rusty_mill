# rusty_term project docs

The terminal engine and front-ends: `crates/libs/ui/rusty_term` and its headless
VT engine `rusty_term_core` (`core/`). Moved out of `docs/projects/mux/` on
2026-10-27; the mux project links `rusty_term_core` but owns none of these docs.

| Doc | What it holds |
|---|---|
| [ROADMAP.md](ROADMAP.md) | Draft order of work across rusty_term and rusty_mux, and the open owner questions (2026-11-04) |
| [HANDOFF.md](HANDOFF.md) | Remaining work, roadmap, rules and validation as of the 2026-10-23 handoff |
| [NOTES.md](NOTES.md) | Decision log and design context from the 2026-10-21 to 10-23 perf session |
| [STYLEGUIDE.md](STYLEGUIDE.md) | Chrome style guide for the windowed front-end: colour roles, the fixed chrome scale (owner, 2026-11-03), components, UX rules and review |
| [conhost/design.md](conhost/design.md) | Our own Windows console host, replacing ConPTY's conhost (#1222): design, decisions D1–D6, PR plan; spike evidence and review rounds alongside |

Elsewhere:

- Crate usage, features and runtime requirements: [crate README](../../../crates/libs/ui/rusty_term/README.md)
- Perf backlog (the row numbers HANDOFF.md cites): [RUSTY-TERM-PERF-BACKLOG.md](../../research/RUSTY-TERM-PERF-BACKLOG.md)
- Fidelity gate and oracle harness: [rusty-term-perf/](../../research/rusty-term-perf/FIDELITY.md)
- Mux-side engine docs stay in [../mux/](../mux/CHARTER.md): incremental attach ([DELTA.md](../mux/DELTA.md)) and RTCP v6 checkpoints ([REPLAY.md](../mux/REPLAY.md))
