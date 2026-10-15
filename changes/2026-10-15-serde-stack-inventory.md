---
category: Added
changelog: Research inventory of serde, serde_json, toml and serde_yaml use across the workspace, with gaps and follow-up tickets (#685).
---
## 2026-10-15 - Serde stack inventory

- **Added:** `docs/research/SERDE-STACK-INVENTORY.md` lists the 112 workspace crates that depend on `serde`, `serde_json`, `toml` or `serde_yaml`, the API each uses, 15 gaps against `rusty_serde`, `rusty_json`, `rusty_codec::toml` and `rusty_config` with a close/keep decision, and the order of the follow-up tickets #732 to #751.
- **Known limitations:** counts come from a regex scan, not a compiler pass; `json!` counts can include `rusty_json::json!`. No code changes.
