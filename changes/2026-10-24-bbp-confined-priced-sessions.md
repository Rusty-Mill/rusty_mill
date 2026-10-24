---
category: Added
changelog: `bbp mod` session launchers can now run confined and under a cost cap, and `claude-bbp-harness` resumes the role's conversation.
---
## 2026-10-24 - BBP confined and priced sessions

- **Added:** a `"session": true` launcher with a `sandbox` keeps one home per session (`<work>/turns/<sha256(task)>/session-<id>.home`) across its turns, and gets a fresh workspace or worktree each turn at the session's one path (`session-<id>`), so a provider that keys conversations by directory can resume. Step 3c of `docs/bbp-event-based-plan.md`.
- **Added:** session launchers run under a cost cap. A turn launched to resume is recorded as `agents/<sha256(task)>/turn-N.resumed`; since Claude Code restores a resumed session's totals into `modelUsage` and `total_cost_usd`, such a turn is priced from each result's `model_usage_since_last_result`, cross-checked against its `costUSD`. A resumed turn without that field, or with a gap in `result_index`, has an unknown cost, never a running total.
- **Added:** `claude-bbp-harness` with `BBP_SESSION_ID` starts the conversation with `--session-id` and resumes it with `--resume` (instead of `--no-session-persistence`), in one working directory emptied each turn.
- **Changed:** `codex-bbp-harness` refuses a session: Codex chooses its own session ids.
- **Known limitations:** validated with scripted agents and provider stubs; no live-model run. The resumed-turn fields were read from Claude Code 2.1.310's own result schema, not from a captured resumed run. A session's home is writable by its harness, so a turn sees what the role's earlier turns left there.
