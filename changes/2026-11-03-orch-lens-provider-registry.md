---
category: Added
changelog: rusty_orch builds the lens runner's provider registry from the lens config, with operator assertions that default off.
---
## 2026-11-03 - rusty_orch: lens provider registry

- **Added:** `rusty_orch::lens::registry` (Let Em Loose #594, design §2). `Registry::build` makes one `orch_lens::Model` per configured provider: `claude-cli` (`model`, `timeout_secs`), `codex-cli` (`model`, `timeout_secs`) and `openai-http` (`base_url`, `model`, `api_key_env`, `timeout_secs`, `json_mode`; 300 s when no timeout is given). Any other setting, an empty string, a `timeout_secs` outside 1 to 86400, `https` or a bad key variable name is refused at load as `providers.<id>.<field>`. Claude gets a tool-less adapter and a read-only one, picked by the lens's `tools`; Codex and the HTTP adapter serve both.
- **Added:** `Assertions { no_overage, host_cleared }`, both false by default. They are passed to the Claude and Codex adapters, which stay `Unavailable` and run nothing without them. Building a registry makes no call.
- **Fixed:** `orch-openai` builds its call deadline with a checked add, so a timeout too large for a deadline is `Permanent` before connecting instead of overflowing.
- **Known limitations:** nothing builds a registry yet; the `rusty_orch lens` subcommand that reads the config and the assertions comes in a later slice.
