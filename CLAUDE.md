@AGENTS.md

Follow the current CONTRIBUTING workflow in AGENTS.md. The super-board robot is active under
`docs/super-board/autonomy-policy.md`; `/super-review` is part of its lanes, not of the human workflow.

## Project experts
Before asking Nano any design, scope or behaviour question, ask the expert for the paths involved.
Never ask Nano first. Most specific row first; paths not listed have no expert yet.

| Paths | Expert |
|---|---|
| `crates/libs/ui/rusty_term/src/gui/**` and GUI visual design (`docs/projects/term/gui-design/**`) | `rusty-term-design-expert`. Needs the Paper MCP server, which runs locally on Nano's Windows machine; in a cloud session it answers from its pack only and can't edit designs. Code behaviour of the GUI goes to `rusty-term-expert`. |
| `crates/libs/ui/rusty_term/**`, `crates/libs/ui/rusty_conhost/**`, `crates/apps/rusty_mux/**`, `crates/platform/rustils/crates/platform-windows/**` (condrv), `docs/projects/rusty_term/**`, `docs/projects/term/**`, `docs/projects/mux/**` | `rusty-term-expert` |
| `crates/apps/rusty_bbp_host/**`, `crates/libs/protocol/rusty_bbp/**`, `crates/apps/rusty_orch/**`, `crates/foundation/rusty_crdt/**`, `docs/bbp/**`, `docs/projects/let-em-loose/**`, `crates/apps/rusty_mill_board/**` for #595 inbox work, and any Mill Board card tagged `let-em-loose` | `let-em-loose-expert`. It records Nano's answers only with a reference to where he said them (message link or id, card comment, or remind-me id), and its open questions go to `docs/projects/let-em-loose/handoff/OPEN_QUESTIONS.md`. Every Let Em Loose design is written to `docs/projects/let-em-loose/` and committed in its first draft PR, never only to `/mnt/project-files/` or session scratch (Nano, 2026-11-04, D94). |
| `docs/projects/ai-env-manager/**`, `library/**` (design changes; adding an entry needs no expert), and `crates/apps/ai_env_manager/**` once it exists | `ai-env-manager-expert`. `library/` is Ai Env Manager's domain model and is absorbed over time (pack D14). |
| `docs/projects/millwright/**` and any Mill Board card tagged `millwright`, for programme questions only (phase, gate, sequencing, which milestone, what waits). A system's design still goes to its row above. | `millwright-expert` |

Call it with `subagent_type: "<expert>"`. **Fallback:** if that type isn't available (cloud sessions can
fail to load agents), read `.claude/agents/<expert>.md` fresh at call time and spawn a general-purpose
Agent whose prompt is the file's body (everything after the frontmatter) followed by the question. That
counts as the expert. Never answer without it, and never copy the file's text anywhere.
**Missing expert:** if `.claude/agents/<expert>.md` doesn't exist either, use the next row that matches
the paths. Until `rusty-term-design-expert.md` lands, GUI visual-design questions go to
`rusty-term-expert`, which escalates them to Nano rather than answering (its pack, D10).
- ANSWER: act on it; the source is cited. An answer settles what to build, not permission: review,
  merge and owner gates still apply.
- ESCALATE: ask Nano exactly that block: question, options, recommendation. Nothing more.
- When Nano answers, pass the answer to the expert with his words and where he said them. Don't write
  the pack's DECISIONS.md or remind-me decisions yourself; one writer keeps each answer recorded once.
- Non-blocking? Take the expert's recommendation, keep going, and let the expert add the question to
  the pack's OPEN_QUESTIONS.md.

## Memory: rusty-remind-me

Use RemindMe for relevant prior decisions and context when it is available through
configured connections. Cross-surface access depends on those connections and any
opt-in sync; this file does not enable sharing. Current repository instructions,
source and review/merge safeguards take precedence over memory. Note conflicting
or stale entries rather than treating them as authority.

### Search first

- Before work about Nano, this repository, a past decision or a named crate, PR or
  issue, search available RemindMe tools with the relevant project, crate or ID.
- If the tools are not visible, search for `remind_me` tools when tool discovery is
  available. If the connection or tools remain unavailable, continue unrelated
  work using repository evidence and report the missing context or unsaved checkpoint.

### Save relevant decisions and facts

- When a relevant durable fact or decision is learned, save a terse entry with
  `remind_me_add` if available and permitted by the user's instructions. Use one
  fact per entry; set category (`fact` or `preference`), `memory_type: decision`
  for decisions, tags (project, crate, PR/issue IDs), entities and
  `metadata.rationale` explaining why.
- Include relevant PR activity (opened, merged, blocked and why), subject to the
  same privacy and availability rules. Verify the saved result before reporting
  a checkpoint as recorded.

### Capture at the end

- For a substantive session, use `remind_me_auto_capture` when available and
  permitted, with a descriptive title, tags, available relevant dialogue and a
  terse summary of topics, decisions, facts, action items and PR activity.
- Honor explicit do-not-save requests. Exclude secrets, tokens, credentials and
  unrelated sensitive personal information from all memory writes. Do not rely
  on credential scrubbing to remove other sensitive details. Never reconstruct
  unavailable dialogue or submit a blanket full conversation.
- For any mutating memory call, including capture, verify the saved result. If
  a timeout or connection failure leaves the outcome uncertain, check for the
  saved entry before retrying. Retry once only when the write is known not to
  have succeeded and the failure has been addressed. If success cannot be
  determined, report the uncertainty and do not risk a duplicate write.
- If saving is unavailable, continue unrelated work and report the unsaved
  checkpoint without claiming it was captured.
