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

Dependencies between these fourteen crates are wired as workspace `path`
dependencies now that they live in one repo. Dependencies on crates
**outside** this set at the time — `rusty_simd`, `rusty_std`, `rusty_wire`,
`rusty_lsp` — remained pinned `git` dependencies with an explicit `rev`,
unchanged by this merge; each swapped to a `path` dependency once its own
crate joined this monorepo later (`rusty_libc`'s three in-set consumers in
the second wave below, `rusty_lsp`'s own later merge, and `rusty_wire`'s
three consumers — `rusty_font`, `rusty_gui`, `rusty_stream` — noted where
each is discussed, and `rusty_std`'s four consumers — `rusty_font`,
`rusty_gui`, `rusty_gpu`, `rusty_tokio` — likewise). `rusty_simd` is the
one still outstanding.
`mill-term` locates `rusty_git`/`rusty_text`'s `rgit`/`rsed`/
`rawk` binaries via a `PATH`/shared-`target/`-dir lookup rather than a
Cargo library dependency — it shells out to them, not their APIs — so that
relationship stays a build-artifact lookup even though all three now live
in this same workspace.

`rusty_term`'s `gui`/`gui-gpu` backends onto `rusty_gui`/`rusty_gpu` are
currently disabled/unused pending a fix tracked upstream at
[`rusty_gui#9`](https://github.com/baileyrd/rusty_gui/issues/9) — not
addressed by this migration.

`rusty_tokio` doesn't get depended on by any of the other crates in this
repo, and until `rusty_std` merged it didn't depend on any either. Its
`rusty_std` dependency swapped to a `path` dependency along with
`rusty_std`'s three other pre-existing consumers below — its pinned rev
(`3ab2361e`) predated the merge commit that landed on `rusty_std`'s own
`main` (`99135f3`) but was content-identical (an empty diff between the
two), so the swap is a pure mechanical change, not a version bump. Its
`platform`/`platform-linux`/`platform-bsd`/`platform-windows` dependencies
(from `baileyrd/rustils`) stay pinned `git` dependencies — that crate is
outside this monorepo's scope, same as `rusty_simd` above.

`rusty_rusqlite` has no dependencies at all (`[dependencies]` is empty in
its own manifest) and nothing in this repo depends on it yet, so nothing
needed swapping there either.

`rusty_libc` is different: it was already a Linux-only pinned `git`
dependency of three first-wave crates (`rusty_lines`, `rusty_gui`, `rush`),
all pinned to the exact commit this merge brought in — those three now use
a workspace `path` dependency on `crates/rusty_libc` instead. Its own
`crates/rusty_libc/bench` is a standalone benchmark against the real
`libc` crate, deliberately outside the library's own zero-dependency
build (its own `[workspace]` table, same shape as `rusty_lines/bench`) —
excluded from this workspace the same way.

`rusty_acp` has no dependency relationship with any crate already in this
repo (nothing here depends on it, and its own dependencies are all
crates.io crates, not sibling `baileyrd/*` repos), so nothing needed
swapping.

`rusty_tls` had an optional (behind its `rusty-tokio` feature) pinned
`git` dependency on `rusty_tokio`, 27 commits behind what this repo's
`crates/rusty_tokio` had already reached — switched to a workspace `path`
dependency, same as `rusty_libc`'s three first-wave consumers. Its
`crates/rusty_tls/fuzz` is the same standalone-`[workspace]` shape as
`rusty_libc/bench`/`rusty_lines/bench` (needs nightly for libFuzzer
instrumentation) — excluded from this workspace the same way.

`rusty_serde` is itself a three-crate Cargo workspace (`rusty_serde`,
`rusty_serde_derive`, `rusty_serde_erased`), all with empty or purely
intra-workspace `[dependencies]` — no nested `[workspace]` table to
exclude, and no dependency relationship with anything already in this
repo, so nothing needed swapping.

`rusty_lsp` already carried optional `path = "../rusty_tokio"` and
`path = "../rusty_json"` dependencies (behind its `rusty-tokio`/`rusty-json`
features) from before this merge — its own repo was set up assuming a flat
sibling checkout next to those two crates. `../rusty_tokio` now resolves
correctly on its own, since `crates/rusty_tokio` already exists in this
workspace. `rusty_json` hasn't been merged yet, so its entry was pinned
back to a `git` dependency at the commit this merge found at
`baileyrd/rusty_json`'s HEAD — the same shape every other on-deck sibling
crate uses before its own merge lands — to be swapped to a `path`
dependency once `crates/rusty_json` exists. Its own `crates/rusty_lsp/fuzz`
is the same standalone-`[workspace]` shape excluded elsewhere in this file.

`rusty_a2a` has no dependency relationship with anything already in this
repo — its `client`/`server`/`grpc`/`signing` features pull in only
crates.io crates (`reqwest`, `axum`, `tonic`, `p256`, ...), no sibling
`baileyrd/*` repos — so nothing needed swapping.

`rusty_mcp` is itself a two-crate Cargo workspace (`rusty-mcp`, its
scaffold library, and `rusty-mcp-demo`, an example server built on it),
with no dependency relationship with anything already in this repo — its
`[dependencies]` are all crates.io crates (`rmcp`, `axum`, `tokio`, ...).
Its `template/` directory is a `cargo-generate` scaffold whose
`{{ 'Cargo' }}.toml` is a templated filename, not a real `Cargo.toml`, so
it was never a workspace member and needed no exclusion.

Its two member crates leaned on `<field>.workspace = true` inheritance
(edition, license, lints, and most `[dependencies]`) from their own nested
`crates/rusty_mcp/Cargo.toml` — resolved against *this* workspace's root
once they're listed as its members, which doesn't declare a
`[workspace.package]`/`[workspace.dependencies]` of its own. Both
manifests were rewritten with the equivalent literal values instead,
matching every other already-merged crate's shape.

Unifying `rusty_mcp` into the same dependency graph as `rusty_acp`
surfaced a real conflict, the same class of bug as the `rusty_tls`
`CryptoProvider` one: `rmcp` (a `rusty_mcp` dependency) requires
`reqwest >=0.13.2`, while `rusty_acp` requested reqwest's
`rustls-native-certs` feature — present only in `reqwest` 0.13.0/0.13.1,
folded into the `rustls` feature's now-default `rustls-platform-verifier`
backend from 0.13.2 onward. Dropped `rustls-native-certs` from
`rusty_acp`'s feature list (verified against `reqwest` 0.13.2's source
that `rustls-platform-verifier` is used unconditionally once `rustls` is
enabled, so no behavior changed) rather than pin `reqwest` backward,
which would have fought the rest of the ecosystem's forward motion.
`rusty_mcp`'s own dev-dependency on `reqwest = "0.13.4"` was also relaxed
to `"0.13"`, matching its other `reqwest` requirement in the same file —
the exact pin was what forced the unified resolution past 0.13.1 in the
first place.

`rusty_stream` had a pinned `git` dependency on `rusty_tokio` (with its
`thread-per-core`/`io-uring-fs` features, at the exact commit ADR-0002 D3
verified) — switched to a workspace `path` dependency on
`crates/rusty_tokio`, which already carries that commit's
`uring_global_driver` API and beyond. Its other dependency, `rusty_wire`,
was outside this monorepo's scope at the time and stayed pinned `git`;
now that `rusty_wire` has its own entry in this workspace, that pin
became a `path` dependency too (`rusty_font` and `rusty_gui`'s own
`rusty_wire` pins swapped the same way at the same time — see below).

`rusty_stream`'s design is built on io_uring, a Linux-only kernel
interface — its own CI only ever ran on `ubuntu-latest`, and its source
uses `rusty_tokio`'s `io-uring-fs` types unconditionally rather than
behind a `cfg(target_os = "linux")` no-op shell the way `rusty_win32`
does the reverse (Windows-only behind `cfg`, portable everywhere else).
Rather than rewrite its internals to be cross-platform, the workflow
excludes it (`--exclude rusty_stream`) from the `windows-latest` leg's
`--workspace` clippy/build/test steps specifically.

`rusty_url` has no dependency relationship with anything already in this
repo — its one dependency, `idna`, is a crates.io crate — so nothing
needed swapping. Unlike `rusty_stream`, it already `cfg`-gates its
Windows-specific `file://` path handling internally, so it's expected to
be portable across both CI platforms despite its own CI only ever having
run on `ubuntu-latest`.

`rusty_http`'s optional `rusty-tokio` feature pulled `rusty_tokio` as a
pinned `git` dependency, rev-locked to match what its sibling consumers
(`rusty_tls`, `rusty_request`) pinned — swapped to a `path` dependency on
the now-merged `crates/rusty_tokio`, unifying `AsyncRead`/`AsyncWrite`
trait identity across the workspace by construction instead of by rev
bookkeeping. Its other optional adapter depends on real crates.io `tokio`
directly (a `rusty_tail` migration need, per its own `ARCHITECTURE.md`)
and is unrelated to anything in this workspace, so it's left untouched.
Despite its name, `rusty_http`'s own `Url` type is a from-scratch
sans-IO parsing primitive, not a dependency on `rusty_url` — the two
crates don't relate.

`rusty_json` has no dependency relationship of its own — its one
non-dev dependency, `serde`, is a crates.io crate — but it retires a
forward pin: `rusty_lsp`'s optional `rusty-json` feature pinned
`rusty_json` to a `git` rev (tracking a not-yet-merged sibling, same
shape as `rusty_lsp`'s own `rusty_json` pin before this crate existed
in the workspace) matching `rusty_json`'s current `main` HEAD exactly —
swapped to a `path` dependency on the now-merged `crates/rusty_json`.
`rusty_json-derive`, its proc-macro companion, was already a `path`
dependency within the standalone repo and needed no changes beyond
joining the workspace `members` list alongside it.

`rusty_oauth` has zero dependencies of any kind (its own `Cargo.toml`
lists none, in either `[dependencies]` or `[dev-dependencies]`), so
nothing needed swapping despite its `Cargo.toml` naming an HTTP client
and JSON in its description — it hand-rolls both concerns internally
rather than depending on `rusty_http`/`rusty_json`.

`rustils_async` was itself a six-crate nested Cargo workspace
(`reactor-core`, `platform-async`, `platform-async-mock`,
`platform-async-linux`, `threading`, `coreutils-async`) — de-inherited
from `.workspace = true` the same way `rusty_mcp` and `rusty_serde`
were, since this outer root doesn't declare a `[workspace.package]`/
`[workspace.dependencies]` for a nested workspace's fields to keep
resolving against once it joins `members` directly. Its `platform`/
`platform-mock`/`platform-linux` dependencies are pinned `git` revs
against `baileyrd/rustils` — a separate repo outside this monorepo's
consolidation scope, the same shape `rusty_stream`'s `rusty_wire`
dependency used to have before `rusty_wire` itself joined this
workspace (below) — so those stayed pinned `git` dependencies rather
than becoming `path` dependencies; the sibling crates within this same
crate group (`platform-async`, `reactor-core`, `platform-async-linux`)
became `path` dependencies on each other. `platform-async-linux`
self-gates its entire body behind `#![cfg(target_os = "linux")]` at
the crate root (a no-op crate everywhere else), so — unlike
`rusty_stream`'s io_uring dependency — it needed no CI workaround: it
follows `rusty_win32`'s portable-by-internal-`cfg` shape instead.

`rusty_wire` itself has no dependencies of its own (a minimal,
zero-dependency byte cursor) — its own merge is entirely about
retiring three forward pins that predate it: `rusty_font`, `rusty_gui`,
and `rusty_stream` each pinned it to a `git` rev (all three at the
exact same commit, `rusty_wire`'s current `main` HEAD) before it had a
home in this workspace. All three swapped to a `path` dependency on
`crates/rusty_wire` as part of this same import.

`rusty_std` retires four forward pins the same way — `rusty_font`,
`rusty_gui`, `rusty_gpu`, `rusty_tokio` — three at `rusty_std`'s exact
current `main` HEAD, `rusty_tokio`'s one commit earlier but
content-identical (see above). `rusty_std` also has its own two
dependencies on already-merged siblings, `rusty_libc` and `rusty_win32`
(optional, feature-gated per target OS) — both swapped to `path`
dependencies too. Unlike every other pin retired in this series so
far, `rusty_win32`'s pinned rev was **not** current: a real diff (2,984
insertions across 31 files) separated it from this workspace's
`crates/rusty_win32`, reflecting genuine API growth since `rusty_std`
took that pin. Verified by building and testing `rusty_std` against
the current `rusty_win32` (both toolchains, plus a Windows
cross-compile check) rather than assuming compatibility from the
mechanical rev-to-path pattern the other swaps in this series follow.

`rusty_err` retires its one forward pin, `rusty_std`, the same way —
swapped to a `path` dependency on the now-merged `crates/rusty_std`.
Unusually, nothing in `rusty_err`'s own source actually references
`rusty_std` (verified via a source grep); the dependency line exists in
`Cargo.toml` but is otherwise unused, so the swap carries no compatibility
risk regardless of how far the pinned rev had drifted. `rusty_err` was
also its own tiny nested Cargo workspace (a `[workspace]` table with one
member, `derive`) — de-inherited the same way `rustils_async` was: the
nested `[workspace]` table removed, `derive` added directly to this
workspace's `members` alongside `rusty_err` itself.

`rusty_request` retires three forward pins at once — `rusty_tokio`,
`rusty_tls`, `rusty_http` — all previously pinned to specific `git` revs
that had to move together by construction (Cargo treats two different
revs of the same git dependency as two unrelated crates, and
`rusty_tls::AsyncTlsStream`/`rusty_http::async_tokio::AsyncTransport` are
both generic over `rusty_tokio`'s `AsyncRead`/`AsyncWrite` traits — a
`path` dependency resolves that concern by construction, since there's
only ever one `rusty_tokio` in the build graph now). All three pins were
substantially behind this workspace's current `main` (tens of thousands
of lines of diff across all three, more than any prior swap in this
series) — verified safe not by diff-reading but by running
`rusty_request`'s own extensive integration suite against the swapped
path deps: 43 unit tests plus 54 client tests and 5 HTTPS/TLS/pinned-CA
tests (`tests/client.rs`/`tests/https.rs`, which exercise the default
`rusty_tokio` backend directly and are `cfg`'d out under the crate's own
optional `tokio` feature) all pass unmodified.

`rusty_sqlite` wraps real crates.io `rusqlite` (bundled SQLite) — no
relationship to `rusty_rusqlite` (a from-scratch reimplementation aiming
for `rusqlite` API parity); the two are independent, distinctly-purposed
crates that happen to share a naming root. Merging it surfaced a real
Cargo constraint rather than an actual code bug: `rusty_acp`'s optional
`sqlx` dependency declares (but doesn't activate) `sqlx-sqlite`, an
optional dependency of its own requiring `libsqlite3-sys ^0.30.1`.
Cargo's `links` uniqueness check considers every optional dependency
*declared* in the graph, not just the ones actually activated for a given
build — so `rusty_sqlite`'s original `rusqlite = "0.31"` (pulling
`libsqlite3-sys ^0.28.0`) was a hard resolve failure the moment both
crates shared a workspace, regardless of features. Bumped to
`rusqlite = "0.32.1"`, the lowest version pulling the same
`libsqlite3-sys ^0.30.1` `sqlx-sqlite` already needs, unifying the two;
the crate's full test suite (17 tests, 2 doc-tests) passes unmodified
against the bump.

`rusty_time` retires its one forward pin, `rusty_std`, the same way —
swapped to a `path` dependency on the now-merged `crates/rusty_std`.

`rusty_uuid` has zero dependencies of any kind, so nothing needed
swapping — its own merge is just the subtree add plus workspace wiring.

`rusty_wiremock` needed no pin retirement at all: its three dependencies
(`rusty_http`, `rusty_json`, `rusty_std`) were already `path` dependencies
in the standalone repo's own `Cargo.toml`, anticipating this exact
workspace layout — all three were already merged siblings by the time
`rusty_wiremock` joined, so the subtree merge just needed wiring into
`members`. Its own crate has no platform-specific code, so no Windows
cross-compile check applies.

`rusty_search` is twelve crates behind one nested workspace merged at
once — `rusty-search-core` plus ten backend implementations
(`rusty-search-memory`, `rusty-search-tantivy`, `rusty-search-sqlite-fts5`,
`rusty-search-elasticsearch`, `rusty-search-meilisearch`,
`rusty-search-opensearch`, `rusty-search-solr`, `rusty-search-algolia`,
`rusty-search-azure-search`, `rusty-search-cloud`) plus the top-level
`rusty-search` facade crate. Its own `Cargo.toml` was a full nested Cargo
workspace with `[workspace.package]` and `[workspace.dependencies]` tables
(not just a plain member list, unlike `rustils_async`/`rusty_err`) — the
former de-inherited by hoisting both tables into this workspace's root
`Cargo.toml` (scoped so only `rusty_search`'s own crates opt in via
`field.workspace = true`; every other crate here keeps its own literal
`[package]`/`[dependencies]` and is unaffected), the latter's internal
crossrefs (`rusty-search-core = { workspace = true }` and friends)
re-pathed to the crates' new nested location. Six pins across the
workspace retired to `path` dependencies on now-merged siblings:
`rusty_serde` (root `workspace.dependencies`), `rusty_err`
(`rusty-search-core`), `rusty_request`/`rusty_uuid` (four HTTP-backed
crates plus `rusty-search-opensearch`, which only pins `rusty_request`),
`rusty_time` (`rusty-search-tantivy`/`rusty-search-sqlite-fts5`), and
`rusty_sqlite` (`rusty-search-sqlite-fts5`). `rusty_serde` and `rusty_uuid`
matched their pins exactly; `rusty_err`/`rusty_time`/`rusty_request`'s
pins differed only in the same mechanical git-to-path edits already made
to those crates' own `Cargo.toml`s. `rusty_sqlite`'s pin was genuinely
stale — its `Pool`/`PooledConnection` type moved off `r2d2` onto a
hand-rolled implementation, and `Error::Pool(r2d2::Error)` became
`Error::PoolTimeout` — but `rusty-search-sqlite-fts5` never touches the
`pool` feature or either type, so the swap carries no compatibility risk
in practice; verified by grepping its source for every symbol that moved
before swapping, not just building green after. Full test suite (196
tests across the twelve crates, 3 doc-tests) passes unmodified.

`rusty_vulkan` needed no pin retirement: its one dependency, `rusty_win32`,
was already a `path` dependency in the standalone repo's own `Cargo.toml`.
The standalone repo had no CI, so two real bugs reached this merge
undetected: `QueueFamilyProperties`'s capability-flag accessors
(`supports_graphics`/`_compute`/`_transfer`/`_sparse_binding`) masked
against a stub `ffi_consts` module that hardcoded every flag constant to
`0` on non-Windows targets — `raw_flags & 0 != 0` is always `false`
regardless of the real flags, which is exactly what `clippy`'s
`bad_bit_mask` lint (`-D warnings` in this workspace's CI) exists to catch.
Fixed by splitting the impl per `cfg(windows)`, with the non-Windows arm
honestly returning `false` from each accessor instead of laundering the
same answer through a fake all-zero mask; `raw_flags` itself is now
`cfg(windows)`-only too, since `QueueFamilyProperties` is never
constructed off Windows. Separately, `creating_an_instance_succeeds_when_a_driver_is_present`'s
driver-absent skip logic matched only `VulkanError::LoaderNotFound`, not
the distinct `VulkanError::UnsupportedPlatform` this crate returns on any
non-Windows target (including this workspace's own `ubuntu-latest` CI
runner) — a real panic, not a flake, now fixed to skip on either variant.
CI surfaced a third gap in the same skip logic: GitHub's `windows-latest`
runner ships `vulkan-1.dll` but has no real GPU driver registered behind
it, so `vkCreateInstance` there returns `VK_ERROR_INCOMPATIBLE_DRIVER`
rather than failing to find a loader at all — also now handled as a skip
case, alongside the other two.

`rusty_sync` needed no pin retirement either: its one dependency,
`rusty_std`, was already a `path` dependency in the standalone repo's own
`Cargo.toml`. Its existing concurrency test suite (spinlock mutual
exclusion and channel send/recv, each also exercised under real OS
threads, not just single-threaded) passes unmodified.

`rusty_simd` has zero dependencies of any kind, so nothing needed
swapping — its own merge is just the subtree add plus workspace wiring.

`rusty_codec` needed no pin retirement: its two dependencies, `rusty_wire`
and `rusty_std`, were already `path` dependencies in the standalone repo's
own `Cargo.toml` (`../rusty_wire`, `../rusty_std`), and both are already
siblings under this workspace's own `crates/`, so the relative paths
resolved unchanged.

`rusty_h2` has zero dependencies of any kind, so nothing needed
swapping — its own merge is just the subtree add plus workspace wiring.

`rusty_audio` needed no pin retirement: its dependencies (`rusty_std`,
plus `rusty_win32` on Windows and `rusty_libc` on Linux) were already
`path` dependencies pointing at sibling directories under `crates/`.

`rusty_crypto_key` has zero dependencies of any kind, so nothing needed
swapping — its merge is just the subtree add plus workspace wiring.

`rusty_db` was its own nested Cargo workspace (six crates: `rusty-db-core`,
`rusty-db-derive`, `rusty-db-sqlite`, `rusty-db-postgres`, `rusty-db-mysql`,
and the `rusty-db` facade), de-inherited the same way as `rusty_search`
before it — with one difference: `rusty_search`'s `[workspace.package]`
(version/edition/license/repository) was hoisted directly into this
workspace's own, since none of its fields collided. `rusty_db`'s did
collide (`license = "MIT"` vs. `rusty_search`'s `"MIT OR Apache-2.0"`;
a different `repository` URL), so its six crates were converted to
literal `[package]` fields instead — only its *dependencies* were
hoisted into `[workspace.dependencies]`, including a path swap for its
already-merged siblings `rusty_tokio`/`rusty_wire`/`rusty_json`/`rusty_std`,
and a widened `tokio` feature set (union of what `rusty_search` and
`rusty_db` each need). Full test suite (108 test-result blocks, all
passing) — the ~50 MySQL/PostgreSQL-backed test files gracefully skip
without a reachable server, by the crate's own design (opt in via
`MYSQL_TEST_URL`/a default local URL; connect-or-skip, never fail).

`rusty_ansi` needed no pin retirement: its one dependency, `unicode-width`,
is an ordinary crates.io crate, not a sibling `baileyrd` repo. The
standalone repo's README carried a CI badge but had no `.github/workflows`
directory, so this workspace's `-D warnings` clippy gate was the first
time it actually ran: `manual_strip` (indexing a slice by hand after
`starts_with` instead of `strip_prefix`, twice) and
`manual_pattern_char_comparison` (an `||` closure comparing against two
chars instead of a `[char; 2]` pattern) — both fixed, no behavior change.

`rusty_config` has zero dependencies of any kind, so nothing needed
swapping — its merge is just the subtree add plus workspace wiring.

`rusty_jinja` depends on `rusty_regx`, `rusty_json`, and `rusty_std` via
relative `path` dependencies, all already merged as siblings under this
workspace's `crates/` — no dependency swap needed, just the subtree add
plus workspace wiring.

`rusty_ansder` depends on `rusty_wire`, `rusty_simd`, `rusty_regx`,
`rusty_json`, and `rusty_std` via relative `path` dependencies, all
already merged as siblings under this workspace's `crates/` — no
dependency swap needed. The standalone repo had no clippy CI: this
workspace's `-D warnings` gate caught two `clippy::bool_assert_comparison`
lints in its own test module (`assert_eq!(x, true)`/`assert_eq!(x, false)`
instead of `assert!(x)`/`assert!(!x)`), fixed with no behavior change.
Also not `cargo fmt`-clean under this workspace's `rustfmt.toml`; reformatted.

`rusty_boot` depends on twenty already-merged siblings (`rusty_wire`,
`rusty_regx`, `rusty_json`, `rusty_compress`, `rusty_std`, `rusty_sync`,
`rusty_time`, `rusty_err`, `rusty_codec`, `rusty_audio`, `rusty_jinja`,
`rusty_tokio`, `rusty_http`, `rusty_lines`, `rusty_font`, `rusty_gui`,
`rusty_gpu`, `rusty_vulkan`, `rush`, `rusty_term`, plus `rusty_libc`/
`rusty_win32` behind target-cfg'd dependencies) via relative `path`
dependencies — no swap needed, since it demonstrates the full merged
stack booting end to end. Not `cargo fmt`-clean under this workspace's
`rustfmt.toml`; reformatted with `cargo fmt --all`, no behavior change.

`rusty_whisper` depends on `rusty_simd`, `rusty_wire`, and `rusty_std`
via relative `path` dependencies, all already merged as siblings under
this workspace's `crates/` — no dependency swap needed. Its optional
`mic` feature (off by default, only enabled here by this workspace's
`--all-features` build) pulls in `cpal` for native microphone capture,
which links against ALSA via `alsa-sys`/pkg-config on Linux — this
workspace's CI now installs `libasound2-dev` on the Linux leg
accordingly (no Windows equivalent needed; `cpal` talks to WASAPI
directly there).

`rustils` was its own nested Cargo workspace of eight crates (`platform`,
`platform-mock`, `platform-parity`, `platform-linux`, `platform-windows`,
`platform-bsd`, `winargv`, `coreutils`), de-inherited the same way as
`rusty_search` and `rusty_db` before it: its `license = "MIT"` and
`version = "0.27.0"` collide with values already hoisted into this
workspace's own `[workspace.package]`, so its eight crates keep literal
`[package]` fields instead of inheriting, and only its dependencies and
`[workspace.lints]` table were hoisted. `platform-linux` and
`platform-windows` each carried an optional, feature-gated pinned git
dependency on `rusty_libc`/`rusty_win32` respectively (their "Track P"/
"Track W" raw-syscall paths) — both are already merged siblings here, so
both pins retired to plain path dependencies, same as every other pin
retired in this series. `coreutils` had the same treatment for its
`rusty_regx`/`rpath` dependencies. `platform-linux`'s Secret Service
integration tests spawn a real `dbus-daemon`/`gnome-keyring-daemon`, so
this workspace's own CI now installs both on the Linux leg, matching
`rustils`' own CI.

`rusty_rdp` depends on `rusty_wire` and `rusty_ansder` (both already
merged siblings) unconditionally via relative `path` dependencies, plus
`rusty_tls` (also an already-merged sibling `path` dependency) and
`rustils`' `platform`/`platform-mock`/`platform-linux` behind optional
features. Those three `platform*` crates were previously pinned to a
specific `rustils` git rev; retired to plain path dependencies now that
`rustils` is a merged sibling here, same as every other pin retired in
this series.

`rusty_voice` depends on nine already-merged siblings (`rusty_whisper`,
`rusty_audio`, `rusty_std`, `rusty_tokio`, `rusty_json`, `rusty_err`,
`rusty_gui`, `rusty_gpu`, `rusty_font`), all unconditionally via relative
`path` dependencies. The standalone repo already laid these out as
`../rusty_whisper`-style siblings, so nothing needed rewiring — the
subtree add plus the workspace member entry was the whole job.

`rusty_croc` has no dependency relationship with anything already in this
repo — its dependencies are all crates.io crates (RustCrypto's `aes-gcm`,
`chacha20poly1305`, `argon2`, `p256`/`p384`/`p521`, plus `zip`, `ignore`,
`socket2`, ...), no sibling `baileyrd/*` repos — so nothing needed
swapping. Its `crates/rusty_croc/fuzz` is the same standalone-`[workspace]`
shape as `rusty_tls/fuzz`/`rusty_lsp/fuzz` (nightly-only libFuzzer harnesses
over its wire parsers) — excluded from this workspace the same way.
Joining the workspace's single dependency resolution did surface one real
`-D warnings` failure the standalone repo's own CI never hit, because its
committed lockfile pinned `generic-array` at 0.14.7 while this workspace
already resolves 0.14.9 — the release that deprecates the whole crate,
`GenericArray::from_slice` included ("please upgrade to generic-array
1.x"). Four calls in `crypt.rs` (the AES-256-GCM and XChaCha20-Poly1305
nonce constructions) therefore became hard errors under this workspace's
clippy gate. Rewritten to the `From<&[T]> for &GenericArray` conversion
that `from_slice` itself delegates to — same panic-on-wrong-length
contract, same wire format, no behavior change — verified by its full
49-test suite (including the vectors checked against the real Go croc)
passing unmodified.

`rusty_test` (the `portable-runtime-contract` spike) is six crates behind
one nested workspace: `contract` (the trait boundary), `compat` (the
per-host adapter), `conformance` (the verification layer), and three
reference tools — `stat-tool`, `proc-runner`, `pty-shell`. None of them
depend on anything already in this repo; their dependencies are all
crates.io crates (`cap-std`, `portable-pty`, `dirs`, `thiserror`,
`anyhow`), so nothing needed swapping. Its own `[workspace.package]` did
**not** collide — `edition = "2021"` and `license = "MIT OR Apache-2.0"`
are the values this root already carries — so, unlike `rusty_db`/`rustils`,
its six crates keep inheriting via `field.workspace = true` (the
`rusty_search` treatment); only its `publish = false` was new and was added
to this root's `[workspace.package]`, opted into by those six alone. Its
`[workspace.dependencies]` were hoisted the same way, with one exception:
`thiserror`. `rusty_test` asked for `"2.0"` while this root already pins
`"1"` for `rusty_db`/`rustils`, so `contract` keeps a literal
`thiserror = "2"` of its own rather than forcing a major bump on unrelated
crates — the two majors coexist in the graph, which is exactly what
Cargo's semver-compatibility rules allow.

The one thing this merge genuinely broke, and had to fix, is
`conformance`'s `tests/layering.rs` — a manifest-reading test that enforces
the layer model (`contract` depends on nothing in-workspace; adapters may
see `contract`; tools may see `contract`+`compat`; nothing reaches upward
or sideways) precisely because "nothing in the Rust toolchain enforces
this". It located the workspace manifest two directories above its own
`CARGO_MANIFEST_DIR` and demanded a declared layer for *every* member it
found there. After the merge that manifest is this monorepo's root, four
levels up and ~100 members wide, so all three of its manifest-driven tests
panicked. Repointed at the real root and filtered through a
`GROUP_PREFIX = "crates/rusty_test/"` constant, so the layer model stays a
claim about this crate group rather than silently becoming an unanswerable
claim about the whole monorepo. The check itself is unchanged — including
its deliberate refusal to skip dependency lines it cannot parse — and all
four of its tests pass again, alongside the rest of the group's suite (31
tests).

`rusty_inventrory` is three crates behind one nested workspace —
`inventory-core` (the encrypted SQLite index and the per-tool readers),
`inventory-cli` (the `inv` binary), and `inventory-tauri` (the menu-bar
shell). None of them depends on a sibling in this workspace, so there were
no pins to retire; the interesting parts of the merge were all collisions.
Its `[workspace.package]` collided three ways with this root's
(`rust-version = "1.82"` vs. `"1.75"`, `license = "MIT"` vs.
`"MIT OR Apache-2.0"`, and its own `repository` URL), so its three crates
were converted to literal `[package]` fields — the `rusty_db`/`rustils`
treatment, not `rusty_search`'s — and only its dependencies were hoisted.
`thiserror` is the same `"2"`-vs-`"1"` split `rusty_test` hit, resolved the
same way (a literal `thiserror = "2"` on `inventory-core`).

The real collision was `rusqlite`. `inventory-core` asked for `"0.37"`,
which needs `libsqlite3-sys ^0.35`, and `libsqlite3-sys` is a `links =
"sqlite3"` crate: exactly one version of it may exist in a build graph.
This workspace already pins the other end of that constraint — `sqlx 0.8`'s
`sqlx-sqlite` requires `libsqlite3-sys ^0.30.1`, which is why `rusty_sqlite`
was itself bumped to `rusqlite = "0.32.1"` when it merged (see above). There
is no version of `rusqlite` that satisfies both, and the only way *up* would
be `sqlx 0.9` (whose `sqlx-sqlite` widens to `>=0.30.1, <0.38`) — a major
upgrade of a dependency shared by `rusty_db`'s five crates and
`rusty-search-sqlite-fts5`, far outside this merge's blast radius. So
`inventory-core` came *down* to `rusqlite = "0.32.1"` instead, unifying all
three consumers on `libsqlite3-sys 0.30.1`. Five minor versions backward is
more API risk than any pin retired in this series so far, so it was verified
by running the crate's own suite rather than by reading the changelog: 79
tests pass unmodified, and `inventory-core`'s usage turns out to be the
stable core of the API (`Connection`, `OpenFlags`, `params!`,
`prepare`/`execute`/`query_map`, `rusqlite::Error`) throughout.

Joining this workspace's dependency resolution also surfaced the same
`generic-array` deprecation `rusty_croc` hit — the standalone lockfile
pinned 0.14.7, this workspace resolves 0.14.9 — as three `-D warnings`
errors in `db.rs`'s sealed-index code (`Nonce::from_slice`, and
`Key::<Aes256Gcm>::from_slice` twice). Rewritten to the same
`From<&[T]> for &GenericArray` conversion, no behavior change.

`inventory-tauri` is the first Tauri shell in this workspace. Unlike
`rusty_key`'s desktop shell (which its own repo kept out of its workspace
entirely) it is a full workspace member here, so this workspace's CI now
installs the toolkit its own CI already did — `libwebkit2gtk-4.1-dev`,
`libgtk-3-dev`, `libayatana-appindicator3-dev`, `librsvg2-dev` — plus
`libdbus-1-dev`, which `inventory-core`'s Linux keyring backend
(Secret Service, deliberately not the kernel keyring: keyutils does not
survive a reboot and would silently orphan an existing index) needs
regardless of the shell. Windows and macOS use OS-native APIs for both and
need nothing extra. Its frontend is a checked-in static `ui/index.html`,
not an npm build, so `cargo build -p inventory-tauri` is the whole build.

`rusty_skillopt` is four crates behind one nested workspace
(`skillopt-core`, `skillopt-model`, `skillopt-envs`, `skillopt-cli`), with
no dependency relationship to anything already here — its dependencies are
all crates.io crates — so, again, no pins to retire. Its
`[workspace.package]` collided on `license` (`"MIT"` vs. this root's
`"MIT OR Apache-2.0"`), so its four crates carry literal `[package]` fields
and only its dependencies were hoisted, the `rusty_db` shape. Two of those
hoists needed widening rather than a straight copy: `tokio`'s feature list
is now the union of what `rusty_search`, `rusty_db` and `rusty_skillopt`
each need (`fs`/`process`/`io-util` added), and `chrono` gained `serde`,
which only `skillopt-core` asks for — the same "widen, don't split"
call made when `rusty_db` merged. `thiserror` is the same `"2"`-vs-`"1"`
split as `rusty_test`'s and `rusty_inventrory`'s, resolved the same way.

The one thing worth stating plainly: this workspace now carries two
semver-incompatible `reqwest` majors — 0.13.4 (`rusty_acp`, `rusty_mcp`,
whose unification is described above) and 0.12.28, which `skillopt-model`
requests. They are distinct crates to Cargo, so they coexist without a
resolution conflict, and no type from either crosses between those crate
groups. Bumping `skillopt-model` to 0.13 to collapse them would be an API
change to a crate this merge is not otherwise touching, so it was left
alone; the duplication is a build-size cost, not a correctness one.
The full suite (68 tests, 2 environment-gated ones ignored) passes
unmodified.

`rusty_key` (Rusty Keys) is eight crates behind one nested workspace —
`rk-config`, `rk-observe`, `rk-constrain`, `rk-feed`, `rk-kernel`,
`rk-mcp`, `rk-compose`, and the `rk-app` binary — none of which depends on
a sibling already in this workspace, so there were no pins to retire. Its
`[workspace.package]` collided on `rust-version` (1.88 vs. this root's
1.75) and `license` (`"MIT"`), so its crates carry literal `[package]`
fields and only its dependencies were hoisted — `rusty_db` treatment, for
the fourth time in this wave.

It is the first merged crate group to have had its own
`[workspace.lints]`, and that table *did* collide: this root's is
`rustils`' (`unsafe_code = "warn"`, `undocumented_unsafe_blocks = "deny"`),
while `rusty_key`'s is `unsafe_code = "forbid"` — strictly stronger.
Leaving `[lints] workspace = true` in place would have silently downgraded
eight crates from `forbid` to `warn`, which is exactly the kind of quiet
policy loss a mechanical merge should not cause, so each crate got a
literal `[lints.rust] unsafe_code = "forbid"` instead. `rustils`' eight
crates keep inheriting the root table unchanged.

Its `crates/rusty_key/desktop/src-tauri` is a Tauri 2 shell that its own
repo deliberately kept out of its core workspace (`exclude = ["desktop"]`,
with a `[workspace]` table of its own) so `cargo build --workspace` stays
lean — the same standalone shape as `rusty_libc/bench` and the various
`fuzz` directories, and excluded here the same way. Note this is the
*opposite* call from `inventory-tauri` above, which is a full member: the
difference is upstream's own, and preserving each repo's own decision keeps
`cargo build --workspace` honest in both directions. Verified that the
excluded shell still builds across the boundary after the merge — its
`../../crates/app`-style path dependencies now resolve their
`field.workspace = true` inheritance against *this* root, which is exactly
what the de-inheritance above makes correct.

Two more duplicate-major pairs entered the graph with it, both benign for
the same reason as `rusty_skillopt`'s `reqwest` split: `rmcp` 0.9.1
(`rk-app`/`rk-mcp`'s optional `rmcp` feature) alongside 3.1.4
(`rusty-mcp`, `rusty_homelab_mcp`, `rusty_provider`), and `axum` 0.7.9
(`rk-app`'s optional `gateway` feature) alongside 0.8.9. Nothing passes
types between the two groups, and upgrading either would be an API change
to a crate this merge is not otherwise touching. Its full suite (194 tests
across the eight crates) passes unmodified; it was not `cargo fmt`-clean
under this workspace's settings and was reformatted with `cargo fmt --all`,
same as `rusty_ansder`/`rusty_boot` before it.

`rusty_llama` depends on `rusty_simd` and `rusty_std` via relative `path`
dependencies (`../rusty_simd`, `../rusty_std`), both already merged
siblings under this workspace's `crates/`, so the paths resolved unchanged
— no swap needed, the same situation `rusty_codec` and `rusty_voice` were
in. Two things did need fixing, both of them things the standalone repo's
own CI could not have caught.

First, `--all-features` here means the `cuda` feature is on, and this
workspace's `-D warnings` clippy gate found two `unnecessary_cast`s
(`(i % 7) as i32` where `i` is already `i32`) in `backend/cuda.rs`'s test
fixtures — dead code to any build that doesn't opt into `cudarc`. Removed;
no behavior change.

Second, and more interesting: `chat::tests::render_jinja_threads_context_variables`
started failing on a boolean. `rusty_llama` renders a GGUF's embedded
`tokenizer.chat_template` through `minijinja`, and its standalone lockfile
pinned 2.21.0; unified against this workspace, `minijinja` resolves to
2.24.0, and 2.22.0 deliberately changed none/bool rendering to
`None`/`True`/`False` "for Jinja2 compatibility" (upstream #913). The test
asserted Rust's `true`. Since the entire point of that code path is to
render templates *authored for Python Jinja2*, the new rendering is the
correct one and the assertion was the stale half — updated to `True`, with
the reason recorded at the assertion rather than left as a mystery for the
next reader. Nothing else in the crate interpolates a bare boolean (checked
by grep, not assumed), and chat templates overwhelmingly branch on
`add_generation_prompt` rather than print it. Full suite: 248 tests pass,
49 ignored (those need real model weights on disk, by the crate's own
design). Not `cargo fmt`-clean under this workspace's settings; reformatted
with `cargo fmt --all`.

`rusty_tailscale` (tailscale-rs / `rusty_tail`) is the largest crate group
in this wave: fifteen `ts-*` crates plus its `xtask` harness, and the first
whose `members` list was a `crates/*` glob rather than an explicit set —
expanded to sixteen literal entries here, since this root's `members` names
every crate individually. Its `[workspace.package]` collided on `version`
(0.0.1), `edition` (2024 — the first edition-2024 crates in this workspace)
and `repository`, so its crates carry literal `[package]` fields; only its
dependencies were hoisted, with `rusty_http` and `rusty_crypto_key`
re-pathed from `../rusty_http`-style (relative to its own former root) to
this root's `crates/` layout.

It retires two pins: `ts-magicsock` and `ts-tun` each pinned `platform` and
`platform-linux` to a `rustils` git rev (`b8bf992f`), for rustils' D16 Net
and D14 Tun surfaces respectively. `rustils` is already a merged sibling,
so both became `{ workspace = true }` against this root's existing
`platform`/`platform-linux` path entries — the same retirement `rusty_rdp`
made for its own `platform*` pins.

Verifying that swap turned up something worth stating plainly: `rusty_tail`
has **no CI of its own** (no `.github/` directory at all), and its `main`
does not compile on Linux. Two independent breaks, both confirmed against
the standalone repo before touching anything here, so neither is a
merge artifact:

* `ts-magicsock`'s `MagicUdp` calls `send_to`/`recv_from`/`local_addr` on a
  `LinuxUdpSocket` without `platform::net::UdpSocket` in scope. Those three
  are trait methods (only `bind` and `set_nonblocking` are inherent), at
  the pinned rev exactly as much as on current `rustils` — checked both, so
  this is not drift the pin was hiding. Fixed with the trait import.
* `ts-cli`'s `localapi::Error` declares `Status(StatusCode)`, which nothing
  constructs, while `request()` constructs `Error::Api { status, body }`,
  which does not exist. The definition was left behind when the call site
  grew a response body. Replaced the dead variant with the one the code
  actually builds — the body is what makes a LocalAPI failure diagnosable.

Beyond that, the usual `-D warnings` sweep: the `generic-array` 0.14.9
deprecation again (`Key::from_slice` in `ts-control`'s Noise handshake and
frame codec, `nonce.as_slice()` in `ts-disco` and `ts-derp`), all rewritten
to the non-deprecated equivalents with no behavior change, plus a
`cargo fmt --all` pass. Its 93 tests pass. Despite being a Linux-first
design (TUN devices, netns integration tests, `platform-linux`), all
sixteen crates cross-compile cleanly for `x86_64-pc-windows-gnu` — the
Linux-specific paths are `cfg`-gated internally and `platform-linux`
self-gates its whole body — so, unlike `rusty_stream`, it needs no
`windows-exclude` in CI.

`rusty_adk` is fourteen crates behind one nested workspace — eleven library
crates plus three runnable examples, all of which its own `members` list
carried, so all fourteen join this root's. Its `[workspace.package]`
collided on `rust-version` (1.85), `license` (Apache-2.0) and `repository`,
so its crates carry literal `[package]` fields; only its dependencies were
hoisted.

Three hoists needed widening rather than a straight copy — the same
"widen, don't split" call made for `rusty_db`'s and `rusty_skillopt`'s:
`tokio` gained `io-std`, `uuid` gained `serde`, and `serde_json` gained
`float_roundtrip`. That last one is worth naming: `rusty_adk` needs it
because its SQLite session store holds events as JSON and round-trips an
f64 timestamp through them, and without it `serde_json`'s fast float path
can land a ULP away. Since Cargo unifies features across a single crate
version, enabling it in one member enables it everywhere regardless — so it
is declared at the root, where it is visible, rather than buried in a member
manifest where it would look local and not be. Two hoists were declined:
`thiserror` (`"2"` against this root's `"1"`, the recurring split) and
`schemars` (`"0.8"` against `rusty_key`'s `"1.0.4"`) both stay literal on
the `adk-*` crates that use them.

`adk-sessions` hit exactly the `rusty_inventrory` constraint again — an
optional `rusqlite = "0.37"`, and Cargo's `links` uniqueness check counts
optional dependencies it never activates — so it now uses this root's
`rusqlite = "0.32.1"` entry, unifying on `libsqlite3-sys 0.30.1` with
`rusty_sqlite` and `inventory-core`.

The pin it retires is `adk-a2a`'s `rusty_a2a` — a *branch-tracking* git
dependency (no `rev`), unlike every other pin retired in this series, so
what it actually resolved to was whatever `baileyrd/rusty_a2a`'s default
branch pointed at when its lockfile was last written: `983d9810`. That is
42 commits behind the commit this workspace's `crates/rusty_a2a` was
imported from (9,729 insertions / 344 deletions across 66 files), and three
commits behind current `main` on top of that — one of which, boxing
`require_auth`'s error for `clippy::result_large_err`, is a real signature
change. Swapped to a `path` dependency and verified the way
`rusty_request`'s three-pin retirement was: by running `adk-a2a`'s own
suite against the swap — 13 unit tests, 8 end-to-end and 10 remote-transport
tests, all passing unmodified — rather than by reading the diff. Full group:
278 tests pass, 2 ignored.

`rusty_provider` is six crates behind one nested workspace (`rp-core`,
`rp-providers`, `rp-router`, `rp-mcp`, `rp-server`, `rp-cli`): an
OpenAI-compatible HTTP API in front of six upstream providers, with
config-driven fallback chains. Its `[workspace.package]` collided on
`license` (`"MIT"`), so its crates carry literal `[package]` fields and
only its dependencies were hoisted.

Its one pin retirement is `rusty-mcp`, and — like `rusty_adk`'s `rusty_a2a`
— it was a *branch-tracking* git dependency rather than a `rev`-pinned one.
What it resolved to (`ee6c7637`) is six commits and 328 insertions / 54
deletions behind the commit this workspace's `crates/rusty_mcp` was
imported from, plus two commits since (the `rusty_url` sovereignty swap and
its `auth/config` follow-on, 53 insertions / 43 deletions). Swapped to a
`path` dependency on `crates/rusty_mcp/crates/rusty-mcp` and verified by
running the full group's suite — 905 tests, including `rp-mcp`'s own
scaffold tests and `rp-server`'s HTTP surface tests — all passing
unmodified.

Two more root hoists were widened rather than copied: `reqwest` gained
`stream` (`rp-providers` streams SSE deltas from upstream providers), and
`tokio` gained `full` — `rusty_provider` asks for the bundle, and since
Cargo unifies a crate's features across the graph, declaring it at the root
is the honest place for it rather than leaving it to look member-local.
`rp-router` also keeps `rusqlite`'s `bundled` feature explicitly when
opting into this root's `rusqlite = "0.32.1"` entry, which its own manifest
had asked for at `"0.32"` — the one crate in this wave whose SQLite
requirement already matched the workspace's `libsqlite3-sys` line without
any adjustment. No lint or format fixes were needed.

`rusty_yirp` (sessionmgr) is nine crates behind one nested workspace: a
Windows-native session manager for AI coding-agent CLIs, where each session
optionally lives in its own git worktree and survives the manager closing.
Its `[workspace.package]` collided on `rust-version` (1.88) and `license`
(`"MIT"`), so its crates carry literal `[package]` fields; `publish = false`
happens to match the value `rusty_test` put in this root's
`[workspace.package]`, but it is written literally too rather than
half-inheriting one table.

Four pins retire here. `rusty_tokio` was pinned at rev `6e6f1847` in its own
`[workspace.dependencies]` — deliberately, with an ADR behind it — and is
now this root's `path` entry, which the three crates that use it pick up
unchanged through `rusty_tokio = { workspace = true }`. `sessionmgr-pty`
separately pinned `platform`, `platform-linux` and `platform-windows` to a
`rustils` rev (`ce9259d4`) with a comment explaining that a second,
differing pin would build two copies of the platform layer with
non-interoperating handle types into one binary. That concern is exactly
what a `path` dependency settles by construction — and note this wave
merged a *third* consumer, `rusty_tailscale`, pinning `rustils` at a
different rev again (`b8bf992f`); all of them now resolve to the one
`crates/rustils` in this workspace, which is the whole point.

`sessionmgr-desktop` is the third Tauri shell to arrive in this wave and
the second to join as a full workspace member (its own repo had it in
`members`, so it stays there — the same "preserve upstream's own call"
rule that keeps `rusty_key`'s excluded). Its frontend is a checked-in
static `ui/`, so no npm step is involved.

Its 130 tests pass — but one of them, `sessionmgr-daemon`'s
`a_fresh_claude_session_reaches_needs_input_on_its_own`, is worth naming as
a known environment-dependent case in the same class as `mill-term`'s: it
drives a *real* `claude` session to prove its tier-3 `needs_input` pattern
match against live output, and skips cleanly when `claude` is not on
`PATH`, which is the state of a GitHub-hosted runner. On a machine that
does have the CLI installed but cannot complete an interactive
trust-folder prompt, the guard passes and the test then times out after 60
seconds. Not a merge artifact and not a CI concern — verified by running
the same test with `claude` off `PATH`, where it skips and the suite is
130/130. All eight non-Tauri crates also cross-compile for
`x86_64-pc-windows-gnu`, as befits a Windows-first design.

`rusty_agent_gateway` closes the fourth wave, and closes the
consolidation: nine crates behind one nested workspace, a Rust data plane
built to be a drop-in for agentgateway's own `config.yaml`. Its
`[workspace.package]` collided on five fields at once (`edition` 2024,
`rust-version` 1.88, `license` Apache-2.0, `repository`, `authors`), so its
crates carry literal `[package]` fields; only `clap` needed widening
(`env`, for its CLI) since everything else it asks for was already in this
root from the eight crate groups merged before it.

Like `rusty_key`, it brought its own `[workspace.lints]`, and a stricter
one than this root's `rustils`-derived table:
`unsafe_code = "forbid"` plus `missing_docs = "warn"`,
`clippy::todo = "warn"` and `clippy::unwrap_used = "warn"`. Inheriting the
root's would have quietly dropped three lints and downgraded a fourth, so
all nine crates carry the table literally.

It retires four pins — the most of any crate in this series, and all four
to siblings already here:

* `rusty_a2a` (`agentgateway-a2a`, rev `b9778e1`) — 11,324 insertions /
  360 deletions behind this workspace's copy.
* `rusty-mcp` (`agentgateway-auth`, `agentgateway-mcp`, `agentgateway`,
  tag `v0.4.1` → `0ca6a288`) — 1,367 / 210 behind. The only *tag*-pinned
  dependency retired in this series; every other has been a rev or a
  branch.
* `rusty_tls` (`agentgateway-tls`, rev `7ac6956e`) — 109 / 27 behind. Its
  manifest explained the pin was a bare SHA rather than a tag because
  `TlsAcceptor::accept_async` had not landed in v0.9.0 yet; a `path`
  dependency retires that reasoning along with the pin.
* `rusty_tokio` (`agentgateway-tls`, rev `6d3bb05a`) — 3,158 / 587 behind,
  and load-bearing: `rusty_tls`'s async adapter is generic over
  `rusty_tokio`'s `AsyncRead`/`AsyncWrite`, so the two had to move
  together, exactly the constraint `rusty_request`'s three-pin retirement
  documented above. A `path` dependency makes it structural rather than
  bookkeeping.

Two of those swaps needed a root-manifest detail worth recording, because
it is a Cargo rule rather than a judgment call: a member inheriting a
workspace dependency may not set `default-features = false` when the root
entry does not. `agentgateway-a2a` and the three `rusty-mcp` consumers all
do, deliberately (the gateway *fronts* agents rather than becoming one; the
comment in `agentgateway-a2a` explains that turning `rusty_a2a`'s agent
harness off is what keeps its `axum` 0.7 / `tonic` / `reqwest` 0.12 out of
this graph). So this root's `rusty_a2a` and `rusty-mcp` entries carry
`default-features = false` themselves — a no-op for their other consumers,
since both crates' `default` feature is empty, checked before making the
change rather than assumed from the swap pattern.

Its 73 tests pass and it needed no lint or format fixes. Note that
`agentgateway`'s own manifest flags one thing no test can protect and this
merge does not change: `hyper`'s `http2` feature is load-bearing for the
shipped binary (the TLS listener advertises `h2` over ALPN), while Cargo's
feature unification means the *test* binary would have it regardless — so
that flag must be verified against a built binary with `curl`, not by
`cargo test`, exactly as it was before the merge.

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
