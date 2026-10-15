# Serde stack inventory

Read-only inventory, 2026-10-15, base `316df938a`. Question: which workspace
crates use `serde`, `serde_json`, `toml` and `serde_yaml`, which API each uses,
and what `rusty_serde`, `rusty_json`, `rusty_codec::toml` and `rusty_config`
lack before those crates can switch. Issue #685. No code changes.

## 1. Totals

112 workspace crates depend directly on at least one of the four (normal, dev
or build). Frozen Nexus is outside the workspace and not counted.

| Dependency | Crates | In-house replacement |
|---|---|---|
| `serde_json` | 102 | `rusty_json` |
| `serde` | 68 | `rusty_serde` |
| `toml` | 9 | `rusty_codec::toml` (`rusty_config` is INI/KV only) |
| `serde_yaml` | 3 | none |

The ticket counted 104 / 69. This count resolves workspace inheritance through
`cargo metadata` and counts one entry per package, which explains the gap.

Most used API, by crates that use it (calls):

| API | Crates | Uses |
|---|---|---|
| `json!` | 77 | 3,673 |
| `serde_json::Value` | 75 | 673 |
| `serde_json::from_str` | 66 | 407 |
| `#[derive(Deserialize)]` / `#[derive(Serialize)]` | 58 / 54 | 1,336 / 1,277 |
| `serde_json::to_string` / `to_string_pretty` | 41 / 16 | 116 / 102 |
| `serde_json::from_slice` / `to_vec` / `to_vec_pretty` | 30 / 22 / 6 | 121 / 47 / 14 |
| `serde_json::to_value` / `from_value` | 28 / 25 | 300 / 232 |
| `serde_json::Map` / `Number` | 26 / 5 | 64 / 11 |
| `serde::de::DeserializeOwned` | 20 | 44 |
| `serde::Deserializer` / `serde::Serializer` in hand-written code | 13 / 8 | 29 / 17 |
| `toml::from_str` | 9 | 13 |
| `serde_yaml::from_str` / `from_value` / `to_string` | 3 | 12 |

`#[serde(...)]` appears 2,582 times in 51 crates. Keys used: `default`,
`rename_all`, `skip_serializing_if`, `rename`, `flatten`, `tag`,
`deserialize_with`, `untagged`, `transparent`, `deny_unknown_fields`, `skip`,
`content`, `with`, `serialize_with`, `skip_deserializing`, `try_from`, `into`,
`bound`. `serde_json` is built with `float_roundtrip` in 69 crates and
`raw_value` in 1.

## 2. Gaps

Each row is a difference between the external API in use and the in-house
crate. "Close" says whether to close it in-house.

| # | Gap | Evidence | Close | Ticket |
|---|---|---|---|---|
| 1 | `rusty_json`'s typed API (`from_str`, `to_string`, `to_value`, `from_value`) is built on external `serde`, so swapping `serde_json` for `rusty_json` keeps `serde` | `rusty_json/Cargo.toml:20,28,31`; `src/de.rs:10` | yes: add a `rusty_serde` backend | #732 |
| 2 | `rusty_serde::json` has only `from_str`, `from_value`, `to_string`, `to_value`: no pretty printer, `to_vec`, `from_slice`, reader/writer, `Map`, `Number`, `json!` | `rusty_serde/src/json/mod.rs:11-14` | no: gap 1 makes `rusty_json` the JSON format for `rusty_serde` | — |
| 3 | `rusty_serde` lacks impls for `Arc`, `Rc`, `HashSet`, `BTreeSet`, `VecDeque`, `[T; N]`, `PathBuf`, `Duration`, `u128`/`i128`, and has no `DeserializeOwned` (20 crates use it) | `rusty_serde/src/impls.rs`, `src/de.rs` | yes | #733 |
| 4 | `uuid` (10 crates) and `chrono` (6) values serialize through those crates' `serde` feature | per-crate table, Extra column | yes: optional features in `rusty_serde` | #734 |
| 5 | `postgres` with its serde feature (`remind_me_hub`) | `remind_me_hub` manifest | no: keep at the boundary | — |
| 6 | `rusty_serde_derive` reads `#[rusty_serde(...)]` only; every `#[serde(...)]` would need rewriting | `rusty_serde_derive/src/lib.rs:27,32`; `src/parse.rs:1207` | yes: accept `#[serde(...)]` too | #735 |
| 7 | derive attribute keys | `parse.rs:1253-1817` parses every key in section 1 | no gap | — |
| 8 | custom (de)serializer code: `Visitor`, `MapAccess`, `SerializeMap`, `IgnoredAny`, `de::Error::custom`, `forward_to_deserialize_any!` | `rusty_serde/src/de.rs:184,338,421,519`; `src/ser.rs:238`; `src/error.rs:10` | no gap: all exist | — |
| 9 | `serde_json::value::RawValue` (`rusty-croc`, 1 use) | per-crate table | no: use `Value` or `String` at the call site | — |
| 10 | `serde_json::Deserializer::from_str` streaming (`rusty_json` tests only) | `rusty_json` exports `StreamDeserializer` | no gap | — |
| 11 | `float_roundtrip` | `rusty_json/src/parser.rs:141,164` parse with `str::parse::<f64>`, which is correctly rounded | no gap | — |
| 12 | `rusty_codec::toml` has no typed `from_str::<T>`, no writer, no `[[array.of.tables]]`, no multi-line strings; `rusty_provider` config uses `[[clients]]`, `rp-cli` writes TOML back | `rusty_codec/src/toml.rs:10-13`; `rp-cli/src/setup.rs:122-208` | yes | #736 |
| 13 | `rusty_config` parses INI/KV only, 0 dependents | `rusty_config/src/lib.rs:1-4` | no: TOML belongs in `rusty_codec`, not here | — |
| 14 | no in-house YAML (`agentgateway-config`, `skillopt-core`, `skillopt-envs`); `serde_yaml` is archived upstream | root `Cargo.toml:653,966` | no: keep `serde_yaml`, or move to the `serde_norway` fork the workspace already uses | — |
| 15 | third-party APIs that take `serde` types: `axum` `Json`, `reqwest` `.json()`, `rmcp`, `jsonwebtoken`, `schemars`, `bincode`, `serde_norway`, `tracing-subscriber` JSON | Extra column | no: keep `serde` on types that cross these boundaries until the framework itself is replaced | — |

## 3. Order

1. Gap tickets #732 to #736 have no blockers and can run in parallel.
2. #728 moves `rusty_multimodal_db_engine` to `rusty_serde`. It needs #732,
   #733 and #734 (`uuid`) in practice; its own ticket lists none.
3. #729 (`rusty_tick` pilot) follows #728.
4. The family tickets below follow #732, #733 and #735. Those with `uuid` or
   `chrono` also follow #734, those with `toml` follow #736, and every crate
   marked `engine` follows #728.

| Ticket | Family | Crates | After |
|---|---|---|---|
| #737 | rocket_league | 12 | #728, #734 |
| #738 | agent_gateway | 8 | — |
| #739 | rusty_key | 9 | #736 |
| #740 | rusty_provider | 6 | #736 |
| #741 | remind_me | 6 | #728, #734 |
| #742 | skillopt | 4 | — |
| #743 | tailscale | 5 | — |
| #744 | yirp | 6 | #736 |
| #745 | inventory | 3 | — |
| #746 | engine apps | 6 | #728, #734 |
| #747 | protocol libs | 8 | #734, #736 |
| #748 | rusty_adk | 16 | — |
| #749 | rusty-db/search | 10 | #734 |
| #750 | homelab | 5 | #736 |
| #751 | small crates | 5 | #734 |

`rusty_json` keeps `serde_json` as a dev-dependency test oracle; #732 lets it
build without `serde`. Families over 400 changed lines are split by crate
when built.

## 4. Per-crate table

Deps: `s` serde, `sj` serde_json, `t` toml, `y` serde_yaml, `(dev)` dev only.
Files: source files that name one of the crates or derive its traits. API:
the six most used items with call counts (`s::`, `sj::`, `t::`, `y::`
prefixes). Extra: `engine` (depends on `rusty_multimodal_db_engine`), types
whose `serde` feature is on, and boundary crates from gap 15.

Method: `cargo metadata --no-deps` for direct dependencies, then a regex scan
of each package's `.rs` files (nested packages skipped). Counts of `json!` can
include `rusty_json::json!` in crates that depend on both.

| Crate | Path | Deps | Files | API used (top 6, count) | `#[serde]` keys | Extra | Ticket |
|---|---|---|---|---|---|---|---|
| `bc-clone` | `apps/rocket_league/crates/bc-clone` | s sj | 5 | derive(Serialize)×10, derive(Deserialize)×7, sj::Value×3, sj::to_value×3, sj::json!×3, sj::Map×1, +5 | skip_serializing_if | — | #737 |
| `rb_capture_ingest` | `apps/rocket_league/crates/rb_capture_ingest` | s sj | 1 | derive(Deserialize)×6, s::Deserialize×1, sj::Error×1, sj::from_str×1 | default | — | #737 |
| `rb_env` | `apps/rocket_league/crates/rb_env` | sj(dev) | 1 | sj::Value×4, sj::json!×2, sj::from_str×1 | — | — | #737 |
| `rb_scenario` | `apps/rocket_league/crates/rb_scenario` | s sj | 1 | derive(Serialize)×6, derive(Deserialize)×6, sj::Error×2, s::Deserialize×1, s::Serialize×1, sj::from_str×1, +1 | skip_serializing_if, default, flatten | — | #737 |
| `recon-check` | `apps/rocket_league/crates/recon-check` | s sj | 2 | derive(Serialize)×4, derive(Deserialize)×4, s::Deserialize×1, s::Serialize×1, sj::Value×1, sj::from_str×1 | rename_all | — | #737 |
| `replay-analyzer` | `apps/rocket_league/crates/replay-analyzer` | s sj | 8 | derive(Serialize)×31, derive(Deserialize)×31, s::Deserialize×6, s::Serialize×4, sj::from_slice×4, sj::to_vec_pretty×2, +1 | default, rename_all, tag | — | #737 |
| `replay-pacifist` | `apps/rocket_league/crates/replay-pacifist` | s sj | 2 | sj::from_slice×3, s::Deserialize×2, derive(Deserialize)×2 | default | — | #737 |
| `replay-scoring` | `apps/rocket_league/crates/replay-scoring` | s sj | 19 | derive(Serialize)×34, derive(Deserialize)×32, sj::from_slice×14, s::Serialize×10, s::Deserialize×10, sj::from_str×6, +6 | default, rename_all, tag, skip_serializing_if, deserialize_with | — | #737 |
| `replay-skills` | `apps/rocket_league/crates/replay-skills` | s sj | 10 | derive(Serialize)×14, derive(Deserialize)×13, s::Deserialize×7, s::Serialize×7, sj::from_slice×4, sj::to_vec_pretty×2, +2 | default, rename_all | — | #737 |
| `replay-value` | `apps/rocket_league/crates/replay-value` | s sj | 8 | derive(Serialize)×9, derive(Deserialize)×7, s::Serialize×5, s::Deserialize×4, sj::from_slice×3, sj::to_vec_pretty×2, +3 | default, tag, content, rename_all | — | #737 |
| `replay-viewer` | `apps/rocket_league/crates/replay-viewer` | s sj | 4 | derive(Serialize)×9, derive(Deserialize)×8, s::Serialize×2, sj::to_vec_pretty×1, sj::to_string×1, s::Deserialize×1, +1 | default, skip_serializing_if | — | #737 |
| `rleval-app` | `apps/rocket_league/crates/rleval-app` | s sj | 12 | derive(Serialize)×41, sj::Value×12, sj::from_str×11, s::Serialize×10, sj::from_slice×8, derive(Deserialize)×6, +5 | default, skip_serializing_if, rename_all | engine, uuid | #737 |
| `agentgateway` | `apps/rusty_agent_gateway/crates/agentgateway` | sj(dev) | 13 | sj::json!×117, sj::from_str×10, sj::Value×8, sj::from_slice×6, sj::Map::new×3, sj::Value::String×2, +3 | — | axum, tracing-subscriber | #738 |
| `agentgateway-a2a` | `apps/rusty_agent_gateway/crates/agentgateway-a2a` | sj | 2 | sj::from_slice×4, sj::to_value×3, sj::Value×2, sj::from_value×2, sj::json!×2, sj::to_vec×2, +1 | — | reqwest | #738 |
| `agentgateway-agui` | `apps/rusty_agent_gateway/crates/agentgateway-agui` | sj | 1 | sj::Value×5, sj::json!×5, sj::Value::String×2, sj::Value::as_str×1, sj::from_str×1, sj::Value::Null×1, +1 | — | — | #738 |
| `agentgateway-auth` | `apps/rusty_agent_gateway/crates/agentgateway-auth` | sj | 2 | sj::json!×9, sj::Error×1, sj::from_str×1, sj::Value×1 | — | jsonwebtoken, reqwest | #738 |
| `agentgateway-config` | `apps/rusty_agent_gateway/crates/agentgateway-config` | s sj y | 7 | derive(Deserialize)×68, derive(Serialize)×66, sj::Value×11, s::Deserialize×6, s::Serialize×6, s::Deserializer×3, +10 | default, skip_serializing_if, rename_all, flatten, deserialize_with | — | #738 |
| `agentgateway-llm` | `apps/rusty_agent_gateway/crates/agentgateway-llm` | sj | 20 | sj::json!×291, sj::Value×12, sj::Map×5, sj::from_str×4, sj::to_string×3, sj::from_slice×2, +1 | — | reqwest | #738 |
| `agentgateway-mcp` | `apps/rusty_agent_gateway/crates/agentgateway-mcp` | sj | 8 | sj::json!×44, sj::Value×22, sj::Value::String×8, sj::Value::Null×5, sj::Value::Object×4, sj::Value::Bool×3, +7 | — | — | #738 |
| `agentgateway-proxy` | `apps/rusty_agent_gateway/crates/agentgateway-proxy` | sj | 1 | sj::json!×1 | — | — | #738 |
| `rusty-croc` | `apps/rusty_croc` | s sj | 4 | sj::from_slice×8, sj::to_vec×6, derive(Serialize)×6, derive(Deserialize)×6, sj::to_string×5, sj::from_str×5, +9 | rename, default, skip_serializing_if, serialize_with, deserialize_with | — | #751 |
| `rusty_fair_play` | `apps/rusty_fair_play` | s | 3 | derive(Serialize)×16, derive(Deserialize)×9, s::Deserialize×3, s::Serialize×3, s::Deserializer×1 | default, rename_all, deny_unknown_fields, deserialize_with, skip_serializing_if | engine, uuid | #746 |
| `rusty_fedora_agent` | `apps/rusty_fedora_agent` | s sj t | 3 | derive(Deserialize)×14, derive(Serialize)×10, sj::json!×6, s::Deserialize×5, t::from_str×2, s::Serialize×2, +5 | default, rename_all | — | #750 |
| `rusty-hister-extractor` | `apps/rusty_hister/crates/rusty-hister-extractor` | s | 2 | derive(Deserialize)×5, s::Deserialize×3 | default | chrono | #751 |
| `rusty-hister-model` | `apps/rusty_hister/crates/rusty-hister-model` | sj | 2 | sj::json!×32 | — | — | #751 |
| `rusty_homelab_mcp` | `apps/rusty_homelab_mcp` | s sj t | 7 | derive(Deserialize)×42, sj::json!×20, sj::Value×19, s::Deserialize×5, sj::from_str×3, s::Serialize×2, +5 | default, rename_all | schemars, tracing-subscriber | #750 |
| `inventory-cli` | `apps/rusty_inventory/crates/inventory-cli` | sj | 1 | sj::to_string_pretty×1 | — | — | #745 |
| `inventory-core` | `apps/rusty_inventory/crates/inventory-core` | s sj | 10 | sj::json!×24, derive(Serialize)×12, derive(Deserialize)×12, sj::Value×7, sj::from_str×4, sj::Value::String×3, +7 | rename_all, default | — | #745 |
| `inventory-tauri` | `apps/rusty_inventory/crates/inventory-tauri` | s | 1 | derive(Serialize)×7, s::Serialize×1 | — | — | #745 |
| `rk-agui` | `apps/rusty_key/crates/agui` | s sj | 2 | sj::json!×9, sj::Value×2, s::Serialize×1, sj::from_str×1 | — | — | #739 |
| `rk-app` | `apps/rusty_key/crates/app` | s sj | 15 | sj::json!×108, sj::Value×12, sj::from_str×7, sj::to_value×3, sj::to_string×2, derive(Serialize)×2, +1 | rename_all | axum | #739 |
| `rk-compose` | `apps/rusty_key/crates/compose` | s sj t | 9 | derive(Serialize)×17, sj::json!×12, derive(Deserialize)×8, sj::to_value×8, s::Deserialize×5, s::Serialize×5, +9 | default, rename_all, rename | — | #739 |
| `rk-config` | `apps/rusty_key/crates/config` | s t | 1 | s::Deserialize×3, derive(Deserialize)×3, t::from_str×2, s::Serialize×1, derive(Serialize)×1 | rename_all, default | — | #739 |
| `rk-constrain` | `apps/rusty_key/crates/constrain` | sj | 4 | sj::json!×29, sj::Value×5, sj::from_str×1 | — | — | #739 |
| `rk-feed` | `apps/rusty_key/crates/feed` | s sj | 15 | sj::json!×43, derive(Deserialize)×13, sj::Value×12, derive(Serialize)×8, s::Deserialize×6, s::Serialize×4, +5 | default, rename_all, rename, skip_serializing_if | reqwest | #739 |
| `rk-kernel` | `apps/rusty_key/crates/kernel` | sj | 3 | sj::json!×6, sj::Value×3, sj::from_value×1 | — | — | #739 |
| `rk-mcp` | `apps/rusty_key/crates/mcp` | s sj t | 6 | sj::Value×5, sj::json!×5, derive(Deserialize)×4, t::from_str×2, s::Deserialize×1, sj::from_str×1, +1 | default, rename_all | — | #739 |
| `rk-observe` | `apps/rusty_key/crates/observe` | s sj | 8 | sj::json!×20, derive(Serialize)×14, derive(Deserialize)×14, s::Deserialize×6, s::Serialize×6, sj::Value×4, +5 | rename_all | — | #739 |
| `rusty_mill_board` | `apps/rusty_mill_board` | s sj | 10 | sj::json!×52, derive(Deserialize)×24, sj::Value×10, derive(Serialize)×10, sj::from_str×10, s::Deserialize×5, +7 | flatten, rename_all, default, deny_unknown_fields | engine, uuid, schemars, tracing-subscriber | #746 |
| `rusty_mill_runner` | `apps/rusty_mill_board/runner` | s sj | 2 | sj::json!×9, sj::from_value×4, sj::from_str×2, s::Deserialize×1, derive(Deserialize)×1, sj::Value×1 | default, deny_unknown_fields | engine, uuid, tracing-subscriber | #746 |
| `rusty_multimodal_db` | `apps/rusty_multimodal_db` | s | 15 | derive(Serialize)×40, derive(Deserialize)×39, s::Serialize×19, s::Deserialize×14, s::de::DeserializeOwned×5 | — | engine, uuid, bincode, rusqlite | #746 |
| `orch-store` | `apps/rusty_orch/crates/orch-store` | s | 1 | derive(Serialize)×12, derive(Deserialize)×12, s::Deserialize×1, s::Serialize×1 | — | engine | #746 |
| `rp-cli` | `apps/rusty_provider/crates/cli` | s sj t | 1 | sj::Value×6, t::Value×5, sj::from_str×4, derive(Deserialize)×4, sj::Value::Object×2, t::Value::Table×2, +6 | default, rename_all | — | #740 |
| `rp-core` | `apps/rusty_provider/crates/core` | s sj | 1 | derive(Serialize)×35, derive(Deserialize)×35, sj::to_value×11, sj::json!×10, sj::from_str×7, sj::Value×3, +4 | default, skip_serializing_if, skip_deserializing, rename_all, tag | — | #740 |
| `rp-mcp` | `apps/rusty_provider/crates/mcp` | s sj | 3 | derive(Serialize)×4, sj::Value×3, sj::json!×3, derive(Deserialize)×3, sj::Map×2, sj::from_str×2, +5 | default, skip_serializing_if | — | #740 |
| `rp-providers` | `apps/rusty_provider/crates/providers` | s sj | 8 | sj::json!×229, derive(Deserialize)×22, sj::from_str×14, derive(Serialize)×14, sj::Value×11, sj::from_slice×5, +3 | skip_serializing_if, default, rename, skip | reqwest | #740 |
| `rp-router` | `apps/rusty_provider/crates/router` | s sj t | 10 | derive(Deserialize)×32, sj::json!×25, sj::from_value×10, derive(Serialize)×9, s::Serialize×8, s::Deserialize×3, +4 | default, rename_all, skip_serializing_if, tag, flatten | reqwest | #740 |
| `rp-server` | `apps/rusty_provider/crates/server` | s sj | 4 | sj::json!×124, derive(Serialize)×7, sj::Value×6, derive(Deserialize)×6, sj::from_slice×5, s::Deserialize×2, +3 | default, flatten, skip_serializing_if, deserialize_with | axum, jsonwebtoken, reqwest, tracing-subscriber | #740 |
| `remind_me_api` | `apps/rusty_remind_me/crates/remind_me_api` | s sj | 18 | sj::json!×144, sj::Value×13, sj::to_value×5, sj::from_str×5, sj::from_slice×2, sj::to_vec×1, +4 | — | engine | #741 |
| `rusty-remind-me` | `apps/rusty_remind_me/crates/remind_me_cli` | s sj | 5 | sj::json!×15, sj::to_string_pretty×9, sj::from_str×9, sj::Value×8, s::de::DeserializeOwned×1, sj::to_string×1, +1 | — | engine | #741 |
| `remind_me_core` | `apps/rusty_remind_me/crates/remind_me_core` | s sj | 191 | sj::json!×507, derive(Serialize)×233, derive(Deserialize)×233, sj::Value×131, sj::from_str×95, s::Serialize×69, +24 | default, skip_serializing_if, rename_all, tag, rename | engine, chrono, rusqlite | #741 |
| `remind_me_hub` | `apps/rusty_remind_me/crates/remind_me_hub` | s sj | 19 | sj::json!×127, sj::Value×19, sj::from_str×5, derive(Serialize)×5, derive(Deserialize)×5, sj::Map×4, +7 | — | engine, chrono, postgres, rusqlite | #741 |
| `remind_me_mcp` | `apps/rusty_remind_me/crates/remind_me_mcp` | sj | 6 | sj::json!×470, sj::to_string_pretty×72, sj::from_str×51, sj::from_value×43, sj::to_value×14, sj::Value×12, +2 | default | engine | #741 |
| `remind_me_remote` | `apps/rusty_remind_me/crates/remind_me_remote` | s sj | 8 | sj::json!×57, derive(Deserialize)×7, sj::Value×5, sj::from_str×4, derive(Serialize)×3, s::Deserialize×2, +4 | skip_serializing_if, default, flatten | engine, axum | #741 |
| `skillopt-cli` | `apps/rusty_skillopt/crates/skillopt-cli` | sj | 1 | sj::to_string_pretty×1 | — | tracing-subscriber | #742 |
| `skillopt-core` | `apps/rusty_skillopt/crates/skillopt-core` | s sj y | 3 | derive(Serialize)×17, derive(Deserialize)×17, y::from_str×7, s::Deserialize×3, s::Serialize×3, y::Value×1, +1 | default, rename, rename_all, tag | — | #742 |
| `skillopt-envs` | `apps/rusty_skillopt/crates/skillopt-envs` | s sj y | 4 | derive(Deserialize)×7, sj::from_str×6, s::Deserialize×3, y::from_value×3, sj::Value×2, sj::to_string×2, +2 | rename_all, default | — | #742 |
| `skillopt-model` | `apps/rusty_skillopt/crates/skillopt-model` | s sj | 3 | derive(Deserialize)×8, derive(Serialize)×6, s::Deserialize×3, s::Serialize×3, sj::from_str×3 | skip_serializing_if, rename | reqwest | #742 |
| `ts-cli` | `apps/rusty_tailscale/crates/ts-cli` | sj | 2 | sj::from_slice×3, sj::from_str×2, sj::Error×1, sj::to_vec×1 | — | — | #743 |
| `ts-control` | `apps/rusty_tailscale/crates/ts-control` | s sj | 2 | sj::to_vec×3, sj::from_slice×3, sj::Error×1, s::Deserialize×1, derive(Deserialize)×1 | rename | — | #743 |
| `ts-key` | `apps/rusty_tailscale/crates/ts-key` | s sj | 1 | s::Serialize×1, s::Deserialize×1, sj::from_str×1, sj::to_string_pretty×1, derive(Serialize)×1, derive(Deserialize)×1 | — | — | #743 |
| `ts-localapi` | `apps/rusty_tailscale/crates/ts-localapi` | s sj | 2 | sj::from_slice×5, sj::to_vec×2, s::Serialize×1 | — | — | #743 |
| `ts-types` | `apps/rusty_tailscale/crates/ts-types` | s sj | 9 | sj::from_str×20, derive(Deserialize)×19, derive(Serialize)×18, s::Deserialize×10, s::Serialize×9, sj::Value×6, +9 | rename, default, deserialize_with, rename_all, skip_serializing_if | — | #743 |
| `rusty_tick` | `apps/rusty_tick` | s | 11 | derive(Serialize)×28, derive(Deserialize)×22, s::Serialize×8, sj::json!×8, s::Deserialize×7, s::Deserializer×1, +1 | rename_all, default, deny_unknown_fields, deserialize_with | engine, uuid | #729 |
| `sessionmgr-agents` | `apps/rusty_yirp/crates/sessionmgr-agents` | sj t(dev) | 3 | sj::json!×6, sj::Value×5, sj::from_str×5, sj::Map::new×3, sj::to_string_pretty×2, sj::Value::String×2, +3 | — | — | #744 |
| `sessionmgr-core` | `apps/rusty_yirp/crates/sessionmgr-core` | s | 3 | derive(Serialize)×9, derive(Deserialize)×9, s::Serialize×3, s::Deserialize×3 | default, rename_all, try_from, into | — | #744 |
| `sessionmgr-daemon` | `apps/rusty_yirp/crates/sessionmgr-daemon` | s sj | 6 | sj::from_str×7, sj::to_string×4, s::Serialize×3, sj::to_string_pretty×2, derive(Serialize)×2, sj::Value×2, +5 | — | — | #744 |
| `sessionmgr-desktop` | `apps/rusty_yirp/crates/sessionmgr-desktop/src-tauri` | s sj | 2 | s::Serialize×2, sj::from_str×2, derive(Serialize)×1, s::de::DeserializeOwned×1, sj::to_string×1 | — | — | #744 |
| `sessionmgr-protocol` | `apps/rusty_yirp/crates/sessionmgr-protocol` | s sj(dev) | 2 | derive(Serialize)×5, derive(Deserialize)×5, s::Deserialize×2, sj::from_str×2, s::Deserializer×1, s::Serializer×1, +3 | default, rename_all, with, tag | — | #744 |
| `sessionmgr-tui` | `apps/rusty_yirp/crates/sessionmgr-tui` | s sj | 2 | sj::to_string×2, sj::from_str×2, s::de::DeserializeOwned×1, s::Serialize×1, sj::Error×1 | — | — | #744 |
| `rusty_json` | `foundation/rusty_json` | s sj(dev) | 13 | sj::json!×17, sj::from_str×8, sj::to_string×7, s::Serialize×7, derive(Serialize)×5, s::Deserializer×4, +17 | — | — | none (dev oracle) |
| `rusty_llama` | `libs/ai/rusty_llama` | s sj | 2 | derive(Serialize)×9, sj::Value×6, sj::from_str×6, derive(Deserialize)×5, sj::to_string×4, sj::from_slice×3, +4 | default, flatten, skip_serializing_if, rename | — | #751 |
| `rusty_fedora` | `libs/homelab/rusty_fedora` | sj | 3 | sj::json!×4, sj::Value×2, sj::to_string×2, sj::from_str×1, sj::Error×1 | — | — | #750 |
| `rusty_opnsense` | `libs/homelab/rusty_opnsense` | sj | 4 | sj::json!×10, sj::Value×2, sj::to_string×1, sj::from_str×1, sj::Error×1 | — | — | #750 |
| `rusty_proxmox` | `libs/homelab/rusty_proxmox` | sj | 4 | sj::json!×6, sj::Value×2, sj::to_string×2, sj::from_str×1, sj::Error×1 | — | — | #750 |
| `rusty_a2a` | `libs/protocol/rusty_a2a` | s sj | 32 | sj::Value×70, sj::json!×68, derive(Deserialize)×57, derive(Serialize)×52, sj::to_value×24, sj::from_value×22, +13 | default, rename, skip_serializing_if, untagged, with | chrono, axum, reqwest | #747 |
| `rusty-acp` | `libs/protocol/rusty_acp` | s sj | 45 | sj::Value×60, sj::json!×59, derive(Serialize)×41, derive(Deserialize)×41, sj::to_string×18, s::Serialize×16, +12 | default, skip_serializing_if, rename, rename_all, transparent | chrono, uuid, axum, reqwest, serde_norway | #747 |
| `rusty_lsp` | `libs/protocol/rusty_lsp` | s sj | 39 | sj::json!×454, derive(Deserialize)×315, derive(Serialize)×313, sj::to_value×144, sj::from_value×60, s::Serialize×34, +14 | default, skip_serializing_if, rename_all, flatten, untagged | — | #747 |
| `rusty-mcp-client` | `libs/protocol/rusty_mcp/crates/rusty-mcp-client` | s sj t(dev) | 4 | derive(Deserialize)×9, derive(Serialize)×6, sj::json!×4, sj::Map×3, sj::Value×3, s::Deserialize×2, +4 | default, skip_serializing_if, flatten, untagged, rename_all | — | #747 |
| `rusty-mcp-demo` | `libs/protocol/rusty_mcp/crates/rusty-mcp-demo` | sj(dev) | 1 | sj::json!×20, sj::Value×5, sj::from_value×2, sj::Map×1, sj::from_str×1 | — | — | #747 |
| `rusty_mcp_client_native` | `libs/protocol/rusty_mcp_client_native` | sj(dev) | 1 | sj::Value::as_i64×1, sj::json!×1 | — | — | #747 |
| `rusty_mcp_proto` | `libs/protocol/rusty_mcp_proto` | s(dev) sj(dev) | 2 | sj::from_str×7, sj::to_value×3, s::de::DeserializeOwned×1, s::Serialize×1, sj::Value×1, sj::json!×1 | — | — | #747 |
| `rusty_mcp_server` | `libs/protocol/rusty_mcp_server` | s(dev) sj(dev) | 1 | sj::Value×4, sj::json!×4, s::Serialize×1, sj::to_value×1, sj::Map×1 | — | — | #747 |
| `adk-a2a` | `libs/rusty_adk/crates/adk-a2a` | sj | 4 | sj::json!×5, sj::Value×1, sj::from_value×1, sj::Value::Null×1 | — | — | #748 |
| `adk-agents` | `libs/rusty_adk/crates/adk-agents` | sj | 4 | sj::json!×23, sj::Value×11, sj::from_str×3, sj::Value::String×2 | — | — | #748 |
| `adk-agui` | `libs/rusty_adk/crates/adk-agui` | sj | 3 | sj::json!×8, sj::Value::Object×3, sj::Value×2, sj::from_str×1, sj::from_value×1, sj::Map×1, +2 | — | — | #748 |
| `adk-core` | `libs/rusty_adk/crates/adk-core` | s sj | 9 | sj::json!×32, derive(Serialize)×22, derive(Deserialize)×22, s::Deserialize×7, s::Serialize×7, sj::Value×6, +6 | default, skip_serializing_if, rename_all, rename, flatten | — | #748 |
| `adk-graph` | `libs/rusty_adk/crates/adk-graph` | s sj | 5 | sj::json!×45, derive(Serialize)×5, derive(Deserialize)×5, s::Serialize×4, sj::Value×4, s::Deserialize×3, +4 | default, skip_serializing_if, tag, rename_all | — | #748 |
| `adk-macros` | `libs/rusty_adk/crates/adk-macros` | sj(dev) | 2 | sj::json!×11, sj::Value×6, sj::from_value×2, sj::Value::Null×1, sj::to_value×1, sj::Value::Object×1 | — | — | #748 |
| `adk-mcp` | `libs/rusty_adk/crates/adk-mcp` | sj | 6 | sj::json!×39, sj::from_str×6, sj::Value×5, sj::Map::new×3, sj::from_value×2, sj::to_value×1, +2 | — | axum | #748 |
| `adk-models` | `libs/rusty_adk/crates/adk-models` | s sj | 4 | sj::json!×45, sj::Value×4, derive(Serialize)×4, derive(Deserialize)×4, sj::to_value×3, sj::Map×2, +3 | default, skip_serializing_if | reqwest | #748 |
| `adk-runner` | `libs/rusty_adk/crates/adk-runner` | sj | 1 | sj::json!×6 | — | — | #748 |
| `adk-sessions` | `libs/rusty_adk/crates/adk-sessions` | sj | 5 | sj::json!×16, sj::to_string×4, sj::from_str×4, sj::Value×2 | — | — | #748 |
| `adk-tools` | `libs/rusty_adk/crates/adk-tools` | s sj | 4 | sj::json!×11, sj::Value×10, sj::Value::Object×2, s::Deserialize×1, sj::Value::String×1 | — | — | #748 |
| `rusty-adk` | `libs/rusty_adk/crates/rusty-adk` | sj(dev) | 1 | sj::json!×5, sj::Value×2 | — | — | #748 |
| `a2a-agent-server` | `libs/rusty_adk/examples/a2a-agent-server` | sj | 1 | sj::json!×2, sj::Value×1 | — | axum | #748 |
| `agui-agent-server` | `libs/rusty_adk/examples/agui-agent-server` | sj | 1 | sj::json!×2, sj::Value×1 | — | — | #748 |
| `mcp-tool-server` | `libs/rusty_adk/examples/mcp-tool-server` | sj | 1 | sj::Value×2, sj::json!×2 | — | — | #748 |
| `weather-agent` | `libs/rusty_adk/examples/weather-agent` | sj | 1 | sj::json!×12, sj::Value×2 | — | — | #748 |
| `rusty-db-core` | `libs/storage/rusty_db/crates/rusty-db-core` | sj | 2 | sj::from_str×4, sj::Value×2, sj::Number::from_f64×1 | — | chrono, uuid | #749 |
| `rusty-db` | `libs/storage/rusty_db/rusty_db` | sj(dev) | 6 | sj::json!×13 | — | — | #749 |
| `rusty_fair_play_domain` | `libs/storage/rusty_fair_play_domain` | s | 1 | derive(Serialize)×5, derive(Deserialize)×5, s::Deserialize×1, s::Serialize×1 | — | engine, uuid | #746 |
| `rusty_multimodal_db_engine` | `libs/storage/rusty_multimodal_db_engine` | s | 17 | derive(Serialize)×24, derive(Deserialize)×23, s::Serialize×21, s::Deserialize×15, s::de::DeserializeOwned×7 | — | uuid, bincode | #728 |
| `rusty-search-algolia` | `libs/storage/rusty_search/crates/rusty-search-algolia` | sj | 3 | sj::json!×30, sj::Value×5, sj::Map::new×2, sj::from_str×2, sj::from_slice×2, sj::to_vec×1, +1 | — | — | #749 |
| `rusty-search-azure-search` | `libs/storage/rusty_search/crates/rusty-search-azure-search` | sj | 3 | sj::json!×27, sj::from_slice×7, sj::Value×5, sj::Map::new×2, sj::to_vec×1, sj::Number×1 | — | — | #749 |
| `rusty-search-core` | `libs/storage/rusty_search/crates/rusty-search-core` | sj | 5 | derive(Serialize)×13, derive(Deserialize)×13, sj::json!×6, sj::Value×4, sj::Map×1, sj::Number::from_f64×1 | — | — | #749 |
| `rusty-search-elasticsearch` | `libs/storage/rusty_search/crates/rusty-search-elasticsearch` | sj | 4 | sj::json!×40, sj::Value×6, sj::Map×2, sj::from_slice×2, sj::from_str×2, s::Serialize×1, +2 | — | — | #749 |
| `rusty-search-meilisearch` | `libs/storage/rusty_search/crates/rusty-search-meilisearch` | sj | 2 | sj::json!×6, sj::Value×5, sj::Map::new×2, sj::from_slice×2 | — | — | #749 |
| `rusty-search-opensearch` | `libs/storage/rusty_search/crates/rusty-search-opensearch` | sj(dev) | 1 | sj::json!×9 | — | — | #749 |
| `rusty-search-solr` | `libs/storage/rusty_search/crates/rusty-search-solr` | sj | 3 | sj::json!×11, sj::Value×5, sj::from_slice×3, sj::Map::new×2, sj::from_str×1, sj::to_vec×1 | — | — | #749 |
| `rusty-search-tantivy` | `libs/storage/rusty_search/crates/rusty-search-tantivy` | sj | 2 | sj::Value×3, sj::Value::Object×2, sj::Map::new×2, sj::Map×1, sj::Value::String×1, sj::to_value×1, +3 | — | — | #749 |
| `rusty_term_l13` | `libs/ui/rusty_term/l13` | sj | 1 | sj::json!×35, sj::from_str×5, sj::Value×1, sj::to_value×1, sj::to_string×1 | — | — | #751 |
