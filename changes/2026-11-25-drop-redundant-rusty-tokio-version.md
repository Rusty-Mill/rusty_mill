---
category: Fixed
changelog: Removed the redundant `version` beside `workspace = true` for `rusty_tokio` in `rusty_tls` and `rusty_h2`, which silenced Cargo's "unused manifest key" warning.
---
## 2026-11-25 - Drop redundant rusty_tokio version key

- **Fixed:** `rusty_tls` and `rusty_h2` declared `rusty_tokio = { workspace = true, version = "0.2.0" }`; Cargo ignores `version` next to `workspace = true` and warned on every build. The key is removed; resolution is unchanged.
- **Known limitations:** none.
