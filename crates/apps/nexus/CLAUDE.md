# CLAUDE.md

This file provides guidance to Claude Code (claude.ai/code) when working with code in this repository.

## Common commands

Rust workspace (root `Cargo.toml`, excludes `shell/`):

```bash
cargo build --workspace                          # build all crates
cargo test  --workspace                          # run all tests
cargo clippy --workspace --all-targets           # lint
cargo test  -p <crate-name>                      # one crate's tests
cargo test  -p <crate-name> <test_name>          # one test by name substring
cargo build -p nexus-cli   # or -p nexus-tui     # build a single binary
```

pnpm workspace (`shell/` + `packages/*`, requires Node ≥18, pnpm ≥10):

```bash
pnpm install                                     # install workspace deps
pnpm --filter nexus-shell lint                   # eslint shell/src
pnpm --filter nexus-shell typecheck              # tsc --noEmit
pnpm --filter nexus-shell test                   # node --test tests/*.test.ts
pnpm --filter nexus-shell tauri:dev              # run desktop shell (needs webkit2gtk-4.1, libsoup-3.0)
```

IPC contract drift check — run before opening a PR that changes any IPC-typed struct:

```bash
scripts/check_ipc_drift.sh
```

It regenerates `packages/nexus-extension-api/src/generated/ipc/*.ts` (via ts-rs) and `crates/nexus-bootstrap/schemas/ipc/*.json` (via schemars) and fails if `git diff` is non-empty.

The remaining `scripts/` are repo-root-agnostic value-add helpers: `check_ipc_docs_drift.sh` (hand-maintained IPC docs vs the cap matrix), `check_token_usage.sh` (legacy CSS token audit), `download_fonts.sh`, `seed_fixtures.sh` / `seed_notes.sh` (test-forge seeding), and `migrate-shell-state.ts`. The old thin `cargo test`/`cargo check` wrapper scripts with hard-coded WSL paths were removed (V7, 2026-06-10) — run cargo directly; CI is the reproducible runner.

## Architecture

Nexus is a **microkernel** Rust workspace. Authoritative current docs are under `docs/0.1.2/` — start at [`docs/0.1.2/README.md`](docs/0.1.2/README.md), then [`docs/0.1.2/architecture.md`](docs/0.1.2/architecture.md), [`docs/0.1.2/crates.md`](docs/0.1.2/crates.md), [`docs/0.1.2/ipc-handlers.md`](docs/0.1.2/ipc-handlers.md), and [`docs/0.1.2/capabilities.md`](docs/0.1.2/capabilities.md). Settings live at [`docs/0.1.2/settings/`](docs/0.1.2/settings/) — every config knob is listed there, and hardcoded values flagged for promotion are at [`docs/0.1.2/settings/hardcoded-rust.md`](docs/0.1.2/settings/hardcoded-rust.md) / [`docs/0.1.2/settings/hardcoded-shell.md`](docs/0.1.2/settings/hardcoded-shell.md). The pre-0.1.2 doc set (PRDs, ADRs 0001–0029, prior audits) is preserved verbatim under `docs/archive/pre-0.1.2/` — useful historical reference, but when it disagrees with the code or with `docs/0.1.2/`, the code wins.

**Four invariants drive the shape of the system:**

1. **File-as-truth.** Markdown files on disk are authoritative. The SQLite index in `.forge/index.db` and the Tantivy FTS index are rebuildable from the files; never write code that treats them as the source of record.
2. **Microkernel isolation.** `nexus-kernel` depends only on `nexus-types` and `nexus-plugin-api` (both leaf crates). Subsystem crates depend on the kernel; the kernel never depends on a subsystem. This is enforced by `crates/nexus-bootstrap/tests/dep_invariants.rs` — if you hit it, the architecture is telling you to route through IPC instead.
3. **IPC over direct calls.** CLI, TUI, MCP server, and the Tauri shell all reach storage / AI / editor / etc. through one path:
   ```
   context.ipc_call(plugin_id, command, args) -> Result<serde_json::Value>
   ```
   Community WASM plugins use the same call. New capability ⇒ new IPC handler in the right service crate, not a new direct dependency from a frontend.
4. **Capabilities gate everything.** `fs.read`, `kv.write`, `ipc.call`, `events.publish`, etc. Every kernel-mediated operation checks a capability before it runs. See ADR 0002.

**Where things live:**

- `crates/nexus-kernel/` — event bus, IPC dispatcher, capability system, plugin lifecycle. Keep small.
- `crates/nexus-storage/` — file-as-truth, SQLite, Tantivy, file watcher, knowledge graph. Owns the forge.
- `crates/nexus-<service>/` — service plugins (`ai`, `ai-runtime`, `agent`, `comments`, `editor`, `git`, `linkpreview`, `mcp`, `memory`, `lsp`, `dap`, `acp`, `skills`, `templates`, `terminal`, `theme`, `workflow`, `database`, `kv`, `security`, `formats`, `notifications`, `audio`, `collab`, `crdt`, `panic-log`, `remote`, `fuzz`). Each `CorePlugin` is registered by `nexus-bootstrap` in deterministic order. The full Cargo workspace has 41 members — including two not-yet-wired staging libraries (`nexus-context`, `nexus-protocol`, tracked by #188), the standalone `nexus-memory-hub` sync server (a deployable HTTP binary, not a bootstrap plugin), a leaf crate adopted from the sibling `baileyrd/*` repos: `nexus-vt` (vendored `rusty_term` core — the headless VT grid behind `nexus-terminal`, RFC 0003), and `nexus-hashline` (the RFC 0005 content-hash-anchored patch format consumed by `nexus-storage`). `nexus-memory` is now wired as the `com.nexus.memory` service plugin (full remind_me parity — see [`docs/0.1.2/memory.md`](docs/0.1.2/memory.md)). See `Cargo.toml` for the authoritative list and [`docs/0.1.2/crates.md`](docs/0.1.2/crates.md) for a one-row-per-crate inventory.
- `crates/nexus-bootstrap/` — the orchestrator. Frontends call `build_cli_runtime(forge_root)` / `build_tui_runtime(forge_root)` to get a `Runtime` (kernel + registered plugins + invoker context).
- `crates/nexus-cli/` (`nexus`), `crates/nexus-tui/` (`nexus-tui`), `crates/nexus-mcp/` — frontends. They consume `nexus-bootstrap` and route through `context.ipc_call(...)`.
- `shell/` + `shell/src-tauri/` (crate `nexus-shell`) — the **single** active desktop target per ADR 0011. The legacy tri-pane `app/` + `crates/nexus-app` was retired in v0.4.0 (recoverable via `v0.1.0-legacy-shell` git tag).
- `shell/src/plugins/{core,nexus,community}/` — every visible UI element is a plugin contribution loaded by `ExtensionHost`. The shell starts empty.
- `packages/nexus-extension-api/` — the stable `@nexus/extension-api` TypeScript contract for shell plugin authors.

**Guardrails when adding features:**

- New backend capability ⇒ add an IPC handler to the appropriate `nexus-<service>` crate so it's reachable from CLI, TUI, MCP, and the shell uniformly. Do **not** add bespoke `#[tauri::command]` handlers in `shell/src-tauri/` for new capability; route it through `kernel_invoke` → `ipc_call` instead. The Tauri bridge in `shell/src-tauri/src/lib.rs` registers 29 commands today (10 kernel+bridge, 5 plugin-mgmt, 6 persistence, 3 host-platform-primitive utility, 5 popout). Full breakdown at [`docs/0.1.2/shell.md`](docs/0.1.2/shell.md); adherence audit at [`docs/0.1.2/architecture-adherence.md`](docs/0.1.2/architecture-adherence.md). Adding a new shell-management command (popout, persistence) is OK; a new feature command is not unless it's a host-platform primitive wrapping a Tauri-only capability.
- New setting ⇒ add a field to a `Config` struct in the owning service crate with a `serde(default)` helper, document it in [`docs/0.1.2/settings/`](docs/0.1.2/settings/), and **delete the corresponding row** from [`docs/0.1.2/settings/hardcoded-rust.md`](docs/0.1.2/settings/hardcoded-rust.md) or [`hardcoded-shell.md`](docs/0.1.2/settings/hardcoded-shell.md). Do not introduce a new top-level TOML file in `.forge/` without an entry in [`docs/0.1.2/settings/README.md`](docs/0.1.2/settings/README.md).
- New UI ⇒ add a plugin under `shell/src/plugins/nexus/<feature>/`, not a hard-coded shell component.
- If you change an IPC-boundary type (anything that flows through `ipc_call`), regenerate bindings with `scripts/check_ipc_drift.sh` before committing.

## Plugin tiers

- **Core plugins** — native Rust crates registered at bootstrap, full access. 24 in-tree — list at [`docs/0.1.2/plugins/core.md`](docs/0.1.2/plugins/core.md).
- **Community plugins** — WASM-sandboxed via wasmtime or JS-sandboxed via iframe, capability-gated. See [`docs/0.1.2/plugins/community.md`](docs/0.1.2/plugins/community.md). Scaffold via `nexus plugin scaffold --type wasm ...` or `--type script ...`.

## Forge layout

A "forge" is a user's directory of markdown files. Nexus stores its index alongside in `<forge>/.forge/`. Full schema + every file at [`docs/0.1.2/settings/README.md`](docs/0.1.2/settings/README.md). Key entries:

```
<forge>/
├── .forge/
│   ├── index.db          # SQLite — derived state
│   ├── search/           # Tantivy — derived state
│   ├── app.toml          # AppConfig — see settings/forge-config.md
│   ├── workspace.json    # shell layout state
│   ├── ai.toml mcp.toml lsp.toml dap.toml acp.toml notifications.toml
│   ├── kv.sqlite3        # KV store
│   ├── procmgr.sqlite sessions.sqlite agent/transcripts.sqlite
│   ├── .editor/crdt/     # CRDT snapshots
│   ├── .editor/journal/  # crash journal for open notes (RFC 0009 row 5)
│   ├── .kernel/audit.db  # audit log
│   ├── plugins/          # community WASM plugins (forge-scoped)
│   ├── logs/  temp/  .lock
│   └── …
├── .trash/               # recoverable deletes (C3 #356) — excluded from index/watcher/list_dir
├── .aiignore             # AI exclusion patterns (C28 #381) — with `ai: exclude` frontmatter, keeps notes out of embedding/RAG/enrichment
└── <user markdown files>
```

`NEXUS_FORGE_PATH` env var or `--forge-path` CLI flag selects the forge root.

<!-- gitnexus:start -->
# Code impact analysis

Assess impact using available source, dependency, and call-site analysis plus
focused tests. GitNexus is optional assistance, not a prerequisite for editing
or committing; its absence must not block this work.

## Always Do

- **MUST assess impact before editing a symbol.** Inspect its direct callers,
  dependencies, and affected execution flows. Report the blast radius and risk
  level to the user. Use source searches and Cargo metadata when appropriate;
  GitNexus impact/query/context tools may supplement that analysis if available.
- **MUST review changed scope before committing.** Inspect the diff, verify that
  changed symbols and execution flows are expected, and run focused regression
  tests and applicable dependency checks. Optional `gitnexus_detect_changes()`
  can supplement this review.
- **MUST warn the user** about HIGH or CRITICAL risk before proceeding with edits.
- If relying on GitNexus, check index freshness. Refresh a stale index with
  `npx gitnexus analyze` when available, or use current source analysis instead.

## Never Do

- NEVER edit a function, class, or method without first assessing its impact.
- NEVER ignore HIGH or CRITICAL risk findings.
- NEVER rename symbols with blind find-and-replace. Use semantic tooling or
  verify every affected definition and reference with source review and tests.
- NEVER commit without reviewing changed scope and the applicable test results.

## Optional GitNexus resources

| Resource | Use for |
|----------|---------|
| `gitnexus://repo/nexus/context` | Codebase overview, check index freshness |
| `gitnexus://repo/nexus/clusters` | All functional areas |
| `gitnexus://repo/nexus/processes` | All execution flows |
| `gitnexus://repo/nexus/process/{name}` | Step-by-step execution trace |

## Optional GitNexus guidance (when installed)

| Task | Skill file |
|------|------------|
| Understand architecture / "How does X work?" | `.claude/skills/gitnexus/gitnexus-exploring/SKILL.md` |
| Blast radius / "What breaks if I change X?" | `.claude/skills/gitnexus/gitnexus-impact-analysis/SKILL.md` |
| Trace bugs / "Why is X failing?" | `.claude/skills/gitnexus/gitnexus-debugging/SKILL.md` |
| Rename / extract / split / refactor | `.claude/skills/gitnexus/gitnexus-refactoring/SKILL.md` |
| Tools, resources, schema reference | `.claude/skills/gitnexus/gitnexus-guide/SKILL.md` |
| Index, status, clean, wiki CLI commands | `.claude/skills/gitnexus/gitnexus-cli/SKILL.md` |

<!-- gitnexus:end -->
