# Claude interactive worker: disabled

The complete proposed configuration is an **inert template** at
`.github/automation-templates/claude.yml.disabled`; its job gates also remain
literal `false`. The live `.github/workflows/claude.yml` contains only a disabled
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

This workflow is a human-mentioned **code-writing** worker, not automatic review.
It handles newly created `@claude` issue comments and PR review comments from
human owners, members or collaborators. Bots are rejected; there is no placeholder
cross-agent identity or bot allowlist. On an issue the upstream action may create
a branch/PR; on a PR only an open same-repository `claude/*` head is supported.
The API-resolved exact head is checked out without persisted credentials.
The prefix is a routing hint, never proof of identity. The upstream action's own
authorization and current-head handling also apply. No fixed prompt is supplied.

`work.if` contains literal `false`. No repository variable, existing secret,
auto-merge setting, push or later merge enables it. Activation requires a separate
explicit owner decision and reviewed code change removing that literal. That
decision must cover subscription eligibility/terms, token lifecycle, spend and
usage limits, code/PR publication permissions and the accepted prompt-injection
risk of an interactive writer holding a write-capable token. Twenty minutes and
twenty turns bound execution; per-target queued concurrency prevents overlapping
worker requests (GitHub queue limits still apply). No activation or credential
installation is part of this correction.

Future authentication is explicit `github_token: github.token` (job-scoped
contents/PR/issues write); it bypasses the default Claude App OIDC exchange. Thus
`id-token: write` is deliberately absent. Subscription authentication uses only
`claude_code_oauth_token`, never the API-key input. GITHUB_TOKEN publication may
not trigger downstream workflows; choosing a separate App identity to change that
behavior requires another reviewed permission decision. Do not add a PAT or bot
allowlist to work around it. Review-router and Codex remain separate workflows;
their output comments cannot trigger this human-only worker.

Pinned upstream sources reviewed:
- [Action inputs](https://github.com/anthropics/claude-code-action/blob/e4b9cb6aa9618b5977f9faab7ef0b1d5688c449b/action.yml)
- [GitHub token override and default OIDC path](https://github.com/anthropics/claude-code-action/blob/e4b9cb6aa9618b5977f9faab7ef0b1d5688c449b/src/github/token.ts)
- [Official manual subscription setup](https://code.claude.com/docs/en/github-actions#manual-setup)

Non-provider validation: `node --test .github/scripts/test_claude_worker.cjs` and
`python .github/scripts/changes.py check`. Do not dispatch the workflow to test it.
