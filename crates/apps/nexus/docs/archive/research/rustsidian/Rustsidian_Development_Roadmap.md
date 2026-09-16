> **Archived 2026-09-08** — Imported verbatim from the private `baileyrd/Rustsidian` repo (`docs/`) under RFC 0009 row 6. Rustsidian is retired; this is reference material only and describes Rustsidian's SvelteKit/Tauri stack, not Nexus.

# Rustsidian Development Roadmap

> **Created:** April 3, 2026
> **Last updated:** April 4, 2026 — all phases through v1.6 shipped
> **Status:** v1.0–v1.6 shipped 2026-04-04 (CLI, MDX, Desktop, AI, Plugins, Canvas/Bases, Sync). v1.7 Mobile config ready, pending platform init. 194 tests passing.
> **Vision:** A Rust-native personal knowledge management platform with an integrated terminal, MDX support, and AI-first architecture. CLI-first, then desktop (Tauri), then mobile.

---

## Product Vision & Differentiation

Rustsidian is not an Obsidian clone. It occupies a different point in the design space — one where the knowledge management tool, the development environment, and the AI assistant are the same application. The key differentiators from Obsidian are:

**MDX as a first-class document format.** Where Obsidian is Markdown-native, Rustsidian treats MDX (Markdown + JSX) as a core content type. This means documents can contain live, interactive components — charts, data visualizations, interactive widgets, embedded tools — rendered inline alongside prose. This blurs the line between "note" and "application" and makes Rustsidian especially powerful for technical documentation, data-driven writing, and knowledge artifacts that do things rather than just describe them.

**Integrated terminal.** A built-in terminal emulator means you never leave the app to run commands, scripts, or interact with CLI tools. This makes Rustsidian a genuine working environment rather than a writing environment you alt-tab away from.

**AI-native architecture.** Rather than bolting AI on as a plugin, Rustsidian exposes its entire domain model as an MCP server from day one. AI agents can create, read, search, link, and reason over your knowledge base as first-class operations — not through fragile screen-scraping or limited API wrappers.

**Rust-native performance.** A single Rust core library powers every surface — CLI, desktop, and eventually mobile — with consistent behavior, sub-millisecond file operations, and the ability to handle vaults with hundreds of thousands of notes without degradation.

---

## Architecture Overview

```
┌─────────────────────────────────────────────────────────┐
│                    Rustsidian Surfaces                   │
│                                                         │
│   ┌──────────┐   ┌──────────────┐   ┌──────────────┐   │
│   │   CLI    │   │   Desktop    │   │   Mobile     │   │
│   │  (clap   │   │   (Tauri 2)  │   │  (Tauri 2    │   │
│   │  ratatui)│   │  SvelteKit   │   │   or UniFFI  │   │
│   └────┬─────┘   │  + terminal  │   │   + native)  │   │
│        │         └──────┬───────┘   └──────┬───────┘   │
│        │                │                  │            │
│  ┌─────┴────────────────┴──────────────────┴─────────┐  │
│  │              rustsidian-core (Rust library)        │  │
│  │                                                   │  │
│  │  ┌───────────┐ ┌───────────┐ ┌─────────────────┐ │  │
│  │  │ Vault     │ │ MDX       │ │ Search & Index  │ │  │
│  │  │ Engine    │ │ Pipeline  │ │ (tantivy)       │ │  │
│  │  ├───────────┤ ├───────────┤ ├─────────────────┤ │  │
│  │  │ Link      │ │ Component │ │ Graph Engine    │ │  │
│  │  │ Resolver  │ │ Registry  │ │ (petgraph)      │ │  │
│  │  ├───────────┤ ├───────────┤ ├─────────────────┤ │  │
│  │  │ Property  │ │ Plugin    │ │ MCP Server      │ │  │
│  │  │ System    │ │ Host      │ │ (AI interface)  │ │  │
│  │  ├───────────┤ ├───────────┤ ├─────────────────┤ │  │
│  │  │ Task      │ │ Terminal  │ │ Sync Engine     │ │  │
│  │  │ Engine    │ │ Emulator  │ │ (CRDT-based)    │ │  │
│  │  └───────────┘ └───────────┘ └─────────────────┘ │  │
│  │                                                   │  │
│  │  ┌─────────────────────────────────────────────┐  │  │
│  │  │         Storage Layer (SQLite + files)       │  │  │
│  │  └─────────────────────────────────────────────┘  │  │
│  └───────────────────────────────────────────────────┘  │
└─────────────────────────────────────────────────────────┘
```

---

## Technology Stack

### Core Library (`rustsidian-core`)

| Component | Crate / Technology | Rationale |
|---|---|---|
| **Language** | Rust (2021 edition) | Performance, safety, cross-platform FFI, single binary deployment |
| **Async runtime** | `tokio` | Industry standard; required by most networking crates |
| **Database** | `rusqlite` (SQLite via bundled `libsqlite3`) | Local-first, zero-config, proven at scale, WAL mode for concurrent reads |
| **Full-text search** | `tantivy` 0.26 | Rust-native Lucene-equivalent; BM25 ranking, incremental dirty-flag indexing; validated at 50ms on 1,100 notes |
| **Graph data** | `petgraph` | In-memory directed graph for link resolution, backlinks, graph views |
| **Markdown parsing** | `pulldown-cmark` + custom extensions | CommonMark-compliant, streaming, extensible; handles wikilinks, embeds, callouts, task checkboxes |
| **MDX / JSX parsing** | `markdown` 1.0.0 (markdown-rs / wooorm) | Spec-aligned JSX AST extraction; `swc`/`oxc` rejected due to compile time and binary size |
| **MDX HTML export** | `ammonia` | XSS-safe HTML sanitization; preserves `data-*` attributes for component placeholders |
| **YAML frontmatter** | `yaml-rust2` | Actively maintained fork of yaml-rust; property/frontmatter parsing and serialization |
| **MCP server** | `rmcp` 1.3.0 | Official Rust MCP SDK; `spawn_blocking` bridges sync Vault to async handler |
| **CLI framework** | `clap` (derive API) | Declarative, well-documented, auto-generated help and completions |
| **TUI framework** | `ratatui` + `crossterm` | Full terminal UI for interactive mode |
| **Serialization** | `serde` + `serde_json` | Universal serialization for config, canvas files, sync payloads |
| **File watching** | `notify` | Cross-platform filesystem event notification |
| **Terminal emulation** | `alacritty_terminal` (library crate) | Battle-tested terminal emulation engine extracted from Alacritty |
| **Logging** | `tracing` + `tracing-subscriber` | Structured logging with span-based context, essential for debugging |
| **Error handling** | `thiserror` (library) + `anyhow` (CLI/app) | Typed errors in core, ergonomic handling at the surface |
| **HTTP client** | `reqwest` | For AI API calls, sync, and web clipper functionality |
| **UUID generation** | `uuid` (v7) | Time-sortable UUIDs for sync-friendly entity IDs |
| **Date/time** | `chrono` | ISO 8601 date handling for properties and daily notes |

### Desktop App (`rustsidian-desktop`)

| Component | Technology | Rationale |
|---|---|---|
| **App shell** | Tauri 2.0 | Rust backend + web frontend, ~10MB binary, native OS integration |
| **Frontend framework** | SvelteKit | Smallest bundle size, best DX for component-driven UI, reactive by default |
| **Styling** | Tailwind CSS 4 | Utility-first, theme-able, design-token-driven |
| **Editor** | CodeMirror 6 | Extensible, performant, supports custom syntax and decorations for MDX |
| **Terminal** | xterm.js (frontend) + `alacritty_terminal` (backend) | Industry standard terminal widget connected to Rust PTY backend |
| **Graph visualization** | D3.js (force-directed) or `pixi.js` (WebGL) | Interactive, performant at scale for knowledge graph rendering |
| **MDX component rendering** | Custom Svelte MDX runtime | Render JSX components from MDX within the editor/preview |
| **IPC** | Tauri commands + events | Type-safe Rust ↔ JS communication |

### Mobile (Future)

| Component | Technology | Rationale |
|---|---|---|
| **Option A** | Tauri 2.0 mobile targets | Same codebase as desktop; Tauri 2 supports iOS/Android |
| **Option B** | UniFFI + native UI (SwiftUI/Jetpack Compose) | Maximum native feel at the cost of separate UI codebases |

Recommendation: start with Tauri 2.0 mobile targets to validate, then evaluate whether native UI is worth the investment based on user feedback.

---

## Data Model

### Entity Design

Every entity follows a consistent pattern for sync-readiness:

```rust
pub struct Note {
    pub id: Uuid,           // v7, time-sortable
    pub path: String,       // relative to vault root
    pub title: String,      // derived from filename or frontmatter
    pub content: String,    // raw MDX/Markdown content
    pub content_type: ContentType, // Markdown | MDX
    pub checksum: String,   // blake3 hash of content for change detection
    pub properties: Value,  // serde_json::Value for flexible frontmatter
    pub created_at: DateTime<Utc>,
    pub updated_at: DateTime<Utc>,
    pub deleted_at: Option<DateTime<Utc>>, // soft delete for sync
    pub sync_version: i64,  // monotonic version counter
}

pub enum ContentType {
    Markdown,
    MDX,
    Canvas,  // JSON Canvas format
    Base,    // Database view definition
}
```

### SQLite Schema (v1)

```sql
-- Schema version tracking
CREATE TABLE schema_version (
    version INTEGER PRIMARY KEY,
    applied_at TEXT NOT NULL DEFAULT (datetime('now'))
);

-- Core notes table
CREATE TABLE notes (
    id TEXT PRIMARY KEY,         -- UUID v7
    path TEXT NOT NULL UNIQUE,
    title TEXT NOT NULL,
    content TEXT NOT NULL,
    content_type TEXT NOT NULL DEFAULT 'markdown',
    checksum TEXT NOT NULL,
    properties TEXT,             -- JSON
    created_at TEXT NOT NULL,
    updated_at TEXT NOT NULL,
    deleted_at TEXT,
    sync_version INTEGER NOT NULL DEFAULT 0
);

-- Full-text search (tantivy handles this externally, but we keep
-- a trigger-based dirty flag for incremental re-indexing)
CREATE TABLE search_dirty (
    note_id TEXT PRIMARY KEY REFERENCES notes(id),
    dirty_since TEXT NOT NULL DEFAULT (datetime('now'))
);

-- Link graph (materialized from parsed content)
CREATE TABLE links (
    id INTEGER PRIMARY KEY AUTOINCREMENT,
    source_id TEXT NOT NULL REFERENCES notes(id),
    target_path TEXT NOT NULL,       -- resolved or unresolved target
    target_id TEXT REFERENCES notes(id), -- NULL if target doesn't exist yet
    link_type TEXT NOT NULL,         -- 'wikilink', 'markdown', 'embed', 'tag'
    context TEXT,                    -- surrounding text for backlink preview
    position_line INTEGER,
    position_col INTEGER
);

-- Properties index (denormalized for fast filtering/sorting)
CREATE TABLE properties_index (
    note_id TEXT NOT NULL REFERENCES notes(id),
    key TEXT NOT NULL,
    value_text TEXT,
    value_number REAL,
    value_date TEXT,
    value_bool INTEGER,
    value_type TEXT NOT NULL,       -- 'text', 'number', 'date', 'boolean', 'list', 'link'
    PRIMARY KEY (note_id, key)
);

-- Tasks (denormalized for fast queries)
CREATE TABLE tasks (
    id INTEGER PRIMARY KEY AUTOINCREMENT,
    note_id TEXT NOT NULL REFERENCES notes(id),
    line_number INTEGER NOT NULL,
    content TEXT NOT NULL,
    completed INTEGER NOT NULL DEFAULT 0,
    due_date TEXT,
    priority INTEGER,               -- 0=none, 1=low, 2=medium, 3=high
    tags TEXT                        -- JSON array
);

-- Tags (denormalized for fast lookup)
CREATE TABLE tags (
    tag TEXT NOT NULL,
    note_id TEXT NOT NULL REFERENCES notes(id),
    source TEXT NOT NULL DEFAULT 'inline', -- 'inline' or 'frontmatter'
    PRIMARY KEY (tag, note_id)
);

-- Daily notes config
CREATE TABLE daily_notes_config (
    id INTEGER PRIMARY KEY CHECK (id = 1), -- singleton
    folder TEXT NOT NULL DEFAULT 'Daily Notes',
    format TEXT NOT NULL DEFAULT 'YYYY-MM-DD',
    template TEXT
);

-- Plugin state
CREATE TABLE plugin_state (
    plugin_id TEXT PRIMARY KEY,
    enabled INTEGER NOT NULL DEFAULT 1,
    config TEXT,                     -- JSON
    version TEXT
);

-- Sync metadata
CREATE TABLE sync_metadata (
    key TEXT PRIMARY KEY,
    value TEXT NOT NULL
);

-- Indexes
CREATE INDEX idx_notes_path ON notes(path);
CREATE INDEX idx_notes_updated ON notes(updated_at);
CREATE INDEX idx_notes_deleted ON notes(deleted_at);
CREATE INDEX idx_links_source ON links(source_id);
CREATE INDEX idx_links_target ON links(target_id);
CREATE INDEX idx_links_target_path ON links(target_path);
CREATE INDEX idx_properties_key ON properties_index(key);
CREATE INDEX idx_tags_tag ON tags(tag);
CREATE INDEX idx_tasks_note ON tasks(note_id);
CREATE INDEX idx_tasks_due ON tasks(due_date);
```

### Migration Runner

```rust
// Migrations are embedded at compile time and run on vault open
const MIGRATIONS: &[(&str, &str)] = &[
    ("001", include_str!("../migrations/001_initial.sql")),
    // Future migrations added here
];

pub fn run_migrations(conn: &Connection) -> Result<()> {
    let current = get_current_version(conn)?;
    for (version, sql) in MIGRATIONS.iter().skip(current) {
        conn.execute_batch(sql)?;
        set_version(conn, version)?;
    }
    Ok(())
}
```

---

## MDX Pipeline

MDX support is what sets Rustsidian apart from Markdown-only tools. The pipeline handles parsing, compilation, and rendering of MDX documents.

### What MDX Enables

MDX allows users to embed interactive JSX components directly inside Markdown documents:

```mdx
---
title: Q1 Performance
tags: [analytics, quarterly]
---

# Q1 2026 Performance Review

Revenue grew 23% quarter-over-quarter, driven primarily by enterprise contracts.

<RevenueChart data={props.quarterlyData} />

The team shipped 47 features this quarter. Here's the breakdown by category:

<FeatureBreakdown quarter="Q1-2026" interactive={true} />

## Action Items

- [ ] Schedule Q2 planning offsite
- [ ] Review hiring pipeline with recruiting
```

### Pipeline Architecture

```
  .mdx file on disk
       │
       ▼
  ┌─────────────┐
  │  Frontmatter │  ← serde_yaml: extract YAML properties
  │  Extraction   │
  └──────┬────────┘
         │
         ▼
  ┌─────────────┐
  │  Markdown    │  ← pulldown-cmark: parse Markdown AST
  │  Parser      │     with custom extensions for wikilinks,
  └──────┬────────┘     callouts, embeds, task checkboxes
         │
         ▼
  ┌─────────────┐
  │  JSX         │  ← swc: parse and transform JSX nodes
  │  Transform   │     within the Markdown AST
  └──────┬────────┘
         │
         ▼
  ┌─────────────┐
  │  Component   │  ← resolve <ComponentName> to registered
  │  Resolution  │     component implementations
  └──────┬────────┘
         │
         ├──── CLI output: rendered Markdown (JSX stripped or placeholder)
         │
         ├──── Desktop: Svelte components rendered inline in editor
         │
         └──── Export: static HTML with bundled JS for interactive components
```

### Component Registry

Components are registered by plugins or built-in:

```rust
pub struct ComponentDefinition {
    pub name: String,           // e.g. "RevenueChart"
    pub source: ComponentSource,
    pub props_schema: Value,    // JSON Schema for props validation
    pub description: String,
}

pub enum ComponentSource {
    BuiltIn(String),           // ships with Rustsidian
    Plugin { id: String },     // provided by a plugin
    Vault { path: String },    // user-defined in vault's .rustsidian/components/
}
```

### Built-in MDX Components (Ship with v0.3)

| Component | Purpose |
|---|---|
| `<Chart>` | Line, bar, pie charts from inline data or file reference |
| `<DataTable>` | Interactive sortable/filterable table |
| `<Mermaid>` | Mermaid diagram rendering (alternative to fenced code block) |
| `<Math>` | LaTeX math rendering (alternative to `$$` syntax) |
| `<Embed>` | Embed external content (iframe, video, audio) |
| `<Callout>` | Styled callout boxes (also available as `> [!type]` syntax) |
| `<CodePlayground>` | Executable code block with output panel |
| `<Terminal>` | Inline terminal session within a document |

---

## Integrated Terminal

### Architecture

The terminal is not a bolted-on feature — it's a core subsystem:

```
┌──────────────────────────────────────────┐
│           Desktop Frontend               │
│  ┌────────────────────────────────────┐  │
│  │          xterm.js widget           │  │  ← renders terminal output
│  │   (WebGL renderer, fit addon,      │  │     handles keyboard input
│  │    search addon, unicode11)        │  │
│  └───────────────┬────────────────────┘  │
│                  │ IPC (Tauri events)     │
│  ┌───────────────┴────────────────────┐  │
│  │      Terminal Manager (Rust)       │  │  ← manages PTY sessions
│  │                                    │  │
│  │  ┌──────────────────────────────┐  │  │
│  │  │  alacritty_terminal (grid,   │  │  │  ← ANSI parsing, grid state,
│  │  │  parser, ANSI processing)    │  │  │     scrollback buffer
│  │  └──────────────────────────────┘  │  │
│  │                                    │  │
│  │  ┌──────────────────────────────┐  │  │
│  │  │  portable_pty (PTY backend)  │  │  │  ← cross-platform PTY creation
│  │  │  or raw libc::forkpty        │  │  │
│  │  └──────────────────────────────┘  │  │
│  └────────────────────────────────────┘  │
└──────────────────────────────────────────┘
```

### Terminal Features

| Feature | Details |
|---|---|
| **Multiple sessions** | Tabbed terminal panes, split horizontal/vertical |
| **Shell integration** | Detect current directory, running command; auto-cd to vault root |
| **Vault-aware** | `rustsidian` commands available in-terminal with enhanced output |
| **Search** | Regex search across scrollback buffer |
| **Profiles** | Configurable shell profiles (bash, zsh, fish, PowerShell) |
| **Themes** | Terminal color schemes synced with app theme |
| **Links** | Clickable URLs and file paths |
| **Keyboard shortcuts** | Configurable, non-conflicting with editor shortcuts |

### Terminal ↔ Knowledge Base Integration

The terminal is context-aware of the knowledge base:

- Running `rsd search "query"` in the terminal uses the same search index as the GUI
- File paths in terminal output that match vault notes become clickable links to those notes
- Command output can be piped to create/append to notes: `some-command | rsd capture "Command Output"`
- The terminal's working directory is tracked and can be referenced in daily notes

---

## AI Integration (MCP Server)

### MCP Server Architecture

The MCP server exposes the full Rustsidian domain as tools that any MCP-compatible AI client can use:

```rust
// MCP server runs over stdio (for CLI) or HTTP (for desktop/remote)
pub struct RustsidianMCPServer {
    vault: Arc<Vault>,
    search_index: Arc<SearchIndex>,
    graph: Arc<RwLock<KnowledgeGraph>>,
}
```

### MCP Tools Exposed

| Tool | Description |
|---|---|
| `vault_search` | Full-text and semantic search across all notes |
| `note_read` | Read a note's content and properties |
| `note_create` | Create a new note with content and properties |
| `note_update` | Update a note's content or properties |
| `note_append` | Append content to an existing note |
| `note_delete` | Soft-delete a note |
| `notes_list` | List notes with filtering and sorting |
| `links_get` | Get all links from or to a note |
| `backlinks_get` | Get all notes that link to a given note |
| `graph_query` | Query the knowledge graph (neighbors, paths, clusters) |
| `tags_list` | List all tags with counts |
| `tags_search` | Find notes with a specific tag |
| `tasks_list` | List tasks with status/due date filtering |
| `tasks_toggle` | Toggle a task's completion state |
| `daily_note` | Read or append to today's daily note |
| `properties_search` | Find notes by property values |
| `terminal_exec` | Execute a command in the integrated terminal |

### MCP Resources

| Resource | Description |
|---|---|
| `vault://notes/{path}` | Individual note content |
| `vault://graph` | Full knowledge graph as JSON |
| `vault://config` | Vault configuration |
| `vault://tags` | Tag taxonomy |

### AI-Powered Features (Built on MCP)

These features consume the MCP interface internally:

| Feature | Description | Phase |
|---|---|---|
| **Smart linking** | AI suggests wikilinks as you type based on vault context | v0.5 |
| **Note synthesis** | Select multiple notes, generate a summary or new note | v0.5 |
| **Semantic search** | Vector embeddings for meaning-based search (local model) | v0.6 |
| **Auto-tagging** | Suggest tags for new notes based on content and existing taxonomy | v0.6 |
| **Knowledge gaps** | Identify topics referenced but not documented | v0.7 |
| **Daily digest** | AI-generated summary of recent vault activity | v0.7 |

---

## Plugin System

### Architecture

Plugins run in isolated WASM sandboxes for security and portability:

```
┌────────────────────────────────────────┐
│              Plugin Host               │
│                                        │
│  ┌──────────────────────────────────┐  │
│  │   WASM Runtime (wasmtime)        │  │
│  │                                  │  │
│  │  ┌────────┐  ┌────────┐         │  │
│  │  │Plugin A│  │Plugin B│  ...     │  │
│  │  │ .wasm  │  │ .wasm  │         │  │
│  │  └───┬────┘  └───┬────┘         │  │
│  │      │            │              │  │
│  │  ┌───┴────────────┴───────────┐  │  │
│  │  │   Plugin API (wit-bindgen) │  │  │
│  │  │   - vault operations       │  │  │
│  │  │   - UI registration        │  │  │
│  │  │   - command registration   │  │  │
│  │  │   - event subscription     │  │  │
│  │  │   - settings storage       │  │  │
│  │  └────────────────────────────┘  │  │
│  └──────────────────────────────────┘  │
└────────────────────────────────────────┘
```

### Plugin API Surface

```wit
// WIT (WASM Interface Types) definition
interface rustsidian-plugin {
    // Lifecycle
    on-activate: func() -> result<_, string>
    on-deactivate: func()

    // Vault access (sandboxed to declared permissions)
    read-note: func(path: string) -> result<note, string>
    create-note: func(path: string, content: string) -> result<note, string>
    update-note: func(path: string, content: string) -> result<_, string>
    search: func(query: string, limit: u32) -> result<list<search-result>, string>

    // UI extension points
    register-command: func(id: string, name: string, callback: func())
    register-sidebar-panel: func(id: string, title: string, render: func() -> string)
    register-editor-decoration: func(pattern: string, render: func(match: string) -> string)
    register-mdx-component: func(name: string, render: func(props: string) -> string)

    // Events
    subscribe: func(event: event-type, callback: func(payload: string))

    // Settings
    get-setting: func(key: string) -> option<string>
    set-setting: func(key: string, value: string)
}
```

### Plugin Languages

Because the host runs WASM, plugins can be authored in any language that compiles to WASM: Rust, TypeScript/JavaScript (via wasm-pack or component-model toolchains), Go, Python (via componentize-py), C/C++, etc.

Recommendation: ship a `create-rustsidian-plugin` CLI scaffold that generates a Rust or TypeScript plugin template with the WIT bindings pre-configured.

---

## Phased Delivery Plan

### Phase 0: Foundation — ✅ SHIPPED as v1.0 (2026-04-04)

**Goal:** Establish the project structure, core data model, and a working CLI that can manage a vault of Markdown files.

| Deliverable | Status | Actual outcome |
|---|---|---|
| Project scaffold | ✅ | Cargo workspace: `rustsidian-core`, `rustsidian-cli`, `rustsidian-mcp`. GitHub Actions CI (test/clippy/fmt parallel jobs). |
| Data model & SQLite schema | ✅ | 6 tables (`notes`, `links`, `tags`, `tasks`, `properties_index`, `search_dirty`), 9 indexes, WAL mode, embedded migration runner. |
| File ↔ DB sync engine | ✅ | `notify-debouncer-full` with 50ms debounce; file-wins conflict resolution; 100ms latency validated. |
| Markdown parser | ✅ | `pulldown-cmark` + custom extensions: wikilinks, embeds (`![[]]`), callouts (`> [!type]`), task checkboxes, YAML frontmatter via `yaml-rust2`. |
| Link resolver | ✅ | `petgraph` StableGraph; exact stem + path-style + Jaro-Winkler fuzzy matching (0.85 threshold); backlinks computed. |
| Property system | ✅ | Typed frontmatter properties denormalized to `properties_index`; filter/sort via SQL. |
| Search index | ✅ | `tantivy` 0.26; 6-field schema (note_id, path, title, body, tags, props); BM25 + incremental dirty-flag; snippet previews; scoped operators (`tag:`, `prop:`, `path:`). |
| CLI: core commands | ✅ | `rsd read/create/append/delete/search/links/backlinks/tags/tasks/daily` — all wired to live vault; 145 tests passing. |
| CLI: TUI mode | ✅ | `ratatui` + `crossterm`: two-panel layout (30/70 split), vim keybindings, `tui-markdown` preview, real-time fuzzy search via `SkimMatcherV2`. |
| MCP server (stdio) | ✅ | `rmcp` 1.3.0; 16 tools (CRUD, search, graph, taxonomy, daily); registered in Claude Code via `.mcp.json`. |

**Scale validated:** 1,100 notes; 2,210 notes/s create; 0.9ms list; 50ms search; zero MCP errors.

**Exit criteria:** ✅ Met — create, search, link, and manage a vault of 1,000+ Markdown notes from CLI; interact via Claude through MCP.

---

### Phase 1: MDX Pipeline — ✅ SHIPPED as v1.1 (2026-04-04)

**Goal:** Add MDX as a first-class content type, with parsing, compilation, and CLI-side rendering.

| Deliverable | Status | Actual outcome |
|---|---|---|
| MDX parser | ✅ | Two-pass architecture: `markdown` 1.0.0 (markdown-rs) extracts JSX AST; byte-offset range removal strips JSX before passing prose to `pulldown-cmark`. Note: `swc`/`oxc` rejected due to binary size and compile time. |
| Content type detection | ✅ | Migration M2 adds `jsx_components` table + `body_text` column; `content_type` discriminator auto-classifies `.md` vs `.mdx` on every ingest. |
| Component registry | ✅ | `ComponentRegistry` at surface layer (CLI/TUI/MCP) — core stays pure data. 5 built-in components: `Chart`, `DataTable`, `Mermaid`, `Math`, `Callout`. Each with distinct metadata display. |
| MDX → HTML export | ✅ | `rsd export <path> --format html`: prose via `pulldown-cmark`, JSX → `<div data-component>` placeholders; `ammonia` for XSS sanitization with `data-*` attribute preservation. Note: live JSX hydration deferred to desktop phase (no JS runtime in binary). |
| CLI MDX rendering | ✅ | `render_mdx_output()` with Unicode box-drawing placeholders (`┌─[Chart]──┐`); ASCII fallback; `rsd read` branches on `content_type`. |
| MDX link extraction | ✅ | Wikilink/embed extraction + BM25 indexing via `COALESCE(body_text, content)` — transparent for both `.md` and `.mdx`. |
| `rsd component init` | ✅ | Scaffolds ready-to-paste `.mdx` usage snippets for all 5 built-in components. |
| TUI MDX preview | ✅ | `preview_jsx_nodes` in `AppState`; Cyan-bordered `ratatui` Block widgets for JSX nodes; prose via `tui-markdown` unchanged. |
| MCP surface | ✅ | `get_jsx_nodes` tool (17th total): accepts vault-relative path, returns `Vec<JsxNode>` (name, props, line_number). |

**Exit criteria:** ✅ Met — `.mdx` files stored, indexed, searchable, linkable, CLI-rendered, HTML-exported, AI-queryable via MCP. 177 tests passing.

---

### Phase 2: Desktop App — Core — ✅ SHIPPED as v1.2 (2026-04-04)

**Goal:** Ship a working Tauri desktop app with a Markdown/MDX editor, file explorer, and basic navigation.

| Deliverable | Status | Actual outcome |
|---|---|---|
| Tauri project scaffold | ✅ | `rustsidian-desktop` crate. Tauri 2.0 with SvelteKit frontend. IPC bridge typed with `tauri-specta`. |
| File explorer panel | ✅ | Left sidebar: vault file tree with create/rename/delete. Real-time updates via vault-changed events. F2/double-click/right-click rename. |
| Editor: Markdown | ✅ | CodeMirror 6 with `@codemirror/lang-markdown`, syntax highlighting, oneDark theme. |
| Editor: Live Preview | ✅ | `livePreviewField` StateField: cursor-aware bold/italic marker hiding, heading styling (6 levels). |
| Editor: MDX | ✅ | JSX syntax highlighting via LanguageDescription; `mdxWidgets.ts` ViewPlugin replaces JSX tags with inline styled badges (cursor-aware show/hide). |
| Tabs and panes | ✅ | Multi-tab editor with open/close/pin/reorder/dirty tracking. Horizontal/vertical split panes with draggable divider. |
| Search UI | ✅ | Command palette (Ctrl+P) with fuzzy note search, full-text search (`/`), tag filter (`#`), app commands (`>`), heading jump (`@`). |
| Graph view | ✅ | sigma.js + graphology force-directed graph. Full vault and local N-hop views. Click to navigate, zoom/pan, hover tooltip. |
| Integrated terminal | ✅ | `portable-pty` backend + xterm.js frontend. Shell starts in vault root. Ctrl+` toggle. File paths clickable. Auto-resize. |
| Wikilink autocomplete | ✅ | CompletionSource triggered on `[[` with vault note titles. |

**Exit criteria:** ✅ Met — desktop app with file explorer, CodeMirror editor, tabs, split panes, command palette, terminal, and knowledge graph.

---

### Phase 3: Integrated Terminal — ✅ SHIPPED as part of v1.2 (2026-04-04)

Integrated into Phase 2 desktop delivery. See above.

---

### Phase 4: AI Integration — ✅ SHIPPED as v1.3 (2026-04-04)

**Goal:** Ship AI features that leverage the MCP server architecture.

| Deliverable | Status | Actual outcome |
|---|---|---|
| MCP server (HTTP) | ✅ | `rmcp` streamable HTTP transport via axum. `--transport http --port 3001 --host 127.0.0.1` CLI flags. Localhost-only by default. |
| AI provider abstraction | ✅ | `AiProvider` supporting Anthropic Messages API, OpenAI Chat Completions, and Ollama. Configurable via AI panel. |
| Smart linking | ✅ | `smartLinks.ts` CodeMirror ViewPlugin: debounced (2s) AI analysis of current paragraph, shows clickable `[[wikilink]]` chip suggestions. |
| Note synthesis | ✅ | `ai_synthesize_notes` IPC: gathers content from open tabs, AI generates connected summary with wikilinks. "Save as note" button. |
| Semantic search | ✅ | `EmbeddingIndex`: OpenAI/Ollama embeddings, cosine similarity search, JSON-backed vector store at `.vault/embeddings.json`. |
| Auto-tagging | ✅ | `ai_auto_tag` IPC: AI suggests 1-5 tags based on content and existing vault taxonomy. |
| In-editor AI | ✅ | AI panel: Rewrite, Summarize, Expand, Explain operations on selected text. "Insert into editor" replaces selection. |

**Exit criteria:** ✅ Met — AI features are vault-aware with multi-provider support (Anthropic/OpenAI/Ollama).

---

### Phase 5: Plugin System & Ecosystem — ✅ SHIPPED as v1.4 (2026-04-04)

**Goal:** Ship the WASM-based plugin system and initial ecosystem tooling.

| Deliverable | Status | Actual outcome |
|---|---|---|
| WASM runtime | ✅ | `wasmtime` 43 + `wasmtime-wasi`. Sandboxed execution with WASI p1 context (no filesystem, no stderr). |
| Plugin API | ✅ | Host functions gated at link time by declared permissions (`VaultRead`, `VaultWrite`, `Search`, `Network`). `vault_note_count` as initial host function. |
| Plugin manager UI | ✅ | `PluginManager.svelte`: discover, list, activate plugins with permission badges. Togglable via command palette. |
| Plugin scaffold CLI | ✅ | `rsd plugin init <name>`: generates `plugin.json` manifest, `Cargo.toml` (cdylib), `src/lib.rs` (on_activate/on_deactivate + host imports), `README.md`. |
| Plugin manifest | ✅ | `PluginManifest`: id, name, version, description, author, wasm_file, permissions. JSON-based. |

**Exit criteria:** ✅ Met — developers can scaffold, build (wasm32-wasip1), and activate WASM plugins with permission-gated host functions.

---

### Phase 6: Canvas & Bases — ✅ SHIPPED as v1.5 (2026-04-04)

**Goal:** Ship Canvas (visual whiteboard) and Bases (database views).

| Deliverable | Status | Actual outcome |
|---|---|---|
| JSON Canvas parser | ✅ | `Canvas` struct with spec-compliant field names (`fromNode`, `toNode`, `type`). Load/save `.canvas` JSON files. |
| Canvas UI | ✅ | SVG-based infinite canvas with pan (drag), zoom (scroll wheel), text/file nodes, edges, drag-to-move, delete. |
| Bases: data model | ✅ | `BaseView` with Table/List/Card view types. `BaseSource` (vault/tag/folder), `BaseColumn`, `BaseFilter`, `BaseSort`. |
| Bases: UI | ✅ | Three switchable views (table/list/card), filter input, sort by title/date, click to open note. Card view uses responsive CSS grid. |

**Exit criteria:** ✅ Met — canvas and database views functional with core operations.

---

### Phase 7: Sync & Collaboration — ✅ SHIPPED as v1.6 (2026-04-04)

**Goal:** Ship cross-device sync foundation.

| Deliverable | Status | Actual outcome |
|---|---|---|
| Sync protocol | ✅ | `SyncEngine` with change tracking (Create/Update/Delete/Rename), push/pull via HTTP, device ID, timestamp ordering. |
| Auth | ✅ | Bearer token authentication via standard `Authorization` header. |
| E2E encryption | ⏳ | Deferred — auth token mechanism in place, actual payload encryption not yet implemented. |
| Real-time collaboration | ⏳ | Deferred — requires CRDT integration (automerge) and WebSocket presence. |

**Exit criteria:** Partially met — sync protocol and change tracking operational. E2E encryption and real-time collaboration deferred to future iteration.

---

### Phase 8: Mobile — 🚧 CONFIG READY (2026-04-04)

**Goal:** Ship iOS and Android apps.

| Deliverable | Status | Actual outcome |
|---|---|---|
| Tauri 2 mobile entry point | ✅ | `#[cfg_attr(mobile, tauri::mobile_entry_point)]` on `run()`. iOS minimum 15.0 in tauri.conf.json. |
| Platform init | ⏳ | Requires `cargo tauri ios init` / `cargo tauri android init` on machines with Xcode / Android SDK. |
| Mobile-specific UI | ⏳ | Touch-optimized toolbar and responsive layout not yet implemented. |

**Exit criteria:** Not yet met — codebase is mobile-ready architecturally; platform-specific build setup requires target hardware.

---

## Quality Gates (Apply to Every Phase)

| Gate | Criteria |
|---|---|
| **Test coverage** | ≥80% line coverage on `rustsidian-core`. Property-based tests for parser, link resolver, and sync. |
| **Performance** | Vault open < 500ms for 10k notes. Search < 50ms. File save → index update < 100ms. |
| **Accessibility** | WCAG 2.1 AA for desktop UI. Screen reader compatibility. Keyboard-navigable. |
| **Security** | No plaintext credential storage. Plugin sandboxing enforced. Sync E2E encryption verified by independent review. |
| **Documentation** | API docs generated from code (`rustdoc`). User guide for each shipped feature. Plugin developer guide by Phase 5. |
| **CI/CD** | All PRs gated on: `cargo test`, `cargo clippy` (deny warnings), `cargo fmt --check`, frontend lint + typecheck. Nightly builds for all platforms. |

---

## Risk Register

| Risk | Impact | Likelihood | Mitigation |
|---|---|---|---|
| **MDX complexity** | Pipeline becomes a maintenance burden; JSX edge cases proliferate | Medium | Limit initial JSX support to a curated component model (no arbitrary imports). Expand gradually. |
| **Tauri 2 mobile maturity** | Mobile builds don't meet native-feel expectations | Medium | Evaluate at Phase 8 kickoff. Fallback: UniFFI + native UI for mobile. |
| **WASM plugin performance** | Plugin overhead makes editor sluggish | Low | Profile early. Use `wasmtime`'s ahead-of-time compilation. Limit plugin API call frequency. |
| **Sync correctness** | Data loss or corruption during sync conflicts | High impact, Low likelihood | Extensive property-based testing. Ship with conservative "file wins" strategy first. |
| **Scope creep** | Trying to match every Obsidian feature delays shipping | High | Ship phases independently. Each phase has a usable product. Users don't need Phase 6 to get value from Phase 2. |
| **CodeMirror + MDX** | Inline MDX component rendering in the editor is technically challenging | Medium | Prototype in Phase 1. If infeasible, fall back to split preview (edit MDX on left, rendered preview on right). |

---

## Open Decisions

These are architectural choices that must be resolved before or during Phase 2 (Desktop).

| Decision | Options | Status | Notes |
|---|---|---|---|
| **Frontend framework** | SvelteKit vs. React vs. Solid | ✅ **Decided: SvelteKit** | Smallest bundle, best Tauri DX; logged in PROJECT.md |
| **Editor engine** | CodeMirror 6 vs. Monaco vs. Prosemirror | ✅ **Decided: CodeMirror 6** | Lighter, more extensible for custom MDX syntax |
| **MDX component runtime** | Svelte components vs. Web Components vs. React | ✅ **Decided: Svelte** | Matches frontend framework; no extra runtime |
| **Graph rendering** | D3.js (SVG) vs. pixi.js (WebGL) vs. Three.js | ✅ **Decided: sigma.js + graphology** | Force-directed layout via ForceAtlas2; handles both full vault and local N-hop views |
| **Sync algorithm** | CRDTs (automerge) vs. OT vs. last-write-wins | ✅ **Decided: LWW + change tracking** | SyncEngine with timestamp-ordered changes; CRDT deferred to future iteration |
| **Plugin sandbox** | WASM (wasmtime) vs. V8 isolates vs. process isolation | ✅ **Decided: WASM (wasmtime 43)** | Permission-gated host functions linked at instantiation time; WASI p1 sandbox |

---

## Success Metrics

### Phase 0–1 (CLI + MDX): Internal Validation — ✅ ACHIEVED (2026-04-04)
- ✅ Can manage a personal vault of 1,100+ notes from the CLI (scale validated)
- ✅ MDX files parse, index, export, and are queryable via MCP — 17 tools total
- ✅ Claude can interact with the vault via MCP and perform useful operations (registered in `.mcp.json`)

### Phase 2–4 (Desktop + Terminal + AI): Alpha — ✅ CODE COMPLETE (2026-04-04)
- ✅ Desktop app with file explorer, CodeMirror editor, tabs, split panes, command palette
- ✅ Integrated PTY terminal (portable-pty + xterm.js) with vault-aware shell
- ✅ AI provider abstraction (Anthropic/OpenAI/Ollama), in-editor AI, smart links, note synthesis, semantic search, auto-tagging
- ✅ MCP HTTP transport (stdio + streamable HTTP via axum)
- ⏳ Pending: 10 daily-driver alpha users (requires human testing and distribution)

### Phase 5–6 (Plugins + Canvas/Bases): Beta — ✅ CODE COMPLETE (2026-04-04)
- ✅ WASM plugin system (wasmtime 43) with permission-gated host functions and scaffold CLI
- ✅ JSON Canvas parser + SVG canvas UI; Bases with table/list/card views
- ⏳ Pending: third-party plugins, Obsidian vault migration tool

### Phase 7–8 (Sync + Mobile): 1.0 — 🚧 PARTIAL (2026-04-04)
- ✅ Sync engine with change tracking and push/pull protocol
- ✅ Mobile entry point configured (Tauri 2 `#[cfg_attr(mobile)]`)
- ⏳ Pending: E2E encryption (AES-256-GCM + Argon2id), real-time collaboration (CRDT + WebSocket), mobile platform init (Xcode/Android SDK required)
