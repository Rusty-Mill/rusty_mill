# Review router: disabled

The complete proposed configuration is an **inert template** at
`.github/automation-templates/review-router.yml.disabled`; its job gates also remain
literal `false`. The live `.github/workflows/review-router.yml` contains only a disabled
hold job with no action or secret references. Current repository policy does not
allow either `openai/codex-action` or `anthropics/claude-code-action`, and GitHub
validates that policy even for skipped jobs. The initial disabled review-router
run failed at startup for precisely that reason; no generation job started.

Activation therefore also needs a **separate explicit decision on the action
allowlist**, followed by a reviewed code change installing the template in the
live workflow path and reviewing/removing its literal holds. No repository rule
was changed here. Do not copy the template into a live workflow or allowlist a
provider as a routine correction. The design below describes the inert template,
not an active service. Existing secrets, variables and auto-merge cannot install it.

Every job has a literal `false` activation gate. Pushes, merges, existing secrets
and repository variables cannot enable it. A separate explicit activation decision
and reviewed code change must approve provider costs/limits, subscription terms,
runtime dependency versions, credential lifecycle and review-publication scope.
No credentials were installed, inspected or changed during this correction.

Only opened/synchronize/ready-for-review PR events are supported. Drafts and fork
heads are skipped before generation. There are no mention triggers, bot bypasses,
`pull_request_target`, automatic approvals or code-writing steps. Both reviewers
run regardless of branch name: `claude/*` and `codex/*` are routing hints and cannot
authenticate an author. Future self-review suppression needs verified provider
identities and a separate routing change; no account is guessed here.

Generation jobs have only contents/PR read permissions, no PAT, no persisted
checkout credentials, no write token and no OIDC exchange. The exact PR head and
base SHAs produce an immutable diff before generation. Codex uses `:read-only`
plus `drop-sudo` on a fresh GitHub-hosted Linux runner, with its home outside the
untrusted checkout. Claude uses an explicit **read-only** `github_token` override,
so it does not need `id-token: write`; its default App/OIDC path would otherwise
request write-capable contents/PR/issues access. Its only CLI tools are Read,
Glob and Grep, with shell/edit/network/all MCP tools denied, hooks disabled and
project/local settings excluded. This tool boundary
is narrower than prompting for findings; it is not a general OS sandbox or a
claim that the provider token is inaccessible to the action process. No repository
commands, builds, tests or hooks execute during review generation.

The only outputs are Codex `final-message` and Claude `structured_output` with a
single `review` field. These are untrusted text. The clean publisher checks out
only trusted base-SHA scripts, validates shape/size/control characters, neutralizes
mentions and refreshes repository identity, open/draft state and both exact SHAs.
It posts a COMMENT review anchored to the reviewed commit, never an approval or an
instruction to another worker. A racing push after the check leaves an old-commit
review rather than silently attaching findings to a different commit. Publication
has only contents read and PR write using ephemeral GITHUB_TOKEN. No PAT is needed.
GitHub job-output limits apply; missing, oversized or secret-redacted outputs fail
closed. No output is evaluated as code or passed to a shell.

Generation is bounded to 15 minutes per reviewer and ten Claude turns; publication
to five minutes. New PR heads cancel stale reviews. Separate clean jobs implement
the upstream isolation guidance; never move publication onto a generation runner.

Reviewed official sources (pins are immutable):
- [Codex inputs and final-message output](https://github.com/openai/codex-action/blob/1d82a3dba7c7988ec71ecae1ed5b8e8db96495c5/action.yml)
- [Codex sandbox and clean-job security guidance](https://github.com/openai/codex-action/blob/1d82a3dba7c7988ec71ecae1ed5b8e8db96495c5/docs/security.md)
- [Claude tools/structured output/auth inputs](https://github.com/anthropics/claude-code-action/blob/e4b9cb6aa9618b5977f9faab7ef0b1d5688c449b/action.yml)
- [Claude GitHub token override and OIDC permissions](https://github.com/anthropics/claude-code-action/blob/e4b9cb6aa9618b5977f9faab7ef0b1d5688c449b/src/github/token.ts)

Validate without providers: `node --test .github/scripts/test_review_router.cjs`,
`python .github/scripts/changes.py check` and workflow lint. Runtime/provider smoke
testing is deferred until explicit activation approval.
