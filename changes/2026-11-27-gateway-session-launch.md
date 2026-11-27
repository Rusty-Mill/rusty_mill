---
category: Added
changelog: rusty_agent_gateway's stdio targets take a `session` launch whose cmd, args and env are filled from the verified token, so each per_session child can start in its session's jail (Let Em Loose #1379, D226).
---
## 2026-11-27 - Gateway: per-session stdio launch from the token

- **Added:** a `stdio` target takes `session: { cmd, args, env }`. Each value is a `{{jwt.<claim>[:n]}}` template over the verified token, as in `identity`. Each `per_session` child starts from it: for example `lel-jail /srv/lel/{{jwt.lel.claim}}/jail.json`, so orch-fs-shell runs inside that session's jail. The target's own `cmd`, `args` and `env` still start the probe child used at startup and for listings.
- **Security:** a claim value must be `[A-Za-z0-9._-]+` and not `.` or `..`, so it can name a path segment but never leave one. A missing or disallowed claim refuses the call and starts nothing. Claims that fill the launch differently get different children. Nothing in a launch comes from the call.
- **Known limitations:** `per_session` targets only: a `session` launch anywhere else stops startup. Listings come from the probe child, so it must offer the same tools as a session's child.
