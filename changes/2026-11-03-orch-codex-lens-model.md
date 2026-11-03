---
category: Added
changelog: orch-codex serves the lens runner's Model port; a lens question runs in an empty scratch directory unless the lens is read-only.
---
## 2026-11-03 - rusty_orch: orch-codex answers lens questions

- **Added:** `CodexAgent` implements `orch_lens::Model` (Let Em Loose #594). `codex exec` runs in its read-only sandbox with the usual scrubbed environment, the question's prefix and body on stdin, `--json` events on stdout and the last message to a scratch file. The reply must pass `orch_lens::parse`, or the call is `Permanent`. Usage is the sum of the `turn.completed` events, the cached part held to the input count. A lens question runs in an empty scratch directory (with `--skip-git-repo-check`) unless the agent is built with `lens_read_only()`, which runs it in the repository root.
- **Changed:** the `codex exec` argv is built from one shared base, so the dispatcher's argv is unchanged. `orch-codex` declares rust-version 1.89, the floor of `orch-lens`, and `rusty_json` moves from a dev-dependency to a dependency.
- **Known limitations:** nothing calls it yet. Codex has no tool-less mode: the empty working directory keeps the repository out of reach by default, but the read-only sandbox can still read elsewhere on the host, as it can for the dispatcher. `codex exec` has no output-token flag, so the runner enforces the question's output cap from the reported usage.
