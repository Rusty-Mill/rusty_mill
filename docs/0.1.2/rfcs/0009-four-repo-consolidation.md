# RFC 0009 — Consolidating `Rustsidian`, `nexus_forge`, `rusty_nexus` into `nexus`

- **Status:** Draft — assessment
- **Owner:** unassigned
- **Created:** 2026-09-08
- **Tracks:** the four-repo consolidation question; the assessment series ([RFC 0002](0002-bundled-shell-rush.md)–[RFC 0004](0004-lsp-framework-rusty-lsp.md))
- **Touches (if accepted):** `shell/src/plugins/nexus/fileProperties/`, `crates/nexus-storage/` (note-composer + unique-note handlers), `crates/nexus-editor/` (buffer WAL, assess only), `docs/archive/` (research imports), `README.md` (lineage note), and the GitHub state of the three sibling repos (archive)
- **Related:** ADR 0011 (single desktop target), [`../crates.md`](../crates.md), [`../architecture-adherence.md`](../architecture-adherence.md)

---

## Summary

**Consolidate onto `nexus`. Do not merge the other three repositories into it;
harvest a short list of ports, then archive them.**

The four repos are not four competing products. They are one idea attempted
four times, in sequence, by one author:

| Repo | Active | Commits | What it is |
|---|---|---|---|
| `baileyrd/Rustsidian` | 2026-04-04 → 04-07 | 92 | Attempt 1. Obsidian clone; SolidJS + Tauri, 5 crates. |
| `baileyrd/nexus_forge` | 2026-04-08 → 04-10 | 50 | Attempt 2. "Brain and Eyes" rewrite; React + Tauri, 28 crates. |
| `baileyrd/nexus` | 2026-04-12 → 2026-07-22 (pushes to 08-08) | 78 visible (history squashed at 07-02; PRs to #494) | Attempt 3, the one that kept going. Microkernel, 42 crates + shell. |
| `baileyrd/rusty_nexus` | 2026-07-25 → 07-26 | 7 | Attempt 4, a 4-hour spike re-deriving `nexus` on the `rusty_*` platform crates. |

`nexus` is the only one that is alive, CI-gated, releasable, and has a real
feature surface. It is roughly ten times the size of the other three combined
and carries twenty times their tests. Nothing in the other three is a
capability `nexus` lacks at the subsystem level; what remains are a handful of
small, verified feature gaps (typed properties editor, Zettelkasten unique
note, random note, note composer) plus some research docs. Those port in days,
not weeks, and go into existing crates and shell plugins, not new workspace
members.

## Background

### Side-by-side

Numbers are `wc -l` over `*.rs` / `*.ts,*.tsx` (excluding `target/`,
`node_modules/`, generated bindings) and `grep -c '#[test]|#[tokio::test]'`,
taken 2026-09-08 on the `claude/four-repo-consolidation-vb9c1l` branches.

| | Rustsidian | nexus_forge | **nexus** | rusty_nexus |
|---|---|---|---|---|
| Rust LOC | 18.2k | 16.8k | **284k** | 4.0k |
| TS/TSX LOC | 15.6k | 8.9k | **160k** | 0 |
| Rust test fns | 283 | 211 | **4,865** | 24 |
| Workspace crates | 5 | 28 (13 are plugin crates) | **42** + `shell/src-tauri` | 7 |
| Frontend | SolidJS 1.9 + CM6 | React 18 + Zustand + CM6 | React + CM6, 63 shell plugins + 7 core | none (println TUI) |
| Desktop | Tauri 2 + tauri-specta (vendored specta patch) | Tauri 2 + ts-rs | Tauri 2 + ts-rs + schemars, IPC drift check in CI | none |
| Storage | rusqlite 0.39, WAL, file-wins sync, schema v1+v2 | rusqlite 0.31 + FTS5 | rusqlite 0.39 + r2d2, file-as-truth, `.trash`, reconcile | flat files, no DB |
| Full-text search | tantivy 0.26 BM25 | SQLite FTS5 | tantivy 0.26 + vector, RRF hybrid | `to_lowercase().contains()` |
| AI / RAG | Anthropic/OpenAI/Ollama, LanceDB vector store | **stub (19 LOC)** | anthropic/openai/ollama/llama.cpp, fastembed, RAG, enrichment, agent loop, session tree | hardcoded reply string |
| MCP server | rmcp, 17 tools | none | ~65 tools + resources + prompts | hand-rolled JSON-RPC, 15 tools (3 return sample strings) |
| Terminal | portable-pty over Tauri channel | **stub (10 LOC)** | `nexus-terminal` (~15k) + `nexus-vt` grid + `nexus-rush` sandbox shell | none |
| Plugins | wasmtime WASI host w/ permission manifest (281 LOC) | `CorePlugin` trait, **no WASM** despite `extism` dep | wasmtime 44 sandbox + capability system + JS iframe sandbox | 3 hardcoded manifests |
| Editor model | CM6 owns the doc | ropey rope + undo + CRC-framed buffer WAL | `nexus-editor` block tree + `nexus-crdt` (RGA) + collab relay | none |
| MDX / JSX | two-pass parser (365 LOC), JSX placeholders in CLI/TUI | none | `nexus-storage/src/mdx.rs` (481 LOC) | none |
| Canvas / Bases | JSON Canvas | none | canvas (92 files), bases (ADR 0019) | toy parsers (203 + 223 LOC) |
| Git / LSP / DAP / ACP / collab / memory / workflow | none | none | present | inspection-only git, LWW register |
| CI | fmt + clippy + test (no Tauri libs, no frontend job) | 3-OS fmt + clippy + test | fmt, clippy, ~4.2k tests, cargo-deny, IPC drift, pnpm lint/typecheck/test, release workflows | none |
| Builds here | `cargo metadata` OK | `cargo metadata` OK | yes | **no** — depends on `../rusty_*` paths that do not exist |
| GitHub | private, 0 issues | private, 0 issues | public, 42 open issues | public, 0 issues |
| Planning state | STATE.md says Svelte, phase 13 "next" though built | README says Phase 0; ROADMAP unticked while STATE says done | docs say 38 crates; workspace has 42 | none |

### What each one actually is

**Rustsidian** (`/home/user/Rustsidian`). The most *Obsidian-shaped* of the
four. Its distinctive assets are frontend-side: an 11-registry plugin
architecture with 24 core plugins in `frontend/src/plugins/core/`, an MDX
CodeMirror language with wikilink completion, KaTeX/Mermaid widgets, vim mode,
pop-out windows, and a good set of Obsidian reverse-engineering docs under
`docs/`. Backend is a competent but small `Vault` on rusqlite + tantivy, a
17-tool `rmcp` MCP server, a wasmtime plugin host, and a LanceDB RAG crate.
Every backend capability has a larger, tested equivalent in `nexus`. The
frontend cannot be ported: it is SolidJS, `nexus` is React (ADR 0011 already
settled a single shell). It stopped on 2026-04-07, one day before `nexus_forge`
began.

**nexus_forge** (`/home/user/nexus_forge`). A cleaner architecture than
Rustsidian ("Brain owns state, Eyes render") with genuinely careful pieces:
`forge-buffer` (ropey + undo + a CRC32/length-framed write-ahead log with
`base_hash` chaining, `docs/AUTOSAVE.md`), tree-sitter incremental reparse
with Rust-side highlight spans, a proptest-covered typed properties editor, and
a documented ambiguous/unresolved wikilink taxonomy. But the roadmap's hard
parts never happened: `forge-ai` is 19 lines, `forge-pty` is 10, the Extism
WASM host advertised in `CLAUDE.md` does not exist, and `sqlite-vec` /
embeddings claimed in the README are absent. It stopped on 2026-04-10; `nexus`
was created on GitHub on 2026-04-12 and inherited its vocabulary (`forge` for
vault, `CorePlugin`, capabilities, event bus) without ever citing it.

**nexus** (`/home/user/nexus`). The production line. Microkernel with
enforced dependency invariants (`crates/nexus-bootstrap/tests/dep_invariants.rs`),
capability-gated IPC with a completeness test, a 24-plugin service layer,
three frontends plus MCP, and an established method for absorbing sibling
repos (RFCs 0002/0003: vendor the separable core as an in-tree leaf crate, take
the design not the GUI, ship the smallest opt-in step). Its weaknesses are the
weaknesses of success: 42 crates at "0.1.0" with 42 open issues from a
2026-07-02 capability assessment, doc drift on crate counts, and inconsistent
version numbers between `Cargo.toml`, `docs/0.1.2/`, and `CHANGELOG.md`.

**rusty_nexus** (`/home/user/rusty_nexus`). Not a codebase. Seven commits in
one overnight session "leveraging sovereign rusty_mill and rusty_remind_me
dependencies". It declares 20 `../rusty_*` path dependencies of which exactly
one (`rusty_json`) is imported anywhere; the rest are dead manifest entries.
The "REST HTTP server" is a `match` on three path strings with no socket, the
"inverted index" is never instantiated, the template engine bakes in the string
`"2026-07-26"` as today's date, and the AI engine returns
`format!("[{}] AI response for: {}", ...)`. It does not build without the
sibling checkouts. Its one useful artifact is the 21-subcommand CLI taxonomy in
`crates/rusty_nexus_cli/src/main.rs` as a checklist against `nexus-cli`.

## Options

### A. `nexus` is the base; harvest; archive the rest — **recommended**

Keep `nexus` as the sole repository. Port the verified gaps listed below as
ordinary PRs into existing crates and shell plugins. Archive the three sibling
repos on GitHub (read-only, history preserved) with a one-paragraph README
pointer to this RFC.

- **Why:** every subsystem the siblings have, `nexus` has bigger and tested;
  the siblings' frontends are on a different framework or nonexistent; the
  Cargo pins conflict (rusqlite 0.31 vs 0.39, tree-sitter 0.23 vs 0.26, notify
  6 vs 8, thiserror 1 vs 2); and `bootstrap_coverage.rs` would force an
  `EXEMPT_CRATES` entry for every dead crate that entered the workspace.
- **Cost:** a few small PRs. No toolchain or dependency bumps.

### B. Monorepo: `git subtree` all four into one workspace — rejected

Preserves history in one place, but imports three abandoned trees that will
never compile again alongside a workspace that runs `cargo clippy -D warnings`
and `cargo-deny` on every push. Nothing is gained that archived repos plus
`git log` do not already give. It also breaks invariant 2 (microkernel
isolation) the moment any of it is a workspace member, and it doubles the
`docs/archive/` weight (already 9.5 MB / 462 files).

### C. Restart on `rusty_nexus` / the sovereign `rusty_*` platform — rejected

The sovereignty goal (fewer external crates, more `baileyrd/*` crates) is
legitimate, but `rusty_nexus` is a 4k-LOC sketch that would discard 284k LOC
and 4.8k tests to pursue it. `nexus` already pursues sovereignty the right way,
one RFC at a time (`rush`, `rusty_term`, `remind_me` are in; `rusty_lsp` was
assessed and declined). Continue that per-crate, not by rewrite.

## What to port (ranked)

Each row was checked by grep against `nexus` on 2026-09-08. "Gap" means zero
hits for the feature in `crates/`, `shell/src/`, and `packages/`.

| # | Piece | Source | `nexus` today | Verdict | Effort |
|---|---|---|---|---|---|
| 1 | Unique-note creator (Zettelkasten ID formats) | `nexus_forge/crates/forge-plugin-unique-note` (327 LOC) | **gap** (`zettel`, `unique note`: 0 hits) | Port as a storage IPC handler + shell plugin | S |
| 2 | Random note | `nexus_forge/crates/forge-plugin-random-note` (68 LOC) | **gap** | Port as a command in `quickSwitcher` or `commandPalette` | XS |
| 3 | Note composer: merge / split / extract-selection-to-note with backlink | `nexus_forge/crates/forge-plugin-note-composer` (198 LOC) + `AppState::delete_note` | **gap** (only a settings stub label in `SettingsStubPages.tsx:67`) | Port; extract-to-note should go through `nexus-hashline` so agent and human edits share one path | S–M |
| 4 | Typed properties panel + bulk properties view (schema-typed frontmatter, virtualized table, proptest round-trip) | `nexus_forge/crates/forge-plugin-properties-{panel,view}` (1,122 LOC) + `src/features/` | `shell/src/plugins/nexus/fileProperties/index.tsx` (222 LOC, single file, untyped) | Port the Rust typed schema + tests into `nexus-storage`; rebuild the view as a shell plugin | M |
| 5 | Buffer write-ahead log with `base_hash` chaining for crash recovery | `nexus_forge/crates/forge-buffer/src/wal.rs`, `docs/AUTOSAVE.md` | `nexus-editor` has no rope and no buffer WAL; `nexus-crdt` snapshots may already cover the crash case | **Assess first.** Port the design only if an unsaved-edits-after-crash test fails today | M |
| 6 | Obsidian capability / UI reverse-engineering research | `Rustsidian/docs/Obsidian_Capabilities_Analysis.md`, `Obsidian_UI_Analysis.md`, `docs/obsidian_reverse_engineering/` | nothing equivalent in `docs/` | Copy into `docs/archive/research/` — zero code. **Check licensing/attribution before moving from a private to a public repo.** | XS |
| 7 | CLI subcommand taxonomy | `rusty_nexus/crates/rusty_nexus_cli/src/main.rs` (21 subcommands) | `nexus-cli` covers most; #430 tracks `--format` gaps | Use as a checklist while working #430; port nothing | XS |
| 8 | Brain-side syntax highlighting (`HighlightSpan` from Rust → CM6 decorations) | `nexus_forge/crates/forge-parser/src/highlights.rs` | `nexus` highlights in TS (`editor/cm/syntaxHighlight.ts`) | Do not port. Record as a design option; it adds an IPC round-trip per keystroke | — |
| 9 | wasmtime WASI plugin host with permission manifest | `Rustsidian/crates/rustsidian-core/src/plugin/` (281 LOC) | `nexus-plugins` wasmtime 44 + capability system | Do not port. Diff the manifest permission vocabulary against `plugin-capabilities.md` for anything missing | XS |
| 10 | LanceDB RAG crate | `Rustsidian/crates/rustsidian-rag` | fastembed + vector store + RRF | Do not port. New heavy dependency for a solved problem | — |
| 11 | MDX two-pass parser, JSX placeholder rendering in CLI/TUI | `Rustsidian/crates/rustsidian-core/src/parser/mdx/`, `rustsidian-cli/src/rendering/` | `nexus-storage/src/mdx.rs` (481 LOC) | Do not port the parser. Check whether `nexus-tui` renders JSX placeholders; if not, that is a 100-line follow-up | XS |
| 12 | SolidJS frontend, 24 core plugins, 11 registries | `Rustsidian/frontend/` | React shell, 63 plugins | Do not port. Different framework; ADR 0011 | — |
| 13 | Everything else in `rusty_nexus` | — | — | Nothing. Archive | — |

Rows 1–4 are the whole code deliverable: roughly 1.7k source lines of
`nexus_forge`, all landing in existing crates and `shell/src/plugins/nexus/`.
No new workspace member is needed, which matters because `nexus`'s main
structural risk is crate count, not missing features.

## Mechanics

Follow the pattern already used for `rush` and `rusty_term`:

1. Accept this RFC (update the row in [`README.md`](README.md)).
2. Tag each sibling repo (`final-2026-09`), add a README pointer to this RFC,
   and archive it on GitHub. Do not delete. `Rustsidian` and `nexus_forge` are
   private; leave them private.
3. One PR per row 1–4, in that order (smallest first). Each PR:
   - adds IPC handlers to the owning service crate with typed
     `#[serde(deny_unknown_fields)]` args,
   - classifies them in `crates/nexus-bootstrap/cap_matrix.toml`,
   - regenerates bindings with `scripts/check_ipc_drift.sh`,
   - adds the UI under `shell/src/plugins/nexus/<feature>/`,
   - brings the source's tests across (the `nexus_forge` plugin crates have
     proptest and unit coverage worth keeping).
4. Row 5 starts with a failing test, not a port.
5. Row 6 lands as a docs-only PR after the attribution check.
6. Add one sentence to `README.md` acknowledging lineage:
   `nexus` succeeds `Rustsidian` (2026-04) and `nexus_forge` (2026-04);
   the `forge` vocabulary comes from the latter.

## Risks and open questions

- **Scope pressure on `nexus`.** It already has 42 crates and 42 open
  capability-assessment issues. The ports above must not add crates; if a
  port wants its own crate, that is a sign it should wait.
- **Private → public.** `Rustsidian`'s Obsidian reverse-engineering docs and
  screenshots were written in a private repo. Confirm they are the author's
  own analysis and carry no third-party material before copying into public
  `nexus`.
- **Sovereignty direction.** Rejecting option C is not rejecting the goal.
  The `rusty_nexus` manifest is a useful list of which `rusty_*` crates the
  author wants `nexus` to eventually prefer (`rusty_db`, `rusty_search`,
  `rusty_http`, `rusty_provider`, …). Each is its own future RFC in the
  0002–0004 series, judged on the crate, not on the spike.
- **Doc drift in `nexus` itself.** README and `architecture.md` say 38
  crates; the workspace has 42. Not caused by this RFC, but the lineage note
  is a natural moment to fix the count.
