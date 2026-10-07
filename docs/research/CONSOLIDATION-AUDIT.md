# Consolidation audit: duplicated capabilities across rusty_mill

Date: 2026-10-07 · Status: report only (no code, issues or PRs) · Base: `main` @ 4506e96

## Status (2026-10-07)

- Nexus **frozen** (Q1 answered "freeze first"): excluded from the workspace, not deleted; `crates/apps/nexus/FROZEN.md`. Deletion still needs separate approval.
- Priority step 1 **done** for rows 1 and 2 (non-Nexus sites): `rusty_crypto_key::constant_time_eq`, `rusty_oauth::bearer::token_from_authorization`. Left: the two Nexus `ct_eq` copies and `nexus-memory-hub`'s Bearer parse (frozen), `rusty_acp/examples/authenticated_server.rs` (example kept standalone).
- Priority step 2: row 3 **done** (`Emitter::{text_delta, end_text, tool_call, tool_result}` in `rusty_agui`; `adk-agui`, `rk-agui`, `rusty_tick`, `echo_agent` use them). Row 4 (`strip_ansi`) **done**: `rusty_ansi`'s parser gained an `Escape` token (two-byte sequences, charset designations, DCS/SOS/PM/APC strings up to ST) so its `strip_ansi` matches the stricter test helper; `rp-router`'s `rtk.rs` and `rusty_lines/tests/pty.rs` now call it (all 43 PTY tests pass). Side effect for `rtk.rs`: OSC and two-byte sequences are stripped too, and other C0 controls (bell, backspace) are dropped. The Nexus copies (`nexus-cli`, `nexus-terminal`) are frozen and untouched.
- Priority step 3 **done** for rows 5, 6, 7: `rusty_base64::decode_standard_lenient` (6 base64 copies), new `rusty_hex` (8 crates), new `rusty_percent` (11 crates; `rusty_url` rejected as the home because it pulls `idna`). Left: `platform-bsd` (BSD-only, not built here), Nexus copies (frozen), `rusty_url`'s internal decoder, the eight external `percent-encoding` users, row 21 (`Url` ×2).
- Priority step 4 **done** (row 8): `rusty_retry` now owns `Retry-After` incl. HTTP-date (no `rusty_time` needed; it stays zero-dependency) and `Backoff` replaces the three hand-rolled doublings (`rusty_a2a` push, `rp-mcp` gateway, `rusty-croc`). Left: Nexus copies (frozen).
- Priority step 5 **partly done**: `remind_me_remote` on the workspace `rmcp` (row 12); `rsi-runtime` blob writes on `rusty_atomic_file` (row 13). Not moved, with reasons: `rusty_fair_play` (its tests inject failure through the fixed `.tmp` name) and `rusty_lines` (preserves existing permissions). Both would need `rusty_atomic_file` or the tests to change first. `rusty_atomic_file` does fsync the parent directory, which settles the earlier open question.
- Priority step 6: row 14 **done in the narrow form** (Q6 answered: option 1): new `rusty_dirs::{config_root, config_dir}` used by `rusty_term` and `rusty-croc`. The audit overstated the duplication: the other sites resolve state/data dirs or `~`, with their own overrides, and `rusty_yirp`'s two copies are deliberate. Row 17 (Nexus retirement) **not done**: needs explicit approval to delete.

## 0. Scope, method, limits

- Prior decisions honoured (remind-me + repo docs):
  - Nexus is not in use; `nexus-agui` removed in PR #531. **No new Nexus integrations.** Nexus is a source to extract from or retire. (remind-me `mem_e81f28b4…`)
  - Keep Nexus and remind_me memory stores separate; do not embed `remind_me_core` (`docs/research/memory-sync-contract/README.md`, "Approved architecture").
  - Reusable MCP protocol client belongs in `rusty_mcp`, Nexus owns only config/policy adapters (same README).
  - ADR-0002 (sovereignty tiers: `rusty_tls`, `rusty_sqlite`, `sqlx`, `rmcp` MCP crates are Tier A, permanent); ADR-0003 (a crate depends only on its own or lower layers; libs cannot depend on apps); ADR-0007 (rusty_agui, `rusty_serve` streaming).
  - ADR-0009 (rusty_orch hand-rolled arg parsing is deliberate, cited in `apps/rusty_orch/crates/rusty_orch/src/args.rs`; the ADR file itself is app-local, not in `docs/adr/`).
- Prior dedupe sweeps: merge-title grep (`dedup|duplic|sovereign|hoist|consolid|nexus`) found only Nexus MCP work (#470 rmcp security bump, #473 rmcp 0.9→3.1, #498 rusty-mcp client, #500 nexus-mcp onto `rusty_mcp::serve`). No earlier cross-crate dedupe sweep found. Title grep only; PR bodies not read.
- Tools: repo-inspector scripts (index 20,055 items; module/`new`/`get` clusters triaged as noise; sovereignty scan across all manifests), six parallel read-only manual passes (MCP, HTTP, memory, agui/protocols, config/error/log/auth/serialization, CLI/terminal/LLM/Nexus copies), then grep spot-checks.
- Spot-checks (grep -c on candidate files) passed for: `struct Relay` (adk-agui, rk-agui), two `Url` structs, `fn strip_ansi` ×4, `parse_retry_after`/`retry_after`, `fn constant_time_eq|ct_eq` (9 production sites + 1 test in rusty_crypto_key), 11 `strip_prefix("Bearer ")` files, `rusty_config` dependents = 0, `rusty_ansi`/`rusty_h2` dependents = 0, `nexus-vt` has no `rusty_term` dependency. Two first-pass misses were naming only (`JsonRpcMessage` is an enum; Nexus RRF fn is `fuse` + `RRF_K`).
- **Not verified:** LOC/test counts are `wc -l`/attribute greps, not run results; nothing was built or tested. Items marked (u) were not read in full.

Classification: **E** exact duplicate · **N** near duplicate · **D** diverged · **C** coincidental (logged, no action).

## 1. Summary table

Order = priority (section 4). "Nexus?" = whether Nexus copies are involved; those die with Nexus rather than being migrated.

| # | Capability | Crates involved (evidence §2) | Class | Recommended home | Effort | Risk |
|---|---|---|---|---|---|---|
| 1 | Constant-time compare | 9 sites (§2.1) | N (2 E) | `foundation/rusty_crypto_key` (add pub `ct_eq`) | S | Low |
| 2 | `Authorization: Bearer` extraction | 11 sites (§2.1) | N | `libs/net/rusty_oauth::bearer` (exists, unused by these) | S | Low |
| 3 | AG-UI emit helpers (`Relay`) | adk-agui, rk-agui (+ rusty_tick (u)) | N | `libs/protocol/rusty_agui::serve::Emitter` | S | Low |
| 4 | `strip_ansi` | rusty_ansi (0 dependents), rusty_provider router, nexus-cli, nexus-terminal | N | `libs/ui/rusty_ansi` (adopt) | S | Low |
| 5 | Percent encode/decode | ~10 decoders, 3 encoders (§2.2) | N (behaviour differs) | `foundation/rusty_url` (make `percent_encode` pub) | S–M | Low–Med |
| 6 | Hex encode/decode | 9 sites (§2.2) | N | `foundation` (new fn in `rusty_codec` or `rusty_base64`; decision Q4) | S | Low |
| 7 | base64 hand copies | rusty_term, nexus-vt, platform-linux, platform-bsd, remind_me_remote pkce, rusty_multimodal_db pem | N | `foundation/rusty_base64` | S | Low–Med (platform layer, Q5) |
| 8 | Retry/backoff + Retry-After | rusty_retry vs rusty_request, rusty_acp, rusty_a2a push, rp mcp gateway, nexus ×4 | N/D | `foundation/rusty_retry` | S | Low |
| 9 | Hand-rolled std::net HTTP servers | remind_me ×4, rusty_llama, rusty_whisper, rusty_term, meshed-registry | N purpose / D code | `libs/net/rusty_serve` (+`rusty_http`) | L | Med–High |
| 10 | Raw-TCP HTTP clients | rusty_whisper llm_client, remind_me sync/embedder/query_expansion/telemetry | N | `libs/net/rusty_http` sync / `rusty_request` | M | Med |
| 11 | MCP client (spawn/connect/list/call) | rusty-mcp, adk-mcp, rk-mcp, rp-mcp, agentgateway-mcp, nexus-mcp pool | D (core path N) | `libs/protocol/rusty_mcp` (client feature) | M–L | Med |
| 12 | MCP server scaffold | rusty-mcp, adk-mcp, remind_me_remote (own rmcp 3.0.1 pin), rk-app, remind_me_mcp (hand-rolled) | D | `libs/protocol/rusty_mcp` | M per crate | Med |
| 13 | Atomic tmp+fsync+rename writes | ~13 sites outside `rusty_atomic_file` | N | `foundation/rusty_atomic_file` | M | Med (durability) |
| 14 | Config/home dir resolution | ≥8 non-Nexus sites + 3 Nexus (§2.3) | N | Q6 (no foundation helper exists) | S–M | Low |
| 15 | LLM provider wire types/clients | adk-models, rusty_provider/providers, agentgateway-llm, skillopt-model, rsi-runtime (+nexus-ai) | D | new `libs/ai` crate, base: rusty_provider/providers | L | High |
| 16 | SSE read/write | writers: rusty_agui, rusty_serve, rusty_llama, meshed; readers: rusty_agui Decoder vs `eventsource-stream` in a2a/acp | D | `rusty_agui::sse` (already Tier S) | M | Med |
| 17 | Nexus-vs-workspace copies (VT core, LSP framing, etc.) | nexus-vt, nexus-terminal, nexus-lsp, nexus-git, nexus-security, … | N/D/C | **retire with Nexus** (§3) | S–L | Low if Nexus retired |
| 18 | Memory layer | nexus-memory(+hub) vs rusty_remind_me | N/D | none: keep separate (decided) | – | – |
| 19 | Embedding/cosine/RRF | remind_me_core, nexus-storage, rusty_key/feed, inventory-core, router, rusty_rag (stub) | D | defer (Q9) | M | Med |
| 20 | Redaction, keyring, tracing init, CLI parsing, error types | §2.4 | D/N | defer or decision (Q7, Q8) | – | – |
| 21 | Url struct ×2 | `rusty_http::Url` vs `rusty_url::Url` | N/E-ish | `foundation/rusty_url` | M | Med |
| 22 | WebSocket | rusty_term ws.rs vs nexus-collab (tungstenite) | C-ish | defer | – | – |

Logged as coincidental / no action: A2A (rusty_a2a base; adk-a2a, agentgateway-a2a are adapters), ACP (`rusty-acp` is Agent Communication Protocol REST; `nexus-acp` is stdio JSON-RPC, different protocol, no dependency), `rusty_skillopt` vs `nexus-skills`, nexus-templates vs rusty_jinja, nexus-git (git2) vs rusty_git, orch-* CLI adapters (well factored on `orch-cli`), rusty_db vs rusty_sqlite, rusty_key/feed `Memory` vs remind_me, rusty_multimodal_db `generic/memory.rs` (deliberate projection, ADR-0048).

## 2. Per-cluster detail

Paths relative to `crates/`.

### 2.1 Auth primitives (rows 1, 2)

| Site | File |
|---|---|
| rusty_oauth | `libs/net/rusty_oauth/src/crypto/hmac.rs:38` |
| remind_me_core ×2 | `apps/rusty_remind_me/crates/remind_me_core/src/daemon/server.rs:218`, `…/src/webhook.rs:183` (pub, `black_box`) |
| remind_me_hub | `apps/rusty_remind_me/crates/remind_me_hub/src/lib.rs:121` |
| rusty_key | `apps/rusty_key/crates/app/src/gateway.rs:489` (`&str`) |
| rusty_fair_play, rusty_tick | `apps/rusty_fair_play/src/api.rs:259`, `apps/rusty_tick/src/api.rs:296` (identical to each other) |
| Nexus (retire) | `apps/nexus/crates/nexus-collab/src/auth.rs:55`, `…/nexus-memory-hub/src/lib.rs:428` (`ct_eq`) |

- Differences: early return on length mismatch vs padded loop vs `black_box`. `rusty_crypto_key` only has a test named `constant_time_eq` (`foundation/rusty_crypto_key/src/lib.rs:136`), no pub fn; it already ships `SecretBytes`. `subtle` is used in rusty_croc and rusty_multimodal_db.
- Best base: padded-loop variant (no length leak) in `rusty_crypto_key`.
- Breaks: nothing observable; replace bodies with a call. Adds a `rusty_crypto_key` dependency to ~7 apps (apps→foundation is layer-legal).
- Bearer: 11 files do `strip_prefix("Bearer ")` (list in repo: `rusty_channel/src/teams.rs:89`, `remind_me_core/src/webhook.rs:664`, `remind_me_api/src/lib.rs:219`, `remind_me_remote/src/oauth/routes.rs:910` + `src/auth.rs:138`, `rusty_provider/crates/server/src/routes.rs:26`, `rusty_key/crates/app/src/gateway.rs:143`, `nexus-memory-hub/src/lib.rs:444`, `rusty_fair_play/src/api.rs:248`, `rusty_tick/src/auth.rs:73`, `rusty_acp/examples/authenticated_server.rs:91`). `libs/net/rusty_oauth/src/bearer.rs` exists unused by them. `rusty_oauth` has one dependent (`rusty_channel`).

### 2.2 Encoding helpers (rows 5, 6, 7, 21)

- **Percent**: decoders at `rusty_mcp/…/resources.rs:489`, `rusty_lsp/src/lsp/base.rs:232`, `rusty-meshed-registry/src/http/query.rs:34`, `agentgateway-core/src/router.rs:595`, `remind_me_core/src/sync/server.rs:212`, `remind_me_hub/src/http.rs:202`, `remind_me_api/src/http.rs:100`, `rusty_fedora_agent/src/http.rs:310`, `nexus-formats/src/notion/mod.rs:370`; encoder in `rusty_http/src/url.rs:262`. Cause: `foundation/rusty_url/src/percent_encode.rs` is `pub(crate)`; only `form_urlencoded` is public (`lib.rs:16`). Behaviour differs (`+`, malformed escapes, UTF-8) so each swap needs a test. `rusty_url` pulls external `idna` (Tier T), which is why a hot-path crate like `rusty_http` may not want it (see Url below).
- **Hex**: `rusty_tailscale/crates/ts-types/src/hex.rs`, `ts-key/src/lib.rs:112`, `rusty_croc/src/croc.rs:346`, `remind_me_core/src/telemetry.rs:208`, `remind_me_remote/src/oauth/pkce.rs:19`, `platform-linux/src/sys/dbus/transport.rs:183`, `rsi-core/src/lineage.rs:93`, `rusty_term/src/core/parser.rs:1572` + `nexus-vt/src/core/parser.rs:1032`. No foundation hex helper exists.
- **base64**: `rusty_term/src/core/base64.rs` and `nexus-vt/src/core/base64.rs` (4-line diff), `platform-linux|bsd/src/sys/trust_anchors.rs` (`b64_decode`, near twins), `remind_me_remote/src/oauth/pkce.rs:36`, `rusty_multimodal_db/src/server/pem.rs:88`. Canonical: `foundation/rusty_base64` (`encode_standard`, `decode_standard`, `encode_url_safe*`). Unverified: whether `platform/rustils` is allowed to depend on foundation (ADR-0003 layer order not re-read) (Q5).
- **Url ×2**: `libs/net/rusty_http/src/url.rs:16` vs `foundation/rusty_url/src/url.rs:32` (WHATWG, 5,683 LOC, 138 tests). `rusty_http` has `resolve_redirect`, `with_query_pairs`; six files outside rusty_http use `rusty_http::Url`. Merge = adapter or port those two methods; touches `rusty_request`, `rusty_serve`, `rusty_agui`, `rusty_channel`, ts-*, meshed-*.
- **sha1/sha256**: `libs/net/rusty_rdp/src/crypto/sha1.rs` duplicates `foundation/rusty_sha1` (N). `sha256_hex` ×2 in nexus + external `sha2` in 13 crates; no foundation sha256 found (sovereignty gap, §5).
- **JSON escape**: `rusty_whisper/src/output.rs:211`, `wasm.rs:114`, `rusty_term/src/web_bridge/stats.rs:200`, `nexus-notifications/src/lib.rs:422` (N). `rush/src/value.rs` hand-rolls a JSON parser (1 site, not a finding).

### 2.3 Config / paths / files (rows 13, 14)

- `foundation/rusty_config` (171 LOC INI/KV parser) has **0 dependents**; real config is `toml` in 27 manifests plus per-app loaders (nexus-kernel/mcp/lsp/notifications/security, rusty_key, rusty_provider router `config.rs` 2,278 LOC, agentgateway-config, rusty_term). Schemas differ (D); only the skeleton repeats. Not a candidate: no shared schema to extract.
- Home/config dir: raw env logic at `rusty_term/src/config.rs:355`, `rusty_croc/src/main.rs:88`, `rusty_yirp/crates/sessionmgr-daemon/src/paths.rs:58` and `sessionmgr-desktop/src-tauri/src/paths.rs:34` (**exact copies**), `sessionmgr-agents/src/{codex,gemini}.rs`, `rush/src/expand.rs:3287`, `rusty_inventory/…/paths.rs:18`, `rusty_key/crates/feed/src/guide.rs:158`, `rusty_provider/crates/cli/src/setup.rs:69`; plus `dirs` in 7 manifests. `platform/portable-runtime/crates/contract` already defines `config_dir`/`data_dir` (`lib.rs:267,269`) with a `dirs`-backed impl, called by no app (Q6).
- Atomic writes: `foundation/rusty_atomic_file` (`write`, `write_private`; 11 dependents). Hand-rolled tmp+fsync+rename: `rusty_fair_play/src/service.rs:664`, `nexus-storage/src/atomic.rs:109`, `rusty_lines/src/lib.rs:1193`, `rsi-runtime/src/lineage_store.rs:215`, `remind_me_hub/src/store/multimodal/mod.rs:442`, engine ×4 (`rusty_multimodal_db_engine/src/{journal,generic/insert_log,generic/slot_file,durability/record_blob}.rs`), `rusty_multimodal_db/src/server/{mvcc,changelog}.rs`, `durability/mmap_store.rs`. Several add directory fsync (ADR-0092 comment in rusty_multimodal_db) and rusty_atomic_file's directory-fsync behaviour was not checked. Do not touch the DB-engine ones without a durability review; the app-level ones (fair_play, rsi, lines) are the safe subset.
- 0o600 file creation duplicated: `nexus-panic-log` (`open_restricted`, append variant), `remind_me_core/src/{remote,ics}.rs`, `sessionmgr-daemon/src/catalog.rs`, `rusty_term/src/gui/control.rs` vs `rusty_atomic_file::write_private`.
- Confined paths: `foundation/rusty_confined_fs` (symlink-hardened) has 2 dependents; 9 lexical-only checks elsewhere, 6 in Nexus (`nexus-types/src/paths.rs:79`, `path_validator.rs`, `nexus-security/src/path.rs`, `nexus-kernel/src/context_impl.rs:390`, …). Non-Nexus: `remind_me_core/src/import_paths.rs:129`, `rusty_fedora_agent/src/allowlist.rs:96`. Defer; only 2 non-Nexus sites and different semantics.

### 2.4 Cross-cutting, low consolidation value (row 20)

| Area | Evidence | Verdict |
|---|---|---|
| Error types | `thiserror` in 90 manifests (nexus 34, tailscale 13, agentgateway 9); `anyhow` 21; `rusty_err` 18 (meshed 9/9, rsi 2). ADR-0002 marks the thiserror/uuid/base64/url migration Transitional (#119-#121); only base64 recorded closed (`RELEASE_NOTES.md:2564`). | Migration backlog, not extraction. Retiring Nexus clears 34. |
| Logging | `tracing` 77 manifests; same `fmt().with_env_filter().init()` block ×8 (provider server, skillopt-cli, ts-daemon, nexus ×4, agentgateway telemetry). `rusty_croc` is the only `log` user. | Defer; 5 non-Nexus copies of a 5-line block. |
| Redaction | 6 redactors, different semantics (`remind_me_core/src/redact.rs`, `rusty_key/crates/observe/src/redact.rs`, `nexus-ai/src/privacy.rs`, `nexus-memory/src/capture.rs`, `nexus-terminal/src/env.rs`, `rusty_provider/crates/router/src/guardrails.rs`) | D; do not merge. |
| Keyring | 4 crates (`nexus-security/src/credential.rs`, `nexus-plugins/src/grants_crypto.rs`, `inventory-core/src/keychain.rs`, `rusty_key/desktop/src-tauri/src/secrets.rs`) | N; only 2 non-Nexus sites. Defer. |
| CLI parsing | clap 4 derive in 11 manifests; hand-rolled `opt_arg`/`has_flag` ≥8 (rusty_provider cli, remind_me_cli 1,516 LOC, ts-cli, rsi-cli, orch, rusty_tick, rusty_key, coreutils). No shared crate, no `--version` in hand-rolled CLIs, exit codes inconsistent. `orch` hand-rolling is deliberate (ADR-0009). | Decision needed (Q7) before any crate. |
| OAuth/JWT | `rusty_oauth` (sans-IO, 5.6k LOC) has one dependent; rusty-mcp `auth/`, remind_me_remote `oauth/` (ADR-0011 hand-rolled by design), rusty_a2a `server/auth.rs`, agentgateway-auth; `jsonwebtoken` in 4 manifests | D; hoist primitives only (Bearer, PKCE) later. |

### 2.5 MCP (rows 11, 12)

Best base: `libs/protocol/rusty_mcp/crates/rusty-mcp` (9,767 src LOC, 230 tests, rmcp 3.1 wrapper; server stdio + Streamable HTTP, client `McpClient`, auth/JWT, pagination, otel). Already consumed by nexus-mcp, nexus-cli, rp-mcp, rp-server, rusty_homelab_mcp, agentgateway(+auth,+mcp).

| Implementation | Path | Stack | Relation to rusty-mcp | Generic vs app-specific |
|---|---|---|---|---|
| adk-mcp | `libs/rusty_adk/crates/adk-mcp` (1,459 LOC, 20 tests) | direct rmcp; own `line_cap.rs`; stateless HTTP; `PROTOCOL_VERSION="2025-06-18"` | not a consumer | generic: stdio/HTTP serve, client, toolset; ADK `McpToolset` is ADK-specific |
| rk-mcp | `apps/rusty_key/crates/mcp` (1,191 LOC, 17 tests) | rmcp optional; own `McpClient` trait | not a consumer | generic: stdio/HTTP client; specific: `McpPolicy`, harness tool mapping |
| rk-app server | `apps/rusty_key/crates/app/src/mcp_server.rs` | rmcp stdio, single `chat` tool | not a consumer | all generic scaffold |
| remind_me_mcp | `apps/rusty_remind_me/crates/remind_me_mcp` (6,700 LOC, 92 tests) | **hand-rolled** JSON-RPC (`src/lib.rs:448,504,1488`), own stdio loop (`:3302`) and raw HTTP (`:3425`) | no | framing generic; ~80-tool `json!` list specific |
| remind_me_remote | `apps/rusty_remind_me/crates/remind_me_remote` (3,539 LOC, 67 tests) | **own rmcp 3.0.1 pin** (workspace is 3.1); OAuth/consent | overlaps `rusty_mcp::auth` | OAuth issuer documented as intentional (ADR-0011) |
| rp-mcp | `apps/rusty_provider/crates/mcp` | rusty-mcp + rmcp | consumer | gateway client small; keep |
| agentgateway-mcp | `apps/rusty_agent_gateway/crates/agentgateway-mcp` (6,271 LOC) | rmcp + rusty-mcp `otel` only; hand-built JSON-RPC wire handling in guardrails/federation | partial | specific: federation, guardrails, mutation |
| nexus-mcp | `apps/nexus/crates/nexus-mcp` (8,197 LOC; `server.rs` 4,809) | already on `rusty_mcp::serve`, client, `apply_cache_hints` (PR #500) | consumer | specific: Nexus tool set, `ConnectionPool`, `DynamicToolRegistry`, IPC types. **"Step 2" is not defined in any doc** |

- Classification: server scaffold **D**; client connect/list/call core **N**; remind_me_mcp framing outlier **D**; tool sets C/app-specific (stay put: homelab Proxmox/OPNsense/Fedora, rp chat/embeddings, remind_me, rk `chat`, Nexus tools).
- What breaks: adk→rusty-mcp changes negotiated protocol version (rusty-mcp targets 2026-07-28 per the MCP agent; (u) not re-read) and loses stateless HTTP + `line_cap` unless ported into rusty-mcp first. remind_me_remote tests (`tests/{http,oauth}_test.rs`) pin wire behaviour. rk's `McpClient` trait/`McpPolicy` are not in rusty-mcp. rmcp 3.x renames already broke nexus-mcp and rusty_key once (ba8fb92, 1e6c552), so one pin is cheap insurance.
- Cheapest safe step: align `remind_me_remote` to workspace rmcp 3.1 (S, low) before any migration.

### 2.6 HTTP / SSE (rows 9, 10, 16, 21)

Not "3 copies": a layered first-party stack plus real duplicates around it.

| Layer | Crate | LOC / tests | Notes |
|---|---|---|---|
| Sans-IO core | `libs/net/rusty_http` | 5,615 / 144 | HTTP/1.1, cookies, sync + 2 async adapters; no TLS/H2/SSE reader |
| Client | `libs/net/rusty_request` | 3,669 / 123 | pool, redirects, proxy, retries (not on `send_streaming`), HTTP/1.1 only, no ALPN, no SSE parser |
| Server | `libs/net/rusty_serve` | 768 / 8 | blocking thread-per-conn; `Body::Stream` for SSE (`lib.rs:63-70,457`) |
| TLS | `libs/net/rusty_tls` | 12,040 / 399 | Tier A over rustls (ADR-0002) |
| H2 | `libs/net/rusty_h2` | 5,582 / 120 | **zero dependents**, no async I/O |
| OAuth | `libs/net/rusty_oauth` | 5,636 / 133 | one dependent |

Real duplicates:

| Group | Sites | Class | Base | What breaks |
|---|---|---|---|---|
| std::net servers | remind_me (`remind_me_core/src/sync/server.rs` 784, `webhook.rs` 996 with 0 in-file tests, `remind_me_hub/src/http.rs` 347, `remind_me_api/src/http.rs` 533), `rusty_llama/src/server.rs` (SSE headers by hand `:1113`), `rusty_whisper/src/http.rs`, `rusty_term/src/web_bridge`, `rusty-meshed-registry/src/http/` | N purpose / D code | `rusty_serve` | needs keep-alive vs `Connection: close` decision, path params, auth/rate-limit/CORS hooks, UDS (ts-localapi), WS upgrade (rusty_term). `remind_me_hub/src/http.rs` states self-contained servers are deliberate: needs owner call (Q2) |
| raw-TCP clients | `rusty_whisper/src/llm_client.rs`, `remind_me_core/src/{sync/http,embedder,query_expansion,telemetry}.rs` | N | `rusty_http` sync transport | `http://` only by design; `rusty_request` is async-only |
| reqwest | 20 manifests (reqwest 0.13 workspace; 0.12 in rusty_key/feed and adk-models), nexus 8 | same capability | `rusty_request` | rusty_request lacks HTTP/2/ALPN, client certs, SSE client, streaming retry. **Not recommended**: gateways need hyper-level proxying; no ADR decides reqwest policy (Q3) |
| SSE writers | `rusty_agui/src/sse.rs`, `rusty_serve`, `rusty_llama/src/server.rs:1113`, `rusty-meshed-registry/src/http/response.rs:55` | D | `rusty_agui::sse` / `rusty_serve::Body::Stream` | meshed is an app, async constraint (u) |
| SSE readers | `rusty_agui::sse::Decoder` vs `eventsource-stream 0.2` in `rusty_a2a/src/client/{mod,rest}.rs`, `rusty_acp/src/client/mod.rs:746` | D | `rusty_agui::sse` | Decoder is sync and ignores `id:`/`event:`/`retry:`; acp `resumption.rs` needs those |
| Retry-After | `rusty_request/src/retry.rs:191`, `rusty_acp/src/client/mod.rs:240`; hand-rolled backoff `rusty_a2a/src/server/push.rs:140`, `rusty_provider/crates/mcp/src/gateway.rs:295`, `rusty_croc/src/croc.rs:56` | N | `rusty_retry` (177 LOC; delta-seconds only) | HTTP-date parsing needs `rusty_time`; check dependency direction |

### 2.7 AG-UI and protocol crates (row 3, 16)

- `rusty_agui` (3,788 src LOC, zero external deps) is the library. Adapters adk-agui (`lib.rs:201`), rk-agui, agentgateway-agui (CEL proxy gate using `rusty_agui::sse::Decoder`/`Verifier`) are legitimate thin adapters. No SSE/encoding/verifier re-implementation found. Step 10 done; Nexus adapter removed (PR #531); `nexus-ai-runtime::AiEvent` (`events.rs`, 12 variants) stays without an adapter, per decision.
- Duplicate: `Relay` in `libs/rusty_adk/crates/adk-agui/src/lib.rs:280` and `apps/rusty_key/crates/agui/src/lib.rs:335` (grep-verified in both): lazy TextMessageStart/Content, close_open → End, ToolCall Start/Args/End, ToolCallResult `role: Tool`, frontend tool call. Differences: adk returns bool and carries `parent_message_id`; rk has a `streamed` flag and wraps non-OK status. `Emitter` today only has `emit`, `next_id`, `text`, `state` (`rusty_agui/src/serve.rs:42-78`).
- Base: add helpers to `Emitter` (additive minor). Regression guard: `adk-agui/tests/end_to_end.rs` and `rk-agui/tests/serve.rs` pin event sequences. `rusty_tick/src/assistant.rs` may be a third site (u).
- Not duplicated: approval gate exists only in `rk-constrain`; "frontend tool call answered by next run's tool message" is a shared convention that could be documented, not a gate.
- Other event vocabularies stay diverged: `rusty_key/crates/app/src/contract.rs` (243 LOC), adk-core `Event`, remind_me `Event`, rusty_acp `Event`.

### 2.8 LLM provider clients (row 15)

Each defines its own Anthropic Messages wire types, `x-api-key`/`anthropic-version` headers and SSE parsing:

| Crate | Path | HTTP |
|---|---|---|
| rusty_provider | `apps/rusty_provider/crates/providers/src/{anthropic,gemini,openai_compatible}.rs` (broadest: image/doc blocks, per-request key) | reqwest |
| adk-models | `libs/rusty_adk/crates/adk-models/src/{anthropic,gemini}.rs` | reqwest 0.12 optional |
| agentgateway-llm | `apps/rusty_agent_gateway/crates/agentgateway-llm/src/{provider,translate,stream}.rs` (7,740 LOC) | hyper + reqwest 0.13 |
| skillopt-model | `apps/rusty_skillopt/crates/skillopt-model/src/{anthropic,azure_openai,openai_compat,claude_cli}.rs` | reqwest |
| rsi-runtime | `apps/rusty_rsi/crates/rsi-runtime/src/model.rs` (904) | `rusty_http`, plain HTTP only (ADR-0005) |
| nexus-ai (retire) | `apps/nexus/crates/nexus-ai/src/{anthropic,openai,ollama}.rs` | reqwest; only native Ollama client |

- 5 non-Nexus call sites, so an abstraction is justified, but they are **D**: gateway translate layer vs library client vs SSE-parsing differences. The shared crate must live in `libs/` (ADR-0003: rusty_provider is an app, libs cannot depend on it), so "base = rusty_provider/providers" means port that code down, not depend on it.
- Tool-call types (`nexus-ai` `ToolSchema`, rusty_provider `FunctionDef`/`ToolCall`, `rusty_agui::ToolCall`, adk-tools `Tool`) are different semantics: C/D, do not unify.
- Hold until after rows 1–8; this is the largest and riskiest hoist (Q10).

### 2.9 Memory and storage (rows 18, 19)

- nexus-memory (7,802 LOC, 117 tests; rusqlite single file) + nexus-memory-hub (923 LOC; JSON-blob `records` table) vs `rusty_remind_me` (123k LOC, 2,298 tests; engine-backed `rusty_multimodal_db_engine`; ~40-field `Memory`). Wire routes identical (`POST /sync/push`, `GET /sync/pull`), semantics diverged (tombstone `status="deleted"` vs `deleted_at`; closed `memory_type` enum collapses types; `remind_at`/`sensitive`/13 v32 fields dropped on round trip; tag-union vs whole-row LWW; `hub_seq` cursor not round-trippable). remind_me is a strict superset.
- **Decided:** keep separate, no coupling. Only open work is the approved "lossless common sync context/provenance spec", not started. With Nexus unused, the clean path is retiring `nexus-memory-hub` (no workspace dependents) and later `nexus-memory` (dependents: nexus-bootstrap, nexus-context). `nexus-memory/src/import/remind_me_db.rs` already imports remind_me DBs; no Nexus→remind_me exporter exists.
- Genuinely Nexus-specific (die with it): IPC-coupled embedding (`nexus-ai::embed_text`), capture pipeline, episodic/semantic/procedural tables.
- Parallel unshared pieces: `rusty_search` (11.7k LOC, 12 crates, **no consumer outside itself**; nexus-storage uses tantivy directly), `rusty_rag` (275 LOC stub, no consumers), `rusty_hister` vector/indexer/server stubs. Cosine/embedding implementations in `remind_me_core/src/{embedder,db/vectors}.rs`, `rusty_key/crates/feed/src/memory/embed.rs`, `inventory-core`, `rusty_provider/crates/router`, `nexus-storage/src/vectorstore.rs`; RRF k=60 duplicated only inside Nexus (`nexus-memory/src/vector.rs`, `nexus-storage/src/hybrid.rs`). `rusty_sqlite` is the shared SQLite wrapper (7 families) but nexus-* and remind_me use rusqlite directly. Unreviewed in depth (u); defer (Q9).

## 3. Nexus disposition (source to extract or retire, not a target)

Nexus: 42 crates under `apps/nexus/crates` (43 path entries in root `Cargo.toml`). `nexus-mcp` already uses `rusty-mcp`; no other Nexus crate has a path dependency on its workspace twin (only `rusty_url`, `rusty_ip`, `rusty_atomic_file` are shared).

| Nexus crate (LOC) | Workspace twin | Class | Disposition |
|---|---|---|---|
| nexus-vt (10.4k) | `libs/ui/rusty_term/src/core` (22.8k) | N, stale fork | identical: charset, inflate, png; near: base64, sixel; diverged: grid (2,562 vs 4,969 lines), parser, osc, kitty, jpeg …; lacks arabic/bidi/gif/webp. Header says "in-tree port". Retire; unique part is `lib.rs` (265-line headless `Vt` wrapper) |
| nexus-terminal (17.8k) | rusty_term backend/runtime | C | consumes nexus-vt; retire together. Does not reuse rusty_term PTY (`portable-pty`) |
| nexus-lsp (4.9k) / nexus-dap / nexus-acp / nexus-remote | `rusty_lsp` (18.7k) | N (JSON-RPC + Content-Length framing ×3 in Nexus, plus rusty_lsp, rusty_a2a) | retire; no extraction (non-Nexus sites: 2) |
| nexus-ai (provider clients) | §2.8 | D | retire; nothing generic missing except native Ollama client (if wanted, extract into the §2.8 crate) |
| nexus-memory(+hub), nexus-kv, nexus-storage, nexus-context | §2.9 | N/D | retire per decision; `nexus-storage` has no twin (tantivy FTS, vectorstore, bases) |
| nexus-security (4.1k) | `rusty_sandbox` (1.8k) | partial | both do Landlock+seccomp (crates vs raw syscalls via `rusty_libc`); credential vault, audit, tls_pins Nexus-specific |
| nexus-git (7.3k) | `rusty_git` (2.1k) | C | libgit2 vs pure Rust; no action |
| nexus-templates, nexus-skills, nexus-workflow | `rusty_jinja` | C | internal Nexus dup: frontmatter split ×4 (`nexus-skills/src/parse.rs:58`, `nexus-storage/src/parser.rs:256`, `nexus-formats/src/markdown/mod.rs:267`, templates), `{{}}` substitution ×3; die with Nexus |
| nexus-crdt, nexus-hashline, nexus-formats, nexus-theme, nexus-dap, nexus-workflow (cron) | none | Nexus-specific | extract **only if** a second consumer appears; none today |
| nexus-panic-log (411) | none | single-site | panic hook used by nexus-cli + shell only; retire or lift if another app wants it |

Retiring Nexus resolves outright: thiserror backlog −34, uuid −24, chrono, schemars/ts-rs (30/26), and 20+ rows' Nexus-side copies. It does **not** remove the non-Nexus duplicates in §1 rows 1–16, which are the real consolidation work.

## 4. Prioritized order (highest duplication, lowest risk first)

| Step | Rows | Why now | Gate |
|---|---|---|---|
| 1 | 1, 2 | 9 + 11 sites, security-relevant, trivial, tests exist | none |
| 2 | 3, 4 | additive helper with e2e guards; `rusty_ansi` already written and unused | none |
| 3 | 5, 6, 7 | ~25 hand copies; needs two foundation API additions (`rusty_url` percent pub, hex home) | Q4, Q5 |
| 4 | 8 | `rusty_retry` already adopted by 4 crates; finish it | `rusty_time` dependency check |
| 5 | 12 (rmcp pin only), 13 (app-level subset) | cheap hygiene before big moves | none |
| 6 | 14, 17 | home-dir + Nexus retirement | Q1, Q6 |
| 7 | 11, 12 | MCP client/server convergence onto `rusty_mcp`, one crate per PR: adk-mcp, rk-mcp, remind_me_remote, remind_me_mcp last | owner call on remind_me (Q2) |
| 8 | 9, 10, 16, 21 | HTTP server/client/Url/SSE convergence | Q2, Q3 |
| 9 | 15 | LLM wire types crate | Q10 |
| – | 18, 19, 20, 22 | no action / deferred | see questions |

## 5. Sovereignty notes (external deps with internal candidates; not source-confirmed)

From `scan_workspace_sovereignty.sh` (direct deps): serde_json 116, serde 97, thiserror 90, tokio 77, tracing 74, async-trait 43, uuid 32, chrono 31, schemars 30, ts-rs 26, toml 24, reqwest 20, rmcp 12, rusqlite 12, clap 11, regex-lite 10, axum 10.

| External | Internal candidate | Status |
|---|---|---|
| uuid (32, 24 Nexus) | `rusty_uuid` (14 dependents) | candidate; ADR-0002 Transitional |
| base64 (4, all Nexus) | `rusty_base64` (13) | candidate |
| serde_json/serde | `rusty_json`/`rusty_serde` (32/11 dependents) | partial by design; not relitigated |
| regex/regex-lite | `rusty_regx` | candidate (u) |
| toml (24) | `rusty_codec::toml` (673 LOC, 2 dependents) | partial (u) |
| eventsource-stream (a2a, acp) | `rusty_agui::sse::Decoder` | partial, §2.6 |
| sha2 (13), chrono, `hex` | no foundation sha256/hex found / `rusty_time` (u) | none found; parity-loop candidates |
| reqwest, rmcp, rustls, sqlx, rusqlite | Tier A or decided | not relitigated |

Orphaned first-party crates (built, zero dependents): `rusty_config`, `rusty_ansi`, `rusty_h2`, `rusty_rag`, `rusty_search` (outside itself), most of `rusty_oauth`. Adoption beats new code in several rows.

## 6. Open questions (need your decision)

| Q | Question | My recommendation |
|---|---|---|
| Q1 | Retire Nexus (delete the 42 crates, package.json/pnpm tree, `nexus-memory-hub` deploy units) or archive/freeze? It resolves most Nexus rows and is large and hard to reverse. | Freeze first (exclude from default workspace build/CI), then delete in a separate approved PR. Ask before either. |
| Q2 | remind_me deliberately hand-rolls HTTP (`remind_me_hub/src/http.rs` header) and MCP/OAuth (ADR-0010/0011). Keep that, or converge on `rusty_serve`/`rusty_mcp`? | Keep OAuth (ADR-0011). Converge MCP framing and std::net servers only after rows 1–8, one crate at a time. |
| Q3 | Is reqwest/axum/hyper policy "keep" (agentgateway, protocol libs) while only std::net copies move to `rusty_http`/`rusty_serve`? No ADR states this. | Yes. Align reqwest 0.12→0.13 and axum 0.7→0.8 only. Write a short ADR. |
| Q4 | Where does hex live (`rusty_codec`, `rusty_base64`, new `rusty_hex`)? | `rusty_codec` if it already hosts text encodings; else a function in `rusty_base64`. No new crate for 3 functions. |
| Q5 | May `platform/rustils` crates depend on foundation (`rusty_base64`)? ADR-0003 layer order was not re-read. | Check; if not allowed, leave trust_anchors copies and note why. |
| Q6 | Home/config dir: use existing `platform::portable-runtime` `config_dir`/`data_dir`, or add a foundation fn? No app calls the platform one. | Use the platform contract if layer rules allow; else a tiny foundation fn. |
| Q7 | CLI parsing: standardise on clap, a small sovereign arg crate, or leave as is? (ADR-0009 allows orch to stay hand-rolled.) | Leave. Revisit only if you want uniform `--version`/exit codes. |
| Q8 | Do error-type (thiserror→rusty_err) and tracing-init consolidation matter beyond Nexus? | No standalone project; fold into Nexus retirement. |
| Q9 | Embedding/cosine/RRF/search: pursue a shared retrieval crate, or leave `rusty_search`/`rusty_rag` as unused spikes (delete?)? | Defer; verify `rusty_search` has no planned consumer before deleting anything. |
| Q10 | LLM wire-types crate (row 15): worth L effort/high risk, given rusty_provider and agentgateway-llm already overlap as gateways? Maybe decide which gateway survives first. | Decide gateway overlap first; do not start the crate before that. |
| Q11 | `nexus-mcp` "step 2" (PR #500) is undefined in docs. Drop it (Nexus unused), or finish the `ConnectionPool`/`DynamicToolRegistry` extraction? | Drop unless Q1 says keep Nexus. |
| Q12 | Memory sync "lossless common context spec" (approved, unstarted): still wanted if Nexus is retired? | Likely moot; confirm. |

## 7. Not covered / follow-ups

- Rate limiters (agentgateway-core `ratelimit.rs`, rusty_provider, remind_me_core ×10 files, rusty_multimodal_db ×7) were listed, not compared.
- PTY/termios copies (rusty_term backend, rusty_lines `term_sys.rs`, `platform/rustils` pty/term, sessionmgr-pty) not compared.
- 36 files with hard-coded `\x1b[`; not classified.
- `rusty_tick/src/assistant.rs` third `Relay` instance, `rusty_meshed` async constraint, and `rusty_atomic_file` directory-fsync not verified.
- `rusty-hister-mcp` is an 18-line stub with no `rusty-mcp` dependency yet: a future call site, not a duplicate.
- Re-run repo-inspector after any extraction; it flags only same-name clusters and missed most rows above (found by pattern grep instead).
