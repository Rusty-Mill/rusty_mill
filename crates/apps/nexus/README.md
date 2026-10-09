# Nexus

A personal, plugin-extensible knowledge environment built in Rust. Nexus combines file-based note management with full-text search, a knowledge graph, AI-powered RAG, and a plugin system — accessible via CLI, terminal UI, desktop shell, or MCP server.

The plugin-first desktop shell at [`shell/`](shell/) + [`shell/src-tauri/`](shell/src-tauri/) (crate `nexus-shell`) is the single desktop target per [ADR 0011](docs/adr/0011-adopt-plugin-first-shell.md). The legacy tri-pane shell was removed in 2026-04 — see [`docs/architecture/legacy-shell-retirement.md`](docs/architecture/legacy-shell-retirement.md) for the migration story, or recover the code via the `v0.1.0-legacy-shell` git tag.

## Architecture

Nexus follows a **microkernel** design. A small core (kernel + event bus) coordinates independent subsystems, each in its own crate. The Cargo workspace has 38 members (the `shell/` desktop target is intentionally excluded — see [`docs/0.1.2/crates.md`](docs/0.1.2/crates.md) for the full inventory); the most load-bearing are:

```
nexus-kernel        Event bus, plugin lifecycle, capability enforcement, IPC dispatcher
nexus-storage       File-as-truth, SQLite index, Tantivy FTS, file watcher, knowledge graph
nexus-security      OS keyring credential vault, audit logging, path validation
nexus-plugins       WASM sandbox (wasmtime), plugin manifests, hot-reload
nexus-ai            Provider traits (Claude, OpenAI, Ollama, llama.cpp), embeddings (fastembed), RAG
nexus-mcp           MCP server library — 65 built-in nexus_* tools for forge operations
nexus-cli           `nexus` binary — headless CLI with full subcommands (also hosts `nexus mcp`)
nexus-tui           `nexus-tui` binary — ratatui-based terminal interface
nexus-bootstrap     Runtime assembler (build_cli_runtime, build_tui_runtime, init_forge)
nexus-theme         Theming engine: CSS variables, theme packages, layout, snippet cascade
nexus-shell         Tauri 2 desktop shell at `shell/` — plugin-first, hosts `@nexus/extension-api`
nexus-types         Shared type definitions (leaf)
```

Service plugins (each a `CorePlugin` registered by `nexus-bootstrap`):
`nexus-acp`, `nexus-agent`, `nexus-ai-runtime`, `nexus-audio`, `nexus-collab`,
`nexus-comments`, `nexus-crdt`, `nexus-dap`, `nexus-database`, `nexus-editor`,
`nexus-formats`, `nexus-git`, `nexus-kv`, `nexus-linkpreview`, `nexus-lsp`,
`nexus-notifications`, `nexus-panic-log`, `nexus-remote`, `nexus-skills`,
`nexus-templates`, `nexus-terminal`, `nexus-workflow`, plus `nexus-plugin-api`
(SDK surface) and `nexus-fuzz` (fuzzing harness).
See [`Cargo.toml`](Cargo.toml) for the authoritative list.

The central concept is the **Forge** — a directory of markdown files that Nexus indexes, links, searches, and extends with AI. Files on disk are always the source of truth; the SQLite index is rebuildable.

## Quick Start

### Prerequisites

- Rust stable toolchain ([rustup.rs](https://rustup.rs))
- A directory of markdown files (or start fresh)

### Build

```bash
git clone <repo-url> && cd nexus
cargo build --release
```

Binaries land in `target/release/`:
- `nexus` — headless CLI
- `nexus-tui` — terminal UI

### Initialize a Forge

```bash
# Create a new forge (creates .forge/ metadata directory)
./target/release/nexus forge init ~/notes

# Or set the env var to avoid passing --forge-path every time
export NEXUS_FORGE_PATH=~/notes
```

### Create and Search Notes

```bash
nexus content create projects/nexus.md --content "# Nexus\nMy AI-native knowledge base."
nexus content search "knowledge"
nexus content daily                    # Create/open today's daily note
nexus content tasks                    # List all tasks ([ ] items) across files
nexus content backlinks projects/nexus.md
```

### Browse in the TUI

```bash
nexus-tui ~/notes
# or with env var set:
nexus-tui
```

### Knowledge Graph

```bash
nexus graph status                     # Node/edge counts, density
nexus graph unresolved                 # Broken [[wikilinks]]
nexus graph neighbors projects/nexus.md --depth 2
```

### Desktop (Tauri) shell

The plugin-first shell at [`shell/`](shell/) is the single active
desktop target. Needs Node.js + pnpm and the Linux webview libs
(`webkit2gtk-4.1`, `libsoup-3.0`) on top of the Rust toolchain.

```bash
cd shell
pnpm install
pnpm tauri:dev    # launches the Rust shell + Vite + webview
```

Every visible UI element is a plugin contribution loaded by
`ExtensionHost` from `shell/src/plugins/{core,nexus,community}/`. See
[`shell/README.md`](shell/README.md) and the stable contract at
[`packages/nexus-extension-api/`](packages/nexus-extension-api/).

### MCP Server

Start the MCP server for use with Claude Code, Cursor, or any MCP client:

```bash
nexus mcp serve    # Serves 65 built-in nexus_* tools over stdio
```

The CLI server is stdio-only; the former `--transport` and `--bind` options
are no longer accepted. Nexus can still connect to external HTTP MCP servers
through its MCP Host configuration in `.forge/mcp.toml`.

## CLI Reference

```
nexus [OPTIONS] <COMMAND>

Global options:
  --forge-path <PATH>    Forge directory (or set NEXUS_FORGE_PATH)
  --format <FMT>         Output: text | json | jsonl | table  [default: text]
  -v...                  Verbosity: -v (info), -vv (debug), -vvv (trace)
  --no-color             Disable color output

Commands:
  forge      init, status
  content    create, read, delete, search, tasks, task-toggle, links,
             backlinks, daily, export
  graph      status, unresolved, neighbors
  tags       list, locate
  plugin     install, list, call, uninstall, scaffold, enable, disable,
             reset, settings
  skill      list, render
  bases      query, validate
  canvas     render
  agent      run, list, history
  workflow   run, list
  db         query, schema (forge index introspection)
  config     get, set, list
  git        status, log, blame, diff
  proc       list, kill (process manager via nexus-terminal)
  term       saved, run (saved-command snippets)
  watch      monitor filesystem changes (glob patterns)
  logs       tail, show, path
  ai         ask, embed, status, config
  mcp        serve (stdio server), servers, tools, call (external MCP host)
  tui        Launch the terminal UI in the current terminal
  desktop    Launch the Tauri desktop shell (forwards args to nexus-shell)
```

For details on individual subcommands see [`docs/users/cli.md`](docs/users/cli.md)
or run `nexus <subcommand> --help`.

## TUI Key Bindings

| Key | Action |
|-----|--------|
| `j`/`k` or arrows | Navigate |
| `Tab` | Toggle focus: tree / viewer |
| `Enter` / `l` | Open file or expand directory |
| `h` | Collapse directory |
| `b` | Toggle backlinks panel |
| `t` | Toggle task list view |
| `e` | Open in `$EDITOR` |
| `Ctrl+f` | Full-text search overlay |
| `/` | In-file find |
| `g` / `G` | Top / bottom |
| `Ctrl+d` / `Ctrl+u` | Page down / up |
| `q` / `Ctrl+c` | Quit |

## MCP Tools

When running as an MCP server (`nexus mcp serve`), the following 65 built-in
tools are exposed over stdio. Plugins may also contribute dynamic tools; use MCP
`tools/list` to discover the complete runtime surface. The authoritative handlers
are in [`crates/nexus-mcp/src/server.rs`](crates/nexus-mcp/src/server.rs).

| Tool | Description |
|------|-------------|
| `nexus_read_note` | Read a note's content by vault-relative path |
| `nexus_terminal_get_screen` | Read a terminal session's current visible screen (server-side VT grid) as text, plus the cursor position. |
| `nexus_terminal_get_scrollback` | Read a terminal session's scrollback (lines that scrolled off the top), oldest first. |
| `nexus_terminal_get_cwd` | Read a terminal session's working directory as reported by the child via OSC 7. |
| `nexus_terminal_get_cursor` | Read a terminal session's cursor position (col, row; zero-based). |
| `nexus_terminal_get_last_exit` | Read a terminal session's last finished command exit code and captured output (OSC 133). |
| `nexus_create_note` | Create a new note with the given path and markdown content |
| `nexus_update_note` | Update an existing note's content (creates if it does not exist) |
| `nexus_delete_note` | Delete a note by vault-relative path |
| `nexus_memory_search` | Full-text search the persistent cross-model memory store |
| `nexus_memory_add` | Store a new memory in the persistent cross-model memory store |
| `nexus_memory_recent` | List the most recent memories, newest first |
| `nexus_memory_facts` | Recall SPO entity facts from memory, optionally filtered by subject, predicate, and/or object |
| `nexus_memory_entities` | List the distinct entities mentioned by memory's SPO facts, each with its fact count (most-frequent first) |
| `nexus_memory_tags` | List the distinct tags across memories, each with the number of memories carrying it (most-frequent first) |
| `nexus_memory_vitality` | List active memories ranked by vitality (frequency + recency of recall) — the ones most likely to matter right now |
| `nexus_memory_recall` | Hybrid recall over memory: fuses full-text and semantic (vector) search via Reciprocal Rank Fusion. The best general way to find relevant memories |
| `nexus_memory_vector_sync` | Backfill embeddings for stored memories so semantic recall has data to search. Run once after importing or bulk-adding memories |
| `nexus_memory_sync` | Sync the memory store with a central memory hub (push local + pull remote, last-write-wins). Hub URL/secret/node default to NEXUS_MEMORY_* env vars |
| `nexus_memory_wiki_compile` | Synthesize a Markdown wiki page about a topic from related memories (saved to `wiki/<slug>.md` in the forge) and return its metadata |
| `nexus_memory_wiki_read` | Read a synthesized wiki page's Markdown by topic/slug |
| `nexus_memory_wiki_list` | List the synthesized wiki pages (slugs + paths) |
| `nexus_sandbox_policy` | Show the active OS-sandbox configuration (from sandbox.toml): the process-confinement mode (read-only / workspace-write / danger-full-access), writable roots, network access, and the brokered-download allowlist. Read-only introspection |
| `nexus_sandbox_download` | Perform a brokered, allowlisted download into a sandbox writable root on behalf of a network-confined process. Doubly gated: the net.http capability plus the sandbox.toml host allowlist + writable-root checks. Returns { bytes_written } |
| `nexus_memory_capture` | Capture a conversation turn / note as a memory; optionally decompose it into atomic facts (LLM). The deliberate 'remember this' call |
| `nexus_memory_consolidate` | Deduplicate memories: supersede exact (normalized) duplicates, keeping the freshest. Use dry_run to preview |
| `nexus_memory_export` | Export every stored memory as full records (oldest first), suitable for backup or re-import into another store |
| `nexus_memory_import` | Restore or merge memory records previously produced by nexus_memory_export. Last-write-wins per id: an incoming record only overwrites a local one when it's strictly newer, so replaying an old backup can't clobber newer local edits |
| `nexus_memory_get` | Fetch a single memory by id (records access for vitality ranking) |
| `nexus_memory_update` | Patch a stored memory's fields (content, category, tags, status, memory_type, SPO fact fields). Only provided fields change |
| `nexus_memory_delete` | Permanently forget a memory by id |
| `nexus_export_html` | Render a forge note to a standalone styled HTML document. Returns the HTML inline, or writes it to `dest` when given |
| `nexus_list_notes` | List notes in the forge, optionally filtered by a path prefix |
| `nexus_search` | Full-text search across notes. Rebuilds the search index before querying. |
| `nexus_backlinks` | Find all notes that link to the specified note (backlinks) |
| `nexus_outgoing_links` | Find all outgoing links from the specified note |
| `nexus_graph_status` | Get knowledge graph statistics: node count, edge count, unresolved links |
| `nexus_entity_get` | Fetch a forge entity by canonical id or alias: type, aliases, description, and outgoing relations |
| `nexus_entity_search` | Search forge entities by substring against id / aliases / description, optionally filtered by entity_type |
| `nexus_entity_relations` | List an entity's relations (outgoing / incoming / both), alias-resolved to canonical ids |
| `nexus_list_tags` | List all occurrences of a tag by name across the forge |
| `nexus_list_tasks` | List tasks (checkboxes) across notes with optional completed/file filters |
| `nexus_toggle_task` | Toggle a task's completed/incomplete state by its database ID |
| `nexus_ask` | Ask a question via RAG over your notes |
| `nexus_semantic_search` | Embedding-driven retrieval over your notes: returns raw ranked chunk hits (file_path, score, excerpt/chunk_text) rather than a synthesized answer. Pass hybrid=true to fuse vector similarity with lexical BM25 (RRF) instead of vector-only. Requires an AI embedding provider to be configured. |
| `nexus_list_skills` | List all skills (authored prompt templates) declared in the forge's .forge/skills directory |
| `nexus_render_skill` | Render a skill template to its expanded prompt body, given an optional `values` map of placeholder substitutions |
| `nexus_context` | Resolve a code symbol from the BL-114 index and return its source location, doc comment, enclosing impl/class/module, and sibling symbols (other methods on the same impl). Pass `name` plus an optional `path` to disambiguate symbols defined in multiple files. |
| `nexus_impact` | Assess the blast radius of changing a symbol. v1 uses a kind-based heuristic (functions are MEDIUM, traits/interfaces HIGH, modules/impls CRITICAL, methods LOW, …) and surfaces sibling symbols as a proxy for direct callers. Returns a `degraded` flag because BL-114's index does not yet carry call-edges; agents should temper recommendations accordingly. `depth` is accepted but treated as `1`. |
| `nexus_detect_changes` | List uncommitted forge files plus every BL-114 indexed symbol that lives in them. Powers a pre-commit blast-radius preview: an agent can run this before editing to know which code-symbols the user has already touched in their working tree. |
| `nexus_git_remotes` | List the forge repository's configured git remote names (e.g. ["origin"]). Read-only, no network access. |
| `nexus_git_fetch` | Fetch all refs from a remote into the forge repository's remote-tracking branches. Does not modify the working tree or HEAD — use nexus_git_pull to also merge. |
| `nexus_git_pull` | Fetch from a remote and merge the named branch into HEAD. Returns { fast_forward, conflicts, commit_hash } — a non-empty `conflicts` list means the merge paused mid-flight and needs manual resolution (or com.nexus.git::abort_merge). |
| `nexus_comment_list` | List every comment thread on a note, each with its full reply history. Threads are anchored to a specific block; use nexus_comment_create_thread / nexus_comment_add_reply to write. |
| `nexus_comment_create_thread` | Start a new comment thread on a note, anchored to one of its top-level blocks (block_index, 0-based, default 0 = the first block). Comments are file-as-truth JSON sidecars — a non-destructive annotation channel, distinct from editing the note body directly. Internally resolves the anchor via com.nexus.editor's open/get_tree/stamp_block chain, the same machinery the shell's comment pane uses. |
| `nexus_comment_add_reply` | Append a reply to an existing comment thread. |
| `nexus_comment_set_resolved` | Mark a comment thread resolved or unresolved. |
| `nexus_comment_edit_comment` | Edit an existing comment's body in place. |
| `nexus_comment_delete_comment` | Delete a single comment from a thread. Deleting a thread's only comment leaves an empty thread — use nexus_comment_delete_thread to remove the whole thread instead. |
| `nexus_comment_delete_thread` | Delete an entire comment thread, including all its replies. |
| `nexus_agent_run` | Delegate a goal to a Nexus agent session and run it end-to-end (auto-approving every tool call — there is no interactive approval channel over MCP). Returns the full transcript: rounds, tool calls, and outcome. Archetype selects the system prompt / tool profile (writer / coder / researcher / general, default general). |
| `nexus_agent_sessions` | List stored agent sessions — id, outcome, goal, and fork lineage (parent_id / branch_point) for sessions created via resume/branch/rewind. Read-only. |
| `nexus_workflow_list` | List every loaded `.workflow.toml` — name, trigger config, and step count. Read-only. |
| `nexus_workflow_run` | Fire a user-authored workflow on demand and run every step to completion. Each step is still gated by its own target handler's capabilities (issue #77 — the workflow boundary itself imposes no additional cap ceiling), so this can have side effects as broad as the workflow's steps. |
| `nexus_kernel_stats` | Snapshot the kernel's BL-093 metrics: per-(plugin, command) IPC call counters + duration histograms (p50/p95/p99), event-bus publish counters, capability-check counters by outcome, plugin-lifecycle-hook histograms, current event-bus queue depth, and `metrics_dropped_total` (sentinel for the per-metric key cap). Read-only. Useful for monitoring kernel hot paths or diagnosing latency / capability-deny regressions from an agent. |

## Plugin System

Nexus supports two plugin tiers:

- **Core plugins** — native Rust, full access
- **Community plugins** — WASM-sandboxed via wasmtime, capability-gated

```bash
# Scaffold a new plugin
nexus plugin scaffold --type wasm --id my-plugin --name "My Plugin" --author "Me"

# Install and use
nexus plugin install ./my-plugin
nexus plugin call my-plugin some-command --args '{"key": "value"}'
```

## Configuration

### Forge Structure

```
~/notes/                  # Your files (source of truth)
├── .forge/
│   ├── index.db          # SQLite index (WAL mode, rebuildable)
│   ├── search/           # Tantivy FTS index
│   ├── config.toml       # Forge-level config
│   ├── logs/             # Operation logs
│   └── temp/             # Atomic write staging
├── projects/
│   └── nexus.md
├── daily/
│   └── 2026-04-13.md
└── ...
```

### Environment Variables

| Variable | Purpose | Default |
|----------|---------|---------|
| `NEXUS_FORGE_PATH` | Forge root directory | `~/.nexus/default` |
| `RUST_LOG` | Tracing filter | `warn` |

## Development

```bash
cargo test --workspace          # Run all tests
cargo clippy --workspace        # Lint
cargo build -p nexus-cli        # Build just the CLI
cargo build -p nexus-tui        # Build just the TUI
```

### Key Docs

- [`docs/architecture/C4.md`](docs/architecture/C4.md) — current architecture overview
- [`docs/adr/`](docs/adr/) — architecture decision records
- [`docs/PRDs/`](docs/PRDs/00-index.md) — product requirements (see `IMPLEMENTATION_STATUS.md` for current state)
- [`docs/developer/getting-started.md`](docs/developer/getting-started.md) — plugin quickstart
- [`docs/shell/writing-a-plugin.md`](docs/shell/writing-a-plugin.md) — plugin author reference
- [`docs/archive/planning/`](docs/archive/planning/) — historical phase plans and audits

## License

MIT OR Apache-2.0
