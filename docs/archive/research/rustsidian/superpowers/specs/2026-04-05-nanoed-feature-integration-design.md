> **Archived 2026-09-08** — Imported verbatim from the private `baileyrd/Rustsidian` repo (`docs/`) under RFC 0009 row 6. Rustsidian is retired; this is reference material only and describes Rustsidian's SvelteKit/Tauri stack, not Nexus.

# NanOEd Feature Integration into Rustsidian — Design Spec

**Date:** 2026-04-05
**Status:** Approved
**Scope:** Frontend replacement + 6 feature additions across 3 implementation groups

## Overview

Replace Rustsidian's SvelteKit frontend with NanOEd's Solid.js frontend, rewired to Rustsidian's Rust backend. Add KaTeX math rendering, Mermaid diagram rendering, hover previews, rename-with-link-update, and a Rust-native RAG pipeline with LanceDB.

The Rust backend (rustsidian-core, rustsidian-cli, rustsidian-mcp) remains the authoritative codebase. The frontend is adapted to conform to Rustsidian's existing IPC contract, with targeted additions to fill gaps.

## Implementation Groups

**Group A: Frontend Swap + Rendering** (do first)
- Replace SvelteKit with NanOEd's Solid.js UI
- Rewrite IPC bridge for Rustsidian commands
- Add KaTeX and Mermaid in-editor rendering

**Group B: Editor UX** (builds on Group A)
- Hover previews on wikilinks
- Rename with automatic link update

**Group C: RAG Pipeline** (largest, new crate)
- New `rustsidian-rag` crate with LanceDB
- Chunking, vector search, RAG orchestration
- Frontend AI panel rewired to Rust IPC

---

## Group A: Frontend Swap + Rendering

### A1: Frontend Replacement

Replace `frontend/` directory contents. Switch from SvelteKit/Svelte 5 to Solid.js with Vite.

**What to keep from NanOEd's frontend:**
- `shell/` — App.tsx, Sidebar.tsx, FileTree.tsx, TabBar.tsx, StatusBar.tsx, CommandPalette.tsx, PropertiesPanel.tsx, SettingsPanel.tsx, OutlinePanel.tsx, BacklinksPanel.tsx, Notifications.tsx
- `editor/` — EditorPane.tsx, LiveEditor.tsx, MdxWysiwyg.tsx, mdx-language.ts, completions.ts, wikilink-decoration.ts, focus-mode.ts, markdown-decorations.ts
- `components/` — GraphView.tsx, TableView.tsx, Canvas.tsx, TerminalPanel.tsx, AiPanel.tsx, ResizeHandle.tsx
- `store/` — app-store.ts
- `lib/` — events.ts (rewrite), tauri.ts (rewrite), ai.ts (rewrite)
- `styles/` — global.css, themes/

**What to remove from NanOEd's frontend:**
- `renderer/MdxRenderer.tsx` — Rustsidian doesn't compile MDX to JS modules
- `renderer/MdxPreview.tsx` — replaced by in-editor rendering
- `renderer/mdx-compiler.ts` — not applicable
- `renderer/hydrate-blocks.ts` — replaced by CM6 widget extensions
- `renderer/safe-html.ts` — not needed without HTML preview

**Package.json changes:**
- Remove: `@mdx-js/mdx`, `@mdx-js/rollup`, `@mdxeditor/editor`, `marked`, `react`, `react-dom`
- Keep: `solid-js`, `vite-plugin-solid`, all `@codemirror/*`, `@xterm/*`, `d3-force`, `katex`, `mermaid`, `@tauri-apps/api`

**Vite config changes:**
- Remove `@mdx-js/rollup` plugin
- Keep `vite-plugin-solid`, Tauri HMR config, base `"./"` for Tauri custom protocol

**Store changes:**
- Rename localStorage keys from `vaultmdx-*` to `rustsidian-*`

**App.tsx view mode changes:**
- Keep: `"editor"`, `"live"`, `"graph"`, `"table"`, `"canvas"`
- Defer: `"preview"`, `"split"` (these relied on MdxPreview which is removed; can be re-added later with a markdown HTML renderer)

### A2: IPC Bridge Rewrite

Rewrite `frontend/src/lib/tauri.ts` to call Rustsidian's Tauri commands. Use Rustsidian's auto-generated TypeScript bindings from specta (`frontend/src/lib/bindings.ts`).

**Command mapping:**

| NanOEd Call | Rustsidian Command | Adaptation |
|---|---|---|
| `listNotes()` | `list_notes` | Map `NoteListItem` → `NoteDto` shape |
| `openNote(path)` | `get_note` | Extract `.content` from `NoteContent` |
| `saveNote(path, content)` | `update_note` | Direct |
| `createNote(path, content?)` | `create_note` | Derive title from path stem |
| `deleteNote(path)` | `delete_note_cmd` | Direct |
| `renameNote(old, new)` | `rename_note` | Add `new_title` derived from new path |
| `listDirectory(path)` | `list_folder_tree` | Filter recursive tree to requested path |
| `searchNotes(query, limit)` | `search_notes` | Direct |
| `getGraph()` | `get_graph` | Direct |
| `getLocalGraph(path, depth)` | `get_local_graph` | Rename `depth` → `hops` |
| `listPlugins()` | `list_plugins` | Direct |
| `terminalCreate(rows, cols)` | `spawn_terminal` | Ignore return ID (single terminal) |
| `terminalWrite(id, data)` | `write_terminal` | Drop `id` param |
| `terminalResize(id, rows, cols)` | `resize_terminal` | Drop `id` param |
| `getConfig()` | `get_ai_config` | Partial — AI config only for now |
| `configureAi(config)` | `configure_ai` | Direct |
| AI commands | `ai_*` (8 commands) | Remap names to Rustsidian's conventions |

**New Tauri commands to add** (in `crates/rustsidian-desktop/src/commands/vault.rs`):

```rust
#[tauri::command]
#[specta::specta]
pub async fn create_directory(
    path: String,
    state: State<'_, AppState>,
) -> Result<(), String>
```

```rust
#[tauri::command]
#[specta::specta]
pub async fn get_backlinks(
    path: String,
    state: State<'_, AppState>,
) -> Result<Vec<String>, String>
```

```rust
#[tauri::command]
#[specta::specta]
pub async fn get_backlinks_with_context(
    path: String,
    state: State<'_, AppState>,
) -> Result<Vec<BacklinkContextItem>, String>
```

```rust
#[tauri::command]
#[specta::specta]
pub async fn get_vault_stats(
    state: State<'_, AppState>,
) -> Result<VaultStats, String>
```

```rust
#[tauri::command]
#[specta::specta]
pub async fn open_daily_note(
    state: State<'_, AppState>,
) -> Result<NoteContent, String>
```

**New IPC types** (in `ipc_types.rs`):

```rust
pub struct BacklinkContextItem {
    pub source_path: String,
    pub source_title: String,
    pub context: String,  // ~100 chars surrounding the link
}

pub struct VaultStats {
    pub note_count: u64,
    pub link_count: u64,
    pub tag_count: u64,
    pub component_count: u64,
}
```

**Deferred commands** — stub in IPC bridge with defaults:

| NanOEd Call | Stub Return |
|---|---|
| `compileNote(path)` | `{ js_output: "", html: "", mdx_source: "", frontmatter: {} }` |
| `getNoteHistory(path)` | `[]` |
| `getNoteAtVersion(path, hash)` | `""` |
| `getNoteWordCount(path)` | Compute client-side from content string |
| `terminalClose(id)` | No-op |
| `listTemplates()` | `[]` |
| `createNoteFromTemplate(...)` | No-op |
| `exportNoteHtml(path)` | `""` (core has this logic, wire up later) |
| `getSyncStatus()` | `"Disabled"` |
| `syncCommit/Pull/Push()` | No-op |
| `readComponentSource(name)` | `""` |
| `listComponentNames()` | `[]` |

**Event mapping:**

| NanOEd Event | Rustsidian Event |
|---|---|
| `"vault-event"` | `"vault-changed"` — adapt payload shape |
| `"terminal-output"` | `"terminal-output"` — direct match |
| `"terminal-closed"` | `"terminal-exit"` — rename listener |

**Response wrapper adaptation:**

NanOEd expects raw values. Rustsidian returns `{status: "ok", data: T} | {status: "error", error: string}`. The IPC bridge unwraps:

```typescript
async function invoke<T>(cmd: string, args?: Record<string, unknown>): Promise<T> {
  const result = await bindings.commands[cmd](args);
  if (result.status === "error") throw new Error(result.error);
  return result.data as T;
}
```

### A3: KaTeX In-Editor Rendering

**New file:** `frontend/src/editor/mathWidget.ts`

CodeMirror 6 `ViewPlugin` that:
1. Scans document for `<Math expr="..."/>` patterns via regex
2. Uses `Decoration.replace()` to swap JSX syntax with rendered KaTeX HTML
3. When cursor enters the decorated range, collapses back to raw text for editing
4. Renders synchronously (KaTeX is fast enough for inline)
5. Display mode for multi-line blocks, inline for single-line
6. Error state shows raw expression with red border

**Regex pattern:** `<Math\s+expr="([^"]*)"(?:\s*\/)?\s*>`

**Registration:** Add to editor extension array in EditorPane.tsx alongside existing extensions.

**Dependency:** `katex` (already in NanOEd's package.json)

### A4: Mermaid In-Editor Rendering

**New file:** `frontend/src/editor/mermaidWidget.ts`

CodeMirror 6 `ViewPlugin` that:
1. Scans document for `<Mermaid chart="..."/>` patterns via regex
2. Uses `Decoration.replace()` to swap JSX with rendered Mermaid SVG
3. Caches rendered SVGs by content hash to avoid re-rendering on every keystroke
4. Debounces rendering by 200ms after edits inside a Mermaid node
5. Theme colors from CSS variables to match Rustsidian's theme
6. When cursor enters range, collapses to raw text

**Regex pattern:** `<Mermaid\s+chart="([^"]*)"(?:\s*\/)?\s*>`

Note: Mermaid chart props may contain escaped newlines (`\n`). The regex captures the full prop value; the widget unescapes before rendering.

**Registration:** Add to editor extension array alongside mathWidget.

**Dependency:** `mermaid` (already in NanOEd's package.json)

---

## Group B: Editor UX

### B1: Hover Previews

**New file:** `frontend/src/editor/hoverPreview.ts`

CodeMirror 6 `hoverTooltip` extension:
- Regex matches `[[target]]` or `[[target|alias]]` under cursor position
- Calls new `note_preview` Tauri command via IPC
- LRU cache (64 entries) to avoid repeated IPC calls
- 300ms hover delay before triggering
- Max width 400px, styled with CSS variables

**Tooltip content (structured summary):**
- Title (bold)
- Tags (as colored pills)
- First paragraph (plain text, ~200 chars)
- Backlink count

**New Tauri command** (in `commands/vault.rs`):

```rust
#[tauri::command]
#[specta::specta]
pub async fn note_preview(
    path: String,
    state: State<'_, AppState>,
) -> Result<NotePreview, String>
```

Implementation:
1. Resolve wikilink target via `resolver::resolve_wikilink(target, vault_paths)`
2. Fetch note from vault
3. Extract tags from parsed note
4. Grab first paragraph (strip frontmatter, take text before first blank line)
5. Query `graph.backlinks(path).len()` for count

**New IPC type** (in `ipc_types.rs`):

```rust
pub struct NotePreview {
    pub title: String,
    pub tags: Vec<String>,
    pub first_paragraph: String,
    pub backlink_count: usize,
}
```

### B2: Rename with Automatic Link Update

**Modify:** `crates/rustsidian-core/src/vault/mod.rs` — `Vault::rename_note()`

After the existing rename logic (file move + DB update), add:

1. Query `graph.backlinks(old_path)` for all notes linking to the renamed note
2. For each backlink source note:
   - Read content
   - Regex replace: `(\[\[|\!\[\[)old_stem(\|[^\]]+)?\]\]` (case-insensitive)
   - Replace `old_stem` with `new_stem` (filename without extension)
   - Preserve existing aliases in `[[target|alias]]` form
   - Write updated content via `update_note_content()`
3. Rebuild links for affected notes in graph
4. Return list of updated note paths

**Modify return type** of `rename_note` Tauri command:

```rust
pub struct RenameResult {
    pub new_path: String,
    pub updated_notes: Vec<String>,
}
```

**Frontend handling:**
- On receiving `RenameResult`, refresh content for any `updated_notes` open in tabs
- File explorer refreshes via existing `vault-changed` event

---

## Group C: Rust-Native RAG Pipeline

### C1: New Crate — `rustsidian-rag`

**Location:** `crates/rustsidian-rag/`

**Cargo.toml dependencies:**
- `rustsidian-core` (workspace)
- `lancedb` — embedded vector database
- `arrow-array`, `arrow-schema` — Arrow types for LanceDB
- `tokio` (workspace)
- `serde`, `serde_json` (workspace)
- `anyhow`, `thiserror` (workspace)
- `tracing` (workspace)
- `uuid` (workspace)

**Module structure:**

```
src/
├── lib.rs          // Public API: RagEngine
├── chunker.rs      // Note → Vec<Chunk> splitting
├── store.rs        // LanceDB wrapper
├── pipeline.rs     // RAG orchestration
├── types.rs        // RagQuery, RagResult, Chunk, ChunkMetadata, ChunkMatch
└── error.rs        // RagError enum
```

### C2: Chunker

**File:** `crates/rustsidian-rag/src/chunker.rs`

Splits a note into semantic chunks by heading boundaries (H1–H6).

```rust
pub struct Chunk {
    pub note_path: String,
    pub heading: String,       // Heading text, or "Introduction" for pre-heading content
    pub chunk_index: u32,
    pub text: String,
    pub char_range: (usize, usize),
}

pub fn chunk_note(path: &str, content: &str) -> Vec<Chunk>
```

**Behavior:**
- Strip frontmatter before chunking
- Split at heading boundaries (lines starting with `#`)
- Content before first heading becomes chunk with heading "Introduction"
- If no headings, chunk by paragraph with max ~500 tokens per chunk
- Each chunk carries its source note path and heading for attribution

### C3: LanceDB Vector Store

**File:** `crates/rustsidian-rag/src/store.rs`

Wraps LanceDB with storage at `<vault-root>/.vault/lance/`.

```rust
pub struct VectorStore {
    db: lancedb::Connection,
    table: lancedb::Table,
    embedding_dim: usize,
}
```

**Schema:** `id` (Utf8), `note_path` (Utf8), `heading` (Utf8), `chunk_index` (UInt32), `text` (Utf8), `vector` (FixedSizeList[Float32])

**Operations:**
- `open(vault_root: &Path, embedding_dim: usize) -> Result<Self>` — open or create DB + table
- `index_note(path: &str, chunks: &[Chunk], embeddings: &[Vec<f32>]) -> Result<()>` — delete existing chunks for path, insert new
- `remove_note(path: &str) -> Result<()>` — delete all chunks for a note
- `search(query_embedding: &[f32], limit: usize) -> Result<Vec<ChunkMatch>>` — cosine similarity search
- `stats() -> Result<StoreStats>` — total chunks, unique note paths, approximate size

### C4: RAG Pipeline

**File:** `crates/rustsidian-rag/src/pipeline.rs`

Orchestrates the full RAG flow.

```rust
pub struct RagEngine {
    store: VectorStore,
    ai: Arc<Mutex<AiProvider>>,  // From rustsidian-core
}
```

**Operations:**

`index_vault(vault: &Vault) -> Result<IndexResult>`:
1. Iterate all notes in vault
2. Chunk each note via `chunker::chunk_note()`
3. Batch embed chunks via `AiProvider::embed()`
4. Upsert into LanceDB via `store.index_note()`
5. Return `IndexResult { notes_indexed, chunks_created }`

`index_note(path: &str, content: &str) -> Result<()>`:
- Incremental: re-index a single note after edit
- Chunk → embed → upsert

`query(question: &str, n_context: usize) -> Result<RagResult>`:
1. Embed question via `AiProvider::embed(question)`
2. Vector search: `store.search(embedding, n_context)`
3. Assemble context prompt with chunk texts and source attribution
4. Call `AiProvider::chat()` with system prompt instructing citation of sources
5. Return `RagResult { answer, sources, model }`

**System prompt for RAG generation:**
```
You are a knowledge assistant for a personal vault. Answer the question using ONLY
the provided context. Cite sources using [[Note Name]] wikilink syntax. If the
context doesn't contain enough information, say so.
```

### C5: Types

**File:** `crates/rustsidian-rag/src/types.rs`

```rust
pub struct RagResult {
    pub answer: String,
    pub sources: Vec<RagSource>,
    pub model: String,
}

pub struct RagSource {
    pub note_path: String,
    pub heading: String,
    pub score: f32,
    pub chunk_text: String,
}

pub struct IndexResult {
    pub notes_indexed: u32,
    pub chunks_created: u32,
}

pub struct StoreStats {
    pub total_chunks: u64,
    pub total_notes: u64,
}

pub struct ChunkMatch {
    pub note_path: String,
    pub heading: String,
    pub chunk_index: u32,
    pub text: String,
    pub score: f32,
}
```

### C6: Desktop Integration

**New file:** `crates/rustsidian-desktop/src/commands/rag.rs`

```rust
#[tauri::command]
#[specta::specta]
pub async fn rag_index_vault(state: State<'_, AppState>) -> Result<IndexResult, String>

#[tauri::command]
#[specta::specta]
pub async fn rag_index_note(path: String, state: State<'_, AppState>) -> Result<(), String>

#[tauri::command]
#[specta::specta]
pub async fn rag_query(
    question: String,
    n_context: u32,
    state: State<'_, AppState>,
) -> Result<RagResult, String>

#[tauri::command]
#[specta::specta]
pub async fn rag_stats(state: State<'_, AppState>) -> Result<StoreStats, String>
```

**AppState extension:**
- Add `rag_engine: Mutex<Option<RagEngine>>` to `AppState`
- Initialize when vault opens if AI is configured (embedding provider available)

**File watcher hook:**
- In the existing vault watcher callback, after syncing a changed file to DB, also call `rag_engine.index_note(path, content)` to keep the vector index current

### C7: MCP Integration

Add RAG tools to `crates/rustsidian-mcp/src/server.rs`:

```rust
#[tool(description = "Query the vault using RAG (semantic search + AI generation)")]
async fn rag_query(&self, Parameters(params): Parameters<RagQueryParams>) -> Result<CallToolResult, ErrorData>

#[tool(description = "Get RAG index statistics")]
async fn rag_stats(&self, Parameters(params): Parameters<EmptyParams>) -> Result<CallToolResult, ErrorData>
```

### C8: Frontend AI Panel Rewire

NanOEd's `AiPanel.tsx` currently calls Python sidecar endpoints via `lib/ai.ts`. Rewrite `ai.ts` to call Rustsidian's Tauri commands:

```typescript
// Before (NanOEd — Python sidecar)
export async function aiAsk(question: string, nContext?: number): Promise<RagResult> {
  return fetch(`http://127.0.0.1:9720/rag/query`, { body: { question, n_context } });
}

// After (Rustsidian — Rust IPC)
export async function aiAsk(question: string, nContext: number = 5): Promise<RagResult> {
  return invoke<RagResult>("rag_query", { question, nContext });
}
```

Similar rewrites for `aiIndex()` → `rag_index_vault`, `aiStats()` → `rag_stats`.

---

## Dependencies Summary

**New Rust workspace dependencies:**
- `lancedb` — embedded vector database
- `arrow-array` — Arrow array types
- `arrow-schema` — Arrow schema definitions

**Frontend dependencies (from NanOEd, already present):**
- `katex` — LaTeX rendering
- `mermaid` — diagram rendering
- `solid-js` — UI framework (replaces Svelte)
- `d3-force` — graph layout
- `@xterm/xterm` — terminal
- All `@codemirror/*` packages

**Frontend dependencies removed:**
- `@mdx-js/mdx`, `@mdx-js/rollup` — MDX compilation (not needed)
- `@mdxeditor/editor` — rich MDX editor (not needed)
- `marked` — markdown parser (not needed without preview)
- `react`, `react-dom` — React (not needed)

## File Change Summary

| Action | Location | Description |
|---|---|---|
| **Replace** | `frontend/` | SvelteKit → NanOEd's Solid.js |
| **Rewrite** | `frontend/src/lib/tauri.ts` | IPC bridge for Rustsidian commands |
| **Rewrite** | `frontend/src/lib/events.ts` | Event listeners for Rustsidian events |
| **Rewrite** | `frontend/src/lib/ai.ts` | RAG calls via Tauri IPC |
| **New** | `frontend/src/editor/mathWidget.ts` | KaTeX CM6 widget |
| **New** | `frontend/src/editor/mermaidWidget.ts` | Mermaid CM6 widget |
| **New** | `frontend/src/editor/hoverPreview.ts` | Wikilink hover tooltip |
| **Remove** | `frontend/src/renderer/*` | MDX preview/compile (not needed) |
| **New** | `crates/rustsidian-rag/` | Entire new crate |
| **Modify** | `crates/rustsidian-core/src/vault/mod.rs` | Rename link update logic |
| **Modify** | `crates/rustsidian-desktop/src/commands/vault.rs` | 5 new commands + rename return type |
| **New** | `crates/rustsidian-desktop/src/commands/rag.rs` | 4 RAG commands |
| **Modify** | `crates/rustsidian-desktop/src/ipc_types.rs` | New DTOs |
| **Modify** | `crates/rustsidian-desktop/src/state.rs` | Add RagEngine to AppState |
| **Modify** | `crates/rustsidian-desktop/src/lib.rs` | Register new commands |
| **Modify** | `crates/rustsidian-mcp/src/server.rs` | Add RAG tools |
| **Modify** | `Cargo.toml` (workspace) | Add rustsidian-rag member + deps |

## Storage Layout

```
<vault-root>/
├── .vault/
│   ├── vault.db              # SQLite (notes, links, tags, tasks, properties)
│   ├── search-index/         # tantivy (full-text search)
│   ├── lance/                # NEW: LanceDB (vector embeddings)
│   └── config.toml            # Vault configuration
├── notes/
│   └── *.md, *.mdx
└── .plugins/
    └── <plugin-id>/
```
