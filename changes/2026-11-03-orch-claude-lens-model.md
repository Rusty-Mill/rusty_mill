---
category: Added
changelog: orch-claude serves the lens runner's Model port; a lens question runs with no tools unless the lens is read-only.
---
## 2026-11-03 - rusty_orch: orch-claude answers lens questions

- **Added:** `ClaudeAgent` implements `orch_lens::Model` (Let Em Loose #594). After the login probe, `claude -p` runs with the question's prefix as the whole system prompt and its body on stdin, in the repository root with the usual scrubbed environment. The reply text must pass `orch_lens::parse`, or the call is `Permanent`. A lens question runs with `--tools ""` unless the agent is built with `lens_read_only()`, which gives the usual `Read,Grep,Glob`. Usage counts fresh input, cache writes and cache reads as input, cache reads as cached.
- **Changed:** the `claude -p` argv is built from one shared base, so the dispatcher's argv is unchanged.
- **Known limitations:** nothing calls it yet. `claude -p` has no output-token flag, so the runner enforces the question's output cap from the reported usage.
