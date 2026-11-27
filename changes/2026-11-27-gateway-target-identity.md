---
category: Security
changelog: rusty_agent_gateway stamps each MCP call's identity from the verified token through a per-target `identity` block, stripping agent-supplied actors (Let Em Loose #1376, D223).
---
## 2026-11-27 - Gateway: caller identity upstream from the token

- **Security:** an `mcp` target can carry `identity` {`strip`, `set`, `headers`}. On every `tools/call` the gateway strips the arguments an agent could use to name itself (per tool glob), then sets arguments and headers from `{{jwt.<claim>}}` templates over the verified token. It runs after the guardrail chain. A call whose claims can't fill a template is refused, not sent anonymous. This is how Mill Board's actor becomes `lel-gateway/<lens>-<claim>` for jailed agents (726-sandbox-design §3). Approved by Nano as part of D223.
- **Changed:** the `mock_mcp_server` fixture echoes its arguments when `MOCK_ECHO_ARGS` is set.
- **Known limitations:** `identity` applies to `tools/call` only, not to prompts, resources or listings. remind-me writes stay closed (D184). The `lel-gateway` Mill Board credential and deployment are separate ask-first items.
