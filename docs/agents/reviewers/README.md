# Domain reviewer prompts

Read-only PR reviewers for two areas where review needs rare expertise. Each one
checks a diff against the failure classes that earlier reviews of its area caught,
cites primary sources, and returns findings with a trace, a fix and a regression
test, plus the checks it ran and a verdict.

| Area | Claude subagent | Codex prompt |
|---|---|---|
| rusty_conhost, the Windows console host | [`.claude/agents/conhost-expert.md`](../../../.claude/agents/conhost-expert.md) | [`conhost-expert-codex.md`](conhost-expert-codex.md) |
| rusty_mux, the terminal multiplexer for agents | [`.claude/agents/mux-expert.md`](../../../.claude/agents/mux-expert.md) | [`mux-expert-codex.md`](mux-expert-codex.md) |
| anything else | — | [`general-reviewer-codex.md`](general-reviewer-codex.md) |

[`codex-review-dispatch.md`](codex-review-dispatch.md) is the procedure for running
the Codex prompts on a PR with `codex exec`: routing by changed paths, SHA pinning,
result checks and a dispatch ledger. A review adds to required CI, the D4 Windows
run and owner confirmation; it replaces none of them, and the reviewers never post,
approve or merge.

## Trials

Each prompt was run blind on the first reviewed head of a past PR, with no access to
later commits or review comments, and scored against that PR's review.

| Reviewer | PR and head | Recall | Precision |
|---|---|---|---|
| conhost | #1246 at `197fae02` | 3 of 3 review findings (one in part) | not measured; 16 further findings untriaged, one confirmed on `main` as #1256 |
| mux | #1251 at `55d4bbf9` | 2 of 2 review findings | not measured; 3 further findings untriaged |

Both prompts were revised from the trials (severity, scope, running new tests at the
base SHA) and have not been re-run since.
