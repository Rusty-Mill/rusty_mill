# Release Notes

Tracks the **monorepo itself** — crate merges, workspace-wide CI, and
cross-crate changes (like the duplication sweeps below) — not each crate's
own internal changes, which are logged in that crate's own
`crates/<name>/RELEASE_NOTES.md` where one exists (many crates kept theirs
from before the merge; see ADR-0001 for why root and per-crate logs are
separate rather than one superseding the other).

One entry per merged PR against `main`, reverse chronological, each linking
to its PR. Bolded inline category tags (`**Added:**` / `**Changed:**` /
`**Fixed:**`), known limitations stated plainly.

---

## Fourth-wave merge — `rusty_adk`
**2026-09-01** · branch [`claude/rusty-repos-migration-iwvuld`](https://github.com/Rusty-Mill/rusty_mill/tree/claude/rusty-repos-migration-iwvuld)

- **Added:** `crates/rusty_adk` — a Rust port of the Agent Development Kit
  (ADK) 2.0 architecture: the same data model, graph execution engine, and
  tool/callback contracts, plus MCP and A2A bridges. Eleven library crates
  and three runnable examples behind one nested workspace, merged via
  `git subtree` with full history.
- **Changed:** `adk-a2a`'s `rusty_a2a` dependency retired from a
  *branch-tracking* git dependency (no `rev`, unlike every other pin in
  this series) to a `path` dependency on the merged sibling. What it had
  actually resolved to was 42 commits behind the commit this workspace
  imported — 9,729 insertions across 66 files — plus three commits since,
  one of which changed `require_auth`'s error type. Verified by running
  `adk-a2a`'s own suite against the swap (13 unit, 8 end-to-end, 10
  remote-transport tests), not by reading the diff.
- **Changed:** `[workspace.package]` collided on `rust-version`, `license`
  and `repository`, so its crates carry literal `[package]` fields. Root
  `tokio` gained `io-std`, `uuid` gained `serde`, and `serde_json` gained
  `float_roundtrip` (which `rusty_adk`'s SQLite session store needs for
  exact f64 round-trips, and which Cargo unifies globally anyway, so it is
  declared where it is visible). `thiserror` and `schemars` stay literal on
  the `adk-*` crates — `"2"` and `"0.8"` against this root's `"1"` and
  `rusty_key`'s `"1.0.4"`.
- **Fixed:** `adk-sessions`' optional `rusqlite = "0.37"` moved to this
  root's `"0.32.1"` — the same `libsqlite3-sys` `links` conflict
  `inventory-core` hit, since Cargo's uniqueness check counts optional
  dependencies it never activates.
- 278 tests pass, 2 ignored. No lint or format fixes were needed.

## Fourth-wave merge — `rusty_tailscale`
**2026-09-01** · branch [`claude/rusty-repos-migration-iwvuld`](https://github.com/Rusty-Mill/rusty_mill/tree/claude/rusty-repos-migration-iwvuld)

- **Added:** `crates/rusty_tailscale` — a sovereign pure-Rust Tailscale
  client: ts2021 control plane, WireGuard data plane, DERP/STUN/disco NAT
  traversal, a userspace smoltcp stack, a daemon and a CLI. Fifteen `ts-*`
  crates plus `xtask` behind one nested workspace (whose `members` was a
  `crates/*` glob, expanded to literal entries here), merged via
  `git subtree` with full history.
- **Changed:** `[workspace.package]` collided on `version`, `edition`
  (2024 — the first edition-2024 crates here) and `repository`, so its
  crates carry literal `[package]` fields; only its dependencies were
  hoisted, with `rusty_http`/`rusty_crypto_key` re-pathed to this root's
  `crates/` layout.
- **Changed:** `ts-magicsock` and `ts-tun`'s pinned `rustils` git
  dependencies (`platform`, `platform-linux`, rev `b8bf992f`) retired to
  this root's path entries, same as `rusty_rdp`'s.
- **Fixed:** two pre-existing breaks in `rusty_tailscale`'s own `main` —
  it has no CI of its own and does not compile on Linux. `ts-magicsock`
  called three `platform::net::UdpSocket` trait methods without the trait
  in scope (true at the pinned rev too, so not drift the pin was hiding),
  and `ts-cli`'s `localapi::Error` declared a `Status(StatusCode)` variant
  nothing constructs while `request()` constructed a nonexistent
  `Api { status, body }`. Both confirmed against the standalone repo first.
- **Fixed:** four more `generic-array` 0.14.9 deprecations (`ts-control`'s
  Noise handshake and frame codec, `ts-disco`, `ts-derp`), plus a
  `cargo fmt --all` pass.
- 93 tests pass. All sixteen crates also cross-compile for
  `x86_64-pc-windows-gnu` despite the Linux-first design, so — unlike
  `rusty_stream` — no `windows-exclude` was needed.

## Fourth-wave merge — `rusty_llama`
**2026-09-01** · branch [`claude/rusty-repos-migration-iwvuld`](https://github.com/Rusty-Mill/rusty_mill/tree/claude/rusty-repos-migration-iwvuld)

- **Added:** `crates/rusty_llama` — a from-scratch Llama/GGUF inference
  engine: CPU SIMD kernels, optional `wgpu` and CUDA backends, an
  OpenAI-compatible server, and GGUF-embedded Jinja chat templating. Merged
  via `git subtree` with full history.
- No dependency swaps: its `rusty_simd`/`rusty_std` path dependencies
  already pointed at siblings under `crates/`.
- **Fixed:** two `unnecessary_cast` lints in `backend/cuda.rs`'s test
  fixtures. They only appear with the `cuda` feature on, which this
  workspace's `--all-features` clippy gate does and the crate's own CI
  never did.
- **Fixed:** `render_jinja_threads_context_variables` asserted a bool
  interpolates as `true`. `minijinja` 2.22 deliberately changed
  none/bool rendering to `None`/`True`/`False` for Jinja2 compatibility;
  the standalone lockfile pinned 2.21, this workspace resolves 2.24. Since
  the code path exists to render templates authored for Python Jinja2, the
  new rendering is the correct one — assertion updated, reason recorded
  inline. Nothing else in the crate interpolates a bare boolean.
- **Changed:** reformatted with `cargo fmt --all` (not fmt-clean under this
  workspace's settings).
- 248 tests pass; 49 stay ignored because they need real model weights on
  disk, by the crate's own design.

## Fourth-wave merge — `rusty_key`
**2026-09-01** · branch [`claude/rusty-repos-migration-iwvuld`](https://github.com/Rusty-Mill/rusty_mill/tree/claude/rusty-repos-migration-iwvuld)

- **Added:** `crates/rusty_key` — Rusty Keys, an AI-native application
  skeleton where the model's agent loop is the kernel and the application
  is the harness around it (constrain / feed / observe / compose). Eight
  crates behind one nested workspace, merged via `git subtree` with full
  history.
- **Changed:** its `[workspace.package]` collided on `rust-version` and
  `license`, so its crates carry literal `[package]` fields and only its
  dependencies were hoisted (`aisdk`, `schemars`, `toml`, and the `rk-*`
  path entries).
- **Fixed:** `rusty_key`'s `[workspace.lints]` (`unsafe_code = "forbid"`)
  is strictly stronger than this root's, which is `rustils`'
  (`unsafe_code = "warn"`). Leaving `[lints] workspace = true` in place
  would have silently downgraded all eight crates, so each carries a
  literal `[lints.rust] unsafe_code = "forbid"`; `rustils`' crates keep
  inheriting the root table unchanged.
- **Changed:** `crates/rusty_key` reformatted with `cargo fmt --all` — it
  was not fmt-clean under this workspace's settings, same as
  `rusty_ansder`/`rusty_boot` when they merged. No behavior change.
- **Known limitation:** its Tauri desktop shell
  (`crates/rusty_key/desktop/src-tauri`) stays a standalone workspace and
  is excluded here, exactly as its own repo had it — the opposite call from
  `inventory-tauri`, and deliberately so, since each is upstream's own.
  Verified it still builds across the boundary post-merge.
- **Known limitation:** two more duplicate-major pairs now resolve —
  `rmcp` 0.9.1 alongside 3.1.4, and `axum` 0.7.9 alongside 0.8.9. Cargo
  keeps them as unrelated crates and no type crosses between the groups.
- No dependency swaps. Full suite (194 tests) passes unmodified.

## Fourth-wave merge — `rusty_skillopt`
**2026-09-01** · branch [`claude/rusty-repos-migration-iwvuld`](https://github.com/Rusty-Mill/rusty_mill/tree/claude/rusty-repos-migration-iwvuld)

- **Added:** `crates/rusty_skillopt` — a from-scratch Rust take on
  Microsoft's SkillOpt: optimize a skill markdown document as the trainable
  state of a frozen LLM agent, with epochs, batches and a validation gate,
  entirely in text space. Four crates behind one nested workspace, merged
  via `git subtree` with full history.
- **Changed:** its `[workspace.package]` collided on `license`, so its four
  crates carry literal `[package]` fields (the `rusty_db` treatment) and
  only its dependencies were hoisted. Root `tokio` widened to the union of
  what `rusty_search`, `rusty_db` and `rusty_skillopt` need; root `chrono`
  gained `serde`. `thiserror` stays literal on `skillopt-core`/
  `skillopt-model`, same `"2"`-vs-`"1"` reason as before.
- **Known limitation:** the workspace now resolves two `reqwest` majors —
  0.13.4 for `rusty_acp`/`rusty_mcp` and 0.12.28 for `skillopt-model`.
  Cargo treats them as unrelated crates so they coexist cleanly and no type
  crosses between the two groups; bumping `skillopt-model` would be an API
  change outside this merge's scope. A build-size cost, not a correctness
  one.
- No dependency swaps: nothing in `rusty_skillopt` depended on a sibling in
  this workspace. Full suite (68 tests, 2 environment-gated ignores) passes
  unmodified.

## Fourth-wave merge — `rusty_inventrory`
**2026-09-01** · branch [`claude/rusty-repos-migration-iwvuld`](https://github.com/Rusty-Mill/rusty_mill/tree/claude/rusty-repos-migration-iwvuld)

- **Added:** `crates/rusty_inventrory` — a local-first encrypted index over
  the conversation history Claude Code, Codex, Cursor, Zed, Kiro and
  Antigravity write to disk, plus its `inv` CLI and a Tauri menu-bar shell.
  Three crates behind one nested workspace, merged via `git subtree` with
  full history.
- **Changed:** its `[workspace.package]` collided with this root's on
  `rust-version`, `license` and `repository`, so its three crates carry
  literal `[package]` fields (the `rusty_db` treatment); only its
  dependencies were hoisted. `thiserror` stays literal on `inventory-core`
  for the same `"2"`-vs-`"1"` reason as `rusty_test`'s `contract`.
- **Changed:** CI's Linux leg installs `libwebkit2gtk-4.1-dev`,
  `libgtk-3-dev`, `libayatana-appindicator3-dev`, `librsvg2-dev` (the Tauri
  shell) and `libdbus-1-dev` (`inventory-core`'s Secret Service keyring),
  matching what `rusty_inventrory`'s own CI installed. Windows and macOS
  use OS-native APIs for both.
- **Fixed:** `inventory-core`'s `rusqlite = "0.37"` needs
  `libsqlite3-sys ^0.35`, which cannot coexist with `sqlx-sqlite`'s
  `^0.30.1` — `libsqlite3-sys` sets `links = "sqlite3"`, so exactly one
  version may exist per graph. Moving *up* would mean `sqlx 0.9` across
  `rusty_db` and `rusty-search-sqlite-fts5`, so `inventory-core` came down
  to `rusqlite = "0.32.1"`, unifying with `rusty_sqlite`. Verified by
  running its suite, not by reading changelogs: 79 tests pass unmodified.
- **Fixed:** three deprecated `GenericArray::from_slice` calls in `db.rs`'s
  sealed-index code — same `generic-array` 0.14.9 cause as `rusty_croc`'s,
  same behavior-preserving rewrite.
- No dependency swaps: nothing in `rusty_inventrory` depended on a sibling
  in this workspace.

## Fourth-wave merge — `rusty_test`
**2026-09-01** · branch [`claude/rusty-repos-migration-iwvuld`](https://github.com/Rusty-Mill/rusty_mill/tree/claude/rusty-repos-migration-iwvuld)

- **Added:** `crates/rusty_test` — the `portable-runtime-contract` spike:
  one execution contract (`contract`), a per-host adapter (`compat`), a
  verification layer (`conformance`), and three reference tools
  (`stat-tool`, `proc-runner`, `pty-shell`). Merged via `git subtree` with
  full history; its nested `[workspace]` table removed and its six crates
  added to this root's `members`.
- **Changed:** its `[workspace.package]` didn't collide with this root's
  (same edition, same license), so its crates keep inheriting via
  `field.workspace = true` — the `rusty_search` treatment, not
  `rusty_db`'s. Only `publish = false` was new here. `thiserror` was
  deliberately left un-hoisted: `rusty_test` wanted `"2.0"`, this root
  pins `"1"` for `rusty_db`/`rustils`, so `contract` keeps a literal
  `thiserror = "2"` instead of forcing a major bump on unrelated crates.
- **Fixed:** `conformance`'s `tests/layering.rs` reads the workspace
  manifest to enforce the layer model, resolving it two directories above
  its own crate and requiring a declared layer for every member found.
  Post-merge that is this monorepo's root — four levels up, ~100 members —
  so three of its four tests panicked. Repointed and filtered through a
  `GROUP_PREFIX` constant; the check's logic is otherwise untouched and all
  four tests pass, alongside the group's other 27.
- No dependency swaps: nothing in `rusty_test` depended on a sibling in
  this workspace.

## Fourth-wave merge — `rusty_croc`
**2026-09-01** · branch [`claude/rusty-repos-migration-iwvuld`](https://github.com/Rusty-Mill/rusty_mill/tree/claude/rusty-repos-migration-iwvuld)

- **Added:** `crates/rusty_croc` — a Rust port of
  [croc](https://github.com/schollz/croc), wire-compatible with stock croc
  v10 (PAKE code phrases, relay, local-network hand-off, resume). Merged
  via `git subtree` with its full commit history, as with every prior
  crate import.
- **Fixed:** four `GenericArray::from_slice` calls in `crypt.rs` (AES-256-GCM
  and XChaCha20-Poly1305 nonces). The standalone repo's lockfile pinned
  `generic-array` 0.14.7; this workspace resolves 0.14.9, which deprecates
  the crate wholesale, so `-D warnings` turned them into errors. Rewritten
  to the `From<&[T]> for &GenericArray` conversion `from_slice` delegates
  to — no behavior change, 49 tests pass unmodified.
- No dependency swaps: `rusty_croc` depends only on crates.io crates, not
  on any sibling in this workspace. Its nightly-only `fuzz/` harness keeps
  its own `[workspace]` table and is excluded from this one, same as
  `rusty_tls/fuzz` and `rusty_lsp/fuzz`.

## PR #65 — Deduplicate `rusty_rdp`'s byte cursor and split `rusty_ansder`'s two crates
**2026-09-01** · [#65](https://github.com/Rusty-Mill/rusty_mill/pull/65)

- **Fixed:** `rusty_rdp`'s hand-rolled byte Reader/Writer duplicated
  `rusty_wire`'s (a dependency `rusty_rdp` already declared but never
  used) — now re-exports `rusty_wire`'s cursor types.
- **Changed:** `rusty_ansder` bundled two unrelated libraries (an ASN.1 DER
  codec and a sovereign RAG/Q&A engine). Split the RAG engine into a new
  `rusty_rag` crate; `rusty_ansder` now holds just the DER codec.
- Also investigated and deferred (different-scoped tools sharing a name,
  not true duplication): `rusty_term` vs. `rusty_ansi`; `rusty_ansder`'s DER
  codec vs. `rusty_tls`'s hand-rolled DER; `rusty_http::Url` vs.
  `rusty_url::Url`.

## PR #10 — Collapse workspace duplication: to_wide, read_lines, SHA-1, IFS splitting, glob, raw-mode
**2026-08-27** · [#10](https://github.com/Rusty-Mill/rusty_mill/pull/10)

- **Fixed:** six of eight findings from a five-sweep duplication review
  (issues #1–#8) — `rusty_win32`'s 7x-duplicated `to_wide()` hoisted;
  `rsed`/`rawk`'s shared stdin-reading extracted to `read_lines()`;
  `rusty_git`/`rusty_term`'s independent SHA-1 implementations merged into
  a new `rusty_sha1` crate; `rush`'s two independent IFS-splitting
  implementations merged into `ifs_run_end()`; `rush`'s backtracking glob
  matcher now tries `rusty_regx::Glob` first; a duplicated Windows
  raw-mode flag transformation (`rusty_term`/`rusty_lines`) hoisted into
  `rusty_win32::console::raw_mode_core()`.
- **Known limitation:** two findings closed `no action` — Unix termios
  save/restore (`rusty_term`/`rusty_lines`) is the same shape by
  deliberate, different policy; a `no_std` rounding workaround in
  `rusty_font` (`round_nonneg` vs. `round_f32`) likewise.
- Filed #9 for the remaining gap (`rusty_regx::Glob` needs embedded `!(p)`
  negation support before rush's fallback matcher can be fully deleted) —
  a capability gap, not duplication; still open as of this writing.

## Earlier crate-import history

Every `Import <crate> into crates/<crate>` merge and the CI-scoping work
(`Speed up CI: affected-crate filtering, rust-cache, nextest, parallel
clippy`) predate this file. See `git log --oneline --merges` for the full
list — not backfilled entry-by-entry here since each import is already its
own reviewable commit with a descriptive message, and there are dozens of
them (see the crate table in `README.md` for the two-wave merge history).
