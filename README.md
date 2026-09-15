# rusty_mill

The Rusty Mill monorepo: a Cargo workspace consolidating previously
standalone `baileyrd/*` crates into one repository, one build, and one CI
pipeline. Each crate keeps its full original commit history, merged in via
`git subtree` under `crates/`.

A first wave merged fourteen crates (below the `rusty_term` row through
`rusty_text`). A second wave added fifteen more (`rusty_tokio` through
`rustils_async`), and a third wave twenty-six more (`rusty_wire` through
`rusty_voice`). A fourth wave merged the last eleven standalone
`baileyrd/*` repos — `rusty_croc`, `rusty_test`, `rusty_inventrory`,
`rusty_skillopt`, `rusty_key`, `rusty_llama`, `rusty_tailscale`,
`rusty_adk`, `rusty_provider`, `rusty_yirp`, and `rusty_agent_gateway` —
one crate at a time, the same way. All four waves are complete: every
`baileyrd/rusty_*` repo that was in scope now lives under `crates/`, and
the standalone repos carry an archive notice pointing here.

A fifth merge, outside that `baileyrd/rusty_*` wave numbering, brought in
`baileyrd/nexus` — a 42-crate microkernel note-taking/AI-agent workspace,
not a `rusty_*`-prefixed leaf crate — under `crates/nexus/`, keeping the
same `git subtree` process. Its own nested `[workspace]` and `Cargo.lock`
were dropped the same way rusty_agent_gateway's and rusty_yirp's were; its
42 crates join this workspace's member list directly rather than through
their own path prefix, since nexus's crate names (`nexus-*`) don't
collide with anything already here. `crates/nexus/shell` (nexus's own
Tauri desktop shell, `shell/src-tauri`) is `exclude`d the same way as
`rusty_key`'s `desktop/src-tauri` — it's a separate pnpm-driven Tauri
workspace, not a `cargo test` target.

A sixth merge, also outside the wave numbering, brought in
`baileyrd/rusty_multimodal_db` — a single-crate benchmark harness for
record-store backend design — under `crates/rusty_multimodal_db/`, the
same `git subtree` process. Its one pinned git dependency on `rusty_tls`
(`Rusty-Mill/rusty_mill`, a specific commit) retired to a plain path
dependency on this workspace's own `crates/rusty_tls` (ADR-0002), and its
`rusqlite` pin bumped `0.32` → `0.39` to match `crates/rusty_inventrory`'s
`inventory-core` — `rusqlite`'s `links = "sqlite3"` key allows only one
version in the whole dependency graph, the same constraint the `nexus`
merge hit.

A seventh addition, `rusty_hister` under `crates/rusty_hister/`, is fresh
work rather than a merge: a Rust port of
[asciimoo/hister](https://github.com/asciimoo/hister) (AGPL-3.0-or-later),
bootstrapped as eight native crates (`rusty-hister-core` through
`rusty-hister-mcp`, see `crates/rusty_hister/README.md`) with a full
capability inventory and two open decision-requests (search engine, JS-
rendering crawler) — no port implementation lands until those are decided.

An eighth addition, `rusty_proxmox`, `rusty_opnsense`, `rusty_homelab_mcp`,
`rusty_fedora_agent`, and `rusty_fedora`, is a homelab-infrastructure
cluster: Proxmox VE and OPNsense REST clients, an MCP server exposing that
homelab control as tools, and an agent/client pair for scoped local Fedora
host control. Unlike the numbered waves and the nexus/`rusty_multimodal_db`/
`rusty_hister` merges above, no record of when or how these 5 crates landed
survives — they predate `RELEASE_NOTES.md`'s earliest recorded entries, so
their arrival is placed here, alongside the broader crate build-out,
without a specific date.

## Crates

| Crate | Path | Purpose |
|---|---|---|
| [`rusty_term`](crates/rusty_term) | `crates/rusty_term` | Terminal emulator (VT/ANSI parser, optional native GUI/GPU backends) |
| [`rusty_term_l13`](crates/rusty_term/l13) | `crates/rusty_term/l13` | `rusty_term`'s L13 structured side-channel (MCP + LSP/ACP over private OSC) |
| [`rusty_gpu`](crates/rusty_gpu) | `crates/rusty_gpu` | `no_std` software framebuffer presenter and SIMD rasterizer |
| [`rusty_gui`](crates/rusty_gui) | `crates/rusty_gui` | `no_std` windowing, event loop, and clipboard manager |
| [`rusty_font`](crates/rusty_font) | `crates/rusty_font` | `no_std` TrueType/OpenType parser and glyph rasterizer |
| [`rusty_regx`](crates/rusty_regx) | `crates/rusty_regx` | Zero-dependency, linear-time POSIX ERE regex engine |
| [`rusty_win32`](crates/rusty_win32) | `crates/rusty_win32` | Minimal-dependency Win32 API wrapper (leaf crate) |
| [`rush`](crates/rush) | `crates/rush` | A small, bash-compatible shell |
| [`rusty_lines`](crates/rusty_lines) | `crates/rusty_lines` | Hand-rolled readline alternative (emacs/vi keymaps, history, completion hooks) |
| [`mill-term`](crates/mill-term) | `crates/mill-term` | Integrated terminal + environment launcher hosting `rush` inside `rusty_term` |
| [`rpath`](crates/rpath) | `crates/rpath` | Path translation/normalization for MSYS2/Git Bash/POSIX ↔ Windows |
| [`rusty_git`](crates/rusty_git) | `crates/rusty_git` | Pure-Rust Git object model, index, refs, and `rgit` CLI |
| [`rusty_diff`](crates/rusty_diff) | `crates/rusty_diff` | Myers/Patience diff algorithms, unified diff formatting, patch application |
| [`rusty_compress`](crates/rusty_compress) | `crates/rusty_compress` | Sans-IO DEFLATE/Gzip/Zlib/LZMA stream compression |
| [`rusty_text`](crates/rusty_text) | `crates/rusty_text` | Pure-Rust sed (`rsed`) and awk (`rawk`) engines |
| [`rusty_tokio`](crates/rusty_tokio) | `crates/rusty_tokio` | Hand-rolled, from-scratch async runtime: work-stealing scheduler, epoll/io_uring reactor, timers, async sync primitives |
| [`rusty_tokio-macros`](crates/rusty_tokio/rusty_tokio-macros) | `crates/rusty_tokio/rusty_tokio-macros` | `rusty_tokio`'s `#[main]`/`#[test]` proc-macro attributes |
| [`rusty_rusqlite`](crates/rusty_rusqlite) | `crates/rusty_rusqlite` | Pure-Rust, from-scratch SQLite reimplementation aiming for `rusqlite` API parity |
| [`rusty_libc`](crates/rusty_libc) | `crates/rusty_libc` | `no_std`, zero-dependency, Linux-only raw-syscall replacement for the `libc` crate |
| [`rusty_acp`](crates/rusty_acp) | `crates/rusty_acp` | Agent Communication Protocol (ACP) v0.2.0: protocol types, an HTTP client, and a server framework for hosting agents |
| [`rusty_tls`](crates/rusty_tls) | `crates/rusty_tls` | A `rustls`-based TLS library, with an optional `rusty_tokio`-backed async stream and an experimental hand-rolled record-layer engine |
| [`rusty_serde`](crates/rusty_serde/rusty_serde) | `crates/rusty_serde/rusty_serde` | Hand-rolled, dependency-free `Serialize`/`Deserialize` data model plus JSON and RON-inspired formats |
| [`rusty_serde_derive`](crates/rusty_serde/rusty_serde_derive) | `crates/rusty_serde/rusty_serde_derive` | `rusty_serde`'s `#[derive(Serialize, Deserialize)]` proc-macro, hand-written directly on `proc_macro` (no `syn`/`quote`) |
| [`rusty_serde_erased`](crates/rusty_serde/rusty_serde_erased) | `crates/rusty_serde/rusty_serde_erased` | Minimal unsafe primitive erasing a serializer/deserializer's associated `Ok` type across an object-safe boundary — internal to `rusty_serde` |
| [`rusty_lsp`](crates/rusty_lsp) | `crates/rusty_lsp` | Small, reusable async Language Server Protocol framework: own the protocol plumbing, implement one trait for your language |
| [`rusty_a2a`](crates/rusty_a2a) | `crates/rusty_a2a` | Reusable implementation of the Agent2Agent (A2A) protocol: JSON-RPC/REST/gRPC transports, client and server |
| [`rusty-mcp`](crates/rusty_mcp/crates/rusty-mcp) | `crates/rusty_mcp/crates/rusty-mcp` | Reusable scaffold for building Model Context Protocol servers, built on `rmcp` |
| [`rusty-mcp-demo`](crates/rusty_mcp/crates/rusty-mcp-demo) | `crates/rusty_mcp/crates/rusty-mcp-demo` | Example MCP server built on the `rusty-mcp` scaffold |
| [`rusty_stream`](crates/rusty_stream) | `crates/rusty_stream` | Single-node durable log, built on `rusty_wire` and `rusty_tokio` |
| [`rusty_url`](crates/rusty_url) | `crates/rusty_url` | From-scratch WHATWG URL Standard implementation, aiming for parity with the `url` crate |
| [`rusty_http`](crates/rusty_http) | `crates/rusty_http` | Sans-IO HTTP/1.1 message layer and `Url` type, with optional sync/`rusty_tokio`/real-tokio async adapters |
| [`rusty_json`](crates/rusty_json) | `crates/rusty_json` | From-scratch JSON library, `no_std`-capable, with `serde` interop |
| [`rusty_json-derive`](crates/rusty_json/rusty_json-derive) | `crates/rusty_json/rusty_json-derive` | `rusty_json`'s `#[derive(RustyJson)]` proc-macro |
| [`rusty_oauth`](crates/rusty_oauth) | `crates/rusty_oauth` | Hand-rolled, zero-dependency OAuth 2.0 / 2.1 protocol implementation |
| [`rusty_proxmox`](crates/rusty_proxmox) | `crates/rusty_proxmox` | Async client for the Proxmox VE REST API: nodes, guests (QEMU/LXC), and power control |
| [`rusty_opnsense`](crates/rusty_opnsense) | `crates/rusty_opnsense` | Async client for the OPNsense REST API: system status, services, interfaces, firewall aliases, and gateways |
| [`rusty_homelab_mcp`](crates/rusty_homelab_mcp) | `crates/rusty_homelab_mcp` | MCP server exposing homelab control (Proxmox VE, OPNsense) as tools, built on the `rusty-mcp` scaffold |
| [`rusty_fedora_agent`](crates/rusty_fedora_agent) | `crates/rusty_fedora_agent` | Unprivileged local agent exposing scoped systemd/dnf/config-file control over HTTP — the backend `rusty_homelab_mcp`'s fedora module talks to |
| [`rusty_fedora`](crates/rusty_fedora) | `crates/rusty_fedora` | Async client for `rusty_fedora_agent`'s local HTTP API: system status, systemd services, journal reads, dnf updates/install/remove, and allowlisted config file read/write |
| [`reactor-core`](crates/rustils_async/crates/reactor-core) | `crates/rustils_async/crates/reactor-core` | Runtime-agnostic async-io primitives (a provider framework, not a universal capability) |
| [`platform-async`](crates/rustils_async/crates/platform-async) | `crates/rustils_async/crates/platform-async` | Async trait counterparts to `rustils::platform`'s process domain |
| [`platform-async-mock`](crates/rustils_async/crates/platform-async-mock) | `crates/rustils_async/crates/platform-async-mock` | In-memory async process backend for `platform-async`, for consumer tests without a real OS reactor |
| [`platform-async-linux`](crates/rustils_async/crates/platform-async-linux) | `crates/rustils_async/crates/platform-async-linux` | The real Linux backend for `platform-async`: `pidfd` + `epoll` async wait path |
| [`threading`](crates/rustils_async/crates/threading) | `crates/rustils_async/crates/threading` | Minimal multithreading primitives: scoped-thread spawn, `Mutex`/`RwLock` with explicit poisoning policy |
| [`coreutils-async`](crates/rustils_async/crates/coreutils-async) | `crates/rustils_async/crates/coreutils-async` | Reference consumer for `platform-async`: `arun`, an async port of `rustils`' `rrun` |
| [`rusty_wire`](crates/rusty_wire) | `crates/rusty_wire` | Minimal, zero-dependency endian-explicit byte cursor Reader/Writer |
| [`rusty_std`](crates/rusty_std) | `crates/rusty_std` | `no_std` + `alloc` sovereign standard library, built on `rusty_libc`/`rusty_win32` |
| [`rusty_err`](crates/rusty_err) | `crates/rusty_err` | `no_std` + `alloc` sovereign error trait, context extension, and proc-macro error derive library, built on `rusty_std` |
| [`rusty_err_derive`](crates/rusty_err/derive) | `crates/rusty_err/derive` | `rusty_err`'s `#[derive(Error)]` proc-macro |
| [`rusty_request`](crates/rusty_request) | `crates/rusty_request` | Async HTTP client (a Rust take on Python's `requests`), built on `rusty_tokio`/`rusty_tls`/`rusty_http` |
| [`rusty_sqlite`](crates/rusty_sqlite) | `crates/rusty_sqlite` | A thin, ergonomic wrapper over `rusqlite`: bundled SQLite, typed FTS5 schema building, and connection/migration lifecycle management |
| [`rusty_time`](crates/rusty_time) | `crates/rusty_time` | `no_std` + `alloc` sovereign DateTime, Date, Time, ISO-8601, and timezone offset calculation crate, built on `rusty_std` |
| [`rusty_uuid`](crates/rusty_uuid) | `crates/rusty_uuid` | Minimal, dependency-free UUID v4 generation |
| [`rusty_wiremock`](crates/rusty_wiremock) | `crates/rusty_wiremock` | `no_std` + `alloc` sovereign HTTP mock server and request matcher for Rusty Mill test suites, built on `rusty_http`/`rusty_json`/`rusty_std` |
| [`rusty-search-core`](crates/rusty_search/crates/rusty-search-core) | `crates/rusty_search/crates/rusty-search-core` | Backend-agnostic async search interface: documents, schema, query DSL, and the pluggable `SearchBackend` trait |
| [`rusty-search-memory`](crates/rusty_search/crates/rusty-search-memory) | `crates/rusty_search/crates/rusty-search-memory` | In-memory `SearchBackend` implementation: no external engine required |
| [`rusty-search-tantivy`](crates/rusty_search/crates/rusty-search-tantivy) | `crates/rusty_search/crates/rusty-search-tantivy` | Tantivy-backed `SearchBackend` implementation: embedded full-text search |
| [`rusty-search-sqlite-fts5`](crates/rusty_search/crates/rusty-search-sqlite-fts5) | `crates/rusty_search/crates/rusty-search-sqlite-fts5` | SQLite FTS5-backed `SearchBackend` implementation: embedded full-text search via SQL virtual tables |
| [`rusty-search-elasticsearch`](crates/rusty_search/crates/rusty-search-elasticsearch) | `crates/rusty_search/crates/rusty-search-elasticsearch` | Elasticsearch-backed `SearchBackend` implementation: a remote HTTP search cluster |
| [`rusty-search-meilisearch`](crates/rusty_search/crates/rusty-search-meilisearch) | `crates/rusty_search/crates/rusty-search-meilisearch` | Meilisearch-backed `SearchBackend` implementation: a remote HTTP search engine |
| [`rusty-search-opensearch`](crates/rusty_search/crates/rusty-search-opensearch) | `crates/rusty_search/crates/rusty-search-opensearch` | OpenSearch-backed `SearchBackend` implementation: a remote HTTP search cluster |
| [`rusty-search-solr`](crates/rusty_search/crates/rusty-search-solr) | `crates/rusty_search/crates/rusty-search-solr` | Apache Solr-backed `SearchBackend` implementation: a remote HTTP search cluster |
| [`rusty-search-algolia`](crates/rusty_search/crates/rusty-search-algolia) | `crates/rusty_search/crates/rusty-search-algolia` | Algolia-backed `SearchBackend` implementation: a hosted search SaaS |
| [`rusty-search-azure-search`](crates/rusty_search/crates/rusty-search-azure-search) | `crates/rusty_search/crates/rusty-search-azure-search` | Azure AI Search-backed `SearchBackend` implementation: a hosted search-as-a-service on Azure |
| [`rusty-search-cloud`](crates/rusty_search/crates/rusty-search-cloud) | `crates/rusty_search/crates/rusty-search-cloud` | Sovereign zero-dependency HTTP JSON remote cloud search provider |
| [`rusty-search`](crates/rusty_search/crates/rusty-search) | `crates/rusty_search/crates/rusty-search` | Async, pluggable search interface: swap search engines without changing application code |
| [`rusty_vulkan`](crates/rusty_vulkan) | `crates/rusty_vulkan` | `no_std` + `alloc` sovereign raw Vulkan hardware command buffer and GPU surface layer (Windows-only for now), built on `rusty_win32` |
| [`rusty_sync`](crates/rusty_sync) | `crates/rusty_sync` | `no_std` + `alloc` sovereign atomic spinlock, spinlock-protected MPMC channel, and ring buffer crate, built on `rusty_std` |
| [`rusty_simd`](crates/rusty_simd) | `crates/rusty_simd` | Zero-dependency SIMD (AVX2/NEON/FMA) accelerated block dequantization kernel library for LLM and Whisper inference |
| [`rusty_codec`](crates/rusty_codec) | `crates/rusty_codec` | `no_std` + `alloc` sovereign TOML configuration parser and binary buffer serialization crate, built on `rusty_wire`/`rusty_std` |
| [`rusty_h2`](crates/rusty_h2) | `crates/rusty_h2` | A from-scratch HTTP/2 (RFC 9113) implementation, including HPACK header compression |
| [`rusty_audio`](crates/rusty_audio) | `crates/rusty_audio` | `no_std` + `alloc` sovereign PCM audio capture and playback device driver (hand-written WASAPI COM FFI on Windows, ALSA on Linux) |
| [`rusty_crypto_key`](crates/rusty_crypto_key) | `crates/rusty_crypto_key` | A zeroize-on-drop key storage and file persistence micro-crate (`0600` permissions on Unix) |
| [`rusty-db-core`](crates/rusty_db/crates/rusty-db-core) | `crates/rusty_db/crates/rusty-db-core` | Database-agnostic query builder and driver abstraction (the SQLAlchemy-Core-like layer of `rusty_db`) |
| [`rusty-db-derive`](crates/rusty_db/crates/rusty-db-derive) | `crates/rusty_db/crates/rusty-db-derive` | `#[derive(Mapped)]` macro for `rusty_db`: maps a struct to a table |
| [`rusty-db-sqlite`](crates/rusty_db/crates/rusty-db-sqlite) | `crates/rusty_db/crates/rusty-db-sqlite` | SQLite driver for `rusty_db`, built on `sqlx` |
| [`rusty-db-postgres`](crates/rusty_db/crates/rusty-db-postgres) | `crates/rusty_db/crates/rusty-db-postgres` | PostgreSQL driver for `rusty_db`, built on `sqlx` |
| [`rusty-db-mysql`](crates/rusty_db/crates/rusty-db-mysql) | `crates/rusty_db/crates/rusty-db-mysql` | MySQL/MariaDB driver for `rusty_db`, built on `sqlx` |
| [`rusty-db`](crates/rusty_db/rusty_db) | `crates/rusty_db/rusty_db` | A database-agnostic query builder and connection abstraction, in the spirit of SQLAlchemy Core |
| [`rusty_ansi`](crates/rusty_ansi) | `crates/rusty_ansi` | Zero-allocation, `no_std` VT100/CSI/OSC ANSI escape sequence parser core |
| [`rusty_config`](crates/rusty_config) | `crates/rusty_config` | Zero-dependency, `no_std` INI and Key-Value configuration file parser |
| [`rusty_jinja`](crates/rusty_jinja) | `crates/rusty_jinja` | `no_std` + `alloc` sovereign, zero-dependency Jinja2 LLM chat template evaluator |
| [`rusty_ansder`](crates/rusty_ansder) | `crates/rusty_ansder` | ASN.1 BER/DER TLV encoder and decoder, built on `rusty_wire` |
| [`rusty_rag`](crates/rusty_rag) | `crates/rusty_rag` | Sovereign AI Retrieval-Augmented Generation (RAG) & Question Answering engine, built on `rusty_simd`; split out of `rusty_ansder`, which used to bundle both under one portmanteau name |
| [`rusty_boot`](crates/rusty_boot) | `crates/rusty_boot` | `no_std` + `alloc` sovereign bootstrapper demonstrating kernel-to-application execution without Rust `std`, exercising the full stack of merged crates |
| [`rusty-whisper`](crates/rusty_whisper) | `crates/rusty_whisper` | A pure-Rust port of whisper.cpp (OpenAI Whisper speech recognition) |
| [`rusty_rdp`](crates/rusty_rdp) | `crates/rusty_rdp` | A minimal, dependency-free implementation of the Remote Desktop Protocol (RDP) wire format |
| [`rusty_voice`](crates/rusty_voice) | `crates/rusty_voice` | A sovereign voice-to-text application leveraging `rusty_whisper` and `rusty_audio`, built exclusively with Rusty Mill libraries |
| [`platform`](crates/rustils/crates/platform) | `crates/rustils/crates/platform` | rustils' portable trait surface and types — the PAL's api layer, no I/O, no unsafe |
| [`platform-mock`](crates/rustils/crates/platform-mock) | `crates/rustils/crates/platform-mock` | In-memory backend implementing every `platform` trait — the injectable test double |
| [`platform-parity`](crates/rustils/crates/platform-parity) | `crates/rustils/crates/platform-parity` | Shared behavior-spec assertion sets for the PAL parity suites (test-support only) |
| [`platform-linux`](crates/rustils/crates/platform-linux) | `crates/rustils/crates/platform-linux` | Linux backend for `platform`: libc floor, with a `rusty_libc`-backed raw-syscall track behind a feature flag |
| [`platform-windows`](crates/rustils/crates/platform-windows) | `crates/rustils/crates/platform-windows` | Windows backend for `platform`: `windows-sys` floor, with a `rusty_win32`-backed track behind a feature flag |
| [`platform-bsd`](crates/rustils/crates/platform-bsd) | `crates/rustils/crates/platform-bsd` | BSD backend for `platform` (net-only slice): macOS, FreeBSD, OpenBSD, NetBSD, DragonFly |
| [`winargv`](crates/rustils/crates/winargv) | `crates/rustils/crates/winargv` | Windows argv → command-line construction (MSVCRT + cmd-rules quoting, refuse-unrepresentable) |
| [`coreutils`](crates/rustils/crates/coreutils) | `crates/rustils/crates/coreutils` | Modular pure-Rust implementation of core GNU/POSIX utilities (`rcat`, `rls`, `rrun`, `rgrep`, and more) |
| [`rusty-croc`](crates/rusty_croc) | `crates/rusty_croc` | Rust port of [croc](https://github.com/schollz/croc): wire-compatible secure peer-to-peer file transfer (PAKE, relay, resume) |
| [`contract`](crates/rusty_test/crates/contract) | `crates/rusty_test/crates/contract` | Portable tool-runtime trait boundary: one execution contract, no OS-specific code |
| [`compat`](crates/rusty_test/crates/compat) | `crates/rusty_test/crates/compat` | Per-host adapter implementing `contract` over `cap-std`/`portable-pty`/`dirs` plus `std`'s file locking |
| [`conformance`](crates/rusty_test/crates/conformance) | `crates/rusty_test/crates/conformance` | Cross-cutting verification of `contract`/`compat`: probe suite and conformance report |
| [`stat-tool`](crates/rusty_test/tools/stat-tool) | `crates/rusty_test/tools/stat-tool` | Reference tool over `contract`: scoped filesystem primitive |
| [`proc-runner`](crates/rusty_test/tools/proc-runner) | `crates/rusty_test/tools/proc-runner` | Reference tool over `contract`: process spawn + stdio capture primitive |
| [`pty-shell`](crates/rusty_test/tools/pty-shell) | `crates/rusty_test/tools/pty-shell` | Reference tool over `contract`: interactive PTY primitive (manual, not CI) |
| [`inventory-core`](crates/rusty_inventrory/crates/inventory-core) | `crates/rusty_inventrory/crates/inventory-core` | Local-first encrypted index over the conversation history AI coding tools write to disk |
| [`inventory-cli`](crates/rusty_inventrory/crates/inventory-cli) | `crates/rusty_inventrory/crates/inventory-cli` | `inv`: search every AI agent and IDE conversation on your machine, from the terminal |
| [`inventory-tauri`](crates/rusty_inventrory/crates/inventory-tauri) | `crates/rusty_inventrory/crates/inventory-tauri` | Menu-bar app over `inventory-core`: one keystroke to every AI conversation on your machine |
| [`skillopt-core`](crates/rusty_skillopt/crates/skillopt-core) | `crates/rusty_skillopt/crates/skillopt-core` | Text-space optimizer for skill markdown: epochs, batches, and a validation gate over a frozen LLM agent |
| [`skillopt-model`](crates/rusty_skillopt/crates/skillopt-model) | `crates/rusty_skillopt/crates/skillopt-model` | LLM provider adapters for `skillopt-core` |
| [`skillopt-envs`](crates/rusty_skillopt/crates/skillopt-envs) | `crates/rusty_skillopt/crates/skillopt-envs` | Task environments and benchmark adapters `skillopt-core` optimizes against |
| [`skillopt-cli`](crates/rusty_skillopt/crates/skillopt-cli) | `crates/rusty_skillopt/crates/skillopt-cli` | `skillopt`: the training-loop command line front end |
| [`rk-config`](crates/rusty_key/crates/config) | `crates/rusty_key/crates/config` | Rusty Keys' configuration layer: typed settings, env overrides, workspace discovery |
| [`rk-observe`](crates/rusty_key/crates/observe) | `crates/rusty_key/crates/observe` | Rusty Keys' *observe* pillar: structured attribution and turn-level observation records |
| [`rk-constrain`](crates/rusty_key/crates/constrain) | `crates/rusty_key/crates/constrain` | Rusty Keys' *constrain* pillar: policy enforcement around tool dispatch |
| [`rk-feed`](crates/rusty_key/crates/feed) | `crates/rusty_key/crates/feed` | Rusty Keys' *feed* pillar: guides, memory, recall, and the built-in tool set |
| [`rk-kernel`](crates/rusty_key/crates/kernel) | `crates/rusty_key/crates/kernel` | Rusty Keys' kernel: the model's agent loop (turn, stream, complete) |
| [`rk-mcp`](crates/rusty_key/crates/mcp) | `crates/rusty_key/crates/mcp` | Rusty Keys' MCP client layer: server config, policy, and stdio/SSE transports |
| [`rk-compose`](crates/rusty_key/crates/compose) | `crates/rusty_key/crates/compose` | Rusty Keys' *compose* pillar: subagent composition and the ratchet |
| [`rk-app`](crates/rusty_key/crates/app) | `crates/rusty_key/crates/app` | `rusty-keys`: the harness binary wiring the four pillars around the kernel |
| [`rusty_llama`](crates/rusty_llama) | `crates/rusty_llama` | From-scratch Llama/GGUF inference engine (CPU SIMD, optional wgpu and CUDA backends, OpenAI-compatible server) |
| [`ts-types`](crates/rusty_tailscale/crates/ts-types) | `crates/rusty_tailscale/crates/ts-types` | Tailscale wire types shared across the client: node keys, status, netmap |
| [`ts-key`](crates/rusty_tailscale/crates/ts-key) | `crates/rusty_tailscale/crates/ts-key` | Key material for the Tailscale client: machine, node, and disco keypairs |
| [`ts-control`](crates/rusty_tailscale/crates/ts-control) | `crates/rusty_tailscale/crates/ts-control` | ts2021 control-plane client: Noise (control base) handshake and the map session |
| [`ts-derp`](crates/rusty_tailscale/crates/ts-derp) | `crates/rusty_tailscale/crates/ts-derp` | DERP relay client: the always-available fallback data path |
| [`ts-stun`](crates/rusty_tailscale/crates/ts-stun) | `crates/rusty_tailscale/crates/ts-stun` | STUN client for discovering the server-reflexive endpoint |
| [`ts-disco`](crates/rusty_tailscale/crates/ts-disco) | `crates/rusty_tailscale/crates/ts-disco` | Disco protocol: ping/pong/call-me-maybe path probing |
| [`ts-magicsock`](crates/rusty_tailscale/crates/ts-magicsock) | `crates/rusty_tailscale/crates/ts-magicsock` | Path multiplexer: direct UDP and DERP, path upgrade and live migration |
| [`ts-wg`](crates/rusty_tailscale/crates/ts-wg) | `crates/rusty_tailscale/crates/ts-wg` | WireGuard data plane over the magicsock transport |
| [`ts-tun`](crates/rusty_tailscale/crates/ts-tun) | `crates/rusty_tailscale/crates/ts-tun` | TUN device, routes, and DNS platform adapters |
| [`ts-filter`](crates/rusty_tailscale/crates/ts-filter) | `crates/rusty_tailscale/crates/ts-filter` | Packet filter evaluating the tailnet ACL rules the control plane hands down |
| [`ts-engine`](crates/rusty_tailscale/crates/ts-engine) | `crates/rusty_tailscale/crates/ts-engine` | The node engine wiring control, magicsock, WireGuard, TUN and filter together |
| [`ts-localapi`](crates/rusty_tailscale/crates/ts-localapi) | `crates/rusty_tailscale/crates/ts-localapi` | LocalAPI server: the daemon's Unix-socket control surface |
| [`ts-net`](crates/rusty_tailscale/crates/ts-net) | `crates/rusty_tailscale/crates/ts-net` | Userspace TCP/IP stack (smoltcp) on the tailnet — a tailnet service with no TUN and no root |
| [`ts-daemon`](crates/rusty_tailscale/crates/ts-daemon) | `crates/rusty_tailscale/crates/ts-daemon` | `ts-daemon`: the long-running node daemon |
| [`ts-cli`](crates/rusty_tailscale/crates/ts-cli) | `crates/rusty_tailscale/crates/ts-cli` | `ts-cli`: the LocalAPI-driven command line client |
| [`xtask`](crates/rusty_tailscale/xtask) | `crates/rusty_tailscale/xtask` | `rusty_tailscale`'s integration harness: Headscale in a container, multi-node NAT simulation |
| [`adk-core`](crates/rusty_adk/crates/adk-core) | `crates/rusty_adk/crates/adk-core` | ADK 2.0's data model: events, content, state, and the tool/callback contracts |
| [`adk-macros`](crates/rusty_adk/crates/adk-macros) | `crates/rusty_adk/crates/adk-macros` | `#[tool]` and friends: `adk-core`'s derive/attribute macros |
| [`adk-tools`](crates/rusty_adk/crates/adk-tools) | `crates/rusty_adk/crates/adk-tools` | Built-in tool implementations and the tool registry |
| [`adk-models`](crates/rusty_adk/crates/adk-models) | `crates/rusty_adk/crates/adk-models` | LLM provider adapters for the ADK runtime |
| [`adk-sessions`](crates/rusty_adk/crates/adk-sessions) | `crates/rusty_adk/crates/adk-sessions` | Session and event persistence (in-memory and SQLite stores) |
| [`adk-graph`](crates/rusty_adk/crates/adk-graph) | `crates/rusty_adk/crates/adk-graph` | ADK 2.0's graph-based execution engine |
| [`adk-agents`](crates/rusty_adk/crates/adk-agents) | `crates/rusty_adk/crates/adk-agents` | Agent types built on the graph engine: LLM, sequential, parallel, loop |
| [`adk-runner`](crates/rusty_adk/crates/adk-runner) | `crates/rusty_adk/crates/adk-runner` | The runner: drives an agent over a session and streams its events |
| [`adk-mcp`](crates/rusty_adk/crates/adk-mcp) | `crates/rusty_adk/crates/adk-mcp` | MCP bridge: consume MCP servers as ADK tools, and serve ADK tools over MCP |
| [`adk-a2a`](crates/rusty_adk/crates/adk-a2a) | `crates/rusty_adk/crates/adk-a2a` | A2A bridge: serve a Rust ADK agent over the Agent2Agent protocol |
| [`rusty-adk`](crates/rusty_adk/crates/rusty-adk) | `crates/rusty_adk/crates/rusty-adk` | The `rusty-adk` facade crate re-exporting the ADK stack |
| [`weather-agent`](crates/rusty_adk/examples/weather-agent) | `crates/rusty_adk/examples/weather-agent` | `rusty-adk` example: a tool-using LLM agent |
| [`mcp-tool-server`](crates/rusty_adk/examples/mcp-tool-server) | `crates/rusty_adk/examples/mcp-tool-server` | `rusty-adk` example: serving ADK tools over MCP |
| [`a2a-agent-server`](crates/rusty_adk/examples/a2a-agent-server) | `crates/rusty_adk/examples/a2a-agent-server` | `rusty-adk` example: serving an ADK agent over A2A |
| [`rp-core`](crates/rusty_provider/crates/core) | `crates/rusty_provider/crates/core` | Unified OpenAI-shaped request/response types and the provider trait |
| [`rp-providers`](crates/rusty_provider/crates/providers) | `crates/rusty_provider/crates/providers` | Provider adapters: OpenAI, Anthropic, Gemini, Groq, Together AI, Fireworks |
| [`rp-router`](crates/rusty_provider/crates/router) | `crates/rusty_provider/crates/router` | Config-driven routing: fallback chains, budgets, metrics, and usage persistence |
| [`rp-mcp`](crates/rusty_provider/crates/mcp) | `crates/rusty_provider/crates/mcp` | MCP surface over the router, built on the `rusty-mcp` scaffold |
| [`rp-server`](crates/rusty_provider/crates/server) | `crates/rusty_provider/crates/server` | The OpenAI-compatible HTTP server front end |
| [`rp-cli`](crates/rusty_provider/crates/cli) | `crates/rusty_provider/crates/cli` | `rp-cli`: config inspection and routing dry-runs from the terminal |
| [`sessionmgr-core`](crates/rusty_yirp/crates/sessionmgr-core) | `crates/rusty_yirp/crates/sessionmgr-core` | sessionmgr's pure domain logic: session state machine, identifiers, crash-recovery policy |
| [`sessionmgr-protocol`](crates/rusty_yirp/crates/sessionmgr-protocol) | `crates/rusty_yirp/crates/sessionmgr-protocol` | Wire types shared by the sessionmgr daemon, its workers, and its clients |
| [`sessionmgr-proc`](crates/rusty_yirp/crates/sessionmgr-proc) | `crates/rusty_yirp/crates/sessionmgr-proc` | Process adapter: detached spawn, PID-reuse-safe liveness, stdio-inheritance hardening |
| [`sessionmgr-git`](crates/rusty_yirp/crates/sessionmgr-git) | `crates/rusty_yirp/crates/sessionmgr-git` | Git adapter: worktree lifecycle, status, and diff |
| [`sessionmgr-pty`](crates/rusty_yirp/crates/sessionmgr-pty) | `crates/rusty_yirp/crates/sessionmgr-pty` | PTY adapter over `rustils`' `Pty` capability (ConPTY on Windows, `openpty` on Linux) |
| [`sessionmgr-tui`](crates/rusty_yirp/crates/sessionmgr-tui) | `crates/rusty_yirp/crates/sessionmgr-tui` | The TUI grid dashboard (ratatui + `tui-term`'s vt100 screen) |
| [`sessionmgr-agents`](crates/rusty_yirp/crates/sessionmgr-agents) | `crates/rusty_yirp/crates/sessionmgr-agents` | Per-agent-CLI adapters for Claude Code, Codex, and Gemini CLI |
| [`sessionmgr-daemon`](crates/rusty_yirp/crates/sessionmgr-daemon) | `crates/rusty_yirp/crates/sessionmgr-daemon` | `sessionmgr`: the composition root — supervisor daemon, detached workers, CLI client |
| [`sessionmgr-desktop`](crates/rusty_yirp/crates/sessionmgr-desktop/src-tauri) | `crates/rusty_yirp/crates/sessionmgr-desktop/src-tauri` | sessionmgr's Tauri 2 desktop shell over the daemon socket |
| [`agentgateway-config`](crates/rusty_agent_gateway/crates/agentgateway-config) | `crates/rusty_agent_gateway/crates/agentgateway-config` | Configuration model, wire-compatible with agentgateway's own `config.yaml` |
| [`agentgateway-core`](crates/rusty_agent_gateway/crates/agentgateway-core) | `crates/rusty_agent_gateway/crates/agentgateway-core` | Route matching and policy evaluation |
| [`agentgateway-auth`](crates/rusty_agent_gateway/crates/agentgateway-auth) | `crates/rusty_agent_gateway/crates/agentgateway-auth` | JWT authentication policy, over `rusty-mcp`'s JWKS validator |
| [`agentgateway-a2a`](crates/rusty_agent_gateway/crates/agentgateway-a2a) | `crates/rusty_agent_gateway/crates/agentgateway-a2a` | A2A method gating and agent-card discovery |
| [`agentgateway-llm`](crates/rusty_agent_gateway/crates/agentgateway-llm) | `crates/rusty_agent_gateway/crates/agentgateway-llm` | OpenAI-compatible LLM gateway pillar |
| [`agentgateway-mcp`](crates/rusty_agent_gateway/crates/agentgateway-mcp) | `crates/rusty_agent_gateway/crates/agentgateway-mcp` | MCP federation: several upstream MCP servers behind one endpoint, with guardrails |
| [`agentgateway-proxy`](crates/rusty_agent_gateway/crates/agentgateway-proxy) | `crates/rusty_agent_gateway/crates/agentgateway-proxy` | HTTP reverse proxying for host backends |
| [`agentgateway-tls`](crates/rusty_agent_gateway/crates/agentgateway-tls) | `crates/rusty_agent_gateway/crates/agentgateway-tls` | TLS termination, over `rusty_tls` |
| [`agentgateway`](crates/rusty_agent_gateway/crates/agentgateway) | `crates/rusty_agent_gateway/crates/agentgateway` | `agentgateway`: the AI-native gateway binary for MCP, speaking agentgateway's config |
| [`nexus-types`](crates/nexus/crates/nexus-types) | `crates/nexus/crates/nexus-types` | Nexus: shared plain-data types with no I/O, at the base of every other nexus crate |
| [`nexus-plugin-api`](crates/nexus/crates/nexus-plugin-api) | `crates/nexus/crates/nexus-plugin-api` | Nexus: the versioned `CorePlugin`/capability/IPC ABI every plugin crate implements against |
| [`nexus-hashline`](crates/nexus/crates/nexus-hashline) | `crates/nexus/crates/nexus-hashline` | Nexus: content-hash-anchored patch format for concurrent note edits (RFC 0005) |
| [`nexus-kernel`](crates/nexus/crates/nexus-kernel) | `crates/nexus/crates/nexus-kernel` | Nexus: the microkernel — event bus, IPC dispatcher, capability system, plugin lifecycle |
| [`nexus-kv`](crates/nexus/crates/nexus-kv) | `crates/nexus/crates/nexus-kv` | Nexus: the forge-scoped key/value store service plugin |
| [`nexus-security`](crates/nexus/crates/nexus-security) | `crates/nexus/crates/nexus-security` | Nexus: capability grants, at-rest encryption, and the Linux Landlock/seccomp OS sandbox |
| [`nexus-storage`](crates/nexus/crates/nexus-storage) | `crates/nexus/crates/nexus-storage` | Nexus: file-as-truth — SQLite index, Tantivy FTS, file watcher, knowledge graph |
| [`nexus-plugins`](crates/nexus/crates/nexus-plugins) | `crates/nexus/crates/nexus-plugins` | Nexus: community plugin lifecycle — WASM (wasmtime) and JS-sandboxed plugin hosting |
| [`nexus-ai`](crates/nexus/crates/nexus-ai) | `crates/nexus/crates/nexus-ai` | Nexus: AI provider integration — chat, embeddings, RAG |
| [`nexus-ai-runtime`](crates/nexus/crates/nexus-ai-runtime) | `crates/nexus/crates/nexus-ai-runtime` | Nexus: local model runtime plumbing for `nexus-ai` |
| [`nexus-mcp`](crates/nexus/crates/nexus-mcp) | `crates/nexus/crates/nexus-mcp` | Nexus: Host-side MCP client/server integration |
| [`nexus-lsp`](crates/nexus/crates/nexus-lsp) | `crates/nexus/crates/nexus-lsp` | Nexus: Language Server Protocol integration |
| [`nexus-dap`](crates/nexus/crates/nexus-dap) | `crates/nexus/crates/nexus-dap` | Nexus: Debug Adapter Protocol integration |
| [`nexus-acp`](crates/nexus/crates/nexus-acp) | `crates/nexus/crates/nexus-acp` | Nexus: Agent Client Protocol integration |
| [`nexus-remote`](crates/nexus/crates/nexus-remote) | `crates/nexus/crates/nexus-remote` | Nexus: remote/hosted forge connectivity |
| [`nexus-cli`](crates/nexus/crates/nexus-cli) | `crates/nexus/crates/nexus-cli` | `nexus`: the CLI frontend, built on `nexus-bootstrap` |
| [`nexus-tui`](crates/nexus/crates/nexus-tui) | `crates/nexus/crates/nexus-tui` | `nexus-tui`: the terminal UI frontend, built on `nexus-bootstrap` |
| [`nexus-git`](crates/nexus/crates/nexus-git) | `crates/nexus/crates/nexus-git` | Nexus: Git integration (built on `git2`) |
| [`nexus-formats`](crates/nexus/crates/nexus-formats) | `crates/nexus/crates/nexus-formats` | Nexus: import/export format converters (e.g. Notion `.zip` export) |
| [`nexus-database`](crates/nexus/crates/nexus-database) | `crates/nexus/crates/nexus-database` | Nexus: user-facing embedded-database note views |
| [`nexus-theme`](crates/nexus/crates/nexus-theme) | `crates/nexus/crates/nexus-theme` | Nexus: shell theming service |
| [`nexus-bootstrap`](crates/nexus/crates/nexus-bootstrap) | `crates/nexus/crates/nexus-bootstrap` | Nexus: the orchestrator — assembles the kernel + every registered `CorePlugin` into a `Runtime` |
| [`nexus-editor`](crates/nexus/crates/nexus-editor) | `crates/nexus/crates/nexus-editor` | Nexus: the note editor backend — CRDT snapshots, crash journal |
| [`nexus-rush`](crates/nexus/crates/nexus-rush) | `crates/nexus/crates/nexus-rush` | Nexus: in-tree port of `rush` (RFC 0002) — the bundled shell for sandboxed sessions |
| [`nexus-vt`](crates/nexus/crates/nexus-vt) | `crates/nexus/crates/nexus-vt` | Nexus: in-tree, GUI-free port of `rusty_term`'s core (RFC 0003) — the headless VT grid behind `nexus-terminal` |
| [`nexus-terminal`](crates/nexus/crates/nexus-terminal) | `crates/nexus/crates/nexus-terminal` | Nexus: terminal/process-manager service plugin, built on `nexus-vt` + `portable-pty` |
| [`nexus-agent`](crates/nexus/crates/nexus-agent) | `crates/nexus/crates/nexus-agent` | Nexus: the in-app agent — tool registry, planning, delegation |
| [`nexus-skills`](crates/nexus/crates/nexus-skills) | `crates/nexus/crates/nexus-skills` | Nexus: agent skill loading |
| [`nexus-templates`](crates/nexus/crates/nexus-templates) | `crates/nexus/crates/nexus-templates` | Nexus: note template service |
| [`nexus-workflow`](crates/nexus/crates/nexus-workflow) | `crates/nexus/crates/nexus-workflow` | Nexus: multi-step workflow orchestration |
| [`nexus-linkpreview`](crates/nexus/crates/nexus-linkpreview) | `crates/nexus/crates/nexus-linkpreview` | Nexus: URL link-preview fetching |
| [`nexus-notifications`](crates/nexus/crates/nexus-notifications) | `crates/nexus/crates/nexus-notifications` | Nexus: notification inbox + SMTP email transport |
| [`nexus-comments`](crates/nexus/crates/nexus-comments) | `crates/nexus/crates/nexus-comments` | Nexus: note comment threads |
| [`nexus-panic-log`](crates/nexus/crates/nexus-panic-log) | `crates/nexus/crates/nexus-panic-log` | Nexus: structured panic capture and logging |
| [`nexus-crdt`](crates/nexus/crates/nexus-crdt) | `crates/nexus/crates/nexus-crdt` | Nexus: CRDT primitives backing `nexus-editor`/`nexus-collab` |
| [`nexus-fuzz`](crates/nexus/crates/nexus-fuzz) | `crates/nexus/crates/nexus-fuzz` | Nexus: fuzz targets for parser/format boundaries |
| [`nexus-audio`](crates/nexus/crates/nexus-audio) | `crates/nexus/crates/nexus-audio` | Nexus: STT/TTS provider traits (local / provider-routed / platform backends) |
| [`nexus-collab`](crates/nexus/crates/nexus-collab) | `crates/nexus/crates/nexus-collab` | Nexus: real-time collaboration over a WebSocket relay |
| [`nexus-memory`](crates/nexus/crates/nexus-memory) | `crates/nexus/crates/nexus-memory` | Nexus: the `com.nexus.memory` service plugin — full `remind_me` schema/API parity |
| [`nexus-memory-hub`](crates/nexus/crates/nexus-memory-hub) | `crates/nexus/crates/nexus-memory-hub` | Nexus: standalone `axum` HTTP sync server for `nexus-memory` — a deployable binary, not a bootstrap plugin |
| [`nexus-context`](crates/nexus/crates/nexus-context) | `crates/nexus/crates/nexus-context` | Nexus: staging library, not yet wired into `nexus-bootstrap` (tracked upstream by nexus#188) |
| [`nexus-protocol`](crates/nexus/crates/nexus-protocol) | `crates/nexus/crates/nexus-protocol` | Nexus: staging library, not yet wired into `nexus-bootstrap` (tracked upstream by nexus#188) |
| [`rusty_multimodal_db`](crates/rusty_multimodal_db) | `crates/rusty_multimodal_db` | Benchmark harness comparing AoS, SoA, and UUID-canonical-store views as storage backends, plus a production store, network server, and schema-driven client built on the winning design |
| [`rusty_sha1`](crates/rusty_sha1) | `crates/rusty_sha1` | Zero-dependency SHA-1 (FIPS 180-1) implementation, shared by `rusty_git`'s object hashing and `rusty_term`'s WebSocket handshake |
| [`rusty_base64`](crates/rusty_base64) | `crates/rusty_base64` | Hand-rolled, dependency-free Base64 (RFC 4648) codec (standard and URL-safe alphabets, encode/decode), extracted from `rusty_oauth` and now shared by `rusty_acp`/`rusty-mcp`/`rusty_a2a` |
| [`rusty_rand`](crates/rusty_rand) | `crates/rusty_rand` | OS-backed cryptographically secure random bytes (`/dev/urandom`/`BCryptGenRandom`), the CSPRNG shared by `rusty_oauth`, `rusty_uuid`, and `sessionmgr-proc` |
| [`rusty_retry`](crates/rusty_retry) | `crates/rusty_retry` | Exponential backoff with jitter and `Retry-After` delta-seconds parsing, the retry mechanism shared by `rusty_request` and `rusty_acp` |
| [`rusty_rsa`](crates/rusty_rsa) | `crates/rusty_rsa` | Hand-rolled, dependency-free BigUint (RSA/ECC arithmetic) and SHA-256, the primitives `rusty_oauth` and `rusty_rdp` each independently reimplemented for RSA public-key verification/encryption |
| [`rusty_kafka`](crates/rusty_kafka) | `crates/rusty_kafka` | Hand-rolled Kafka wire-protocol client: producer, consumer, and admin APIs, built on `rusty_wire` and `rusty_tokio` |
| [`rusty-meshed-core`](crates/rusty_meshed/crates/rusty-meshed-core) | `crates/rusty_meshed/crates/rusty-meshed-core` | `rusty_meshed`'s shared platform config: env-prefixed settings loaded once and injected into every other `rusty_meshed` crate |
| [`rusty-meshed-schema-registry`](crates/rusty_meshed/crates/rusty-meshed-schema-registry) | `crates/rusty_meshed/crates/rusty-meshed-schema-registry` | Confluent Schema Registry client and compatibility-mode enforcement, ported from `meshed.schema_registry` |
| [`rusty-meshed-governance`](crates/rusty_meshed/crates/rusty-meshed-governance) | `crates/rusty_meshed/crates/rusty-meshed-governance` | Policy-as-code governance engine and built-in policies, ported from `meshed.governance` |
| [`rusty-meshed-observability`](crates/rusty_meshed/crates/rusty-meshed-observability) | `crates/rusty_meshed/crates/rusty-meshed-observability` | Lineage tracking, metrics collection, SLO monitoring, and the CI contract gate, ported from `meshed.observability` |
| [`rusty-meshed-sdk`](crates/rusty_meshed/crates/rusty-meshed-sdk) | `crates/rusty_meshed/crates/rusty-meshed-sdk` | The data-product producer/consumer SDK, transactional outbox, and topic lifecycle management, ported from `meshed.sdk`/`meshed.infrastructure` |
| [`rusty-meshed-registry`](crates/rusty_meshed/crates/rusty-meshed-registry) | `crates/rusty_meshed/crates/rusty-meshed-registry` | The data-product registry HTTP API: models, CRUD routers, and governance/lineage/metrics/monitor endpoints, ported from `meshed.registry` |
| [`rusty-meshed-cli`](crates/rusty_meshed/crates/rusty-meshed-cli) | `crates/rusty_meshed/crates/rusty-meshed-cli` | The meshed operator CLI (health/lineage/metrics/slo commands), ported from `meshed.cli` |
| [`rusty-meshed-domains`](crates/rusty_meshed/crates/rusty-meshed-domains) | `crates/rusty_meshed/crates/rusty-meshed-domains` | The manpower domain: event schemas, domain data products, scenario builder, and demo generators, ported from `meshed.domains` |
| [`rusty-meshed-trace`](crates/rusty_meshed/crates/rusty-meshed-trace) | `crates/rusty_meshed/crates/rusty-meshed-trace` | Reverse-trace and domain-maturity model: outcome → domains → sources, with a fidelity verdict and worst-first bottleneck list |
| [`rusty-hister-core`](crates/rusty_hister/crates/rusty-hister-core) | `crates/rusty_hister/crates/rusty-hister-core` | rusty_hister: shared types, IDs, error types, and the `Document`/extractor-SDK data model |
| [`rusty-hister-model`](crates/rusty_hister/crates/rusty-hister-model) | `crates/rusty_hister/crates/rusty-hister-model` | rusty_hister: persisted schema on `rusty_db`, dual SQLite/Postgres (schema implemented; query layer complete — embedding-queue, `WebSession`, `DocumentVersion`, `crawl.go`, `history.go`, and `user.go` all ported) |
| [`rusty-hister-extractor`](crates/rusty_hister/crates/rusty-hister-extractor) | `crates/rusty_hister/crates/rusty-hister-extractor` | rusty_hister: extractor chain-of-responsibility registry, plus all twenty built-in concrete extractors (JSON-LD, EmbeddedVideo, StackExchange, GoDoc, Lobsters, HackerNews, GitHub, ChatGPT, Basic, Wikipedia, Reddit, Discourse, Ytdlp, Markdown, OrgMode, Notion, Readability, Mastodon, Bluesky, Twitter) |
| [`rusty-hister-indexer`](crates/rusty_hister/crates/rusty-hister-indexer) | `crates/rusty_hister/crates/rusty-hister-indexer` | rusty_hister: query language + full-text indexing on `rusty_search` (skeleton; unblocked by ADR-0002, not yet started) |
| [`rusty-hister-vectorstore`](crates/rusty_hister/crates/rusty-hister-vectorstore) | `crates/rusty_hister/crates/rusty-hister-vectorstore` | rusty_hister: embedding pipeline and vector storage for semantic search (skeleton; unblocked by ADR-0002, not yet started) |
| [`rusty-hister-crawler`](crates/rusty_hister/crates/rusty-hister-crawler) | `crates/rusty_hister/crates/rusty-hister-crawler` | rusty_hister: HTTP and JS-rendering crawler backends (skeleton; CDP backend unblocked by ADR-0003, not yet started) |
| [`rusty-hister-server`](crates/rusty_hister/crates/rusty-hister-server) | `crates/rusty_hister/crates/rusty-hister-server` | rusty_hister: HTTP/JSON API + WebSocket search protocol, the v1 backend surface (skeleton) |
| [`rusty-hister-mcp`](crates/rusty_hister/crates/rusty-hister-mcp) | `crates/rusty_hister/crates/rusty-hister-mcp` | rusty_hister: MCP JSON-RPC tool surface (search, get_preview, get_history) on `rusty_mcp` (skeleton) |

Each crate's own README, docs, and issue history describe its design in
depth — the links above point at the original standalone repos' content,
now living under `crates/<name>/`.

## Building

```sh
cargo build --workspace
cargo test --workspace
```

Some `rusty_term` features (`gui`, `gui-gpu`) and `rusty_gui`'s Linux
backend link against X11/Wayland directly, `inventory-tauri` needs
WebKitGTK/GTK3, and `inventory-core`'s Linux keyring backend needs libdbus;
see [`.github/actions/setup-build-env/action.yml`](.github/actions/setup-build-env/action.yml)
for the system packages a Linux build needs. `rusty_win32` and parts of `rusty_gui`/
`rusty_gpu` are Windows-only (`cfg(windows)`-gated) and are exercised by
the workflow's `windows-latest` matrix leg.

`mill-term`'s own test suite has one known, pre-existing, environment-
dependent failure in this monorepo:
`augmented_path_prepends_tool_directories_and_keeps_existing_path` hardcodes
a Windows-style path and only ever passed on a Windows runner — unrelated
to this merge.

`sessionmgr-daemon` has one test in the same class:
`a_fresh_claude_session_reaches_needs_input_on_its_own` drives a real
`claude` session and skips itself when `claude` is not on `PATH` (the state
of a CI runner). On a machine that *does* have the CLI but cannot complete
its interactive trust-folder prompt, the skip guard passes and the test
times out instead.

## How the crates relate

The crate relationship map is generated at [docs/WORKSPACE-MAP.md](docs/WORKSPACE-MAP.md)
by `.github/scripts/generate_workspace_map.py` and checked for staleness in CI.

## History

These crates originated as standalone repos under `baileyrd`:
[`rusty_term`](https://github.com/baileyrd/rusty_term),
[`rusty_gpu`](https://github.com/baileyrd/rusty_gpu),
[`rusty_gui`](https://github.com/baileyrd/rusty_gui),
[`rusty_font`](https://github.com/baileyrd/rusty_font),
[`rusty_regx`](https://github.com/baileyrd/rusty_regx),
[`rusty_win32`](https://github.com/baileyrd/rusty_win32),
[`rush`](https://github.com/baileyrd/rush),
[`rusty_lines`](https://github.com/baileyrd/rusty_lines),
[`mill-term`](https://github.com/baileyrd/mill-term),
[`rpath`](https://github.com/baileyrd/rpath),
[`rusty_git`](https://github.com/baileyrd/rusty_git),
[`rusty_diff`](https://github.com/baileyrd/rusty_diff),
[`rusty_compress`](https://github.com/baileyrd/rusty_compress), and
[`rusty_text`](https://github.com/baileyrd/rusty_text). Their full commit
history, issues, and PRs remain on those repos for reference; only the code
history was merged here.

The second wave, merging in the same way, adds:
[`rusty_tokio`](https://github.com/baileyrd/rusty_tokio) (plus its nested
`rusty_tokio-macros` crate), [`rusty_rusqlite`](https://github.com/baileyrd/rusty_rusqlite),
[`rusty_libc`](https://github.com/baileyrd/rusty_libc),
[`rusty_acp`](https://github.com/baileyrd/rusty_acp),
[`rusty_tls`](https://github.com/baileyrd/rusty_tls),
[`rusty_serde`](https://github.com/baileyrd/rusty_serde),
[`rusty_lsp`](https://github.com/baileyrd/rusty_lsp),
[`rusty_a2a`](https://github.com/baileyrd/rusty_a2a),
[`rusty_mcp`](https://github.com/baileyrd/rusty_mcp),
[`rusty_stream`](https://github.com/baileyrd/rusty_stream),
[`rusty_url`](https://github.com/baileyrd/rusty_url),
[`rusty_http`](https://github.com/baileyrd/rusty_http),
[`rusty_json`](https://github.com/baileyrd/rusty_json),
[`rusty_oauth`](https://github.com/baileyrd/rusty_oauth), and
[`rustils_async`](https://github.com/baileyrd/rustils_async) — merged one
at a time, so the Crates table above only lists the ones already landed.

A third wave continues the same way, starting with
[`rusty_wire`](https://github.com/baileyrd/rusty_wire) and
[`rusty_std`](https://github.com/baileyrd/rusty_std), and now
[`rusty_err`](https://github.com/baileyrd/rusty_err),
[`rusty_request`](https://github.com/baileyrd/rusty_request),
[`rusty_sqlite`](https://github.com/baileyrd/rusty_sqlite),
[`rusty_time`](https://github.com/baileyrd/rusty_time),
[`rusty_uuid`](https://github.com/baileyrd/rusty_uuid),
[`rusty_wiremock`](https://github.com/baileyrd/rusty_wiremock),
[`rusty_search`](https://github.com/baileyrd/rusty_search) (twelve crates
behind one nested workspace),
[`rusty_vulkan`](https://github.com/baileyrd/rusty_vulkan),
[`rusty_sync`](https://github.com/baileyrd/rusty_sync),
[`rusty_simd`](https://github.com/baileyrd/rusty_simd),
[`rusty_codec`](https://github.com/baileyrd/rusty_codec),
[`rusty_h2`](https://github.com/baileyrd/rusty_h2),
[`rusty_audio`](https://github.com/baileyrd/rusty_audio),
[`rusty_crypto_key`](https://github.com/baileyrd/rusty_crypto_key),
[`rusty_db`](https://github.com/baileyrd/rusty_db) (six crates behind a
second nested workspace),
[`rusty_ansi`](https://github.com/baileyrd/rusty_ansi),
[`rusty_config`](https://github.com/baileyrd/rusty_config),
[`rusty_jinja`](https://github.com/baileyrd/rusty_jinja),
[`rustils`](https://github.com/baileyrd/rustils) (eight crates behind one
nested workspace),
[`rusty_ansder`](https://github.com/baileyrd/rusty_ansder),
[`rusty_boot`](https://github.com/baileyrd/rusty_boot),
[`rusty_whisper`](https://github.com/baileyrd/rusty_whisper),
[`rusty_rdp`](https://github.com/baileyrd/rusty_rdp), and
[`rusty_voice`](https://github.com/baileyrd/rusty_voice) — merged one at
a time, same process.

A fourth wave finished the consolidation, merging the last eleven
standalone repos the same way, starting with
[`rusty_croc`](https://github.com/baileyrd/rusty_croc) and
[`rusty_test`](https://github.com/baileyrd/rusty_test) (six crates behind
one nested workspace) and
[`rusty_inventrory`](https://github.com/baileyrd/rusty_inventrory) (three
crates behind another) and
[`rusty_skillopt`](https://github.com/baileyrd/rusty_skillopt) (four behind
a third) and [`rusty_key`](https://github.com/baileyrd/rusty_key) (eight
behind a fourth) and
[`rusty_llama`](https://github.com/baileyrd/rusty_llama) and
[`rusty_tailscale`](https://github.com/baileyrd/rusty_tailscale) (sixteen
crates behind a fifth nested workspace) and
[`rusty_adk`](https://github.com/baileyrd/rusty_adk) (fourteen behind a
sixth) and [`rusty_provider`](https://github.com/baileyrd/rusty_provider)
(six behind a seventh),
[`rusty_yirp`](https://github.com/baileyrd/rusty_yirp) (nine behind an
eighth), and finally
[`rusty_agent_gateway`](https://github.com/baileyrd/rusty_agent_gateway)
(nine behind a ninth) — one crate at a time, same process. Each of those
eleven repos now carries an archive notice in its README pointing at its
new home under `crates/`.

A fifth merge, separate from that `baileyrd/rusty_*` wave numbering,
brought in [`baileyrd/nexus`](https://github.com/baileyrd/nexus) — a
42-crate microkernel note-taking/AI-agent workspace — under
`crates/nexus/`, same `git subtree` process, full history preserved.
