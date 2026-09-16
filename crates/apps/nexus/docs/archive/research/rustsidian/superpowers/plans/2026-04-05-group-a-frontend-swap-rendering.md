> **Archived 2026-09-08** — Imported verbatim from the private `baileyrd/Rustsidian` repo (`docs/`) under RFC 0009 row 6. Rustsidian is retired; this is reference material only and describes Rustsidian's SvelteKit/Tauri stack, not Nexus.

# Group A: Frontend Swap + Rendering — Implementation Plan

> **For agentic workers:** REQUIRED SUB-SKILL: Use superpowers:subagent-driven-development (recommended) or superpowers:executing-plans to implement this plan task-by-task. Steps use checkbox (`- [ ]`) syntax for tracking.

**Goal:** Replace Rustsidian's SvelteKit frontend with NanOEd's Solid.js frontend, rewire the IPC bridge to Rustsidian's Rust backend, add 5 new Tauri commands, and add KaTeX/Mermaid in-editor rendering.

**Architecture:** Copy NanOEd's `frontend/` directory, delete Rustsidian's old `frontend/`, rewrite `lib/tauri.ts` to call Rustsidian's Tauri commands via specta-generated bindings, stub deferred commands, add new backend commands for gaps, then add CodeMirror 6 widget extensions for Math/Mermaid JSX rendering.

**Tech Stack:** Solid.js 1.9, Vite 6, TypeScript 5.7, CodeMirror 6, KaTeX 0.16, Mermaid 11, Tauri 2, specta (auto-generated TS bindings)

**Spec:** `docs/superpowers/specs/2026-04-05-nanoed-feature-integration-design.md` — Sections A1–A4

---

## File Map

### Files to create/copy
- `frontend/` — entire directory from NanOEd (then modify specific files)
- `frontend/src/lib/tauri.ts` — full rewrite (IPC bridge)
- `frontend/src/lib/events.ts` — full rewrite (event listeners)
- `frontend/src/lib/ai.ts` — full rewrite (RAG IPC)
- `frontend/src/editor/mathWidget.ts` — new (KaTeX CM6 extension)
- `frontend/src/editor/mermaidWidget.ts` — new (Mermaid CM6 extension)

### Files to modify in Rust backend
- `crates/rustsidian-desktop/src/commands/vault.rs` — add 5 new commands
- `crates/rustsidian-desktop/src/commands/mod.rs:1-7` — no changes needed (vault module already registered)
- `crates/rustsidian-desktop/src/ipc_types.rs` — add 3 new DTOs
- `crates/rustsidian-desktop/src/lib.rs:14-48` — register 5 new commands in collect_commands!
- `crates/rustsidian-desktop/src/state.rs` — no changes needed for Group A

### Files to delete
- `frontend/src/renderer/` — entire directory (MdxRenderer, MdxPreview, mdx-compiler, hydrate-blocks, safe-html)

---

### Task 1: Copy NanOEd Frontend and Clean Up

**Files:**
- Delete: `frontend/` (entire Rustsidian SvelteKit frontend)
- Copy: `/home/baileyrd/projects/NanOEd/frontend/` → `frontend/`
- Delete: `frontend/src/renderer/` (not needed — no MDX-to-JS compilation)

- [ ] **Step 1: Remove old SvelteKit frontend**

```bash
rm -rf frontend
```

- [ ] **Step 2: Copy NanOEd's Solid.js frontend**

```bash
cp -r /home/baileyrd/projects/NanOEd/frontend frontend
```

- [ ] **Step 3: Remove renderer directory (not needed)**

```bash
rm -rf frontend/src/renderer
```

- [ ] **Step 4: Update index.html title**

Edit `frontend/index.html` — change title from "VaultMDX" to "Rustsidian":

```html
  <title>Rustsidian</title>
```

- [ ] **Step 5: Clean package.json — remove unneeded dependencies**

Edit `frontend/package.json` — remove these from `dependencies`:

```bash
cd frontend
npm uninstall @mdx-js/mdx @mdx-js/rollup @mdxeditor/editor marked react react-dom @types/react @types/react-dom
```

- [ ] **Step 6: Install dependencies**

```bash
cd frontend && npm install
```

- [ ] **Step 7: Commit**

```bash
git add -A
git commit -m "feat: replace SvelteKit frontend with NanOEd Solid.js frontend

Copy NanOEd's Solid.js frontend, remove renderer/ (no MDX compilation),
strip unneeded deps (mdx, react, marked)."
```

---

### Task 2: Add New IPC Types to Rust Backend

**Files:**
- Modify: `crates/rustsidian-desktop/src/ipc_types.rs`

- [ ] **Step 1: Add BacklinkContextItem, VaultStats, and NotePreview types**

Append to the end of `crates/rustsidian-desktop/src/ipc_types.rs` (after `AiConfigPayload`):

```rust
/// Backlink with surrounding context for the backlinks panel.
#[derive(Serialize, Deserialize, Type, Debug)]
pub struct BacklinkContextItem {
    pub source_path: String,
    pub source_title: String,
    pub context: String,
}

/// Aggregate vault statistics.
#[derive(Serialize, Deserialize, Type, Debug)]
pub struct VaultStats {
    pub note_count: u64,
    pub link_count: u64,
    pub tag_count: u64,
    pub component_count: u64,
}

/// Lean preview for hover tooltips — title, tags, first paragraph, backlink count.
#[derive(Serialize, Deserialize, Type, Debug)]
pub struct NotePreview {
    pub title: String,
    pub tags: Vec<String>,
    pub first_paragraph: String,
    pub backlink_count: usize,
}
```

- [ ] **Step 2: Verify it compiles**

```bash
cd crates/rustsidian-desktop && cargo check
```

Expected: compiles with no errors (types are defined but not yet used)

- [ ] **Step 3: Commit**

```bash
git add crates/rustsidian-desktop/src/ipc_types.rs
git commit -m "feat: add BacklinkContextItem, VaultStats, NotePreview IPC types"
```

---

### Task 3: Add create_directory Command

**Files:**
- Modify: `crates/rustsidian-desktop/src/commands/vault.rs`
- Modify: `crates/rustsidian-desktop/src/lib.rs`

- [ ] **Step 1: Add create_directory command**

Append to `crates/rustsidian-desktop/src/commands/vault.rs` (after `vault_info_from_vault` function, before the closing of the file):

```rust
/// Create a directory inside the vault.
#[tauri::command]
#[specta::specta]
pub async fn create_directory(path: String, state: State<'_, AppState>) -> Result<(), String> {
    let guard = state.vault.lock().map_err(|e| e.to_string())?;
    let vault = guard
        .as_ref()
        .ok_or("No vault open — call pick_vault_folder first")?;
    let full_path = vault.root().join(&path);
    std::fs::create_dir_all(&full_path).map_err(|e| format!("create directory: {e}"))
}
```

- [ ] **Step 2: Register in lib.rs**

In `crates/rustsidian-desktop/src/lib.rs`, add inside the `collect_commands![]` macro after `commands::vault::list_folder_tree,` (line 27):

```rust
        commands::vault::create_directory,
```

- [ ] **Step 3: Verify it compiles**

```bash
cd crates/rustsidian-desktop && cargo check
```

Expected: compiles successfully

- [ ] **Step 4: Commit**

```bash
git add crates/rustsidian-desktop/src/commands/vault.rs crates/rustsidian-desktop/src/lib.rs
git commit -m "feat: add create_directory Tauri command"
```

---

### Task 4: Add get_backlinks and get_backlinks_with_context Commands

**Files:**
- Modify: `crates/rustsidian-desktop/src/commands/vault.rs`
- Modify: `crates/rustsidian-desktop/src/lib.rs`

- [ ] **Step 1: Add import for BacklinkContextItem**

In `crates/rustsidian-desktop/src/commands/vault.rs`, update the first import line (line 1):

```rust
use crate::ipc_types::{BacklinkContextItem, FileTreeNode, NoteContent, NoteId, NoteListItem, VaultInfo};
```

- [ ] **Step 2: Add get_backlinks command**

Append to `crates/rustsidian-desktop/src/commands/vault.rs`:

```rust
/// Get paths of all notes that link to the given note.
/// Uses get_all_edges() (same pattern as graph commands) and filters for incoming links.
#[tauri::command]
#[specta::specta]
pub async fn get_backlinks(path: String, state: State<'_, AppState>) -> Result<Vec<String>, String> {
    let guard = state.vault.lock().map_err(|e| e.to_string())?;
    let vault = guard
        .as_ref()
        .ok_or("No vault open — call pick_vault_folder first")?;
    let all_edges = vault.get_all_edges().map_err(|e| e.to_string())?;
    let backlinks: Vec<String> = all_edges
        .into_iter()
        .filter(|(_src, tgt)| tgt == &path)
        .map(|(src, _tgt)| src)
        .collect();
    Ok(backlinks)
}
```

- [ ] **Step 3: Add get_backlinks_with_context command**

Append to `crates/rustsidian-desktop/src/commands/vault.rs`:

```rust
/// Get backlinks with surrounding context (~100 chars around the link).
#[tauri::command]
#[specta::specta]
pub async fn get_backlinks_with_context(
    path: String,
    state: State<'_, AppState>,
) -> Result<Vec<BacklinkContextItem>, String> {
    let guard = state.vault.lock().map_err(|e| e.to_string())?;
    let vault = guard
        .as_ref()
        .ok_or("No vault open — call pick_vault_folder first")?;

    let all_edges = vault.get_all_edges().map_err(|e| e.to_string())?;
    let backlink_paths: Vec<String> = all_edges
        .into_iter()
        .filter(|(_src, tgt)| tgt == &path)
        .map(|(src, _tgt)| src)
        .collect();

    let target_stem = std::path::Path::new(&path)
        .file_stem()
        .and_then(|s| s.to_str())
        .unwrap_or(&path);

    let mut results = Vec::new();
    for source_path in &backlink_paths {
        let note = vault.get_note_by_path(source_path).map_err(|e| e.to_string())?;
        if let Some(note) = note {
            // Find the wikilink in the content and extract surrounding context
            let lower_content = note.content.to_lowercase();
            let search_target = format!("[[{}", target_stem.to_lowercase());
            let context = if let Some(pos) = lower_content.find(&search_target) {
                let start = pos.saturating_sub(50);
                let end = (pos + 50 + target_stem.len()).min(note.content.len());
                // Align to char boundaries
                let start = note.content.floor_char_boundary(start);
                let end = note.content.ceil_char_boundary(end);
                note.content[start..end].to_string()
            } else {
                String::new()
            };

            results.push(BacklinkContextItem {
                source_path: source_path.clone(),
                source_title: note.title.clone(),
                context,
            });
        }
    }
    Ok(results)
}
```

- [ ] **Step 4: Register both commands in lib.rs**

In `crates/rustsidian-desktop/src/lib.rs`, add inside `collect_commands![]` after the `create_directory` line:

```rust
        commands::vault::get_backlinks,
        commands::vault::get_backlinks_with_context,
```

- [ ] **Step 5: Verify it compiles**

```bash
cd crates/rustsidian-desktop && cargo check
```

Expected: compiles successfully. Uses `vault.get_all_edges()` (same pattern as `commands/graph.rs`).

- [ ] **Step 6: Commit**

```bash
git add crates/rustsidian-desktop/src/commands/vault.rs crates/rustsidian-desktop/src/lib.rs
git commit -m "feat: add get_backlinks and get_backlinks_with_context Tauri commands"
```

---

### Task 5: Add get_vault_stats and open_daily_note Commands

**Files:**
- Modify: `crates/rustsidian-desktop/src/commands/vault.rs`
- Modify: `crates/rustsidian-desktop/src/lib.rs`

- [ ] **Step 1: Add VaultStats import**

In `crates/rustsidian-desktop/src/commands/vault.rs`, update the first import line:

```rust
use crate::ipc_types::{BacklinkContextItem, FileTreeNode, NoteContent, NoteId, NoteListItem, VaultInfo, VaultStats};
```

- [ ] **Step 2: Add get_vault_stats command**

Append to `crates/rustsidian-desktop/src/commands/vault.rs`:

```rust
/// Return aggregate vault statistics.
/// Uses existing public vault methods to avoid accessing private conn field.
#[tauri::command]
#[specta::specta]
pub async fn get_vault_stats(state: State<'_, AppState>) -> Result<VaultStats, String> {
    let guard = state.vault.lock().map_err(|e| e.to_string())?;
    let vault = guard
        .as_ref()
        .ok_or("No vault open — call pick_vault_folder first")?;

    let note_count = vault.list_all_note_paths().map_err(|e| e.to_string())?.len() as u64;
    let link_count = vault.get_all_edges().map_err(|e| e.to_string())?.len() as u64;
    let tag_count = vault.list_all_tags().map_err(|e| e.to_string())?.len() as u64;

    // Component count: parse a sample to check, or default to 0 if no method exists.
    // Vault may not expose a jsx_components query — use 0 as safe default.
    let component_count = 0u64;

    Ok(VaultStats {
        note_count,
        link_count,
        tag_count,
        component_count,
    })
}
```

Note: `vault.list_all_tags()` must exist as a public method. If it doesn't compile, replace with `vault.get_all_tags()` or add a public accessor. Check `queries.rs` for the exact method name. If no tag listing method exists, use `0u64` and add a TODO to wire it up when a `count_tags()` method is added to core.

- [ ] **Step 3: Add open_daily_note command**

Append to `crates/rustsidian-desktop/src/commands/vault.rs`:

```rust
/// Open or create today's daily note. Returns the note content.
#[tauri::command]
#[specta::specta]
pub async fn open_daily_note(state: State<'_, AppState>) -> Result<NoteContent, String> {
    let guard = state.vault.lock().map_err(|e| e.to_string())?;
    let vault = guard
        .as_ref()
        .ok_or("No vault open — call pick_vault_folder first")?;

    let today = chrono::Local::now().format("%Y-%m-%d").to_string();
    let path = format!("daily/{}.md", today);

    // Try to get existing daily note
    if let Some(note) = vault.get_note_by_path(&path).map_err(|e| e.to_string())? {
        return Ok(NoteContent::from(note));
    }

    // Create new daily note
    let content = format!("# {}\n\n", today);
    let note = vault
        .create_note(path, today.clone(), content)
        .map_err(|e| e.to_string())?;
    Ok(NoteContent::from(note))
}
```

- [ ] **Step 4: Register both commands in lib.rs**

In `crates/rustsidian-desktop/src/lib.rs`, add inside `collect_commands![]` after the `get_backlinks_with_context` line:

```rust
        commands::vault::get_vault_stats,
        commands::vault::open_daily_note,
```

- [ ] **Step 5: Verify it compiles**

```bash
cd crates/rustsidian-desktop && cargo check
```

Expected: compiles successfully. Uses `vault.get_all_edges()` and `vault.list_all_tags()` — same pattern as existing graph commands. If `list_all_tags` doesn't exist, set `tag_count = 0` and move on.

- [ ] **Step 6: Commit**

```bash
git add crates/rustsidian-desktop/src/commands/vault.rs crates/rustsidian-desktop/src/lib.rs
git commit -m "feat: add get_vault_stats and open_daily_note Tauri commands"
```

---

### Task 6: Rewrite IPC Bridge (tauri.ts)

**Files:**
- Rewrite: `frontend/src/lib/tauri.ts`

This is the critical integration file. It maps NanOEd's frontend API to Rustsidian's Tauri commands.

- [ ] **Step 1: Write the new tauri.ts**

Replace `frontend/src/lib/tauri.ts` entirely with:

```typescript
/**
 * IPC bridge — maps NanOEd frontend API to Rustsidian's Tauri commands.
 *
 * Rustsidian's commands return {status: "ok", data: T} | {status: "error", error: string}.
 * This bridge unwraps the result and throws on error.
 */

// ── Types matching NanOEd's frontend expectations ──

export interface NoteDto {
  path: string;
  title: string;
  tags: string[];
  word_count: number;
}

export interface SearchResultDto {
  path: string;
  title: string;
  score: number;
}

export interface GraphNodeDto {
  id: string;
  label: string;
  path: string;
}

export interface GraphEdgeDto {
  from: string;
  to: string;
  link_type: string;
}

export interface BacklinkDto {
  source_path: string;
  source_title: string;
  context: string;
}

export interface CompileResultDto {
  js_output: string;
  html: string;
  mdx_source: string;
  frontmatter: Record<string, unknown>;
}

export interface VersionEntry {
  hash: string;
  message: string;
  author: string;
  date: string;
}

export interface TemplateDto {
  name: string;
  content: string;
}

export interface DirectoryListingDto {
  directories: string[];
  files: string[];
}

export interface VaultStatsDto {
  note_count: number;
  link_count: number;
  component_count: number;
  plugin_count: number;
  plugins: string[];
}

export interface PluginDto {
  id: string;
  name: string;
  version: string;
  state: string;
}

export interface NoteQueryParams {
  tag_filter?: string;
  sort_by?: string;
  sort_desc?: boolean;
}

// ── Tauri detection and invoke helper ──

function isTauri(): boolean {
  return "__TAURI_INTERNALS__" in window;
}

async function invoke<T>(cmd: string, args?: Record<string, unknown>): Promise<T> {
  if (!isTauri()) {
    console.warn(`[tauri.ts] Not in Tauri — stubbing ${cmd}`);
    return undefined as unknown as T;
  }
  const { invoke: tauriInvoke } = await import("@tauri-apps/api/core");
  const result = await tauriInvoke<{ status: string; data: T; error?: string }>(cmd, args);
  if (result.status === "error") {
    throw new Error(result.error || `Command ${cmd} failed`);
  }
  return result.data;
}

// ── Note Operations ──

export async function listNotes(): Promise<NoteDto[]> {
  const items = await invoke<Array<{
    id: string; path: string; title: string; updated_at: string;
  }>>("list_notes");
  return (items || []).map((n) => ({
    path: n.path,
    title: n.title,
    tags: [],
    word_count: 0,
  }));
}

export async function openNote(path: string): Promise<string> {
  const note = await invoke<{
    id: string; path: string; title: string; content: string; content_type: string;
  } | null>("get_note", { path });
  return note?.content ?? "";
}

export async function saveNote(path: string, content: string): Promise<void> {
  await invoke<null>("update_note", { path, content });
}

export async function createNote(path: string, content?: string): Promise<void> {
  const stem = path.replace(/^.*\//, "").replace(/\.(md|mdx)$/, "");
  const title = stem || "Untitled";
  await invoke<unknown>("create_note", { path, title });
  if (content) {
    await invoke<null>("update_note", { path, content });
  }
}

export async function deleteNote(path: string): Promise<void> {
  await invoke<null>("delete_note_cmd", { path });
}

export async function renameNote(oldPath: string, newPath: string): Promise<void> {
  const stem = newPath.replace(/^.*\//, "").replace(/\.(md|mdx)$/, "");
  const newTitle = stem || "Untitled";
  await invoke<null>("rename_note", { oldPath, newPath, newTitle });
}

// ── Directory ──

export async function listDirectory(path: string): Promise<DirectoryListingDto> {
  const tree = await invoke<Array<{
    name: string; path: string; is_folder: boolean; children: unknown[];
  }>>("list_folder_tree");

  // Navigate to the requested path in the tree
  const segments = path.split("/").filter(Boolean);
  let nodes = tree || [];
  for (const seg of segments) {
    const found = nodes.find((n: any) => n.name === seg && n.is_folder);
    if (!found) return { directories: [], files: [] };
    nodes = (found as any).children || [];
  }

  return {
    directories: nodes.filter((n: any) => n.is_folder).map((n: any) => n.path),
    files: nodes.filter((n: any) => !n.is_folder).map((n: any) => n.path),
  };
}

export async function createDirectory(path: string): Promise<void> {
  await invoke<null>("create_directory", { path });
}

// ── Search & Graph ──

export async function searchNotes(query: string, limit: number = 20): Promise<SearchResultDto[]> {
  const items = await invoke<Array<{
    path: string; title: string; score: number; snippet: string;
  }>>("search_notes", { query, limit });
  return (items || []).map((r) => ({ path: r.path, title: r.title, score: r.score }));
}

export async function getGraph(): Promise<{ nodes: GraphNodeDto[]; edges: GraphEdgeDto[] }> {
  const data = await invoke<{
    nodes: Array<{ path: string; title: string; link_count: number }>;
    edges: Array<{ source: string; target: string }>;
  }>("get_graph");
  return {
    nodes: (data?.nodes || []).map((n) => ({
      id: n.path,
      label: n.title,
      path: n.path,
    })),
    edges: (data?.edges || []).map((e) => ({
      from: e.source,
      to: e.target,
      link_type: "wikilink",
    })),
  };
}

export async function getLocalGraph(
  path: string,
  depth: number = 2,
): Promise<{ nodes: GraphNodeDto[]; edges: GraphEdgeDto[] }> {
  const data = await invoke<{
    nodes: Array<{ path: string; title: string; link_count: number }>;
    edges: Array<{ source: string; target: string }>;
  }>("get_local_graph", { path, hops: depth });
  return {
    nodes: (data?.nodes || []).map((n) => ({
      id: n.path,
      label: n.title,
      path: n.path,
    })),
    edges: (data?.edges || []).map((e) => ({
      from: e.source,
      to: e.target,
      link_type: "wikilink",
    })),
  };
}

// ── Backlinks ──

export async function getBacklinks(path: string): Promise<string[]> {
  return (await invoke<string[]>("get_backlinks", { path })) || [];
}

export async function getBacklinksWithContext(path: string): Promise<BacklinkDto[]> {
  return (await invoke<BacklinkDto[]>("get_backlinks_with_context", { path })) || [];
}

// ── Query / Filter ──

export async function queryNotes(_params: NoteQueryParams): Promise<NoteDto[]> {
  // Rustsidian doesn't have a dedicated query command yet — fall back to listNotes
  return listNotes();
}

// ── Plugins ──

export async function listPlugins(): Promise<PluginDto[]> {
  const items = await invoke<Array<{
    id: string; name: string; version: string; description: string;
  }>>("list_plugins");
  return (items || []).map((p) => ({
    id: p.id,
    name: p.name,
    version: p.version,
    state: "active",
  }));
}

// ── Terminal ──

export async function terminalCreate(_rows: number, _cols: number): Promise<number> {
  await invoke<null>("spawn_terminal");
  return 0; // Single terminal, no ID needed
}

export async function terminalWrite(_id: number, data: string): Promise<void> {
  await invoke<null>("write_terminal", { data });
}

export async function terminalResize(_id: number, rows: number, cols: number): Promise<void> {
  await invoke<null>("resize_terminal", { rows, cols });
}

// ── Vault Info ──

export async function getVaultStats(): Promise<VaultStatsDto> {
  const stats = await invoke<{
    note_count: number; link_count: number; tag_count: number; component_count: number;
  }>("get_vault_stats");
  return {
    note_count: stats?.note_count ?? 0,
    link_count: stats?.link_count ?? 0,
    component_count: stats?.component_count ?? 0,
    plugin_count: 0,
    plugins: [],
  };
}

export async function getNoteWordCount(path: string): Promise<number> {
  const content = await openNote(path);
  return content.split(/\s+/).filter(Boolean).length;
}

// ── Daily Notes ──

export async function openDailyNote(): Promise<string> {
  const note = await invoke<{
    id: string; path: string; title: string; content: string; content_type: string;
  }>("open_daily_note");
  return note?.path ?? "";
}

// ── Config ──

export async function getConfig(): Promise<Record<string, unknown>> {
  const config = await invoke<{
    provider: string; api_key: string | null; base_url: string | null; model: string; max_tokens: number;
  } | null>("get_ai_config");
  return config ? { ai: config } : {};
}

export async function setConfig(config: Record<string, unknown>): Promise<void> {
  if (config.ai) {
    await invoke<null>("configure_ai", { config: config.ai });
  }
}

// ── Export ──

export async function exportNoteHtml(_path: string): Promise<string> {
  return ""; // Deferred — core has export logic, wire up later
}

// ── Deferred Stubs ──

export async function compileNote(_path: string): Promise<CompileResultDto> {
  return { js_output: "", html: "", mdx_source: "", frontmatter: {} };
}

export async function getNoteHistory(_path: string, _limit?: number): Promise<VersionEntry[]> {
  return [];
}

export async function getNoteAtVersion(_path: string, _hash: string): Promise<string> {
  return "";
}

export async function terminalClose(_id: number): Promise<void> {
  // Deferred
}

export async function listTemplates(): Promise<TemplateDto[]> {
  return [];
}

export async function createNoteFromTemplate(
  _path: string,
  _title: string,
  _templateName: string,
): Promise<void> {
  // Deferred
}

export async function getSyncStatus(): Promise<string> {
  return "Disabled";
}

export async function syncCommit(): Promise<void> {}
export async function syncPull(): Promise<void> {}
export async function syncPush(): Promise<void> {}

export async function readComponentSource(_name: string): Promise<string> {
  return "";
}

export async function listComponentNames(): Promise<string[]> {
  return [];
}
```

- [ ] **Step 2: Verify no TypeScript errors**

```bash
cd frontend && npx tsc --noEmit
```

Expected: May have errors from components importing removed renderer modules — those will be fixed in Task 7.

- [ ] **Step 3: Commit**

```bash
git add frontend/src/lib/tauri.ts
git commit -m "feat: rewrite IPC bridge for Rustsidian backend commands"
```

---

### Task 7: Rewrite Event Listeners (events.ts)

**Files:**
- Rewrite: `frontend/src/lib/events.ts`

- [ ] **Step 1: Write the new events.ts**

Replace `frontend/src/lib/events.ts` entirely with:

```typescript
/**
 * Event bridge — maps Rustsidian's Tauri events to NanOEd's frontend event system.
 *
 * Rustsidian emits:
 *   "vault-changed" → { kind: "created"|"modified"|"deleted", path: string }
 *   "terminal-output" → base64-encoded string
 *   "terminal-exit" → ()
 */

export interface VaultEventPayload {
  type: "created" | "modified" | "deleted" | "renamed";
  path: string;
  oldPath?: string;
}

function isTauri(): boolean {
  return "__TAURI_INTERNALS__" in window;
}

export function onVaultEvent(handler: (event: VaultEventPayload) => void): () => void {
  if (!isTauri()) return () => {};

  let unlisten: (() => void) | null = null;

  import("@tauri-apps/api/event").then(({ listen }) => {
    listen<{ kind: string; path: string }>("vault-changed", (event) => {
      handler({
        type: event.payload.kind as VaultEventPayload["type"],
        path: event.payload.path,
      });
    }).then((fn) => {
      unlisten = fn;
    });
  });

  return () => {
    unlisten?.();
  };
}

export function onTerminalOutput(handler: (data: { id: number; data: string }) => void): () => void {
  if (!isTauri()) return () => {};

  let unlisten: (() => void) | null = null;

  import("@tauri-apps/api/event").then(({ listen }) => {
    listen<string>("terminal-output", (event) => {
      handler({ id: 0, data: event.payload });
    }).then((fn) => {
      unlisten = fn;
    });
  });

  return () => {
    unlisten?.();
  };
}

export function onTerminalClosed(handler: (id: number) => void): () => void {
  if (!isTauri()) return () => {};

  let unlisten: (() => void) | null = null;

  import("@tauri-apps/api/event").then(({ listen }) => {
    listen<void>("terminal-exit", () => {
      handler(0);
    }).then((fn) => {
      unlisten = fn;
    });
  });

  return () => {
    unlisten?.();
  };
}
```

- [ ] **Step 2: Commit**

```bash
git add frontend/src/lib/events.ts
git commit -m "feat: rewrite event listeners for Rustsidian event names"
```

---

### Task 8: Rewrite AI IPC Bridge (ai.ts)

**Files:**
- Rewrite: `frontend/src/lib/ai.ts`

- [ ] **Step 1: Write the new ai.ts**

Replace `frontend/src/lib/ai.ts` entirely with:

```typescript
/**
 * AI/RAG operations — calls Rustsidian's existing AI Tauri commands.
 * RAG commands (rag_*) will be added in Group C; stubbed here for now.
 */

function isTauri(): boolean {
  return "__TAURI_INTERNALS__" in window;
}

async function invoke<T>(cmd: string, args?: Record<string, unknown>): Promise<T> {
  if (!isTauri()) return undefined as unknown as T;
  const { invoke: tauriInvoke } = await import("@tauri-apps/api/core");
  const result = await tauriInvoke<{ status: string; data: T; error?: string }>(cmd, args);
  if (result.status === "error") throw new Error(result.error || `Command ${cmd} failed`);
  return result.data;
}

// ── Types ──

export interface RagSource {
  note_path: string;
  heading: string;
  chunk_text: string;
  score: number;
}

export interface RagResult {
  answer: string;
  sources: RagSource[];
  model: string;
}

export interface RagIndexResult {
  notes_indexed: number;
  chunks_created: number;
}

export interface AiStatus {
  available: boolean;
  provider: string;
  model: string;
}

export interface RagStats {
  total_chunks: number;
  total_notes: number;
}

// ── AI Status ──

export async function aiStart(): Promise<void> {
  // No-op — Rustsidian's AI is always available when configured
}

export async function aiStop(): Promise<void> {
  // No-op
}

export async function aiStatus(): Promise<AiStatus> {
  try {
    const config = await invoke<{
      provider: string; model: string;
    } | null>("get_ai_config");
    if (config) {
      return { available: true, provider: config.provider, model: config.model };
    }
  } catch {
    // AI not configured
  }
  return { available: false, provider: "none", model: "" };
}

// ── RAG (stubbed — implemented in Group C) ──

export async function aiAsk(question: string, nContext: number = 5): Promise<RagResult> {
  // Will call rag_query once Group C is implemented
  // For now, fall back to ai_transform for a basic answer
  try {
    const answer = await invoke<string>("ai_transform", {
      text: question,
      operation: "answer",
      context: null,
    });
    return { answer: answer || "RAG not yet configured.", sources: [], model: "unknown" };
  } catch {
    return { answer: "AI not configured. Set up a provider in settings.", sources: [], model: "" };
  }
}

export async function aiIndex(): Promise<RagIndexResult> {
  // Will call rag_index_vault once Group C is implemented
  return { notes_indexed: 0, chunks_created: 0 };
}

export async function aiStats(): Promise<RagStats> {
  // Will call rag_stats once Group C is implemented
  return { total_chunks: 0, total_notes: 0 };
}
```

- [ ] **Step 2: Commit**

```bash
git add frontend/src/lib/ai.ts
git commit -m "feat: rewrite AI IPC bridge for Rustsidian commands (RAG stubbed for Group C)"
```

---

### Task 9: Fix Frontend Imports and Remove Renderer References

**Files:**
- Modify: various `frontend/src/` files that import from `renderer/`

- [ ] **Step 1: Find all imports referencing the removed renderer directory**

```bash
cd frontend && grep -r "renderer/" src/ --include="*.ts" --include="*.tsx" -l
```

- [ ] **Step 2: For each file found, remove or comment out the renderer imports and any code that depends on them**

The specific fixes depend on what `grep` finds, but common cases:

- `shell/App.tsx` — remove `MdxPreview` import and `"preview"`/`"split"` view mode branches. Replace with a placeholder or remove those view modes.
- `editor/LiveEditor.tsx` — remove `MdxPreview` import if present.
- Any file importing from `renderer/hydrate-blocks` — remove the import and hydration calls.

For each file:
1. Remove the import line
2. Remove or stub any JSX/code that references the imported component
3. If a view mode references `MdxPreview`, replace with a `<div>Preview mode coming soon</div>` placeholder

- [ ] **Step 3: Update store localStorage keys**

In `frontend/src/store/app-store.ts`, find and replace all `vaultmdx-` prefixes with `rustsidian-`:

```bash
cd frontend && sed -i 's/vaultmdx-/rustsidian-/g' src/store/app-store.ts
```

- [ ] **Step 4: Verify TypeScript compiles**

```bash
cd frontend && npx tsc --noEmit
```

Fix any remaining type errors.

- [ ] **Step 5: Verify the app builds**

```bash
cd frontend && npm run build
```

Expected: Vite build succeeds.

- [ ] **Step 6: Commit**

```bash
git add -A
git commit -m "fix: remove renderer imports, update localStorage keys, fix TS errors"
```

---

### Task 10: Verify Full Tauri Dev Build

**Files:**
- No new files — integration test

- [ ] **Step 1: Run cargo check on desktop crate**

```bash
cargo check -p rustsidian-desktop
```

Expected: all Rust compiles clean

- [ ] **Step 2: Regenerate TypeScript bindings**

```bash
cd crates/rustsidian-desktop && cargo build
```

This triggers specta to regenerate `frontend/src/lib/bindings.ts`.

- [ ] **Step 3: Run full Tauri dev build**

```bash
cargo tauri dev
```

Expected: App launches with NanOEd's Solid.js UI. The folder picker should appear if no vault is configured. If a vault was previously configured, it should load and display notes.

- [ ] **Step 4: Smoke test core functionality**

Manually verify:
1. Vault opens and file tree displays
2. Clicking a note opens it in the editor
3. Editing and saving works (Ctrl+S)
4. Search works (Ctrl+P)
5. Graph view loads
6. Terminal opens (Ctrl+`)

- [ ] **Step 5: Commit any fixes discovered during testing**

```bash
git add -A
git commit -m "fix: integration fixes from Tauri dev smoke test"
```

---

### Task 11: KaTeX Math Widget (CM6 Extension)

**Files:**
- Create: `frontend/src/editor/mathWidget.ts`
- Modify: `frontend/src/editor/EditorPane.tsx` (add extension import)

- [ ] **Step 1: Create the KaTeX widget extension**

Create `frontend/src/editor/mathWidget.ts`:

```typescript
import {
  Decoration,
  DecorationSet,
  EditorView,
  ViewPlugin,
  ViewUpdate,
  WidgetType,
} from "@codemirror/view";
import { Range } from "@codemirror/state";
import katex from "katex";
import "katex/dist/katex.min.css";

/**
 * CM6 widget that renders <Math expr="..."/> JSX nodes as KaTeX output.
 * When the cursor is inside the Math node, the raw JSX is shown for editing.
 */

const MATH_RE = /<Math\s+expr="([^"]*)"(?:\s*\/)?\s*>/g;

class MathWidget extends WidgetType {
  constructor(readonly expr: string, readonly block: boolean) {
    super();
  }

  eq(other: MathWidget): boolean {
    return this.expr === other.expr && this.block === other.block;
  }

  toDOM(): HTMLElement {
    const container = document.createElement("span");
    container.className = "cm-math-widget";
    try {
      katex.render(this.expr, container, {
        displayMode: this.block,
        throwOnError: false,
      });
    } catch {
      container.textContent = this.expr;
      container.style.color = "var(--error, #ff6b6b)";
      container.style.border = "1px solid var(--error, #ff6b6b)";
      container.style.padding = "2px 4px";
      container.style.borderRadius = "3px";
    }
    return container;
  }

  ignoreEvent(): boolean {
    return false;
  }
}

function buildDecorations(view: EditorView): DecorationSet {
  const decorations: Range<Decoration>[] = [];
  const doc = view.state.doc;
  const cursor = view.state.selection.main;

  for (let i = 1; i <= doc.lines; i++) {
    const line = doc.line(i);
    let match: RegExpExecArray | null;
    MATH_RE.lastIndex = 0;

    while ((match = MATH_RE.exec(line.text)) !== null) {
      const from = line.from + match.index;
      const to = line.from + match.index + match[0].length;
      const expr = match[1];

      // Don't replace if cursor is inside the match range
      if (cursor.from >= from && cursor.from <= to) continue;
      if (cursor.to >= from && cursor.to <= to) continue;

      const isBlock = line.text.trim() === match[0];
      decorations.push(
        Decoration.replace({
          widget: new MathWidget(expr, isBlock),
        }).range(from, to),
      );
    }
  }

  return Decoration.set(decorations, true);
}

export const mathWidgetPlugin = ViewPlugin.fromClass(
  class {
    decorations: DecorationSet;
    constructor(view: EditorView) {
      this.decorations = buildDecorations(view);
    }
    update(update: ViewUpdate) {
      if (update.docChanged || update.selectionSet || update.viewportChanged) {
        this.decorations = buildDecorations(update.view);
      }
    }
  },
  { decorations: (v) => v.decorations },
);

export const mathWidgetTheme = EditorView.baseTheme({
  ".cm-math-widget": {
    display: "inline-block",
    verticalAlign: "middle",
    padding: "2px 4px",
  },
});
```

- [ ] **Step 2: Register the extension in EditorPane.tsx**

In `frontend/src/editor/EditorPane.tsx`, find the extensions array and add:

```typescript
import { mathWidgetPlugin, mathWidgetTheme } from "./mathWidget";
```

Add to the extensions array:

```typescript
mathWidgetPlugin,
mathWidgetTheme,
```

- [ ] **Step 3: Verify it compiles**

```bash
cd frontend && npx tsc --noEmit
```

Expected: no errors

- [ ] **Step 4: Commit**

```bash
git add frontend/src/editor/mathWidget.ts frontend/src/editor/EditorPane.tsx
git commit -m "feat: add KaTeX in-editor rendering for <Math> JSX components"
```

---

### Task 12: Mermaid Diagram Widget (CM6 Extension)

**Files:**
- Create: `frontend/src/editor/mermaidWidget.ts`
- Modify: `frontend/src/editor/EditorPane.tsx` (add extension import)

- [ ] **Step 1: Create the Mermaid widget extension**

Create `frontend/src/editor/mermaidWidget.ts`:

```typescript
import {
  Decoration,
  DecorationSet,
  EditorView,
  ViewPlugin,
  ViewUpdate,
  WidgetType,
} from "@codemirror/view";
import { Range } from "@codemirror/state";

/**
 * CM6 widget that renders <Mermaid chart="..."/> JSX nodes as SVG diagrams.
 * Caches rendered SVGs by content hash. Debounces re-renders.
 * When the cursor is inside the node, raw JSX is shown for editing.
 */

const MERMAID_RE = /<Mermaid\s+chart="([^"]*)"(?:\s*\/)?\s*>/g;

// SVG cache keyed by chart content
const svgCache = new Map<string, string>();
let mermaidLoaded: typeof import("mermaid") | null = null;
let mermaidInitialized = false;
let renderCounter = 0;

async function getMermaid() {
  if (!mermaidLoaded) {
    mermaidLoaded = await import("mermaid");
  }
  if (!mermaidInitialized) {
    mermaidLoaded.default.initialize({
      startOnLoad: false,
      securityLevel: "loose",
      theme: "dark",
      themeVariables: {
        primaryColor: "#c8a228",
        primaryTextColor: "#e8e8e8",
        primaryBorderColor: "#444",
        lineColor: "#888",
        secondaryColor: "#1a1a2e",
        tertiaryColor: "#0a0a0f",
      },
    });
    mermaidInitialized = true;
  }
  return mermaidLoaded.default;
}

class MermaidWidget extends WidgetType {
  constructor(readonly chart: string) {
    super();
  }

  eq(other: MermaidWidget): boolean {
    return this.chart === other.chart;
  }

  toDOM(): HTMLElement {
    const container = document.createElement("div");
    container.className = "cm-mermaid-widget";
    container.style.padding = "8px";
    container.style.margin = "4px 0";
    container.style.borderRadius = "6px";
    container.style.background = "var(--bg-secondary, #111118)";
    container.style.border = "1px solid var(--border, #333)";

    // Unescape newlines from the prop value
    const chartSource = this.chart.replace(/\\n/g, "\n");

    const cached = svgCache.get(chartSource);
    if (cached) {
      container.innerHTML = cached;
      return container;
    }

    container.textContent = "Rendering diagram...";
    container.style.color = "var(--text-muted, #666)";
    container.style.fontStyle = "italic";

    const renderThis = async () => {
      try {
        const mermaid = await getMermaid();
        const id = `mermaid-${++renderCounter}`;
        const { svg } = await mermaid.render(id, chartSource);
        svgCache.set(chartSource, svg);
        container.innerHTML = svg;
        container.style.color = "";
        container.style.fontStyle = "";
      } catch (err) {
        container.textContent = `Mermaid error: ${err}`;
        container.style.color = "var(--error, #ff6b6b)";
      }
    };

    // Debounce render
    setTimeout(renderThis, 200);

    return container;
  }

  ignoreEvent(): boolean {
    return false;
  }
}

function buildDecorations(view: EditorView): DecorationSet {
  const decorations: Range<Decoration>[] = [];
  const doc = view.state.doc;
  const cursor = view.state.selection.main;

  for (let i = 1; i <= doc.lines; i++) {
    const line = doc.line(i);
    let match: RegExpExecArray | null;
    MERMAID_RE.lastIndex = 0;

    while ((match = MERMAID_RE.exec(line.text)) !== null) {
      const from = line.from + match.index;
      const to = line.from + match.index + match[0].length;
      const chart = match[1];

      // Don't replace if cursor is inside the range
      if (cursor.from >= from && cursor.from <= to) continue;
      if (cursor.to >= from && cursor.to <= to) continue;

      decorations.push(
        Decoration.replace({
          widget: new MermaidWidget(chart),
          block: true,
        }).range(from, to),
      );
    }
  }

  return Decoration.set(decorations, true);
}

export const mermaidWidgetPlugin = ViewPlugin.fromClass(
  class {
    decorations: DecorationSet;
    constructor(view: EditorView) {
      this.decorations = buildDecorations(view);
    }
    update(update: ViewUpdate) {
      if (update.docChanged || update.selectionSet || update.viewportChanged) {
        this.decorations = buildDecorations(update.view);
      }
    }
  },
  { decorations: (v) => v.decorations },
);

export const mermaidWidgetTheme = EditorView.baseTheme({
  ".cm-mermaid-widget": {
    cursor: "default",
  },
  ".cm-mermaid-widget svg": {
    maxWidth: "100%",
  },
});
```

- [ ] **Step 2: Register the extension in EditorPane.tsx**

In `frontend/src/editor/EditorPane.tsx`, add:

```typescript
import { mermaidWidgetPlugin, mermaidWidgetTheme } from "./mermaidWidget";
```

Add to the extensions array:

```typescript
mermaidWidgetPlugin,
mermaidWidgetTheme,
```

- [ ] **Step 3: Verify it compiles**

```bash
cd frontend && npx tsc --noEmit
```

Expected: no errors

- [ ] **Step 4: Commit**

```bash
git add frontend/src/editor/mermaidWidget.ts frontend/src/editor/EditorPane.tsx
git commit -m "feat: add Mermaid in-editor rendering for <Mermaid> JSX components"
```

---

### Task 13: End-to-End Smoke Test

**Files:**
- No new files — validation task

- [ ] **Step 1: Build and launch**

```bash
cargo tauri dev
```

- [ ] **Step 2: Test KaTeX rendering**

Create or open a note containing:

```mdx
Here is inline math: <Math expr="E = mc^2"/>

And a block equation:

<Math expr="\int_0^\infty e^{-x^2} dx = \frac{\sqrt{\pi}}{2}"/>
```

Expected: Math expressions render as formatted KaTeX in the editor. Clicking into the rendered area shows raw JSX for editing.

- [ ] **Step 3: Test Mermaid rendering**

Create or open a note containing:

```mdx
<Mermaid chart="graph TD\n  A[Start] --> B{Decision}\n  B -->|Yes| C[Action]\n  B -->|No| D[End]"/>
```

Expected: Diagram renders as SVG in the editor. Clicking into it shows raw JSX.

- [ ] **Step 4: Test core navigation**

1. File tree navigates correctly
2. Search (Ctrl+P) finds notes
3. Graph view renders
4. Tab management works (open, close, switch)
5. Terminal opens and accepts input

- [ ] **Step 5: Commit any fixes**

```bash
git add -A
git commit -m "fix: smoke test fixes for Group A integration"
```

---

## Summary

| Task | Description | New/Modified Files |
|------|-------------|-------------------|
| 1 | Copy NanOEd frontend, clean up | `frontend/` (replace) |
| 2 | Add new IPC types | `ipc_types.rs` |
| 3 | Add create_directory command | `vault.rs`, `lib.rs` |
| 4 | Add backlinks commands | `vault.rs`, `lib.rs` |
| 5 | Add vault_stats + daily_note commands | `vault.rs`, `lib.rs` |
| 6 | Rewrite IPC bridge | `tauri.ts` |
| 7 | Rewrite event listeners | `events.ts` |
| 8 | Rewrite AI bridge | `ai.ts` |
| 9 | Fix imports, remove renderer refs | Various frontend files |
| 10 | Full Tauri dev build verification | Integration test |
| 11 | KaTeX math widget | `mathWidget.ts`, `EditorPane.tsx` |
| 12 | Mermaid diagram widget | `mermaidWidget.ts`, `EditorPane.tsx` |
| 13 | End-to-end smoke test | Validation |
