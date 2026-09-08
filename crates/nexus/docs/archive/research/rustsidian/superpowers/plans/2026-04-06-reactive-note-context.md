> **Archived 2026-09-08** — Imported verbatim from the private `baileyrd/Rustsidian` repo (`docs/`) under RFC 0009 row 6. Rustsidian is retired; this is reference material only and describes Rustsidian's SvelteKit/Tauri stack, not Nexus.

# Reactive Note Context Implementation Plan

> **For agentic workers:** REQUIRED SUB-SKILL: Use superpowers:subagent-driven-development (recommended) or superpowers:executing-plans to implement this plan task-by-task. Steps use checkbox (`- [ ]`) syntax for tracking.

**Goal:** Replace scattered IPC-per-render data fetching in TagsPanel, FileTree, and right sidebar panels with a SolidJS reactive NoteContext store fed by the existing vault watcher event pipeline.

**Architecture:** A `createStore`-based NoteContext hydrated from VaultCache at init, updated granularly via `"metadata-cache-updated"` events. Panels read from the store and react automatically. FileTree subscribes to vault file events directly for structure changes. A prerequisite fix wires `listNotesMetadata()` to the real backend command so the cache has headings, outgoing links, and frontmatter.

**Tech Stack:** SolidJS (`createStore`, `createContext`), existing Tauri event bridge, existing `cache-sync.ts` + `VaultCache`

**Spec:** `docs/superpowers/specs/2026-04-06-reactive-note-context-design.md`

---

## File Map

| Action | File | Responsibility |
|--------|------|---------------|
| Modify | `frontend/src/lib/tauri.ts:437-450` | Fix `listNotesMetadata()` to call real backend command |
| Create | `frontend/src/lib/note-context.ts` | NoteContext store + provider + hook |
| Modify | `frontend/src/lib/cache-sync.ts` | Expose `vaultCache` for NoteContext init reads |
| Modify | `frontend/src/shell/App.tsx` | Wrap app in `NoteContextProvider` |
| Modify | `frontend/src/shell/TagsPanel.tsx` | Read from store instead of `listNotes()` |
| Modify | `frontend/src/shell/OutlinePanel.tsx` | Read from store instead of manual cache tracking |
| Modify | `frontend/src/shell/BacklinksPanel.tsx` | Read backlinks from store, keep IPC for context strings |
| Modify | `frontend/src/shell/OutgoingLinksPanel.tsx` | Read from store instead of IPC |
| Modify | `frontend/src/shell/FileTree.tsx` | Subscribe to vault events, drop `refreshKey` |
| Modify | `frontend/src/shell/Sidebar.tsx` | Remove `refreshKey` management, add `collapseAll` signal |
| Modify | `frontend/src/app/containers/RightSidebarContainer.tsx` | Add scroll preservation |

---

### Task 1: Fix `listNotesMetadata()` to Call Real Backend Command

The frontend's `listNotesMetadata()` currently calls `listNotes()` and returns empty headings/links. The backend `list_notes_metadata` command already returns full data. This must be fixed first — without it, the store would have no headings or link data.

**Files:**
- Modify: `frontend/src/lib/tauri.ts:437-450`

- [ ] **Step 1: Fix the function to invoke the real backend command**

Replace the current `listNotesMetadata()` implementation:

```typescript
export async function listNotesMetadata(): Promise<NoteMetadata[]> {
  const raw = await invoke<{
    path: string;
    title: string;
    tags: string[];
    headings: { level: number; text: string; line: number }[];
    frontmatter: Record<string, string>;
    outgoing_links: string[];
    word_count: number;
  }[]>("list_notes_metadata");
  return raw.map((n) => ({
    path: n.path,
    title: n.title,
    tags: n.tags,
    headings: n.headings,
    frontmatter: n.frontmatter,
    outgoingLinks: n.outgoing_links,
    wordCount: n.word_count,
  }));
}
```

- [ ] **Step 2: Verify the backend command is registered**

Run: `grep -n "list_notes_metadata" crates/rustsidian-desktop/src/lib.rs`

Expected: A line showing `list_notes_metadata` in the command registration.

- [ ] **Step 3: Build and verify no type errors**

Run: `cd frontend && npx tsc --noEmit 2>&1 | grep -v vitest | grep -v EditorPane | head -20`

Expected: No new errors from the change.

- [ ] **Step 4: Commit**

```bash
git add frontend/src/lib/tauri.ts
git commit -m "fix: wire listNotesMetadata to real backend command

The frontend wrapper was calling listNotes() and returning empty
headings/outgoingLinks. The backend list_notes_metadata command
already parses and returns the full data."
```

---

### Task 2: Create NoteContext Store

**Files:**
- Create: `frontend/src/lib/note-context.ts`

- [ ] **Step 1: Create the NoteContext store module**

```typescript
import { createStore, produce } from "solid-js/store";
import { createContext, useContext, type ParentComponent } from "solid-js";
import { appEventBus } from "./app-events";
import type { VaultCache, NoteMetadata } from "./vault-cache";

// ---------------------------------------------------------------------------
// Store shape
// ---------------------------------------------------------------------------

export interface NoteMetaEntry {
  title: string;
  tags: string[];
  backlinks: string[];
  outgoingLinks: string[];
  headings: { level: number; text: string; line: number }[];
  wordCount: number;
}

interface NoteContextState {
  noteMeta: Record<string, NoteMetaEntry>;
  tagIndex: Record<string, string[]>;
  paths: string[];
}

export interface NoteContext {
  readonly store: NoteContextState;
}

// ---------------------------------------------------------------------------
// Internals
// ---------------------------------------------------------------------------

function noteToEntry(note: NoteMetadata, cache: VaultCache): NoteMetaEntry {
  return {
    title: note.title,
    tags: note.tags,
    backlinks: cache.getBacklinks(note.path),
    outgoingLinks: note.outgoingLinks,
    headings: note.headings,
    wordCount: note.wordCount,
  };
}

function buildTagIndex(cache: VaultCache): Record<string, string[]> {
  const index: Record<string, string[]> = {};
  for (const note of cache.getAllNotes()) {
    for (const tag of note.tags) {
      if (!index[tag]) index[tag] = [];
      index[tag].push(note.path);
    }
  }
  return index;
}

// ---------------------------------------------------------------------------
// Factory
// ---------------------------------------------------------------------------

export function createNoteContext(cache: VaultCache): NoteContext {
  // Build initial state from cache (already hydrated by initCacheSync)
  const initial: NoteContextState = { noteMeta: {}, tagIndex: {}, paths: [] };
  for (const note of cache.getAllNotes()) {
    initial.noteMeta[note.path] = noteToEntry(note, cache);
  }
  initial.tagIndex = buildTagIndex(cache);
  initial.paths = cache.getAllPaths();

  const [store, setStore] = createStore<NoteContextState>(initial);

  // Subscribe to cache updates — apply granular changes
  appEventBus.on("metadata-cache-updated", ({ paths: changedPaths }) => {
    const currentPaths = cache.getAllPaths();
    const allNotes = cache.getAllNotes();

    // Update changed note entries
    for (const p of changedPaths) {
      const note = cache.getNote(p);
      if (note) {
        setStore("noteMeta", p, noteToEntry(note, cache));
      } else {
        // Note was deleted — remove from store
        setStore(produce((s) => { delete s.noteMeta[p]; }));
      }
    }

    // Rebuild backlinks for all notes that might be affected
    // (a changed note's outgoing links affect other notes' backlinks)
    for (const note of allNotes) {
      const newBacklinks = cache.getBacklinks(note.path);
      const current = store.noteMeta[note.path]?.backlinks;
      if (!current || JSON.stringify(current) !== JSON.stringify(newBacklinks)) {
        setStore("noteMeta", note.path, "backlinks", newBacklinks);
      }
    }

    // Update paths if set changed
    if (JSON.stringify(currentPaths) !== JSON.stringify(store.paths)) {
      setStore("paths", currentPaths);
    }

    // Rebuild tag index
    const newTagIndex = buildTagIndex(cache);
    setStore("tagIndex", newTagIndex);
  });

  return { store };
}

// ---------------------------------------------------------------------------
// SolidJS Context
// ---------------------------------------------------------------------------

const NoteCtx = createContext<NoteContext>();

export const NoteContextProvider: ParentComponent<{ context: NoteContext }> = (props) => {
  return (
    <NoteCtx.Provider value={props.context}>
      {props.children}
    </NoteCtx.Provider>
  );
};

export function useNoteContext(): NoteContext {
  const ctx = useContext(NoteCtx);
  if (!ctx) throw new Error("useNoteContext: no NoteContextProvider found");
  return ctx;
}
```

- [ ] **Step 2: Verify it compiles**

Run: `cd frontend && npx tsc --noEmit 2>&1 | grep note-context`

Expected: No errors from the new file.

- [ ] **Step 3: Commit**

```bash
git add frontend/src/lib/note-context.ts
git commit -m "feat: add NoteContext reactive store

SolidJS createStore wrapping VaultCache data with granular per-note
reactivity. Hydrates from cache at init, subscribes to
metadata-cache-updated for incremental updates."
```

---

### Task 3: Wire NoteContext into App Bootstrap

**Files:**
- Modify: `frontend/src/shell/App.tsx`
- Modify: `frontend/src/app/bootstrap.ts`

- [ ] **Step 1: Create and export the NoteContext in bootstrap**

In `frontend/src/app/bootstrap.ts`, after `await _app.metadataCache.init();` (line 126), add:

```typescript
import { createNoteContext, type NoteContext } from "../lib/note-context";
import { vaultCache } from "../lib/cache-sync";
```

Add at module level:

```typescript
let _noteContext: NoteContext;

export function getNoteContext(): NoteContext {
  return _noteContext;
}
```

In `initApp()`, after line 126 (`await _app.metadataCache.init();`), add:

```typescript
_noteContext = createNoteContext(vaultCache);
```

- [ ] **Step 2: Wrap App in NoteContextProvider**

In `frontend/src/shell/App.tsx`, add imports:

```typescript
import { NoteContextProvider } from "../lib/note-context";
import { getNoteContext } from "../app/bootstrap";
```

Wrap the app's root JSX in `<NoteContextProvider context={getNoteContext()}>...</NoteContextProvider>`.

- [ ] **Step 3: Build and verify**

Run: `cd frontend && npx tsc --noEmit 2>&1 | grep -v vitest | grep -v EditorPane | head -20`

Expected: No new errors.

- [ ] **Step 4: Commit**

```bash
git add frontend/src/app/bootstrap.ts frontend/src/shell/App.tsx
git commit -m "feat: wire NoteContext store into app bootstrap

Store hydrates from VaultCache after cache init, before any component
renders. NoteContextProvider wraps the app root."
```

---

### Task 4: Migrate TagsPanel to NoteContext Store

**Files:**
- Modify: `frontend/src/shell/TagsPanel.tsx`

- [ ] **Step 1: Rewrite TagsPanel to use the store**

Replace imports and data loading:

```typescript
import { Component, createSignal, For, Show } from "solid-js";
import { useNoteContext } from "../lib/note-context";
import { getApp } from "../app/bootstrap";
import { triggerSearchWithQuery } from "./search-signals";
```

Remove the `NoteDto` import and `listNotes` import.

Replace `buildTagTree` to accept `Record<string, string[]>` (tagIndex):

```typescript
function buildTagTree(tagIndex: Record<string, string[]>): TagNode[] {
  const root: TagNode[] = [];
  const nodeMap = new Map<string, TagNode>();

  const allTags = Object.keys(tagIndex).sort();

  for (const tag of allTags) {
    const segments = tag.split("/");
    let parent: TagNode[] = root;
    let fullPath = "";

    for (let i = 0; i < segments.length; i++) {
      fullPath = i === 0 ? segments[i] : `${fullPath}/${segments[i]}`;
      let node = nodeMap.get(fullPath);
      if (!node) {
        node = {
          name: segments[i],
          fullTag: fullPath,
          count: 0,
          totalCount: 0,
          children: [],
        };
        nodeMap.set(fullPath, node);
        parent.push(node);
      }
      parent = node.children;
    }
  }

  // Set counts from tagIndex
  for (const [tag, paths] of Object.entries(tagIndex)) {
    const node = nodeMap.get(tag);
    if (node) node.count = paths.length;
  }

  // Calculate total counts (self + descendants)
  function calcTotal(node: TagNode): number {
    node.totalCount = node.count + node.children.reduce((sum, c) => sum + calcTotal(c), 0);
    return node.totalCount;
  }
  for (const node of root) calcTotal(node);

  return root;
}
```

Replace the component's data signals:

```typescript
const TagsPanel: Component<TagsPanelProps> = (props) => {
  const { store } = useNoteContext();
  const [expanded, setExpanded] = createSignal<Set<string>>(new Set());

  const tagTree = () => buildTagTree(store.tagIndex);
  const totalTags = () => Object.keys(store.tagIndex).length;
```

Remove the `notes` signal, `onMount`, and old `totalTags` computation entirely. The rest of the component (rendering, expand/collapse, search) stays the same.

- [ ] **Step 2: Build and verify**

Run: `cd frontend && npx tsc --noEmit 2>&1 | grep -v vitest | grep -v EditorPane | head -20`

Expected: No new errors.

- [ ] **Step 3: Commit**

```bash
git add frontend/src/shell/TagsPanel.tsx
git commit -m "feat: migrate TagsPanel to NoteContext store

Reads tagIndex from the reactive store instead of calling listNotes()
on mount. Tag tree now updates automatically when any note's tags
change."
```

---

### Task 5: Migrate OutlinePanel to NoteContext Store

**Files:**
- Modify: `frontend/src/shell/OutlinePanel.tsx`

- [ ] **Step 1: Rewrite OutlinePanel to use the store**

Replace the entire component:

```typescript
import { Component, createMemo, Show, For } from "solid-js";
import { useNoteContext } from "../lib/note-context";

interface OutlinePanelProps {
  path?: string | null;
  onNavigate?: (path: string) => void;
}

interface HeadingItem {
  level: number;
  text: string;
  line: number;
}

const OutlinePanel: Component<OutlinePanelProps> = (props) => {
  const { store } = useNoteContext();

  const headings = createMemo((): HeadingItem[] => {
    if (!props.path) return [];
    return store.noteMeta[props.path]?.headings ?? [];
  });

  const scrollToHeading = (line: number) => {
    window.dispatchEvent(new CustomEvent("rustsidian:scroll-to-line", { detail: { line } }));
  };

  return (
    <div style={{
      width: "100%",
      height: "100%",
      display: "flex",
      "flex-direction": "column",
      background: "var(--bg-secondary)",
      "overflow-y": "hidden",
    }}>
      <div class="sidebar-toolbar">
        <span style={{
          "font-size": "0.65rem",
          "font-weight": "600",
          "text-transform": "uppercase",
          "letter-spacing": "0.06em",
          color: "var(--text-muted)",
          "padding-left": "0.25rem",
        }}>Outline <span style={{ "font-weight": "normal" }}>{headings().length}</span></span>
      </div>

      <div style={{ flex: 1, "overflow-y": "auto" }}>
        <Show when={headings().length === 0}>
          <div style={{
            padding: "2rem 1rem",
            "text-align": "center",
            "font-size": "0.78rem",
            color: "var(--text-muted)",
          }}>
            No headings
          </div>
        </Show>

        <For each={headings()}>
          {(h) => (
            <button
              class={`outline-item outline-item--h${Math.min(h.level, 6)}`}
              style={{ "padding-left": `${0.65 + (h.level - 1) * 0.6}rem` }}
              onClick={() => scrollToHeading(h.line)}
              title={`Line ${h.line}`}
            >
              {h.text}
            </button>
          )}
        </For>
      </div>
    </div>
  );
};

export default OutlinePanel;
```

- [ ] **Step 2: Build and verify**

Run: `cd frontend && npx tsc --noEmit 2>&1 | grep -v vitest | grep -v EditorPane | head -20`

Expected: No new errors.

- [ ] **Step 3: Commit**

```bash
git add frontend/src/shell/OutlinePanel.tsx
git commit -m "feat: migrate OutlinePanel to NoteContext store

Reads headings from reactive store instead of manually tracking
cacheVersion signal. Drops vaultCache and appEventBus imports."
```

---

### Task 6: Migrate BacklinksPanel to NoteContext Store

BacklinksPanel needs context strings (surrounding text) for linked mentions, which only the IPC `getBacklinksWithContext` provides. The store has backlink paths but not context. Strategy: use the store for reactivity triggers and path data, keep a targeted IPC call for context enrichment.

**Files:**
- Modify: `frontend/src/shell/BacklinksPanel.tsx`

- [ ] **Step 1: Rewrite BacklinksPanel to use store + targeted IPC**

```typescript
import { Component, createSignal, createEffect, createMemo, For, Show } from "solid-js";
import { getBacklinksWithContext, type BacklinkDto } from "../lib/tauri";
import { useNoteContext } from "../lib/note-context";

interface BacklinksPanelProps {
  path?: string | null;
  onNavigate?: (path: string) => void;
}

interface UnlinkedMention {
  path: string;
  title: string;
  context: string;
}

const BacklinksPanel: Component<BacklinksPanelProps> = (props) => {
  const { store } = useNoteContext();
  const [linkedMentions, setLinkedMentions] = createSignal<BacklinkDto[]>([]);
  const [unlinkedMentions, setUnlinkedMentions] = createSignal<UnlinkedMention[]>([]);
  const [linkedExpanded, setLinkedExpanded] = createSignal(true);
  const [unlinkedExpanded, setUnlinkedExpanded] = createSignal(false);
  const [showUnlinked, setShowUnlinked] = createSignal(true);
  const [loading, setLoading] = createSignal(false);

  const getNoteTitle = (path: string): string => {
    const name = path.split("/").pop() ?? "";
    return name.replace(/\.(mdx?|txt)$/, "");
  };

  // Reactive: store backlinks for active path (triggers when backlinks change)
  const storeBacklinks = createMemo(() => {
    const p = props.path;
    if (!p) return [];
    return store.noteMeta[p]?.backlinks ?? [];
  });

  // When store backlinks change OR path changes, fetch context via IPC
  createEffect(async () => {
    const p = props.path;
    const _backlinks = storeBacklinks(); // track reactively
    if (!p) {
      setLinkedMentions([]);
      setUnlinkedMentions([]);
      return;
    }

    setLoading(true);
    try {
      // Fetch linked mentions with context from backend
      const backlinks = await getBacklinksWithContext(p);
      setLinkedMentions(backlinks ?? []);

      // Unlinked mentions — scan store for title matches
      if (showUnlinked()) {
        const noteTitle = getNoteTitle(p);
        if (noteTitle.length < 2) {
          setUnlinkedMentions([]);
        } else {
          const linkedPaths = new Set((backlinks ?? []).map((b) => b.source_path));
          const mentions: UnlinkedMention[] = [];
          const titleLower = noteTitle.toLowerCase();

          for (const [notePath, meta] of Object.entries(store.noteMeta)) {
            if (notePath === p || linkedPaths.has(notePath)) continue;
            if (meta.title.toLowerCase().includes(titleLower)) {
              mentions.push({
                path: notePath,
                title: meta.title,
                context: "",
              });
            }
            if (mentions.length >= 50) break;
          }

          setUnlinkedMentions(mentions);
        }
      }
    } catch {
      setLinkedMentions([]);
      setUnlinkedMentions([]);
    } finally {
      setLoading(false);
    }
  });

  // ... rest of JSX is identical to current implementation
```

The JSX return block stays exactly the same as the current `BacklinksPanel.tsx` lines 84-195.

- [ ] **Step 2: Build and verify**

Run: `cd frontend && npx tsc --noEmit 2>&1 | grep -v vitest | grep -v EditorPane | head -20`

Expected: No new errors.

- [ ] **Step 3: Commit**

```bash
git add frontend/src/shell/BacklinksPanel.tsx
git commit -m "feat: migrate BacklinksPanel to NoteContext store

Uses store backlinks for reactivity trigger. Still fetches context
strings via IPC since store only has paths. Unlinked mentions now
scan store metadata instead of calling listNotes()."
```

---

### Task 7: Migrate OutgoingLinksPanel to NoteContext Store

**Files:**
- Modify: `frontend/src/shell/OutgoingLinksPanel.tsx`

- [ ] **Step 1: Rewrite OutgoingLinksPanel to use the store**

```typescript
import { Component, createSignal, createMemo, For, Show } from "solid-js";
import { useNoteContext } from "../lib/note-context";

interface OutgoingLinksPanelProps {
  path?: string | null;
  onNavigate?: (path: string) => void;
}

interface ResolvedLink {
  target: string;
  display: string;
  resolved: boolean;
  resolvedPath: string | null;
}

interface UnlinkedSuggestion {
  path: string;
  title: string;
}

const OutgoingLinksPanel: Component<OutgoingLinksPanelProps> = (props) => {
  const { store } = useNoteContext();
  const [linkedExpanded, setLinkedExpanded] = createSignal(true);
  const [unlinkedExpanded, setUnlinkedExpanded] = createSignal(false);

  // Outgoing link targets from store (reactive)
  const outgoingTargets = createMemo(() => {
    const p = props.path;
    if (!p) return [];
    return store.noteMeta[p]?.outgoingLinks ?? [];
  });

  // Resolve targets against known paths in the store
  const resolvedLinks = createMemo((): ResolvedLink[] => {
    const targets = outgoingTargets();
    const pathSet = new Set(store.paths);
    const noteByName = new Map<string, string>();
    for (const [notePath, meta] of Object.entries(store.noteMeta)) {
      const name = notePath.split("/").pop()?.replace(/\.(mdx?|txt)$/, "") ?? "";
      noteByName.set(name.toLowerCase(), notePath);
    }

    return targets.map((target) => {
      // Direct path match
      if (pathSet.has(target)) {
        return { target, display: target, resolved: true, resolvedPath: target };
      }
      // Wikilink name match
      const byName = noteByName.get(target.toLowerCase());
      if (byName) {
        return { target, display: target, resolved: true, resolvedPath: byName };
      }
      // Try with extensions
      for (const ext of [".mdx", ".md"]) {
        const withExt = `notes/${target}${ext}`;
        if (pathSet.has(withExt)) {
          return { target, display: target, resolved: true, resolvedPath: withExt };
        }
      }
      return { target, display: target, resolved: false, resolvedPath: null };
    });
  });

  // Unlinked suggestions — note titles found as text (simplified: title match)
  const unlinkedSuggestions = createMemo((): UnlinkedSuggestion[] => {
    const p = props.path;
    if (!p) return [];
    const currentTitle = store.noteMeta[p]?.title ?? "";
    const linkedTargets = new Set(
      resolvedLinks().filter((r) => r.resolvedPath).map((r) => r.resolvedPath!)
    );

    const suggestions: UnlinkedSuggestion[] = [];
    for (const [notePath, meta] of Object.entries(store.noteMeta)) {
      if (notePath === p || linkedTargets.has(notePath)) continue;
      if (meta.title.length < 3) continue;
      // Check if this note's title appears in the current note's outgoing context
      // Since we don't have full content in store, suggest notes not yet linked
      // whose titles are short enough to be plausible mentions
      suggestions.push({ path: notePath, title: meta.title });
      if (suggestions.length >= 30) break;
    }
    return suggestions;
  });

  const linkedCount = () => resolvedLinks().filter((l) => l.resolved).length;
  const unresolvedCount = () => resolvedLinks().filter((l) => !l.resolved).length;

  // ... JSX return is identical to current implementation lines 123-229
```

The JSX return block stays exactly the same as the current `OutgoingLinksPanel.tsx` lines 123-229, but replace `loading()` checks with direct rendering (no async loading state needed since all data is from the store).

Remove the `<Show when={loading()}>` block and the `<Show when={!loading()}>` wrapper — just render the content directly since it's synchronous now.

- [ ] **Step 2: Build and verify**

Run: `cd frontend && npx tsc --noEmit 2>&1 | grep -v vitest | grep -v EditorPane | head -20`

Expected: No new errors.

- [ ] **Step 3: Commit**

```bash
git add frontend/src/shell/OutgoingLinksPanel.tsx
git commit -m "feat: migrate OutgoingLinksPanel to NoteContext store

Resolves outgoing links against store paths. No IPC calls needed.
Removes async loading state since all data is synchronous from store."
```

---

### Task 8: Make FileTree Self-Managing

**Files:**
- Modify: `frontend/src/shell/FileTree.tsx:17-23, 114-120, 408-441`
- Modify: `frontend/src/shell/Sidebar.tsx:30, 51, 118, 143-148`

- [ ] **Step 1: Update FileTree to subscribe to vault events**

In `frontend/src/shell/FileTree.tsx`, update the imports:

```typescript
import { Component, createSignal, createEffect, For, Show, onMount, onCleanup } from "solid-js";
import { appEventBus } from "../lib/app-events";
```

Update the props interface — remove `refreshKey`, add `collapseAll`:

```typescript
interface FileTreeProps {
  onNoteSelect: (path: string) => void;
  currentNote: string | null;
  collapseAll?: boolean;
  sortOrder?: SortOrder;
  bookmarkedPaths?: string[];
}
```

In the `TreeNode` component, update the collapse effect (lines 114-120) to use `collapseAll`:

```typescript
  createEffect(() => {
    const sig = props.collapseSignal;
    if (sig) {
      setExpanded(props.depth === 0);
    }
  });
```

Update `TreeNode`'s `collapseSignal` prop type from `number` to `boolean` in the `TreeNodeProps` interface.

In the `FileTree` component (around line 408), update the root component:

```typescript
const FileTree: Component<FileTreeProps> = (props) => {
  const [rootEntries, setRootEntries] = createSignal<TreeEntry[]>([]);
  const [bgMenu, setBgMenu] = createSignal<{ x: number; y: number } | null>(null);
  const bookmarked = () => getBookmarkedPaths();
  const sortOrder = () => props.sortOrder ?? "name-asc";

  const loadRoot = async () => {
    try {
      const listing = await listDirectory("");
      const entries: TreeEntry[] = [
        ...listing.directories.map((d) => ({
          name: d.split("/").pop() || d,
          path: d,
          isDir: true,
        })),
        ...listing.files.map((f) => ({
          name: f.split("/").pop() || f,
          path: f,
          isDir: false,
        })),
      ];
      setRootEntries(sortEntries(entries, sortOrder(), bookmarked()));
    } catch {
      setRootEntries([]);
    }
  };

  onMount(loadRoot);

  // Auto-refresh on vault file structure changes
  const cleanupCreated = appEventBus.on("vault-file-created", () => loadRoot());
  const cleanupDeleted = appEventBus.on("vault-file-deleted", () => loadRoot());
  onCleanup(() => { cleanupCreated(); cleanupDeleted(); });
```

Remove the old `refreshKey` effect (lines 438-441).

Update `collapseSignal` prop passed to child `TreeNode`s from `props.refreshKey ?? 0` to `props.collapseAll ?? false`.

- [ ] **Step 2: Update Sidebar to remove refreshKey management**

In `frontend/src/shell/Sidebar.tsx`:

Replace the `treeRefreshKey` signal with a `collapseAll` toggle:

```typescript
const [collapseAll, setCollapseAll] = createSignal(false);
```

Remove `refreshTree` function (line 51). Remove `refreshTree()` calls after `createNote` and `createDirectory` (lines 78, 96) — FileTree now self-refreshes via vault events.

Update the collapse-all button (line 118):

```typescript
onClick={() => setCollapseAll((v) => !v)}
```

Update the FileTree usage (lines 143-148):

```typescript
<FileTree
  onNoteSelect={props.onNoteSelect}
  currentNote={props.currentNote}
  collapseAll={collapseAll()}
  sortOrder={sortOrder()}
/>
```

- [ ] **Step 3: Build and verify**

Run: `cd frontend && npx tsc --noEmit 2>&1 | grep -v vitest | grep -v EditorPane | head -20`

Expected: No new errors.

- [ ] **Step 4: Commit**

```bash
git add frontend/src/shell/FileTree.tsx frontend/src/shell/Sidebar.tsx
git commit -m "feat: make FileTree self-managing via vault events

FileTree subscribes to vault-file-created/deleted and refreshes
automatically. Drops refreshKey prop. Collapse-all is now a
dedicated boolean prop instead of overloaded negative counter."
```

---

### Task 9: Add Scroll Preservation to Right Sidebar

**Files:**
- Modify: `frontend/src/app/containers/RightSidebarContainer.tsx`

- [ ] **Step 1: Add scroll position tracking**

```typescript
import { Component, For, Show, onCleanup, createEffect } from "solid-js";
import { Dynamic } from "solid-js/web";
import { getApp } from "../bootstrap";
import ResizeHandle from "../../components/ResizeHandle";

// Track scroll positions per panel
const scrollPositions = new Map<string, number>();

const RightSidebarContainer: Component<RightSidebarContainerProps> = (props) => {
  const app = getApp();
  let contentRef: HTMLDivElement | undefined;

  const tabs = () => app.rightSidebar.getAll();
  const activeId = () => app.rightSidebar.getActive();

  const activePanel = () => {
    const id = activeId();
    if (!id) return null;
    return tabs().find((p) => p.id === id) ?? null;
  };

  const sharedProps = () => ({
    path: app.workspace.getActiveNotePath() ?? null,
    onNavigate: (path: string) => app.workspace.openTab(path),
    onClose: () => app.rightSidebar.setActive(null),
  });

  // Save scroll position when switching away from a panel
  let prevActiveId: string | null = null;
  createEffect(() => {
    const current = activeId();
    if (prevActiveId && prevActiveId !== current && contentRef) {
      scrollPositions.set(prevActiveId, contentRef.scrollTop);
    }
    prevActiveId = current ?? null;

    // Restore scroll position for new panel (defer to next frame)
    if (current && contentRef) {
      requestAnimationFrame(() => {
        if (contentRef) {
          contentRef.scrollTop = scrollPositions.get(current) ?? 0;
        }
      });
    }
  });
```

Update the content div to capture the ref:

```typescript
        <div ref={contentRef} style={{ flex: 1, overflow: "hidden" }}>
```

Wait — the content div has `overflow: hidden`, so panels handle their own scrolling. The scroll position needs to be tracked per-panel at their own scroll container level. Since panels are re-rendered by `Dynamic`, their internal scroll state is lost.

Instead, change the approach: keep panels mounted but hidden. Replace `Dynamic` with rendering all panels and showing/hiding via CSS:

```typescript
        {/* All panels rendered, only active visible */}
        <For each={tabs()}>
          {(panel) => (
            <div style={{
              display: activeId() === panel.id ? "contents" : "none",
              height: "100%",
            }}>
              <Dynamic component={panel.component} {...sharedProps()} />
            </div>
          )}
        </For>
```

This keeps all panels mounted — scroll position is preserved natively since the DOM isn't destroyed.

- [ ] **Step 2: Build and verify**

Run: `cd frontend && npx tsc --noEmit 2>&1 | grep -v vitest | grep -v EditorPane | head -20`

Expected: No new errors.

- [ ] **Step 3: Commit**

```bash
git add frontend/src/app/containers/RightSidebarContainer.tsx
git commit -m "feat: preserve scroll position across sidebar tab switches

Render all panels and hide inactive ones via CSS display:none instead
of unmounting. Scroll state is preserved natively since DOM persists."
```

---

### Task 10: Manual Verification

- [ ] **Step 1: Start the dev server**

Run: `cargo tauri dev`

- [ ] **Step 2: Verify TagsPanel updates reactively**

1. Open the Tags panel in the left sidebar
2. Edit a note and add a new tag in frontmatter
3. Save the note
4. Verify the tag appears in the Tags panel without manual refresh

- [ ] **Step 3: Verify FileTree refreshes on file changes**

1. Create a new note via the sidebar "New note" button
2. Verify it appears in the file tree immediately
3. Delete a note via the file tree context menu
4. Verify it disappears from the tree immediately

- [ ] **Step 4: Verify right sidebar panels update**

1. Open a note and check the Outline panel — headings should show
2. Edit the note to add a heading, save — Outline should update
3. Switch to Backlinks panel — backlinks should show
4. Switch to another panel and back — scroll position should be preserved

- [ ] **Step 5: Verify no regressions**

1. File tree expand/collapse works
2. Collapse-all button works
3. Note creation from sidebar still works
4. Tag search from TagsPanel works
5. Backlink navigation works
6. Outgoing link resolution works
