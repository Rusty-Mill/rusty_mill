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
`rusty_libc`, `rusty_lsp` — remain pinned `git` dependencies with an
explicit `rev`, unchanged by this merge; those crates aren't part of this
monorepo. `mill-term` locates `rusty_git`/`rusty_text`'s `rgit`/`rsed`/
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
