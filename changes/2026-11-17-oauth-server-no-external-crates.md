---
category: Changed
changelog: rusty_oauth_server no longer depends on serde or idna; its wire types and state file are read and written by hand on rusty_json.
---
## 2026-11-17 - rusty_oauth_server drops serde and idna

- **Changed:** `rusty_oauth_server` now depends only on first-party crates. `rusty_json` is used without its `serde` feature and `rusty_url` (which pulls `idna`) is replaced by `rusty_percent` for form decoding and query encoding. `ClientMetadata`, `ClientInformation` and `TokenResponse` read and write JSON through `from_value`/`from_slice`/`to_value`; the state file keeps its layout, so existing files load unchanged.
- **Changed:** `ClientSummary`, `RevokeSummary`, `TokenCounts` and the wire types no longer implement `Serialize`/`Deserialize`. `rusty_mill_board` renders the two summaries itself, with the same fields.
- **Known limitations:** the key order of the state file and of `/register` responses may differ from before; no reader depends on it.
