# Changelog

All notable changes to **this monorepo itself** are documented here —
crate merges, workspace-wide CI, cross-crate changes. Each crate's own
internal changes are logged in that crate's own
`crates/<name>/CHANGELOG.md` where one exists (see ADR-0001 for why root
and per-crate logs are separate). Format: Added / Changed / Deprecated /
Removed / Fixed / Security, newest first.

## [Unreleased]
### Added
- `rusty-hister-model`'s `WebSession` query layer (`rusty_hister`'s Phase
  1, continued): `WebSession::{create, get, refresh, delete}`, a Rust
  port of `session.go`'s lookup/expiry helpers (`refresh`, not `update`,
  to avoid colliding with `#[derive(Mapped)]`'s own generated `update()`
  instance method). `create` is this crate's first database-assigned
  surrogate key — `rusty_db::Mapped::insert()` always supplies the
  primary key's current value, so a placeholder like `0` would either
  become the literal row id (SQLite) or be rejected outright (Postgres's
  `GENERATED ALWAYS AS IDENTITY`) — so `create` drops to a raw `INSERT`
  that omits the `id` column and recovers the generated value
  dialect-appropriately (`RETURNING id` where `Dialect::supports_returning()`
  is true, `SELECT last_insert_rowid()` otherwise), the same recipe every
  other autoincrementing model in this crate will need for its own
  `create`. The dialect-placeholder helper introduced for the embedding
  queue is now shared (hoisted to `lib.rs` on this second real call
  site). 7 new unit tests (50 total in the crate), clippy/fmt clean.
- `rusty-hister-model`'s embedding-queue query layer (`rusty_hister`'s
  Phase 1, continued): `EmbeddingJob::{enqueue, claim_next, complete,
  retry, fail, release, in_progress_exists, delete, reset_in_progress}`,
  a Rust port of `embedding.go`'s dedup/claim/retry state machine
  (capability inventory §5.9). `enqueue`'s `ON CONFLICT` upsert and
  `retry`/`release`'s `CASE`-based `SET` clauses need SQL the portable
  query builder can't express (`rusty_db::Update::set` only ever assigns
  a plain `Value`, never an `Expr`), so those three drop to
  `Engine::connect()`/raw `Connection::execute`, built dialect-portably
  by rendering placeholders through `Engine::dialect().placeholder(..)`
  rather than hardcoding `?`/`$N` — untested against real Postgres (no
  instance available in this environment), same risk profile as the
  schema increment's untested `POSTGRES_MIGRATIONS`. The other six model
  files' query-layer helpers remain a separate, not-yet-started
  increment. 18 new unit tests (43 total in the crate), clippy/fmt clean.
- `rusty-hister-model`'s schema (`rusty_hister`'s Phase 1, continued): the
  nine `#[derive(Mapped)]` types from Hister's `automigrate()` list
  (capability inventory §7.2 — `User`, `Link`, `History`, `HistoryLink`,
  `CrawlJob`, `CrawlURL`, `WebSession`, `DocumentVersion`, `EmbeddingJob`)
  on `rusty_db`, soft-delete via `rusty_db`'s `#[table(soft_delete)]`
  (replacing Go's nullable `DeletedAt` convention), and a fresh-install
  migration (`SQLITE_MIGRATIONS`/`POSTGRES_MIGRATIONS`) that creates all
  nine tables plus indexes via `rusty_db::Migrator` — whose own bookkeeping
  table substitutes for Hister's `Database` singleton-row version tracker.
  Deliberately schema-only: each Go model file's domain/query helpers
  (the embedding-queue state machine, history search/pin/timeline
  queries, user auth helpers, ...) are a separate, not-yet-started
  increment, and Hister's three historical migrations plus the legacy
  `indexer_versions` read path are not reproduced (they only matter for
  opening a pre-existing Hister-Go-created database file — a still-open
  question, see `crates/rusty_hister/docs/PROJECT-STATUS.md`). 25 unit
  tests (real SQLite round-trips via an in-memory engine, unique-
  constraint/duplicate-rejection checks, migration up/down/status),
  clippy/fmt clean.
- `rusty-hister-extractor`'s `Registry` (`rusty_hister`'s Phase 1,
  continued): the chain-of-responsibility mechanism (capability inventory
  §4.2) — case-insensitive `register`/`register_before` with duplicate
  rejection, the two-phase enrich-then-extract execution (enricher
  fallback never stops the chain, only abort does; enrichment carries
  forward into the extract phase), a separate preview chain with
  case-insensitive starting-point selection (unregistered/disabled/
  non-preview/non-matching starting points are hard errors), and
  `apply_configs` for merging pre-parsed per-extractor config. 18 unit
  tests, clippy/fmt clean. No concrete extractors yet — this is the
  generic mechanism they'll register into.
- `rusty-hister-core` (`rusty_hister`'s Phase 1): the `Document` working
  type, the `Extractor` trait and its supporting types (`Capabilities`,
  `ExtractorConfig`, `ExtractOutcome`/`PreviewOutcome`, `PreviewResponse` —
  capability inventory §4.1), and the shared `HisterError` type, on
  `rusty_json` (for `Metadata`) and `rusty_err` (for the error type). 16
  unit tests, clippy/fmt clean. First real implementation in the cluster;
  every other `rusty-hister-*` crate is still a skeleton.
- Bootstrapped `rusty_hister` — a native (not `git subtree`-imported) crate
  cluster under `crates/rusty_hister/` for a Rust port of
  [asciimoo/hister](https://github.com/asciimoo/hister). Eight new
  workspace members
  (`rusty-hister-{core,model,extractor,indexer,vectorstore,crawler,server,mcp}`),
  each an empty skeleton crate (no port logic yet). Full
  `rust-migration`-style capability inventory
  (`crates/rusty_hister/docs/capability-inventory/HISTER-CAPABILITY-INVENTORY.md`),
  a sovereignty audit of candidate `rusty_*` crates, and three ADRs
  (bootstrap/scope/split; two decision-requests for the search/indexing
  engine and the JS-rendering crawler approach, both since **decided** —
  see below) — see `crates/rusty_hister/docs/PROJECT-STATUS.md`.
### Changed
- `rusty_hister`'s ADR-0001 §7 licensing recommendation confirmed by the
  user: the cluster ships under this workspace's standard `MIT OR
  Apache-2.0`, and no Hister source file — test files included — is copied
  verbatim into it; extractor and query-grammar tests are written fresh
  from independently reading the Go behavior instead. Binds every future
  PR touching those tests.
- `rusty_hister`'s ADR-0002 (search/indexing engine) and ADR-0003
  (JS-rendering crawler) decided by the user, ahead of ADR-0002's own
  recommended scoping spike: `rusty_search` + `rusty-search-sqlite-fts5`
  for full-text search (multi-language federation, `url_re:` filtering, and
  highlight rendering built as `rusty-hister-indexer`-layer composition,
  not backend changes), `sqlite-vec`/`pgvector` for semantic search
  storage, `chromiumoxide` for the CDP crawler backend, and WebDriver BiDi
  explicitly **descoped** for v1 (not merely deferred). Unblocks
  `rusty-hister-indexer`, `rusty-hister-vectorstore`'s storage side, and
  `rusty-hister-crawler`'s CDP backend for implementation — none of which
  has started yet.
- CI's `plan` job no longer treats every root `Cargo.toml` edit as an
  automatic full workspace sweep. A new classifier
  (`.github/scripts/cargo_toml_diff.py`) distinguishes a pure
  `[workspace.members]` addition (new, independent crates; nothing else in
  the file touched — the common case when bootstrapping a new crate
  cluster, e.g. PR #170) from anything with broader blast radius (a
  `[workspace.dependencies]` version bump, a removed/renamed member, a
  `[profile]` edit); only the latter still forces `full=true`. A pure
  addition now goes through the normal affected-crates path instead,
  scoping CI to just the new crates.
- CI's scoped `cargo nextest run` step now passes `--no-tests=warn`
  instead of nextest's default `--no-tests=fail`: a scoped PR run whose
  affected packages collectively have zero tests (e.g. a PR that only adds
  brand-new skeleton crates, as `rusty_hister`'s bootstrap did) now logs a
  warning and passes instead of failing CI outright. A full sweep is
  unaffected (thousands of tests always exist there).
### Fixed
- 40 correctness/security/reliability findings from a third `/codex-build`
  review (`CODEX-MONOREPO-REVIEW-2026-09-12-round3.md`), across 14 crate
  families neither prior review round touched: `nexus-ai`/`nexus-ai-runtime`/
  `nexus-agent`, `rusty_provider`'s `cli`/`core`/`server`, `rusty_tailscale`'s
  `ts-derp`/`ts-stun`/`ts-disco`/`ts-filter`/`ts-key`, `nexus-kernel`/
  `nexus-collab`/`nexus-mcp`, standalone `rusty_mcp`/`rusty_croc`/
  `rusty_wiremock`/`rusty_homelab_mcp`, `rusty_fedora`(`_agent`),
  `rusty_codec`/`rusty_ansder`, the graphics stack, `rusty_whisper`/
  `rusty_llama`, `rusty_rusqlite`/`rusty_multimodal_db`, `rusty_inventrory`/
  `rusty_skillopt`, terminal/text utils, and low-level platform crates — each
  with a regression test that fails pre-fix and passes post-fix (see the
  review's own **Disposition** section for the one-line outcome of every
  finding). Also fixed round 2's own disclosed-but-unfixed `nexus-storage`
  Windows path-separator bugs (ground-truthed by actually running the suite
  on Windows: only 2 of the claimed 4 were real, both now fixed). Highlights:
  `nexus-mcp`'s dynamic-tool registry no longer lets any plugin route
  internal-only (Core-trust-gated) IPC handlers through the MCP server's own
  privileged context (confused deputy); `rusty_croc`'s receive path no
  longer follows a symlink planted earlier in the same transfer batch to
  write outside the destination root (zip-slip shape, previously defeated
  its own `.ssh` blocklist); `rp-server`'s JWKS verifier no longer amplifies
  an unauthenticated request into a fresh outbound fetch on every distinct
  unrecognized `kid`; `ts-derp`'s connect/handshake now times out instead of
  hanging forever against a non-responsive relay; five separate crash-on-
  untrusted-input bugs closed across `rusty_whisper`'s legacy `.bin`/GGUF/WAV
  loaders; three `rusty_font` TrueType-parser bugs closed, including an
  exponential composite-glyph blowup DoS; `rusty_win32`'s `to_wide()` no
  longer silently truncates at an embedded NUL (a validation-bypass shape)
  across roughly 40 call sites; `rusty_lines`' default Ctrl-W keybinding no
  longer panics on ordinary multi-byte Unicode whitespace; and four separate
  unbounded-recursion stack-overflow bugs closed in hand-rolled parsers
  (`rusty_codec::toml`, `rusty_llama::grammar`, `rusty_text::awk`), the same
  bug class round 2 already fixed elsewhere in this workspace.

### Fixed
- 63 correctness/security/reliability findings from a second `/codex-build`
  review (`CODEX-MONOREPO-REVIEW-2026-09-12.md`), across ~45 crates spanning
  the `nexus` microkernel, `rusty_adk`, `rusty_agent_gateway`,
  `rusty_tailscale`, `rusty_yirp`, `rusty_meshed`, `rusty_search`'s backend
  crates, `rusty_db`, `rusty_key`/`rusty_provider`, `rustils`/`rustils_async`,
  the homelab-management clients, and several standalone protocol crates —
  each with a regression test that fails pre-fix and passes post-fix (see
  the review's own **Disposition** section for the one-line outcome of
  every finding). Highlights: an unauthenticated pre-auth panic in
  `agentgateway-core`'s query-string decoder on any request containing a
  raw multi-byte UTF-8 byte after `%`; a CORS `allowOrigins: ["*"]` +
  `allowCredentials: true` misconfiguration now rejected at config-lint
  time instead of silently reflecting any origin with credentials;
  `ts-engine`'s inbound packet filter no longer trusts a decrypted
  WireGuard packet's plaintext source IP without verifying it against the
  sending peer's own netmap-assigned addresses (no cryptokey routing), and
  no longer default-allows non-IPv4-parseable packets (including all
  IPv6 traffic); `rusty_db`'s soft-delete path no longer bypasses
  optimistic locking, and `Migrator::up`/`down` no longer leak an open
  transaction back into the pool on a mid-migration failure; six
  `rusty_search` backends had Lucene/OData query-injection or path-
  injection gaps closed, and `rusty-search-cloud`'s backend — previously a
  silent no-op reporting fabricated success for every write — now returns
  an explicit not-implemented error; `rusty_request`'s redirect-following
  client now strips `Authorization` on an HTTPS→HTTP downgrade to the same
  host/port, closing the CVE-2018-18074-class gap its own doc comment
  claimed was already closed; workspace `reqwest` unified from a 0.12/0.13
  split onto 0.13 (two follow-on feature additions, `query` and `form`,
  that the version bump itself required); `rmcp`'s and `axum`'s remaining
  version splits (`rk-mcp`/`rk-app` on `rmcp` 0.9 against the root's 3.1;
  `adk-a2a`/`a2a-agent-server`/`rk-app` on `axum` 0.7 against the root's
  0.8) were investigated for a same-session bump and found to be genuine
  API breaks rather than mechanical version-number edits — documented as
  deliberate, tracked splits instead, matching this file's own established
  convention for the workspace's other pre-existing version splits.
  Wiring a real `cargo-deny` CI job for the four crates that carry a
  `deny.toml` (previously claimed by two of those files but not actually
  enforced) was attempted and reverted: running it against this
  workspace's real, shared `Cargo.lock` surfaced a large unrelated
  pre-existing backlog (two Wasmtime CVEs, an `rmcp` DNS-rebinding CVE, a
  yanked crate, six unmaintained-crate advisories, license-metadata gaps
  in this workspace's own `rusty_rusqlite`/`rusty_stream`, and dozens of
  `[bans]` failures against this workspace's own deliberate
  duplicate-version splits) that is a separate, open-ended dependency-audit
  project. The four `deny.toml` files now honestly document themselves as
  local/manual tools, not CI-enforced, and each carries a documented
  `ignore` entry for RUSTSEC-2023-0071 (`rsa` 0.9.x, Marvin Attack, present
  via `jsonwebtoken`'s `rust_crypto` feature in `agentgateway-auth`'s/
  `rusty-mcp`'s production JWT dependency graph, reviewed: only the
  verification path uses it, not the vulnerable signing/decrypt path) for
  a maintainer running `cargo deny check` by hand.

### Fixed
- CI's full-workspace-sweep trigger pattern now includes `.config/`
  (`.config/nextest.toml` lives outside every crate directory, so a
  nextest-config-only PR previously produced an empty affected-package
  list and skipped build/test/clippy for a change that governs every
  crate's test execution).
### Added
- `rusty-config-no-std-check` CI job: `cargo check -p rusty_config
  --no-default-features --all-targets`, exercising `rusty_config`'s
  `no_std`+`alloc` code path for the first time — `--all-features` alone
  can never reach it, since the crate's own default is `default =
  ["std"]` (`CODEX-MONOREPO-REVIEW.md` finding #33).
- `rusty_uuid` gained `.simple()` formatting (32 lowercase hex digits, no
  hyphens) and an optional `rusty_serde`-backed `Serialize`/`Deserialize`
  (canonical hyphenated string) behind a new `rusty_serde` feature —
  deliberately built on this workspace's own dependency-free `rusty_serde`
  rather than external `serde` (repo-inspector Section 2 "uuid" row).
  Does **not** yet unblock `rusty-acp`/`rusty-db-core` dropping external
  `uuid`: both use external `serde` (a different trait than
  `rusty_serde`'s) for their own derives, and both also need `sqlx`
  wire-format support this pass deliberately left untouched — no live
  Postgres/MySQL was available to verify a hand-rolled UUID column
  encoding, and a wrong one would silently corrupt data. Same open
  prerequisite the report named, now documented more precisely.
### Changed
- `adk-sessions`, `rp-router`, `rk-feed`, and `inventory-core` now depend
  on `rusty_sqlite::rusqlite` instead of external `rusqlite` directly
  (repo-inspector Section 2 "rusqlite" row — a pure re-export, so this is
  an import-path change only, zero behavior change).
- `rusty_json`'s `serde` dependency is now optional, behind a `serde`
  feature that stays on by default (zero behavior change for existing
  dependents). With `default-features = false`, the crate has no `serde`
  dependency at all: `Value` parsing (`s.parse::<Value>()` /
  `Value::from_json_str`) and writing (`Value::to_json_string`/
  `Value::to_json_string_pretty`) work via a new direct recursive-descent
  path (`src/value_io.rs`) that reuses the existing hand-rolled tokenizer
  (`src/parser.rs`) and `Formatter` trait instead of going through
  `serde::Deserializer`/`Serializer`. Unblocks repo-inspector Section 1 row
  7 (`rusty_oauth`/`rusty_request`'s hand-rolled `Value` types, hand-rolled
  specifically to avoid a `serde` dependency) from adopting `rusty_json` —
  those two crates' own migration is a separate follow-up, not done here.

### Added
- `crates/rusty_multimodal_db` — new workspace member: `baileyrd/rusty_multimodal_db`,
  a benchmark harness comparing AoS, SoA, and UUID-canonical-store record
  backends, plus the production store, network server, and schema-driven
  client built on the winning design, merged via `git subtree` with full
  history. Already a single, non-nested `Cargo.toml`, so nothing to
  de-nest on merge (unlike `nexus`/`rusty_agent_gateway`/`rusty_yirp`).
- `crates/nexus` — new workspace members: `baileyrd/nexus`, a 42-crate
  microkernel note-taking/AI-agent workspace, merged via `git subtree`
  with full history (separate from the `baileyrd/rusty_*` wave numbering).
  Its own nested `[workspace]`/`Cargo.lock` and `.cargo/config.toml`
  (target-dir override, redundant now that this root's own `target/`
  covers it) were dropped; `crates/nexus/shell` (its Tauri desktop shell)
  is `exclude`d the same way as `rusty_key`'s `desktop/src-tauri`.
- `crates/rusty_rand` — new workspace member: OS-backed CSPRNG bytes
  (`fill`/`bytes`, `Result`-returning; cached `/dev/urandom` handle on
  Unix, hand-declared `BCryptGenRandom` FFI on Windows), no external
  dependencies. Extracted from three identical copies in `rusty_oauth`,
  `rusty_uuid`, and `sessionmgr-proc`, all of which now wrap it
  (repo-inspector Section 1 row 6).
- `rusty_simd::f32_to_f16` — the reverse of the existing `f16_to_f32`
  (round-to-nearest-even, NaN preserved, overflow → ∞, exhaustive
  round-trip test); `rusty_llama` and `rusty_whisper` re-export both
  directions instead of carrying their own (rows 3–4).
- `rusty_wiremock::canned` (behind a new `std` feature) — a working
  sequential canned-response HTTP mock server, moved out of the four
  identical `tests/support/mod.rs` copies in `rusty_proxmox`,
  `rusty_opnsense`, `rusty_fedora`, and `rusty_homelab_mcp` (row 8).
- `repo-inspector-report.md` gained a **Disposition** section recording
  what was done, or deliberately not, for every row of both sections.
### Changed
- `rusty_multimodal_db` added to the `windows-latest` `windows-exclude`
  list (alongside `rusty_stream`/`rusty_fedora_agent`) — its optional
  `external-db-bench` feature's `duckdb` dependency vendors DuckDB's own
  C++ amalgamation, and this workspace's `--all-features` is what first
  compiles it on `windows-latest`; that native build fails under the
  runner's current MSVC toolchain, a third-party build issue with no
  Rust-side fix available here. The crate's own standalone repo never ran
  a Windows CI job at all, so this wasn't a regression, just first
  exposure.
- `rusty_multimodal_db`'s pinned git dependency on `rusty_tls`
  (`Rusty-Mill/rusty_mill` at a specific commit) retired to a plain path
  dependency on this workspace's own `crates/rusty_tls`, now that both
  live in the same workspace (ADR-0002's same-workspace-source rule) —
  the same swap `rusty_yirp`'s `sessionmgr-pty` and `nexus-rush` made.
- `rusty_multimodal_db`'s `rusqlite` pin (its optional
  `external-db-bench` benchmark feature) bumped `0.32` → `0.39` to match
  `inventory-core`'s existing pin — `rusqlite` declares
  `links = "sqlite3"`, and Cargo allows only one version of a
  `links`-declaring crate in the whole graph; the same fix the
  `sqlx`/`rusqlite` collision below needed.
### Fixed
- `rusty_multimodal_db`'s two `clippy::chunks_exact_to_as_chunks`
  failures (`src/durability/mmap_store.rs`, `src/server/pem.rs`) — this
  workspace's clippy version flags `chunks_exact(N)` with a constant `N`
  in favor of `as_chunks::<N>().0`; behavior unchanged, same
  trailing-partial-chunk drop either way. The upstream repo hit the
  identical failure on its own `main` independently of this merge (its
  clippy toolchain updated on its own) and carries the same fix.
### Changed
- `sqlx` bumped `0.8` → `0.9`, workspace-wide: `sqlx-sqlite` 0.8.x pins
  `libsqlite3-sys ^0.30.1`, which collided (Cargo's `links = "sqlite3"`
  uniqueness rule) with the `libsqlite3-sys ^0.37` that nexus's
  `rusqlite` 0.39 needs; `sqlx-sqlite` 0.9.0 widens its own range to
  `>=0.30.1, <0.38.0`, admitting both. `rusty_acp`'s own `sqlx` pin
  (`postgres-store` feature) and `rusty-db-postgres`/`rusty-db-mysql`/
  `rusty-db-sqlite` bumped with it; each crate's dynamic-SQL call sites
  (`sqlx::query(&format!(...))`-shaped) needed wrapping in
  `sqlx::AssertSqlSafe` for sqlx 0.9's new `SqlSafeStr` injection-audit
  bound — all of them build SQL from a config-supplied table name/prefix
  (`rusty-db-*`'s generic `Executor` trait boundary, `rusty_acp`'s
  `table_prefix`) or a caller-supplied connection-hook string, not from
  request data, so the wrap is a straightforward assertion rather than a
  behavior change.
- `rusqlite` bumped `0.32.1` → `0.39` at the workspace root (the version
  nexus's own manifest asked for) for the same `links` reason;
  `inventory-core`, `rusty_sqlite`, and `rk-feed` (previously pinned to
  `0.32.1`/`0.32` for their own reasons — an `sqlx-sqlite` unification and
  a `libsqlite3-sys` stable-toolchain `cfg_select` issue, respectively,
  both still satisfied at `0.39`) now share the one workspace version
  Cargo's `links` uniqueness rule requires.
- `nexus-memory`'s `Memory`/`MemoryType`/`MemoryStatus` (`model.rs`) now
  derive `TS`/`JsonSchema` behind the `ts-export` feature, and its
  `ts-rs`/`schemars` deps gained the `chrono`/`uuid` impl features they
  need — a latent gap in nexus's own manifest (not exercised by nexus's
  own `scripts/check_ipc_drift.sh`, which never built `nexus-memory`)
  that only surfaced once this workspace's own CI compiled it with
  `--all-features`.
- `nexus-rush`'s job-spawn `platform::process::Command` literal gained an
  explicit `detached: false` — a field added to this workspace's own
  `rustils` fork after nexus's former pinned `rustils` git rev, now
  exposed by retiring that pin to this root's path dependency (ADR-0002).
- `rusty_base64`'s decoder now rejects misplaced or excess `=` padding
  (`Z=9v`, `Zm9v====`, `Zg==Zg==`) and a padded input that is not
  4-aligned, instead of stripping every `=` and guessing; `DecodeError`
  variants carry the offending index/byte/length. Missing padding is
  still accepted (base64url needs it).
- The last three hand-rolled base64 copies (`rusty_request`, `ts-control`,
  `sessionmgr-protocol`) and the last four external `base64` crate users
  (`adk-a2a`, `rusty-croc`, and `agentgateway`/`agentgateway-auth`'s test
  suites) all use `rusty_base64`; external `base64` is gone from every
  workspace manifest (Section 1 row 5 + Section 2 `base64`).
- `adk-core` mints ids with `rusty_uuid` instead of external `uuid`;
  `rk-feed` validates egress URLs with `rusty_url` instead of external
  `url` (Section 2 `uuid`/`url`).
- `sessionmgr-proc` no longer needs `windows-sys`'s
  `Win32_Security_Cryptography` feature (its `os_random` is `rusty_rand`).
- `crates/rusty_fedora_agent` — new workspace member: an unprivileged local
  agent exposing scoped systemd/dnf/config-file control over a small
  synchronous (`tiny_http`) HTTP API, for managing a Fedora Server host
  (e.g. baileyai) that has no REST management API of its own. Built on
  `rustils`' `platform`/`platform-linux` process-spawning layer
  (`SystemController`/`PackageController` ports, `SystemdAdapter`/
  `DnfController` adapters); privilege scoping (polkit unit allowlist,
  sudoers-scoped `dnf install`/`remove`, config-path allowlist with
  automatic `.bak` on write) ships as reviewable templates under
  `deploy/`, not applied automatically. `tiny_http` is a new external
  dependency (workspace-hoisted) — deliberately synchronous, no
  tokio/axum, matching `rustils`' own reasoning for keeping tokio out of
  its platform layer.
- `crates/rusty_fedora` — new workspace member: async typed client for
  `rusty_fedora_agent`'s HTTP API, same shape as `rusty_opnsense`/
  `rusty_proxmox` (built on `rusty_request`, passthrough JSON).
- `rusty_homelab_mcp` gained a `fedora` module: 10 new tools
  (`fedora_system_status`, `fedora_list_services`,
  `fedora_service_control`, `fedora_read_journal`,
  `fedora_dnf_list_updates`, `fedora_dnf_install`/`fedora_dnf_remove`,
  `fedora_task_status`, `fedora_read_config`/`fedora_write_config`),
  following the existing OPNsense/Proxmox discovery-then-mutate and
  `$defs` enum conventions exactly.
- `rusty_base64` — hand-rolled, dependency-free Base64 (RFC 4648, standard
  and URL-safe alphabets, encode/decode) extracted from
  `rusty_oauth::encoding::base64`, closing issue #119. `rusty_oauth` now
  depends on it too (dogfooding); `rusty_acp`, `rusty-mcp`, and `rusty_a2a`
  swapped their external `base64` dependency for it after their exact
  call-site needs were verified. Chunking uses `chunks_exact`/`remainder`
  rather than `rusty_oauth`'s original `slice::as_chunks`, which is not
  stable at `rusty_acp`'s `rust-version = "1.86"` floor (confirmed against
  a real `+1.86` toolchain before merging). `rusty_croc`, `adk-a2a`,
  `agentgateway-auth`, and `agentgateway` still depend on external
  `base64` — out of this issue's verified scope, left for separate
  follow-up.
- `crates/rusty_meshed/crates/rusty-meshed-trace` — reverse-trace and
  domain-maturity model for `rusty_meshed` (maturity ladder, scenario types,
  pure `trace()` with fidelity verdict and worst-first bottlenecks, TOML
  scenarios via `rusty_codec`, JSON via `rusty_json`, Markdown gap summary,
  one shipped scenario); new workspace member
- `docs/adr/0002-dependency-sovereignty-policy.md` — a Sovereign /
  Transitional / Adapter tier classification for crate external-dependency
  posture, plus `.github/scripts/check_workspace_deps.py` (with unit tests)
  and a `dependency-policy` CI job enforcing `ATLAS-RWC-0050`: no workspace
  member may resolve from a git source anywhere in the dependency graph
### Fixed
- `crates/rusty_tokio`'s Windows reactor discarded the result of
  re-arming a socket's one-shot `IOCTL_AFD_POLL` after every completion;
  a failed re-arm silently stopped monitoring that socket forever,
  hanging any later `readable()`/`writable()` wait on it — surfaced as
  intermittent ~600s timeouts on unrelated `rusty_tokio`/`rusty_tls`
  tests on `test (windows-latest)` (#153). Both re-arm sites now mark
  both directions ready on a failed resubmission instead of hanging
- `crates/rustils/crates/platform-linux`'s `LinuxPty::spawn` set a
  session's window size *after* spawning the hosted child, racing the
  child's own reads of its terminal size against the resize ioctl;
  surfaced as an intermittent `sessionmgr-pty` test flake on `main`
  (#150). Reordered to size the pty before the child starts
- `crates/rusty_term/l13`, `crates/rusty_font`, and `crates/rusty_gpu`
  resolved `rusty_lsp`/`rusty_simd` via a pinned git dependency instead of
  the workspace's own copy of those crates; switched to path dependencies
### Added
- `.github/scripts/test_affected_crates.py` — unit tests for the CI plan
  step's ownership and reverse-dependency logic (nested crates, directory
  name prefixes, transitive dependents, external deps, cycles), run by a
  new `plan-tests` job; `affected_crates.py`'s graph logic moved into an
  `affected_packages()` function so the tests can drive it without cargo
- `docs/atlas/` — the Rusty Mill → Atlas evidence review (revision 2,
  verified against Rusty Mill `06ca8669` and Atlas `390d6b0f`) and the
  list of corrections applied to its first revision
- `rusty_croc` merged into `crates/rusty_croc` via `git subtree` (fourth
  wave), full history preserved
- `rusty_test` merged into `crates/rusty_test` via `git subtree` (fourth
  wave) — six crates (`contract`, `compat`, `conformance`, `stat-tool`,
  `proc-runner`, `pty-shell`) behind one nested workspace
- `rusty_inventrory` merged into `crates/rusty_inventrory` via `git subtree`
  (fourth wave) — three crates (`inventory-core`, `inventory-cli`,
  `inventory-tauri`) behind one nested workspace
- `rusty_skillopt` merged into `crates/rusty_skillopt` via `git subtree`
  (fourth wave) — four crates (`skillopt-core`, `skillopt-model`,
  `skillopt-envs`, `skillopt-cli`) behind one nested workspace
- `rusty_key` merged into `crates/rusty_key` via `git subtree` (fourth
  wave) — eight crates (`rk-config`, `rk-observe`, `rk-constrain`,
  `rk-feed`, `rk-kernel`, `rk-mcp`, `rk-compose`, `rk-app`) behind one
  nested workspace; its Tauri desktop shell stays excluded, as upstream
  had it
- `rusty_llama` merged into `crates/rusty_llama` via `git subtree` (fourth
  wave) — a single crate; its `rusty_simd`/`rusty_std` path dependencies
  already resolved to merged siblings
- `rusty_tailscale` merged into `crates/rusty_tailscale` via `git subtree`
  (fourth wave) — fifteen `ts-*` crates plus `xtask` behind one nested
  workspace; its `platform`/`platform-linux` git pins retired to this
  root's `rustils` path dependencies
- `rusty_adk` merged into `crates/rusty_adk` via `git subtree` (fourth
  wave) — eleven `adk-*`/`rusty-adk` crates plus three examples behind one
  nested workspace; `adk-a2a`'s branch-tracking `rusty_a2a` git dependency
  retired to a path dependency on the merged sibling
- `rusty_provider` merged into `crates/rusty_provider` via `git subtree`
  (fourth wave) — six `rp-*` crates behind one nested workspace; its
  branch-tracking `rusty-mcp` git dependency retired to a path dependency
  on the merged sibling
- `rusty_yirp` merged into `crates/rusty_yirp` via `git subtree` (fourth
  wave) — eight `sessionmgr-*` crates plus a Tauri desktop shell behind one
  nested workspace; its `rusty_tokio` pin and `sessionmgr-pty`'s three
  `rustils` pins retired to this root's path dependencies
- `rusty_agent_gateway` merged into `crates/rusty_agent_gateway` via
  `git subtree` (fourth wave, and the last of it) — nine
  `agentgateway-*` crates behind one nested workspace; four pins retired
  (`rusty_a2a`, `rusty-mcp` at tag `v0.4.1`, `rusty_tls`, `rusty_tokio`)
- CI's Linux leg now installs `libwebkit2gtk-4.1-dev`, `libgtk-3-dev`,
  `libayatana-appindicator3-dev`, `librsvg2-dev`, and `libdbus-1-dev` for
  `inventory-tauri` and `inventory-core`'s Secret Service keyring backend

### Changed
- `CONTRIBUTING.md` review policy: one independent approval when a reviewer
  is reasonably available; otherwise the author self-reviews against the
  reviewer checklist after CI is green and records that in the PR
  description. Self-review is never represented as independent review, and
  security-sensitive, irreversible, or ecosystem-breaking changes still wait
  for an independent reviewer. All four PR templates gain a matching
  checklist line
- The last `git` pins on `baileyrd/rustils` retired to workspace `path`
  dependencies: `rusty_tokio` (rev `ce9259d4`), `rusty_tls` (rev `93b00ce9`,
  platform 0.22.1), and `rustils_async`'s root plus four member crates (rev
  `83ab7a9e`). Thirteen git-sourced `platform*` lockfile entries collapse
  into the single in-tree 0.27.0 copy of each crate
- Root `[workspace.dependencies]`: `tokio`'s feature list widened to the
  union `rusty_search`/`rusty_db`/`rusty_skillopt`/`rusty_adk` need (`fs`,
  `process`, `io-util`, `io-std`); `chrono` gained `serde` for
  `skillopt-core`; `uuid` gained `serde` and `serde_json` gained
  `float_roundtrip` for `rusty_adk`; `reqwest` gained `stream` and `tokio`
  gained `full` for `rusty_provider`; `clap` gained `env` for
  `rusty_agent_gateway`
- Root `rusty_a2a` and `rusty-mcp` entries carry `default-features = false`
  so `rusty_agent_gateway`'s crates can inherit them — a no-op for their
  other consumers, since both crates' `default` feature is empty

### Fixed
- Two `__pycache__/*.pyc` files committed alongside the affected-crates
  tests removed; `__pycache__/` and `*.pyc` are now ignored
- `rusty_croc`'s four deprecated `GenericArray::from_slice` nonce
  constructions rewritten to the equivalent `From<&[T]>` conversion — the
  workspace resolves `generic-array` 0.14.9, where they fail this repo's
  `-D warnings` clippy gate
- `conformance`'s `layering.rs` layer-boundary check repointed at the
  monorepo root and scoped to `crates/rusty_test/` — it had located the
  workspace manifest two directories up and demanded a layer assignment for
  every member it found
- `inventory-core` pinned to `rusqlite = "0.32.1"` (from `"0.37"`): its
  `libsqlite3-sys ^0.35` requirement conflicts with `sqlx-sqlite`'s
  `^0.30.1` on the `sqlite3` `links` key, the same constraint `rusty_sqlite`
  hit; 79 tests pass unmodified against the pin
- `inventory-core`'s three deprecated `GenericArray::from_slice` calls in
  `db.rs` rewritten, same `generic-array` 0.14.9 cause as `rusty_croc`'s
- `rusty_key`'s eight crates keep a literal `[lints.rust] unsafe_code =
  "forbid"` instead of inheriting this root's `[workspace.lints]`, which is
  `rustils`' weaker `"warn"` — inheriting would have silently downgraded
  the policy
- `crates/rusty_key` reformatted with `cargo fmt --all` (it was not
  fmt-clean under this workspace's settings)
- `rusty_llama`'s two `unnecessary_cast` lints in `backend/cuda.rs`'s test
  fixtures, only visible with `--all-features`
- `rusty_llama`'s `render_jinja_threads_context_variables` expectation
  updated from `true` to `True`: `minijinja` 2.22 changed bool rendering
  for Jinja2 compatibility, and this workspace resolves 2.24 where the
  standalone lockfile pinned 2.21
- `crates/rusty_llama` reformatted with `cargo fmt --all`
- `ts-magicsock` now imports `platform::net::UdpSocket`, without which
  `send_to`/`recv_from`/`local_addr` do not resolve — a pre-existing break
  in `rusty_tailscale`'s own `main` (it has no CI), confirmed against the
  standalone repo
- `ts-cli`'s `localapi::Error::Status(StatusCode)` replaced with the
  `Api { status, body }` variant `request()` actually constructs — the same
  pre-existing break
- Four more `generic-array` 0.14.9 deprecations across `ts-control`,
  `ts-disco` and `ts-derp`; `crates/rusty_tailscale` reformatted
- `adk-sessions` moved from `rusqlite = "0.37"` to this root's `"0.32.1"`
  entry — the same `libsqlite3-sys` `links` conflict `inventory-core` hit,
  and the same resolution
- README: `rusty_simd` is no longer "still outstanding", `rusty_tokio` has
  nineteen in-repo dependents rather than none, and `rustils` is in-tree
  (its surviving `git` pins in `rusty_tokio`/`rustils_async`/`rusty_tls`
  are noted as not yet retired). ARCHITECTURE: the ATLAS-300 reference no
  longer describes it as a seed and points at `docs/atlas/` for the
  crosswalk. `docs/atlas/`: the `rusty_tokio` dependent count corrected to
  the `cargo metadata` figure

## [workspace] - 2026-09-01
### Fixed
- `rusty_rdp`'s hand-rolled byte cursor deduplicated against `rusty_wire`
  ([#65](https://github.com/Rusty-Mill/rusty_mill/pull/65))
- Six workspace-wide duplication findings resolved (glob matching, SHA-1,
  `to_wide()`, `read_lines()`, Windows raw-mode flags, IFS splitting)
  ([#10](https://github.com/Rusty-Mill/rusty_mill/pull/10))

### Changed
- `rusty_ansder` split into itself (DER codec only) and a new `rusty_rag`
  crate (RAG/Q&A engine) ([#65](https://github.com/Rusty-Mill/rusty_mill/pull/65))

<!-- No version tags on this repo as such — "[workspace] - DATE" entries
     group changes to the monorepo's own build/governance surface, distinct
     from any per-crate version a crate under crates/<name> might carry on
     its own. Earlier crate-import history isn't backfilled here entry-by-
     entry; see RELEASE_NOTES.md's note on that and `git log --oneline
     --merges` for the full list. -->
