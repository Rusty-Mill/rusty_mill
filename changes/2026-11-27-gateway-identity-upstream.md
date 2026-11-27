---
category: Security
changelog: rusty_agent_gateway stamps the caller's identity from the verified token on every MCP call to a target with an `identity` block, stripping caller-supplied actor arguments and failing closed (Let Em Loose #1376).
---
## 2026-11-27 - Gateway stamps the caller's identity upstream

- **Security:** an MCP target can carry an `identity` block (`strip`, `set`, `headers`). On every `tools/call` the gateway removes the named arguments, then sets arguments and HTTP headers from `{{jwt.<claim>}}` templates over the caller's verified token. This runs after every gate and guardrail, so neither the caller nor a processor can change it. A call with no token, or a token missing a needed claim, is refused rather than sent unstamped. For Mill Board this means `actor` comes from the token (`lel-gateway/<label>`), never from the agent (design D180 §3, card #1376).
- **Fixed:** follow-ups from the #1877 review in `rusty_mcp_server`: the session owner is computed once per request, the `iss`-less key is documented, and a test covers the GET stream under the wrong principal.
- **Known limitations:** remind-me writes stay closed to jailed agents until rusty-remind-me accepts a trusted identity (D184). Per-session upstreams (#1377) and resume-stream binding (#1625) are separate cards.
