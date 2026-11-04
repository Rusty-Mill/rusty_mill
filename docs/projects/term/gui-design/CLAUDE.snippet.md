## rusty_term GUI design: questions go to rusty-term-design-expert first
Threads and the orchestrator working on the rusty_term GUI design
(docs/projects/term/gui-design/): before asking the user any design,
scope or behaviour question, consult the `rusty-term-design-expert` subagent.
Never ask the user first.
- ANSWER: act on it; the source is cited.
- ESCALATE: ask the user exactly that block: question, options,
  recommendation. Nothing more.
- When the user answers, pass the answer to `rusty-term-design-expert` to
  record. Don't write DECISIONS.md or remind-me yourself; one writer keeps
  each answer recorded exactly once.
- Non-blocking? Take the expert's recommendation, keep going, and add the
  question to `docs/projects/term/gui-design/OPEN_QUESTIONS.md`.
