---
category: Security
changelog: rusty_agent_gateway enforces Let Em Loose lens policies (`lelPolicy`), TOML files that a token selects by `lel.lens` and its sha256 `lel.policy`, filtering tools on list and call and denying unknown lenses (Let Em Loose #1378, D223).
---
## 2026-11-27 - Gateway: TOML lens policies keyed by the token's lens claim

- **Security:** a route's `lelPolicy: {dir}` loads every `<lens>.toml` there at startup (726-sandbox-design §5). A verified token picks a policy by `lel.lens` and must name the file's sha256 as `lel.policy`. Its `[tools]` `allow` and `deny` globs over federated names filter `tools/list` and are checked again on every `tools/call`. An unknown lens, a stale digest or no token sees no tools. A lens-policy route serves no prompts or resources. Lint warns when `lelPolicy` has no `jwtAuth`. A bad policy file stops startup. Approved by Nano as part of D223.
- **Added:** `agentgateway_mcp::policy_digest`, the digest orch puts in the token. The TOML, sha256 and hex all come from first-party crates (`rusty_codec`, `rusty_sha2`, `rusty_hex`), so no third-party dependency is added.
- **Known limitations:** `[workspace]`, `[skills]` and `[grants]` are validated as tables but not enforced here. They belong to the sandbox (#1379, #1380), skill layers (#1381) and token minting (#1382). Policies load once at startup; a change needs a restart.
