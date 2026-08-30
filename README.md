# rusty_mill

The Rusty Mill monorepo: a Cargo workspace consolidating previously
standalone `baileyrd/*` crates into one repository, one build, and one CI
pipeline. Each crate keeps its full original commit history, merged in via
`git subtree` under `crates/`.

A first wave merged fourteen crates (below the `rusty_term` row through
`rusty_text`). A second wave, in progress, is merging fifteen more —
`rusty_tokio`, `rusty_rusqlite`, `rusty_libc`, `rusty_acp`, `rusty_tls`,
`rusty_serde`, `rusty_lsp`, `rusty_a2a`, `rusty_mcp`, `rusty_stream`,
`rusty_url`, `rusty_http`, `rusty_json`, `rusty_oauth`, and `rustils_async`
— one crate per pull request; this row set reflects whichever of those
have landed so far.

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
| [`rusty_h2`](crates/rusty_h2) | `crates/rusty_h2` | A from-scratch HTTP/2 (RFC 9113) implementation, including HPACK header compression |

Each crate's own README, docs, and issue history describe its design in
depth — the links above point at the original standalone repos' content,
now living under `crates/<name>/`.

## Building

```sh
cargo build --workspace
cargo test --workspace
```

Some `rusty_term` features (`gui`, `gui-gpu`) and `rusty_gui`'s Linux
backend link against X11/Wayland directly; see
[`.github/workflows/ci.yml`](.github/workflows/ci.yml) for the system
packages a Linux build needs. `rusty_win32` and parts of `rusty_gui`/
`rusty_gpu` are Windows-only (`cfg(windows)`-gated) and are exercised by
the workflow's `windows-latest` matrix leg.

`mill-term`'s own test suite has one known, pre-existing, environment-
dependent failure in this monorepo:
`augmented_path_prepends_tool_directories_and_keeps_existing_path` hardcodes
a Windows-style path and only ever passed on a Windows runner — unrelated
to this merge.

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

`rusty_h2` has zero dependencies of any kind, so nothing needed
swapping — its own merge is just the subtree add plus workspace wiring.

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
[`rusty_vulkan`](https://github.com/baileyrd/rusty_vulkan), and
[`rusty_h2`](https://github.com/baileyrd/rusty_h2) — merged one
at a time, same process.
