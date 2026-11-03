@AGENTS.md

Follow the current CONTRIBUTING workflow in AGENTS.md. The super-board robot is active under
`docs/super-board/autonomy-policy.md`; `/super-review` is part of its lanes, not of the human workflow.

## Let Em Loose: design questions go to let-em-loose-expert first
Applies only to Let Em Loose work: Mill Board cards tagged `let-em-loose`, and changes under
`crates/apps/rusty_bbp_host`, `crates/libs/protocol/rusty_bbp`, `crates/apps/rusty_orch`,
`crates/foundation/rusty_crdt`, `docs/bbp`, `docs/projects/let-em-loose`, and
`crates/apps/rusty_mill_board` for #595 inbox work. Other projects in this repo are unaffected.

Threads and the orchestrator: before asking Nano any design, scope or behaviour question on that
work, consult the `let-em-loose-expert` subagent. Never ask Nano first.
- ANSWER: act on it; the source is cited.
- ESCALATE: ask Nano exactly that block: question, options, recommendation. Nothing more.
- When Nano answers, pass the answer (quoted, with its date) to `let-em-loose-expert` to record.
  Don't write `docs/projects/let-em-loose/DECISIONS.md` or remind-me decisions yourself; one
  writer keeps each answer recorded exactly once.
- Non-blocking? Take the expert's recommendation, keep going, and let the expert add the question
  to `docs/projects/let-em-loose/handoff/OPEN_QUESTIONS.md`.
