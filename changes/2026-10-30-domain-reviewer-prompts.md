---
category: Added
changelog: Read-only domain reviewer prompts for rusty_conhost and rusty_mux, as Claude subagents and Codex prompts, with a Codex dispatch procedure.
---
## 2026-10-30 - Domain reviewer prompts for rusty_conhost and rusty_mux

- **Added:** `.claude/agents/conhost-expert.md` and `.claude/agents/mux-expert.md`,
  Claude subagents that review a diff in their area against the failure classes
  earlier reviews caught (for rusty_conhost: containment, retained memory,
  amplification, split boundaries, child custody; for rusty_mux: lost handshakes,
  stale connections, split streams, deadlines, ssh config, secrets), with conhost
  under ConPTY and mosh and tmux as reference sources.
- **Added:** Codex versions in `docs/agents/reviewers/`, a general reviewer prompt,
  and `codex-review-dispatch.md`, the procedure for running them with `codex exec`,
  pinned to the PR head, with a dispatch ledger. Reviews add to CI, D4 and owner
  confirmation and replace none of them; the reviewers never post, approve or merge.
- **Known limitations:** each prompt has one blind trial, on #1246 and #1251, which
  measured recall (all review findings caught) but not precision. Both were revised
  after their trial and not yet re-run.
