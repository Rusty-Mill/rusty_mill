@AGENTS.md

Follow the current CONTRIBUTING workflow in AGENTS.md. The super-board robot is active under
`docs/super-board/autonomy-policy.md`; `/super-review` is part of its lanes, not of the human workflow.

## Project experts
Before asking Nano any design, scope or behaviour question, ask the expert for the paths involved.
Never ask Nano first. Most specific row first; paths not listed have no expert yet.

| Paths | Expert |
|---|---|
| `crates/libs/ui/rusty_term/src/gui/**` and GUI visual design (`docs/projects/rusty_term/gui-design/**`) | `rusty-term-design-expert`. Needs the Paper MCP server, which runs locally on Nano's Windows machine; in a cloud session it answers from its pack only and can't edit designs. Code behaviour of the GUI goes to `rusty-term-expert`. |
| `crates/libs/ui/rusty_term/**`, `crates/libs/ui/rusty_conhost/**`, `crates/apps/rusty_mux/**`, `crates/platform/rustils/crates/platform-windows/**` (condrv), `docs/projects/rusty_term/**`, `docs/projects/term/**`, `docs/projects/mux/**` | `rusty-term-expert` |

Call it with `subagent_type: "<expert>"`. **Fallback:** if that type isn't available (cloud sessions can
fail to load agents), read `.claude/agents/<expert>.md` fresh at call time and spawn a general-purpose
Agent whose prompt is the file's body (everything after the frontmatter) followed by the question. That
counts as the expert. Never answer without it, and never copy the file's text anywhere.
- ANSWER: act on it; the source is cited. An answer settles what to build, not permission: review,
  merge and owner gates still apply.
- ESCALATE: ask Nano exactly that block: question, options, recommendation. Nothing more.
- When Nano answers, pass the answer to the expert with his words and where he said them. Don't write
  the pack's DECISIONS.md or remind-me decisions yourself; one writer keeps each answer recorded once.
- Non-blocking? Take the expert's recommendation, keep going, and let the expert add the question to
  the pack's OPEN_QUESTIONS.md.
