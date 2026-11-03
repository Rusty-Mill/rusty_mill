---
category: Added
changelog: orch-claude serves the lens runner's Model port, on a subscription login only; a lens question runs with no tools unless the lens is read-only.
---
## 2026-11-03 - rusty_orch: orch-claude answers lens questions

- **Added:** `ClaudeAgent` implements `orch_lens::Model` (Let Em Loose #594). Each question first proves a subscription login: `claude auth status` must exit 0 and report `authMethod` `claude.ai` or `oauth_token` (`SUBSCRIPTION_AUTH`). Any other method, an unknown one or none reported is `Unavailable`, and nothing runs. `claude -p` then runs with the question's prefix as the whole system prompt and its body on stdin, in the repository root with the scrubbed environment. A lens question runs with `--tools ""` unless the agent is built with `lens_read_only()`, which gives the usual `Read,Grep,Glob`. The reply is refused as `Permanent` if it is not UTF-8, does not pass `orch_lens::parse`, or does not report `input_tokens` and `output_tokens` as counts. Usage counts fresh input, cache writes and cache reads as input, cache reads as cached.
- **Changed:** `SCRUBBED_ENV` also removes `CLAUDE_CODE_USE_FOUNDRY`, `CLAUDE_CODE_USE_ANTHROPIC_AWS` and `CLAUDE_CODE_USE_MANTLE`, for the dispatcher too. The `claude -p` argv is built from one shared base, so the dispatcher's argv is unchanged.
- **Changed:** `orch-claude` now declares rust-version 1.89, the floor of `orch-lens` (via `rusty_bbp`).
- **Known limitations:** nothing calls it yet. `claude -p` cannot bound an agent loop's tokens before the call, so this adapter is a flat-rate subscription provider only; the lens config refuses any nonzero price (ADR-0021). A usage check after the call is an acceptance check, not a spending bound.
