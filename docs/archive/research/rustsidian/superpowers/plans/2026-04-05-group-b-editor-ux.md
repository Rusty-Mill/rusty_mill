> **Archived 2026-09-08** — Imported verbatim from the private `baileyrd/Rustsidian` repo (`docs/`) under RFC 0009 row 6. Rustsidian is retired; this is reference material only and describes Rustsidian's SvelteKit/Tauri stack, not Nexus.

# Group B: Editor UX — Implementation Plan

> **For agentic workers:** REQUIRED SUB-SKILL: Use superpowers:subagent-driven-development (recommended) or superpowers:executing-plans to implement this plan task-by-task. Steps use checkbox (`- [ ]`) syntax for tracking.

**Goal:** Add hover previews on wikilinks (structured summary tooltip) and automatic link update on rename.

**Architecture:** New CM6 hoverTooltip extension calls a new `note_preview` Tauri command. Rename-with-link-update extends `Vault::rename_note()` in rustsidian-core to find backlinks and rewrite wikilinks in referencing notes.

**Tech Stack:** CodeMirror 6 (hoverTooltip), Tauri IPC, Rust regex for wikilink replacement

**Spec:** `docs/superpowers/specs/2026-04-05-nanoed-feature-integration-design.md` — Sections B1–B2

**Prerequisite:** Group A must be completed (NanOEd frontend in place).

---

## File Map

### Files to create
- `frontend/src/editor/hoverPreview.ts` — CM6 hover tooltip extension

### Files to modify
- `frontend/src/editor/EditorPane.tsx` — register hover preview extension
- `crates/rustsidian-desktop/src/commands/vault.rs` — add `note_preview` command, modify `rename_note` return type
- `crates/rustsidian-desktop/src/lib.rs` — register `note_preview` command
- `crates/rustsidian-core/src/vault/mod.rs` — extend `rename_note()` with link update logic

---

### Task 1: Add note_preview Tauri Command

**Files:**
- Modify: `crates/rustsidian-desktop/src/commands/vault.rs`
- Modify: `crates/rustsidian-desktop/src/lib.rs`

- [ ] **Step 1: Add NotePreview import**

In `crates/rustsidian-desktop/src/commands/vault.rs`, add `NotePreview` to the imports from `ipc_types`:

```rust
use crate::ipc_types::{BacklinkContextItem, FileTreeNode, NoteContent, NoteId, NoteListItem, NotePreview, VaultInfo, VaultStats};
```

- [ ] **Step 2: Add note_preview command**

Append to `crates/rustsidian-desktop/src/commands/vault.rs`:

```rust
/// Get a structured preview of a note for hover tooltips.
/// Resolves wikilink targets using case-insensitive stem matching.
#[tauri::command]
#[specta::specta]
pub async fn note_preview(path: String, state: State<'_, AppState>) -> Result<NotePreview, String> {
    let guard = state.vault.lock().map_err(|e| e.to_string())?;
    let vault = guard
        .as_ref()
        .ok_or("No vault open — call pick_vault_folder first")?;

    // Resolve the wikilink target — try exact path first, then stem match
    let all_paths = vault.list_all_note_paths().map_err(|e| e.to_string())?;
    let resolved_path = resolve_wikilink_target(&path, &all_paths)
        .ok_or_else(|| format!("Note not found: {path}"))?;

    let note = vault
        .get_note_by_path(&resolved_path)
        .map_err(|e| e.to_string())?
        .ok_or_else(|| format!("Note not found: {resolved_path}"))?;

    // Extract tags from parsed note
    let parsed = rustsidian_core::parser::parse(&note.content, &note.content_type);
    let tags: Vec<String> = parsed.tags;

    // First paragraph: skip frontmatter, take text before first blank line
    let content_without_frontmatter = if note.content.starts_with("---") {
        if let Some(end) = note.content[3..].find("---") {
            note.content[end + 6..].trim_start().to_string()
        } else {
            note.content.clone()
        }
    } else {
        note.content.clone()
    };
    let first_paragraph = content_without_frontmatter
        .split("\n\n")
        .next()
        .unwrap_or("")
        .chars()
        .take(200)
        .collect::<String>();

    // Backlink count
    let all_edges = vault.get_all_edges().map_err(|e| e.to_string())?;
    let backlink_count = all_edges
        .iter()
        .filter(|(_src, tgt)| tgt == &resolved_path)
        .count();

    Ok(NotePreview {
        title: note.title,
        tags,
        first_paragraph,
        backlink_count,
    })
}

/// Resolve a wikilink target to a vault path.
/// Tries: exact match, then case-insensitive stem match.
fn resolve_wikilink_target(target: &str, all_paths: &[String]) -> Option<String> {
    // Exact path match
    if all_paths.contains(&target.to_string()) {
        return Some(target.to_string());
    }
    // Stem match (case-insensitive): "My Note" matches "folder/My Note.md"
    let target_lower = target.to_lowercase();
    all_paths.iter().find(|p| {
        let stem = std::path::Path::new(p)
            .file_stem()
            .and_then(|s| s.to_str())
            .unwrap_or("");
        stem.to_lowercase() == target_lower
    }).cloned()
}
```

- [ ] **Step 3: Register note_preview in lib.rs**

In `crates/rustsidian-desktop/src/lib.rs`, add inside `collect_commands![]`:

```rust
        commands::vault::note_preview,
```

- [ ] **Step 4: Verify it compiles**

```bash
cd crates/rustsidian-desktop && cargo check
```

Expected: compiles successfully. Note: `rustsidian_core::parser::parse()` must accept `(&str, &str)` — check the actual signature and adjust if needed.

- [ ] **Step 5: Commit**

```bash
git add crates/rustsidian-desktop/src/commands/vault.rs crates/rustsidian-desktop/src/lib.rs
git commit -m "feat: add note_preview Tauri command for hover tooltips"
```

---

### Task 2: Create Hover Preview CM6 Extension

**Files:**
- Create: `frontend/src/editor/hoverPreview.ts`
- Modify: `frontend/src/editor/EditorPane.tsx`

- [ ] **Step 1: Create the hover preview extension**

Create `frontend/src/editor/hoverPreview.ts`:

```typescript
import { EditorView, Tooltip, hoverTooltip } from "@codemirror/view";

/**
 * CM6 hoverTooltip extension for [[wikilink]] previews.
 * Shows: title, tags (pills), first paragraph, backlink count.
 * LRU cache to avoid repeated IPC calls.
 */

interface NotePreview {
  title: string;
  tags: string[];
  first_paragraph: string;
  backlink_count: number;
}

// Simple LRU cache (max 64 entries)
const cache = new Map<string, { data: NotePreview; time: number }>();
const CACHE_MAX = 64;
const CACHE_TTL_MS = 30_000; // 30 seconds

function cacheGet(key: string): NotePreview | null {
  const entry = cache.get(key);
  if (!entry) return null;
  if (Date.now() - entry.time > CACHE_TTL_MS) {
    cache.delete(key);
    return null;
  }
  return entry.data;
}

function cacheSet(key: string, data: NotePreview): void {
  if (cache.size >= CACHE_MAX) {
    // Delete oldest entry
    const oldest = cache.keys().next().value;
    if (oldest !== undefined) cache.delete(oldest);
  }
  cache.set(key, { data, time: Date.now() });
}

const WIKILINK_RE = /\[\[([^\[\]|]+?)(?:\|[^\[\]]+?)?\]\]/g;

async function fetchPreview(target: string): Promise<NotePreview | null> {
  const cached = cacheGet(target);
  if (cached) return cached;

  try {
    const { invoke } = await import("@tauri-apps/api/core");
    const result = await invoke<{ status: string; data: NotePreview; error?: string }>(
      "note_preview",
      { path: target },
    );
    if (result.status === "error") return null;
    cacheSet(target, result.data);
    return result.data;
  } catch {
    return null;
  }
}

function renderTooltip(preview: NotePreview): HTMLElement {
  const container = document.createElement("div");
  container.className = "cm-hover-preview";
  container.style.maxWidth = "400px";
  container.style.maxHeight = "300px";
  container.style.overflow = "auto";
  container.style.padding = "12px";
  container.style.background = "var(--bg-secondary, #111118)";
  container.style.border = "1px solid var(--border, #333)";
  container.style.borderRadius = "8px";
  container.style.boxShadow = "0 4px 12px rgba(0,0,0,0.5)";

  // Title
  const title = document.createElement("div");
  title.style.fontWeight = "bold";
  title.style.fontSize = "14px";
  title.style.color = "var(--accent, #c8a228)";
  title.style.marginBottom = "6px";
  title.textContent = preview.title;
  container.appendChild(title);

  // Tags
  if (preview.tags.length > 0) {
    const tagRow = document.createElement("div");
    tagRow.style.marginBottom = "8px";
    tagRow.style.display = "flex";
    tagRow.style.gap = "4px";
    tagRow.style.flexWrap = "wrap";
    for (const tag of preview.tags) {
      const pill = document.createElement("span");
      pill.textContent = `#${tag}`;
      pill.style.fontSize = "11px";
      pill.style.padding = "1px 6px";
      pill.style.borderRadius = "9999px";
      pill.style.background = "var(--bg-tertiary, #1a1a2e)";
      pill.style.color = "var(--text-secondary, #aaa)";
      tagRow.appendChild(pill);
    }
    container.appendChild(tagRow);
  }

  // First paragraph
  if (preview.first_paragraph) {
    const para = document.createElement("div");
    para.style.fontSize = "12px";
    para.style.color = "var(--text-secondary, #aaa)";
    para.style.lineHeight = "1.5";
    para.style.marginBottom = "8px";
    para.textContent = preview.first_paragraph;
    container.appendChild(para);
  }

  // Backlink count
  const meta = document.createElement("div");
  meta.style.fontSize = "11px";
  meta.style.color = "var(--text-muted, #666)";
  meta.style.borderTop = "1px solid var(--border, #333)";
  meta.style.paddingTop = "6px";
  meta.textContent = `${preview.backlink_count} backlink${preview.backlink_count !== 1 ? "s" : ""}`;
  container.appendChild(meta);

  return container;
}

export const wikilinkHoverPreview = hoverTooltip(
  (view: EditorView, pos: number, side: number): Tooltip | null | Promise<Tooltip | null> => {
    const line = view.state.doc.lineAt(pos);
    const lineText = line.text;
    const offset = pos - line.from;

    // Find if cursor is inside a [[wikilink]]
    WIKILINK_RE.lastIndex = 0;
    let match: RegExpExecArray | null;
    while ((match = WIKILINK_RE.exec(lineText)) !== null) {
      const start = match.index;
      const end = start + match[0].length;
      if (offset >= start && offset <= end) {
        const target = match[1];
        return fetchPreview(target).then((preview) => {
          if (!preview) return null;
          return {
            pos: line.from + start,
            end: line.from + end,
            above: true,
            create: () => ({ dom: renderTooltip(preview) }),
          };
        });
      }
    }
    return null;
  },
  { hoverTime: 300 },
);

export const hoverPreviewTheme = EditorView.baseTheme({
  ".cm-hover-preview": {
    fontFamily: "var(--font-sans, Inter, sans-serif)",
  },
});
```

- [ ] **Step 2: Register the extension in EditorPane.tsx**

In `frontend/src/editor/EditorPane.tsx`, add:

```typescript
import { wikilinkHoverPreview, hoverPreviewTheme } from "./hoverPreview";
```

Add to the extensions array:

```typescript
wikilinkHoverPreview,
hoverPreviewTheme,
```

- [ ] **Step 3: Verify it compiles**

```bash
cd frontend && npx tsc --noEmit
```

- [ ] **Step 4: Commit**

```bash
git add frontend/src/editor/hoverPreview.ts frontend/src/editor/EditorPane.tsx
git commit -m "feat: add wikilink hover preview with structured summary tooltip"
```

---

### Task 3: Extend Vault::rename_note() with Link Updates

**Files:**
- Modify: `crates/rustsidian-core/src/vault/mod.rs`

- [ ] **Step 1: Write test for rename with link update**

Create `crates/rustsidian-core/tests/rename_links.rs`:

```rust
use rustsidian_core::Vault;
use tempfile::TempDir;

#[test]
fn rename_note_updates_wikilinks_in_other_notes() {
    let tmp = TempDir::new().unwrap();
    let vault = Vault::open(tmp.path()).unwrap();

    // Create target note and a note that links to it
    vault.create_note("target.md".into(), "Target".into(), "# Target\nContent".into()).unwrap();
    vault.create_note(
        "linker.md".into(),
        "Linker".into(),
        "See [[target]] for details.\nAlso [[target|the target note]].".into(),
    ).unwrap();

    // Rename target
    let updated = vault.rename_note_with_link_update("target.md", "renamed.md", "Renamed").unwrap();

    // Verify the linking note was updated
    assert!(updated.contains(&"linker.md".to_string()));

    let linker = vault.get_note_by_path("linker.md").unwrap().unwrap();
    assert!(linker.content.contains("[[renamed]]"), "Content was: {}", linker.content);
    assert!(linker.content.contains("[[renamed|the target note]]"), "Content was: {}", linker.content);
    assert!(!linker.content.contains("[[target]]"), "Old link still present: {}", linker.content);
}

#[test]
fn rename_note_updates_embeds() {
    let tmp = TempDir::new().unwrap();
    let vault = Vault::open(tmp.path()).unwrap();

    vault.create_note("image-note.md".into(), "Image Note".into(), "# Pic".into()).unwrap();
    vault.create_note(
        "embedder.md".into(),
        "Embedder".into(),
        "Embedded: ![[image-note]]".into(),
    ).unwrap();

    let updated = vault.rename_note_with_link_update("image-note.md", "photo.md", "Photo").unwrap();

    assert!(updated.contains(&"embedder.md".to_string()));
    let embedder = vault.get_note_by_path("embedder.md").unwrap().unwrap();
    assert!(embedder.content.contains("![[photo]]"), "Content was: {}", embedder.content);
}
```

- [ ] **Step 2: Run tests to verify they fail**

```bash
cd crates/rustsidian-core && cargo test rename_links -- --nocapture
```

Expected: FAIL — `rename_note_with_link_update` doesn't exist yet.

- [ ] **Step 3: Implement rename_note_with_link_update**

In `crates/rustsidian-core/src/vault/mod.rs`, add a new method after `rename_note()` (around line 636):

```rust
    /// Rename a note and automatically update all wikilinks pointing to it.
    ///
    /// Returns the list of note paths whose content was rewritten.
    pub fn rename_note_with_link_update(
        &self,
        old_path: &str,
        new_path: &str,
        new_title: &str,
    ) -> Result<Vec<String>, VaultError> {
        // First, do the standard rename (file move + DB update)
        self.rename_note(old_path, new_path, new_title)?;

        // Extract stems for wikilink matching
        let old_stem = std::path::Path::new(old_path)
            .file_stem()
            .and_then(|s| s.to_str())
            .unwrap_or(old_path);
        let new_stem = std::path::Path::new(new_path)
            .file_stem()
            .and_then(|s| s.to_str())
            .unwrap_or(new_path);

        if old_stem == new_stem {
            return Ok(Vec::new()); // Stem unchanged (e.g. just moved folders)
        }

        // Find all notes that link to the old path via the links table
        // Query source notes whose target_path matches old_stem (case-insensitive)
        let source_paths: Vec<String> = {
            let mut stmt = self.conn.prepare(
                "SELECT DISTINCT n.path FROM links_index li
                 JOIN notes n ON n.id = li.source_note_id
                 WHERE LOWER(li.target_path) = LOWER(?1)
                 AND n.deleted_at IS NULL"
            )?;
            stmt.query_map(rusqlite::params![old_stem], |row| row.get(0))?
                .filter_map(|r| r.ok())
                .collect()
        };

        // Also check for full-path targets
        let source_paths_full: Vec<String> = {
            let mut stmt = self.conn.prepare(
                "SELECT DISTINCT n.path FROM links_index li
                 JOIN notes n ON n.id = li.source_note_id
                 WHERE LOWER(li.target_path) = LOWER(?1)
                 AND n.deleted_at IS NULL"
            )?;
            stmt.query_map(rusqlite::params![old_path], |row| row.get(0))?
                .filter_map(|r| r.ok())
                .collect()
        };

        let mut all_sources: Vec<String> = source_paths;
        for p in source_paths_full {
            if !all_sources.contains(&p) {
                all_sources.push(p);
            }
        }

        // Build regex: matches [[old_stem]], [[old_stem|alias]], ![[old_stem]], ![[old_stem|alias]]
        let escaped_stem = regex::escape(old_stem);
        let pattern = format!(r"(?i)(\[\[|!\[\[){}(\|[^\]]+)?\]\]", escaped_stem);
        let re = regex::Regex::new(&pattern).map_err(|e| VaultError::Other(e.to_string()))?;

        let mut updated_paths = Vec::new();
        for source_path in &all_sources {
            if source_path == new_path {
                continue; // Don't update the renamed note itself
            }
            let note = self.get_note_by_path(source_path)?;
            if let Some(note) = note {
                let new_content = re.replace_all(&note.content, |caps: &regex::Captures| {
                    let prefix = &caps[1]; // [[ or ![[
                    let alias = caps.get(2).map(|m| m.as_str()).unwrap_or("");
                    format!("{}{}{}", prefix, new_stem, alias)
                }).to_string();

                if new_content != note.content {
                    self.update_note_content(source_path, &new_content)?;
                    updated_paths.push(source_path.clone());
                }
            }
        }

        Ok(updated_paths)
    }
```

- [ ] **Step 4: Run tests to verify they pass**

```bash
cd crates/rustsidian-core && cargo test rename_links -- --nocapture
```

Expected: PASS — both tests pass.

- [ ] **Step 5: Commit**

```bash
git add crates/rustsidian-core/src/vault/mod.rs crates/rustsidian-core/tests/rename_links.rs
git commit -m "feat: add rename_note_with_link_update to automatically rewrite wikilinks"
```

---

### Task 4: Wire rename_note_with_link_update into Desktop Command

**Files:**
- Modify: `crates/rustsidian-desktop/src/commands/vault.rs`
- Modify: `crates/rustsidian-desktop/src/ipc_types.rs`

- [ ] **Step 1: Add RenameResult type to ipc_types.rs**

Append to `crates/rustsidian-desktop/src/ipc_types.rs`:

```rust
/// Result from renaming a note, including which other notes had their links updated.
#[derive(Serialize, Deserialize, Type, Debug)]
pub struct RenameResult {
    pub new_path: String,
    pub updated_notes: Vec<String>,
}
```

- [ ] **Step 2: Update rename_note command to use rename_note_with_link_update**

In `crates/rustsidian-desktop/src/commands/vault.rs`, update the `rename_note` command (currently at lines 198-214). Add `RenameResult` to the imports:

```rust
use crate::ipc_types::{BacklinkContextItem, FileTreeNode, NoteContent, NoteId, NoteListItem, NotePreview, RenameResult, VaultInfo, VaultStats};
```

Replace the existing `rename_note` command:

```rust
/// Rename a note: moves file on disk, updates path/title in DB, and
/// automatically rewrites [[wikilinks]] in all notes that referenced the old name.
#[tauri::command]
#[specta::specta]
pub async fn rename_note(
    old_path: String,
    new_path: String,
    new_title: String,
    state: State<'_, AppState>,
) -> Result<RenameResult, String> {
    let guard = state.vault.lock().map_err(|e| e.to_string())?;
    let vault = guard
        .as_ref()
        .ok_or("No vault open — call pick_vault_folder first")?;
    let updated_notes = vault
        .rename_note_with_link_update(&old_path, &new_path, &new_title)
        .map_err(|e| e.to_string())?;
    Ok(RenameResult {
        new_path,
        updated_notes,
    })
}
```

- [ ] **Step 3: Update frontend renameNote to handle new return type**

In `frontend/src/lib/tauri.ts`, update the `renameNote` function:

```typescript
export interface RenameResult {
  new_path: string;
  updated_notes: string[];
}

export async function renameNote(oldPath: string, newPath: string): Promise<RenameResult> {
  const stem = newPath.replace(/^.*\//, "").replace(/\.(md|mdx)$/, "");
  const newTitle = stem || "Untitled";
  const result = await invoke<RenameResult>("rename_note", { oldPath, newPath, newTitle });
  return result ?? { new_path: newPath, updated_notes: [] };
}
```

- [ ] **Step 4: Verify it compiles**

```bash
cargo check -p rustsidian-desktop && cd frontend && npx tsc --noEmit
```

- [ ] **Step 5: Commit**

```bash
git add crates/rustsidian-desktop/src/commands/vault.rs crates/rustsidian-desktop/src/ipc_types.rs frontend/src/lib/tauri.ts
git commit -m "feat: wire rename-with-link-update into desktop command and frontend"
```

---

### Task 5: Smoke Test Group B Features

**Files:**
- No new files — validation

- [ ] **Step 1: Build and launch**

```bash
cargo tauri dev
```

- [ ] **Step 2: Test hover preview**

Open a note containing `[[some-other-note]]`. Hover over the wikilink for ~300ms. A tooltip should appear showing:
- Title in gold
- Tags as pills
- First paragraph text
- Backlink count

- [ ] **Step 3: Test rename with link update**

1. Create note "Alpha" with content "# Alpha\nSome content"
2. Create note "References" with content "See [[Alpha]] for details"
3. Rename "Alpha" to "Beta" via the file tree context menu
4. Open "References" — verify it now says `[[Beta]]` not `[[Alpha]]`

- [ ] **Step 4: Commit any fixes**

```bash
git add -A
git commit -m "fix: Group B smoke test fixes"
```

---

## Summary

| Task | Description | New/Modified Files |
|------|-------------|-------------------|
| 1 | note_preview Tauri command | `vault.rs`, `lib.rs` |
| 2 | Hover preview CM6 extension | `hoverPreview.ts`, `EditorPane.tsx` |
| 3 | rename_note_with_link_update in core | `vault/mod.rs`, `tests/rename_links.rs` |
| 4 | Wire into desktop + frontend | `vault.rs`, `ipc_types.rs`, `tauri.ts` |
| 5 | Smoke test | Validation |
