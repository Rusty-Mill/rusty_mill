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
**outside** this set — `rusty_simd`, `rusty_std`, `rusty_wire`,
`rusty_lsp` — remain pinned `git` dependencies with an explicit `rev`,
unchanged by this merge; those crates aren't part of this monorepo (except
`rusty_libc`, which the second wave below brings in, at which point its
three in-set consumers switched to a path dependency on it too).
`mill-term` locates `rusty_git`/`rusty_text`'s `rgit`/`rsed`/
`rawk` binaries via a `PATH`/shared-`target/`-dir lookup rather than a
Cargo library dependency — it shells out to them, not their APIs — so that
relationship stays a build-artifact lookup even though all three now live
in this same workspace.

`rusty_term`'s `gui`/`gui-gpu` backends onto `rusty_gui`/`rusty_gpu` are
currently disabled/unused pending a fix tracked upstream at
[`rusty_gui#9`](https://github.com/baileyrd/rusty_gui/issues/9) — not
addressed by this migration.

`rusty_tokio` doesn't depend on, or get depended on by, any of the other
crates in this repo, so it needed no path-dependency swap. Its own
`rusty_std` and `platform`/`platform-linux`/`platform-bsd`/`platform-windows`
dependencies (from `baileyrd/rusty_std` and `baileyrd/rustils`) stay pinned
`git` dependencies — those crates are outside this monorepo's scope, same as
`rusty_simd`/`rusty_wire` above.

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
is outside this monorepo's scope (same as `rusty_simd`/`rusty_std`/
`rusty_wire` noted earlier) and stays a pinned `git` dependency.

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
