---
category: Added
changelog: rusty_orch: `orch_bbp::acp`, the #600 builder's ACP client and `acp.log` renderer (#1318 S3).
---
## 2026-11-25 - rusty_orch: builder ACP client (#1318 S3)

- **Added:** `orch_bbp::acp::Client::turn`: one Agent Client Protocol turn over orch's end of the fd-4 socketpair (`initialize` with no fs or terminal, `session/new` in the worktree with no MCP servers, one `session/prompt`, updates until the `stopReason`). Permission requests are answered `allow_once` (else `allow_always`, else cancelled) and logged; other agent requests are refused. Usage is `None` when unreported (D187).
- **Added:** `orch_bbp::acp::Render`, the `acp.log` renderer: agent text as it streams, tool calls, plan entries, permission answers, usage and the stop reason. A display aid, never evidence (design §6).
- **Failure:** a closed or shut-down socket, a malformed line or an error answer fails the turn; it never waits without reading.
- **Dependency:** orch-bbp now uses the in-workspace `rusty_json`, as orch-claude and orch-lens already do. No third-party dependency.
- **Not yet:** nothing calls the client; the launch sequence is S5.
