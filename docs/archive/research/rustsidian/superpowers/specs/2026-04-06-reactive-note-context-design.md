> **Archived 2026-09-08** — Imported verbatim from the private `baileyrd/Rustsidian` repo (`docs/`) under RFC 0009 row 6. Rustsidian is retired; this is reference material only and describes Rustsidian's SvelteKit/Tauri stack, not Nexus.

# Reactive Note Context & Panel Wiring

**Date:** 2026-04-06
**Scope:** Backlog items 1-3 — TagsPanel incremental updates, FileTree reactive refresh, right sidebar shared data source + scroll preservation

## Problem

Three groups of components fetch data independently and don't react to vault changes:

1. **TagsPanel** calls `listNotes()` once on mount, never updates when tags change
2. **FileTree** uses an imperative `refreshKey` counter managed by its parent, overloaded with collapse-all semantics
3. **Right sidebar panels** (Backlinks, OutgoingLinks, Outline) each fetch via IPC on path change only — no reaction to content edits or external file changes. OutlinePanel manually tracks a `cacheVersion` signal. Scroll position is lost on tab switch.

The infrastructure to solve this already exists — Rust vault watcher emits `"vault-changed"`, `bridgeTauriEvents()` converts to app events, `cache-sync.ts` updates `VaultCache` and emits `"metadata-cache-updated"`. But VaultCache is a plain object, not SolidJS-reactive, so consumers can't subscribe to changes automatically.

## Design

### NoteContext Store

New file: `frontend/src/lib/note-context.ts`

A SolidJS `createStore` shaped by what consumers actually need:

```typescript
interface NoteMetaEntry {
  title: string;
  tags: string[];
  backlinks: string[];        // paths of notes linking TO this note
  outgoingLinks: string[];    // paths this note links TO
  headings: Heading[];
  wordCount: number;
}

interface NoteContextState {
  // Per-note metadata — granular access by path
  noteMeta: Record<string, NoteMetaEntry>;

  // Global indexes — for consumers that need cross-note views
  tagIndex: Record<string, string[]>;  // tag -> [note paths]
  paths: string[];                      // all note paths
}
```

**Consumer access patterns and reactivity:**

| Consumer | Reads | Rerenders when |
|----------|-------|----------------|
| TagsPanel | `store.tagIndex` | Any note's tags change |
| BacklinksPanel | `store.noteMeta[activePath].backlinks` | Active note's backlinks change |
| OutgoingLinksPanel | `store.noteMeta[activePath].outgoingLinks` | Active note's outgoing links change |
| OutlinePanel | `store.noteMeta[activePath].headings` | Active note's headings change |

SolidJS tracks access at the property level, so reading `store.noteMeta["daily.mdx"].headings` does NOT trigger rerenders when an unrelated note's tags change.

**Factory function:**

```typescript
function createNoteContext(cache: VaultCache, eventBus: AppEventBus): NoteContext
```

- Called once in `bootstrap.ts`, before any component renders (store hydrates before render — no waterfall)
- On init: reads all data from VaultCache, populates store
- Subscribes to `"metadata-cache-updated"` with `{ paths: string[] }`
- On update: reads changed paths from VaultCache, calls `setStore()` for only affected entries
  - `setStore("noteMeta", changedPath, newMeta)` — per-note granular update
  - If tags changed: `setStore("tagIndex", affectedTag, newPathList)` — only touched tags
  - If note added/removed: `setStore("paths", newPathList)`

**Provided to components** via SolidJS context (`createContext` / `useContext`). A `NoteContextProvider` wraps the app in `App.tsx`, and consumers call `useNoteContext()` to access the store.

### FileTree Self-Managed Reactivity

FileTree becomes self-contained for refresh:

- **Drops** `refreshKey` prop entirely
- **Subscribes** to `"vault-file-created"` and `"vault-file-deleted"` via `appEventBus` on mount
- Created/deleted: calls `loadRoot()` to re-fetch the tree
- Modified: no action (content changes don't affect file structure)
- **Collapse-all** becomes a dedicated boolean prop (`collapseAll`), no longer overloaded on the refresh counter
- `onCleanup()` unsubscribes from events

Parent components that currently manage `refreshKey` lose that responsibility.

### Panel Wiring Changes

**TagsPanel:**
- Drops `onMount` → `listNotes()` IPC call
- Reads `store.tagIndex` directly
- `buildTagTree()` refactored to accept `Record<string, string[]>` (tag → paths) instead of `NoteDto[]`
- Automatic recomputation via SolidJS reactivity when tag assignments change

**BacklinksPanel:**
- Drops `createEffect` → `getBacklinksWithContext()` IPC call
- Reads `store.noteMeta[activePath].backlinks` for linked mentions
- Unlinked mentions: scans `store.noteMeta` entries for title matches in content (same logic, local data)
- Recomputes on active path change or when backlink data updates

**OutgoingLinksPanel:**
- Drops `createEffect` → `openNote()` + `parseLinks()` IPC call
- Reads `store.noteMeta[activePath].outgoingLinks`
- Link resolution (checking targets exist) uses `store.paths` instead of separate IPC
- Recomputes on active path change or when link data updates

**OutlinePanel:**
- Drops manual `cacheVersion` signal and `appEventBus.on("metadata-cache-updated")` subscription
- Reads `store.noteMeta[activePath].headings`
- Pure granular reactivity — no manual tracking

### Scroll Preservation

Sidebar container (not the store) manages scroll state:

- `Map<string, number>` keyed by panel name (e.g., "backlinks", "outline")
- On tab switch away: save `scrollTop` of current panel's container
- On tab switch to: restore saved `scrollTop` after render
- Local UI state — not part of NoteContext store

## Update Flow

```
File changed on disk
  -> Rust watcher (100ms debounce)
  -> "vault-changed" Tauri event
  -> bridgeTauriEvents() -> "vault-file-created/modified/deleted"
  -> cache-sync.ts updates VaultCache, emits "metadata-cache-updated" { paths }
  -> NoteContext store receives event
  -> Reads changed paths from VaultCache
  -> setStore() for only affected entries
  -> SolidJS granular reactivity triggers only affected consumers

FileTree (separate path):
  -> "vault-file-created" / "vault-file-deleted" via appEventBus
  -> loadRoot() re-fetches tree structure
  -> (ignores "vault-file-modified" — content changes don't affect tree)
```

## What Gets Deleted

- `refreshKey` prop from FileTree and all parent wiring
- `listNotes()` call from TagsPanel
- Manual `cacheVersion` signal from OutlinePanel
- Per-path IPC `createEffect` blocks from BacklinksPanel and OutgoingLinksPanel
- Negative-value overloading on refresh counter for collapse-all

## What Stays Unchanged

- Rust vault watcher (both core DB sync and desktop event emission)
- `bridgeTauriEvents()` in `events.ts`
- `cache-sync.ts` (continues updating VaultCache as before)
- VaultCache itself (NoteContext reads from it; no structural changes)
- All backend IPC commands (still available, just not called by these panels)

## VaultCache Relationship

NoteContext store wraps VaultCache in SolidJS reactivity. VaultCache remains the data holder; the store is the reactive access layer. Long-term, NoteContext could absorb VaultCache's role, but that migration is out of scope for this pass.

## Testing

- Verify TagsPanel updates when a note's tags are edited externally
- Verify BacklinksPanel updates when another note adds a link to the active note
- Verify OutlinePanel updates when active note's headings change
- Verify FileTree adds/removes entries on file create/delete without manual refresh
- Verify scroll position preserved when switching sidebar tabs
- Verify no regressions in existing panel functionality
