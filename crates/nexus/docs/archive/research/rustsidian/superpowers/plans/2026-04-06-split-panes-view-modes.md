> **Archived 2026-09-08** — Imported verbatim from the private `baileyrd/Rustsidian` repo (`docs/`) under RFC 0009 row 6. Rustsidian is retired; this is reference material only and describes Rustsidian's SvelteKit/Tauri stack, not Nexus.

# Split Panes & View Mode Switching Implementation Plan

> **For agentic workers:** REQUIRED SUB-SKILL: Use superpowers:subagent-driven-development (recommended) or superpowers:executing-plans to implement this plan task-by-task. Steps use checkbox (`- [ ]`) syntax for tracking.

**Goal:** Wire the existing WorkspaceStore into the UI as the sole tab owner, replace ViewContainer with WorkspaceRenderer, add per-tab view mode switching (live/editor/split), and connect resize handles.

**Architecture:** WorkspaceAPI facade rewired from app-store to workspace-store. App.tsx swaps ViewContainer for WorkspaceRenderer. Editor plugin factory returns a mode-aware wrapper. Status bar gets a view mode toggle. ResizeHandle callbacks wired to setSplitSizes.

**Tech Stack:** SolidJS, existing workspace-store, existing ResizeHandle component, CodeMirror 6 (editor components)

**Spec:** `docs/superpowers/specs/2026-04-06-split-panes-view-modes-design.md`

---

## File Map

| Action | File | Responsibility |
|--------|------|---------------|
| Modify | `frontend/src/lib/workspace-store.ts` | Add `viewMode` and `editorState` to TabState, add nextTab/prevTab/viewMode helpers |
| Modify | `frontend/src/app/registries/WorkspaceAPI.ts` | Rewire facade from app-store to workspace-store |
| Modify | `frontend/src/app/core/App.ts` | Pass workspace-store instance to WorkspaceAPI factory |
| Modify | `frontend/src/store/app-store.ts` | Remove tab state and tab functions |
| Modify | `frontend/src/shell/App.tsx` | Replace ViewContainer with WorkspaceRenderer |
| Modify | `frontend/src/shell/LeafPane.tsx` | Add view resolution logic from ViewContainer |
| Modify | `frontend/src/shell/WorkspaceRenderer.tsx` | Wire resize handle callbacks |
| Modify | `frontend/src/shell/TabBar.tsx` | Read from workspace store instead of appState |
| Modify | `frontend/src/plugins/core/editor.ts` | Mode-aware factory returning wrapper component |
| Create | `frontend/src/editor/EditorModeWrapper.tsx` | Wrapper that switches editor by viewMode |
| Modify | `frontend/src/shell/StatusBar.tsx` | Add view mode toggle button |
| Modify | `frontend/src/plugins/core/tab-navigation.ts` | Add Ctrl+E cycle-modes command |
| Modify | `frontend/src/editor/EditorPane.tsx` | Remove direct app-store imports |
| Delete | `frontend/src/app/containers/ViewContainer.tsx` | Replaced by WorkspaceRenderer + LeafPane |

---

### Task 1: Extend TabState in WorkspaceStore

**Files:**
- Modify: `frontend/src/lib/workspace-store.ts:1-10, 88-101, 105-128, 173-175, 263-270`

- [ ] **Step 1: Add viewMode and editorState to TabState**

In `workspace-store.ts`, add import and update `TabState`:

```typescript
import { createSignal } from "solid-js";
import type { EditorState } from "@codemirror/state";

export type ViewMode = "editor" | "preview" | "split" | "live" | "graph" | "table" | "canvas";

export interface TabState {
  path: string;
  title: string;
  dirty: boolean;
  pinned: boolean;
  viewType: string;
  scrollTop: number;
  viewMode: ViewMode;
  editorState: EditorState | null;
}
```

- [ ] **Step 2: Update serialize() to exclude editorState**

Replace the `serialize()` method:

```typescript
serialize() {
  const s = getState();
  const strip = (node: WorkspaceNode): any => {
    if (node.type === "leaf") {
      return {
        ...node,
        tabs: node.tabs.map(({ editorState, ...rest }) => rest),
      };
    }
    return { ...node, children: node.children.map(strip) };
  };
  return JSON.stringify({ root: strip(s.root), activeLeafId: s.activeLeafId });
},
```

- [ ] **Step 3: Add nextTab, prevTab, and view mode methods to the store interface and implementation**

Add to the `WorkspaceStore` interface:

```typescript
nextTab(): void;
prevTab(): void;
setViewModeInLeaf(leafId: string, tabIndex: number, mode: ViewMode): void;
saveEditorStateInLeaf(leafId: string, tabIndex: number, editorState: EditorState, scrollTop: number): void;
closeOtherTabsInLeaf(leafId: string, keepIndex: number): void;
closeTabsToRightInLeaf(leafId: string, index: number): void;
```

Add implementations:

```typescript
nextTab() {
  update((s) => {
    const leaf = findNode(s.root, s.activeLeafId) as LeafNode | null;
    if (!leaf || leaf.type !== "leaf" || leaf.tabs.length <= 1) return s;
    leaf.activeTabIndex = (leaf.activeTabIndex + 1) % leaf.tabs.length;
    return s;
  });
},

prevTab() {
  update((s) => {
    const leaf = findNode(s.root, s.activeLeafId) as LeafNode | null;
    if (!leaf || leaf.type !== "leaf" || leaf.tabs.length <= 1) return s;
    leaf.activeTabIndex = (leaf.activeTabIndex - 1 + leaf.tabs.length) % leaf.tabs.length;
    return s;
  });
},

setViewModeInLeaf(leafId, tabIndex, mode) {
  update((s) => {
    const leaf = findNode(s.root, leafId) as LeafNode | null;
    if (!leaf || leaf.type !== "leaf" || !leaf.tabs[tabIndex]) return s;
    leaf.tabs[tabIndex].viewMode = mode;
    return s;
  });
},

saveEditorStateInLeaf(leafId, tabIndex, editorState, scrollTop) {
  update((s) => {
    const leaf = findNode(s.root, leafId) as LeafNode | null;
    if (!leaf || leaf.type !== "leaf" || !leaf.tabs[tabIndex]) return s;
    leaf.tabs[tabIndex].editorState = editorState;
    leaf.tabs[tabIndex].scrollTop = scrollTop;
    return s;
  });
},

closeOtherTabsInLeaf(leafId, keepIndex) {
  update((s) => {
    const leaf = findNode(s.root, leafId) as LeafNode | null;
    if (!leaf || leaf.type !== "leaf") return s;
    const kept = leaf.tabs.filter((t, i) => i === keepIndex || t.pinned);
    const newActive = kept.findIndex((t) => t === leaf.tabs[keepIndex]);
    leaf.tabs.splice(0, leaf.tabs.length, ...kept);
    leaf.activeTabIndex = newActive >= 0 ? newActive : 0;
    return s;
  });
},

closeTabsToRightInLeaf(leafId, index) {
  update((s) => {
    const leaf = findNode(s.root, leafId) as LeafNode | null;
    if (!leaf || leaf.type !== "leaf") return s;
    const right = leaf.tabs.slice(index + 1);
    const pinned = right.filter((t) => t.pinned);
    leaf.tabs.splice(index + 1, right.length, ...pinned);
    if (leaf.activeTabIndex >= leaf.tabs.length) {
      leaf.activeTabIndex = leaf.tabs.length - 1;
    }
    return s;
  });
},
```

- [ ] **Step 4: Update openTabInLeaf to set default viewMode and editorState**

In the existing `openTabInLeaf` method, the `tab` parameter already comes from the caller. But ensure the `createLeaf` helper and any default tab creation includes the new fields. Update `createLeaf`:

```typescript
function createLeaf(tabs: TabState[] = [], activeTabIndex = 0): LeafNode {
  return { type: "leaf", id: genId(), tabs, activeTabIndex };
}
```

No change needed — the caller constructs the full `TabState`.

- [ ] **Step 5: Build and verify**

Run: `cd frontend && npx tsc --noEmit 2>&1 | grep workspace-store | head -10`

Expected: No errors from workspace-store.ts.

- [ ] **Step 6: Commit**

```bash
git add frontend/src/lib/workspace-store.ts
git commit -m "feat: extend TabState with viewMode, editorState, and new operations

Adds per-tab viewMode and editorState to workspace store. Serialize
excludes non-serializable editorState. Adds nextTab, prevTab,
viewMode setter, closeOthers, closeToRight operations.

Co-Authored-By: Claude Opus 4.6 (1M context) <noreply@anthropic.com>"
```

---

### Task 2: Rewire WorkspaceAPI Facade

**Files:**
- Modify: `frontend/src/app/registries/WorkspaceAPI.ts`
- Modify: `frontend/src/app/core/App.ts:33-49`

- [ ] **Step 1: Rewrite WorkspaceAPI to delegate to workspace-store**

Replace `WorkspaceAPI.ts` entirely:

```typescript
import { createWorkspaceStore, type WorkspaceStore, type TabState, type ViewMode } from "../../lib/workspace-store";
import { appState } from "../../store/app-store";
import type { EditorState } from "@codemirror/state";

export { type ViewMode } from "../../lib/workspace-store";

export interface Tab {
  path: string;
  title: string;
  dirty: boolean;
  pinned: boolean;
  editorState: EditorState | null;
  viewMode: ViewMode;
  scrollTop: number;
}

function tabStateToTab(t: TabState): Tab {
  return {
    path: t.path,
    title: t.title,
    dirty: t.dirty,
    pinned: t.pinned,
    editorState: t.editorState,
    viewMode: t.viewMode,
    scrollTop: t.scrollTop,
  };
}

function titleFromPath(path: string): string {
  const name = path.split("/").pop() || path;
  return name.replace(/\.(mdx|canvas)$/, "");
}

export interface WorkspaceAPI {
  /** Workspace store instance for direct access in WorkspaceRenderer. */
  store: WorkspaceStore;
  getActiveTab(): Tab | null;
  getActiveNotePath(): string | null;
  getTabs(): Tab[];
  getActiveTabIndex(): number;
  openTab(path: string): void;
  closeTab(index: number): void;
  activateTab(index: number): void;
  newTab(): void;
  pinTab(index: number): void;
  moveTab(fromIndex: number, toIndex: number): void;
  closeOtherTabs(index: number): void;
  closeTabsToRight(index: number): void;
  setViewMode(mode: ViewMode): void;
  getViewMode(): ViewMode;
  nextTab(): void;
  prevTab(): void;
  notifySaved(): void;
  setTabDirty(index: number, dirty: boolean): void;
  saveTabEditorState(index: number, state: EditorState, scrollTop: number): void;
  zoomIn(): void;
  zoomOut(): void;
  zoomReset(): void;
  getZoomLevel(): number;
  splitLeaf(leafId: string, direction: "horizontal" | "vertical"): void;
  closeLeaf(leafId: string): void;
}

export function createWorkspaceAPI(): WorkspaceAPI {
  const ws = createWorkspaceStore();

  // Import zoom and save functions from app-store (non-tab state stays there)
  const { zoomIn, zoomOut, zoomReset, notifySaved, saveVersion } = await import("../../store/app-store").then(m => m);

  // Helper: get active leaf ID
  const activeLeafId = () => ws.state().activeLeafId;

  return {
    store: ws,

    getActiveTab() {
      const tab = ws.getActiveTab();
      return tab ? tabStateToTab(tab) : null;
    },

    getActiveNotePath() {
      return ws.getActiveNotePath();
    },

    getTabs() {
      const leaf = ws.getActiveLeaf();
      return leaf ? leaf.tabs.map(tabStateToTab) : [];
    },

    getActiveTabIndex() {
      return ws.getActiveLeaf()?.activeTabIndex ?? -1;
    },

    openTab(path: string) {
      const tab: TabState = {
        path,
        title: titleFromPath(path),
        dirty: false,
        pinned: false,
        viewType: "markdown",
        scrollTop: 0,
        viewMode: "live",
        editorState: null,
      };
      ws.openTabInLeaf(activeLeafId(), tab);
    },

    closeTab(index: number) {
      ws.closeTabInLeaf(activeLeafId(), index);
    },

    activateTab(index: number) {
      ws.setActiveTabInLeaf(activeLeafId(), index);
    },

    newTab() {
      const tab: TabState = {
        path: "",
        title: "New tab",
        dirty: false,
        pinned: false,
        viewType: "markdown",
        scrollTop: 0,
        viewMode: "live",
        editorState: null,
      };
      ws.openTabInLeaf(activeLeafId(), tab);
    },

    pinTab(index: number) {
      ws.pinTabInLeaf(activeLeafId(), index);
    },

    moveTab(fromIndex: number, toIndex: number) {
      // Workspace store doesn't have intra-leaf reorder — move between leaves
      // For now, this is a no-op for same-leaf reorder. TODO: add reorderTabInLeaf.
      console.warn("moveTab within same leaf not yet implemented in workspace store");
    },

    closeOtherTabs(index: number) {
      ws.closeOtherTabsInLeaf(activeLeafId(), index);
    },

    closeTabsToRight(index: number) {
      ws.closeTabsToRightInLeaf(activeLeafId(), index);
    },

    setViewMode(mode: ViewMode) {
      const leaf = ws.getActiveLeaf();
      if (leaf) {
        ws.setViewModeInLeaf(leaf.id, leaf.activeTabIndex, mode);
      }
    },

    getViewMode() {
      return ws.getActiveTab()?.viewMode ?? "live";
    },

    nextTab() { ws.nextTab(); },
    prevTab() { ws.prevTab(); },

    notifySaved,

    setTabDirty(index: number, dirty: boolean) {
      ws.setDirtyInLeaf(activeLeafId(), index, dirty);
    },

    saveTabEditorState(index: number, editorState: EditorState, scrollTop: number) {
      ws.saveEditorStateInLeaf(activeLeafId(), index, editorState, scrollTop);
    },

    zoomIn() {
      const { zoomIn } = require("../../store/app-store");
      zoomIn();
    },
    zoomOut() {
      const { zoomOut } = require("../../store/app-store");
      zoomOut();
    },
    zoomReset() {
      const { zoomReset } = require("../../store/app-store");
      zoomReset();
    },
    getZoomLevel() {
      return appState.zoomLevel;
    },

    splitLeaf(leafId, direction) { ws.splitLeaf(leafId, direction); },
    closeLeaf(leafId) { ws.closeLeaf(leafId); },
  };
}
```

**Note:** The `await import()` for zoom functions won't work in a sync factory. Instead, import them at the top:

```typescript
import { appState, zoomIn, zoomOut, zoomReset, notifySaved } from "../../store/app-store";
```

And use directly: `zoomIn() { zoomIn(); }`, etc.

- [ ] **Step 2: Update moveTab to handle intra-leaf reorder**

Add `reorderTabInLeaf` to workspace-store.ts:

```typescript
reorderTabInLeaf(leafId: string, fromIndex: number, toIndex: number): void;
```

Implementation:

```typescript
reorderTabInLeaf(leafId, fromIndex, toIndex) {
  if (fromIndex === toIndex) return;
  update((s) => {
    const leaf = findNode(s.root, leafId) as LeafNode | null;
    if (!leaf || leaf.type !== "leaf") return s;
    const [moved] = leaf.tabs.splice(fromIndex, 1);
    if (!moved) return s;
    leaf.tabs.splice(toIndex, 0, moved);
    if (leaf.activeTabIndex === fromIndex) {
      leaf.activeTabIndex = toIndex;
    } else if (fromIndex < leaf.activeTabIndex && toIndex >= leaf.activeTabIndex) {
      leaf.activeTabIndex--;
    } else if (fromIndex > leaf.activeTabIndex && toIndex <= leaf.activeTabIndex) {
      leaf.activeTabIndex++;
    }
    return s;
  });
},
```

Then in WorkspaceAPI's `moveTab`:

```typescript
moveTab(fromIndex: number, toIndex: number) {
  ws.reorderTabInLeaf(activeLeafId(), fromIndex, toIndex);
},
```

- [ ] **Step 3: Build and verify**

Run: `cd frontend && npx tsc --noEmit 2>&1 | grep -v vitest | grep -v EditorPane | head -20`

Expected: Errors from files still importing old app-store tab functions (expected — will fix in later tasks).

- [ ] **Step 4: Commit**

```bash
git add frontend/src/app/registries/WorkspaceAPI.ts frontend/src/lib/workspace-store.ts
git commit -m "feat: rewire WorkspaceAPI facade to workspace-store

All tab operations now delegate to workspace-store instead of
app-store. Interface stays identical — consumers don't change.
Adds store property for direct access by WorkspaceRenderer.

Co-Authored-By: Claude Opus 4.6 (1M context) <noreply@anthropic.com>"
```

---

### Task 3: Strip Tab State from App-Store

**Files:**
- Modify: `frontend/src/store/app-store.ts`

- [ ] **Step 1: Remove all tab-related state and functions**

Keep: `AppState` with only non-tab fields, `togglePalette`, `closePalette`, `toggleSettings`, `closeSettings`, `toggleTerminal`, `toggleAiPanel`, `toggleForgeSwitcher`, `closeForgeSwitcher`, `setForgeName`, zoom functions, `saveVersion`/`notifySaved`.

Remove from `AppState` interface: `tabs`, `activeTabIndex`, `viewMode`.
Remove from `initialState`: `tabs`, `activeTabIndex`, `viewMode`.
Remove functions: `openTab`, `closeTab`, `newTab`, `pinTab`, `moveTab`, `closeOtherTabs`, `closeTabsToRight`, `activateTab`, `setTabDirty`, `saveTabEditorState`, `setTabViewMode`, `activeTabViewMode`, `setViewMode`, `activeTab`, `activeNotePath`, `nextTab`, `prevTab`.
Remove `Tab` interface and `ViewMode` type (now in workspace-store).
Remove `EditorState` import (no longer needed).

Keep the `appState` export for non-tab state consumers (StatusBar reads `zoomLevel`, `terminalOpen`, etc.).

The resulting file should be ~150 lines (down from ~378).

- [ ] **Step 2: Fix remaining exports**

Ensure `appState` is still exported. Ensure zoom functions, toggle functions, `notifySaved`, `saveVersion` are still exported.

Re-export `ViewMode` and `Tab` from WorkspaceAPI for backwards compatibility:

```typescript
// Re-export types that moved to workspace-store
export type { ViewMode, Tab } from "../app/registries/WorkspaceAPI";
```

Wait — this creates a circular dependency. Instead, consumers should import `ViewMode` from workspace-store directly, or from WorkspaceAPI. We'll fix consumer imports in Task 4.

- [ ] **Step 3: Build — expect errors from consumers**

Run: `cd frontend && npx tsc --noEmit 2>&1 | grep -c "error TS"`

Note the count — Task 4 will fix these.

- [ ] **Step 4: Commit**

```bash
git add frontend/src/store/app-store.ts
git commit -m "refactor: strip tab state from app-store

Tab management now lives entirely in workspace-store. App-store
retains only UI flags (modals, zoom, save version). Consumer
imports will be fixed in the next task.

Co-Authored-By: Claude Opus 4.6 (1M context) <noreply@anthropic.com>"
```

---

### Task 4: Fix Consumer Imports

**Files:**
- Modify: `frontend/src/shell/TabBar.tsx:3, 11-12`
- Modify: `frontend/src/editor/EditorPane.tsx` (any direct app-store tab imports)
- Modify: `frontend/src/plugins/core/file-explorer.tsx` (if imports app-store)
- Modify: `frontend/src/app/containers/SidebarContainer.tsx` (if imports app-store tabs)
- Modify: any other file with broken imports

- [ ] **Step 1: Fix TabBar.tsx**

Replace:
```typescript
import { appState } from "../store/app-store";
```

With workspace store access via the app:

```typescript
import { getApp } from "../app/bootstrap";
```

Replace `const tabs = () => appState.tabs;` and `const activeIndex = () => appState.activeTabIndex;` with:

```typescript
const tabs = () => app.workspace.getTabs();
const activeIndex = () => app.workspace.getActiveTabIndex();
```

- [ ] **Step 2: Fix EditorPane.tsx**

Find any imports of `setTabDirty`, `saveTabEditorState`, `appState` from app-store. Replace with `getApp()` and `app.workspace.*` calls.

- [ ] **Step 3: Fix remaining consumer files**

For each file that errors on missing app-store tab exports:
- If it uses `app.workspace.*` already → no change needed
- If it imports tab functions directly from app-store → switch to `getApp().workspace.*`
- If it imports `ViewMode` type → import from `workspace-store` instead

- [ ] **Step 4: Build and verify all import errors resolved**

Run: `cd frontend && npx tsc --noEmit 2>&1 | grep -v vitest | head -20`

Expected: No new errors (pre-existing ones in test files are fine).

- [ ] **Step 5: Commit**

```bash
git add -A
git commit -m "fix: update consumer imports after app-store tab removal

TabBar, EditorPane, and other consumers now read tab state via
app.workspace API instead of direct app-store imports.

Co-Authored-By: Claude Opus 4.6 (1M context) <noreply@anthropic.com>"
```

---

### Task 5: Replace ViewContainer with WorkspaceRenderer in App.tsx

**Files:**
- Modify: `frontend/src/shell/App.tsx:106-108`
- Delete: `frontend/src/app/containers/ViewContainer.tsx`

- [ ] **Step 1: Update App.tsx to render WorkspaceRenderer**

Replace the `<ViewContainer />` line with:

```tsx
import WorkspaceRenderer from "./WorkspaceRenderer";
```

Replace `<ViewContainer />` in the JSX with:

```tsx
{(() => {
  const ws = app.workspace.store;
  const state = ws.state();
  return (
    <WorkspaceRenderer
      node={state.root}
      activeLeafId={state.activeLeafId}
      viewRegistry={app.views}
      onActivateLeaf={(id) => ws.setActiveLeaf(id)}
      onTabSelect={(leafId, index) => ws.setActiveTabInLeaf(leafId, index)}
      onTabClose={(leafId, index) => ws.closeTabInLeaf(leafId, index)}
      onNoteSelect={(path) => app.workspace.openTab(path)}
    />
  );
})()}
```

Remove the `ViewContainer` import.

- [ ] **Step 2: Remove TabBar from App.tsx if it's rendered there**

Check if `TabBar` is rendered in App.tsx. If so, remove it — LeafPane now has its own per-leaf tab bar.

- [ ] **Step 3: Delete ViewContainer.tsx**

```bash
rm frontend/src/app/containers/ViewContainer.tsx
```

- [ ] **Step 4: Build and verify**

Run: `cd frontend && npx tsc --noEmit 2>&1 | grep -v vitest | head -20`

- [ ] **Step 5: Commit**

```bash
git add -A
git commit -m "feat: replace ViewContainer with WorkspaceRenderer in App.tsx

App now renders the split-pane workspace tree. ViewContainer deleted.
Each leaf has its own tab bar via LeafPane.

Co-Authored-By: Claude Opus 4.6 (1M context) <noreply@anthropic.com>"
```

---

### Task 6: Add View Resolution to LeafPane

**Files:**
- Modify: `frontend/src/shell/LeafPane.tsx`

- [ ] **Step 1: Add mode-aware view resolution**

The current LeafPane uses `props.viewRegistry.resolve(tab.path)` which only resolves by file type. It needs the ViewContainer's mode resolution logic.

Update LeafPane's view content section (lines 52-69):

```tsx
{/* View content */}
<div class="flex-1 overflow-hidden">
  <Show when={activeTab()} fallback={
    <div class="empty-state">
      <h2 style={{ color: "var(--accent)" }}>Rustsidian</h2>
      <p>Select a note or press Ctrl+P to search</p>
    </div>
  }>
    {(() => {
      const tab = activeTab()!;
      const mode = tab.viewMode ?? "live";

      // Special view types resolve by type, not path
      if (mode === "graph" || mode === "table" || mode === "canvas") {
        const reg = props.viewRegistry.resolveByType(mode);
        if (reg) {
          return <Dynamic component={reg.factory()} path={tab.path} onNavigate={props.onNoteSelect} />;
        }
      }

      // File-based resolution (editor, markdown, etc.)
      const reg = props.viewRegistry.resolve(tab.path);
      if (reg) {
        return <Dynamic component={reg.factory()} path={tab.path} onNavigate={props.onNoteSelect} />;
      }

      return <div>Unknown file type: {tab.path}</div>;
    })()}
  </Show>
</div>
```

- [ ] **Step 2: Add `resolveByType` to the viewRegistry prop type**

Check that the `ViewRegistry` type imported in LeafPane has `resolveByType`. It's defined in `ViewRegistry.ts` — it does. No change needed.

- [ ] **Step 3: Build and verify**

Run: `cd frontend && npx tsc --noEmit 2>&1 | grep LeafPane`

- [ ] **Step 4: Commit**

```bash
git add frontend/src/shell/LeafPane.tsx
git commit -m "feat: add mode-aware view resolution to LeafPane

LeafPane now resolves views by viewMode (graph/table/canvas by type,
markdown by path). Mirrors the logic that was in ViewContainer.

Co-Authored-By: Claude Opus 4.6 (1M context) <noreply@anthropic.com>"
```

---

### Task 7: Create EditorModeWrapper

**Files:**
- Create: `frontend/src/editor/EditorModeWrapper.tsx`
- Modify: `frontend/src/plugins/core/editor.ts`

- [ ] **Step 1: Create the mode wrapper component**

```tsx
import { Component } from "solid-js";
import { Dynamic } from "solid-js/web";
import { getApp } from "../app/bootstrap";
import MdxWysiwyg from "./MdxWysiwyg";
import EditorPane from "./EditorPane";
import LiveEditor from "./LiveEditor";
import type { ViewProps } from "../app/core/types";

const MODE_COMPONENTS: Record<string, Component<any>> = {
  live: MdxWysiwyg,
  editor: EditorPane,
  split: LiveEditor,
};

const EditorModeWrapper: Component<ViewProps> = (props) => {
  const app = getApp();
  const mode = () => app.workspace.getViewMode();
  const component = () => MODE_COMPONENTS[mode()] ?? MdxWysiwyg;

  return <Dynamic component={component()} path={props.path} onNavigate={props.onNavigate} />;
};

export default EditorModeWrapper;
```

- [ ] **Step 2: Update editor plugin to use the wrapper**

Replace `frontend/src/plugins/core/editor.ts`:

```typescript
import { PluginBase } from "../../app/core/Plugin";
import EditorModeWrapper from "../../editor/EditorModeWrapper";

export default class EditorPlugin extends PluginBase {
  onload() {
    this.registerView({
      type: "markdown",
      displayName: "Markdown",
      icon: null as any,
      factory: () => EditorModeWrapper,
      canOpen: (path: string) => path.endsWith(".md") || path.endsWith(".mdx"),
    });
  }
}
```

- [ ] **Step 3: Build and verify**

Run: `cd frontend && npx tsc --noEmit 2>&1 | grep -i editor | grep -v vitest | head -10`

- [ ] **Step 4: Commit**

```bash
git add frontend/src/editor/EditorModeWrapper.tsx frontend/src/plugins/core/editor.ts
git commit -m "feat: mode-aware editor factory via EditorModeWrapper

Editor plugin now returns a wrapper that reads the active tab's
viewMode and renders MdxWysiwyg (live), EditorPane (editor),
or LiveEditor (split) accordingly.

Co-Authored-By: Claude Opus 4.6 (1M context) <noreply@anthropic.com>"
```

---

### Task 8: Add View Mode Toggle to Status Bar

**Files:**
- Modify: `frontend/src/shell/StatusBar.tsx`
- Modify: `frontend/src/plugins/core/tab-navigation.ts`

- [ ] **Step 1: Add mode toggle button to StatusBar**

In `StatusBar.tsx`, add after the "MDX" label (around line 157):

```tsx
{/* View mode toggle */}
{props.currentNote && (() => {
  const app = getApp();
  const MODE_CYCLE: Record<string, string> = { live: "editor", editor: "split", split: "live" };
  const MODE_LABEL: Record<string, string> = { live: "LIVE", editor: "SOURCE", split: "SPLIT" };
  const mode = () => app.workspace.getViewMode();
  const isEditorMode = () => ["live", "editor", "split"].includes(mode());

  return (
    <Show when={isEditorMode()}>
      <button
        onClick={() => {
          const next = MODE_CYCLE[mode()] ?? "live";
          app.workspace.setViewMode(next as any);
        }}
        style={{
          background: "none",
          border: "1px solid var(--border)",
          "border-radius": "3px",
          color: "var(--accent)",
          cursor: "pointer",
          padding: "0px 6px",
          "font-size": "0.6rem",
          "font-weight": "600",
          "font-family": "var(--font-sans)",
        }}
        title="Toggle view mode (Ctrl+E)"
      >
        {MODE_LABEL[mode()] ?? "LIVE"}
      </button>
    </Show>
  );
})()}
```

Add `getApp` import and `Show` if not already imported.

- [ ] **Step 2: Add Ctrl+E command to tab-navigation plugin**

In `frontend/src/plugins/core/tab-navigation.ts`, add a new command:

```typescript
this.addCommand({
  id: "core:cycle-view-mode",
  name: "Cycle view mode",
  hotkey: "Ctrl+E",
  callback: () => {
    const MODE_CYCLE: Record<string, string> = { live: "editor", editor: "split", split: "live" };
    const current = this.app.workspace.getViewMode();
    const next = MODE_CYCLE[current] ?? "live";
    this.app.workspace.setViewMode(next as any);
  },
});
```

- [ ] **Step 3: Build and verify**

Run: `cd frontend && npx tsc --noEmit 2>&1 | grep -v vitest | head -10`

- [ ] **Step 4: Commit**

```bash
git add frontend/src/shell/StatusBar.tsx frontend/src/plugins/core/tab-navigation.ts
git commit -m "feat: add view mode toggle to status bar and Ctrl+E shortcut

Status bar shows LIVE/SOURCE/SPLIT button for markdown tabs.
Click or Ctrl+E cycles through modes.

Co-Authored-By: Claude Opus 4.6 (1M context) <noreply@anthropic.com>"
```

---

### Task 9: Wire Resize Handles in WorkspaceRenderer

**Files:**
- Modify: `frontend/src/shell/WorkspaceRenderer.tsx:55-62`

- [ ] **Step 1: Add resize callback props and wire them**

Update `WorkspaceRendererProps` to include resize callback:

```typescript
interface WorkspaceRendererProps {
  node: WorkspaceNode;
  activeLeafId: string;
  viewRegistry: ViewRegistry;
  onActivateLeaf: (id: string) => void;
  onTabSelect: (leafId: string, index: number) => void;
  onTabClose: (leafId: string, index: number) => void;
  onNoteSelect: (path: string) => void;
  onResizeSplit?: (splitId: string, sizes: number[]) => void;
}
```

Replace the no-op ResizeHandle (lines 56-61) with:

```tsx
<ResizeHandle
  direction={isHorizontal() ? "horizontal" : "vertical"}
  side={isHorizontal() ? "right" : "bottom"}
  onResize={(delta) => {
    const splitNode = split();
    const container = (document.querySelector(`[data-split-id="${splitNode.id}"]`) as HTMLElement);
    if (!container) return;
    const totalSize = isHorizontal() ? container.offsetWidth : container.offsetHeight;
    if (totalSize === 0) return;

    const deltaFraction = delta / totalSize;
    const i = index();
    const newSizes = [...splitNode.sizes];
    const MIN_FRACTION = 100 / totalSize; // 100px minimum

    newSizes[i] = Math.max(MIN_FRACTION, splitNode.sizes[i] + deltaFraction);
    newSizes[i + 1] = Math.max(MIN_FRACTION, splitNode.sizes[i + 1] - deltaFraction);

    // Normalize
    const total = newSizes.reduce((a, b) => a + b, 0);
    const normalized = newSizes.map((s) => s / total);

    props.onResizeSplit?.(splitNode.id, normalized);
  }}
/>
```

Add `data-split-id` attribute to the split container div:

```tsx
<div
  class="flex-1 overflow-hidden"
  data-split-id={split().id}
  style={{ display: "flex", "flex-direction": isHorizontal() ? "row" : "column" }}
>
```

Pass `onResizeSplit` through recursive WorkspaceRenderer calls.

- [ ] **Step 2: Wire onResizeSplit in App.tsx**

In App.tsx where WorkspaceRenderer is rendered, add:

```tsx
onResizeSplit={(splitId, sizes) => ws.setSplitSizes(splitId, sizes)}
```

- [ ] **Step 3: Build and verify**

Run: `cd frontend && npx tsc --noEmit 2>&1 | grep WorkspaceRenderer`

- [ ] **Step 4: Commit**

```bash
git add frontend/src/shell/WorkspaceRenderer.tsx frontend/src/shell/App.tsx
git commit -m "feat: wire resize handles to setSplitSizes

Drag handles now convert pixel deltas to fractions and update the
workspace store's split sizes. 100px minimum pane size enforced.

Co-Authored-By: Claude Opus 4.6 (1M context) <noreply@anthropic.com>"
```

---

### Task 10: Add Split Context Menu Items

**Files:**
- Modify: `frontend/src/shell/TabBar.tsx` (or LeafPane.tsx tab bar context menu)

- [ ] **Step 1: Add split options to tab context menu**

In `TabBar.tsx`, after the "Pin" context menu item, add:

```tsx
<div class="ctx-menu__divider" />
<button class="ctx-menu__item" onClick={() => ctxAction(() => {
  const ws = app.workspace.store;
  const leafId = ws.state().activeLeafId;
  app.workspace.splitLeaf(leafId, "horizontal");
})}>Split right</button>
<button class="ctx-menu__item" onClick={() => ctxAction(() => {
  const ws = app.workspace.store;
  const leafId = ws.state().activeLeafId;
  app.workspace.splitLeaf(leafId, "vertical");
})}>Split down</button>
```

- [ ] **Step 2: Build and verify**

Run: `cd frontend && npx tsc --noEmit 2>&1 | grep TabBar`

- [ ] **Step 3: Commit**

```bash
git add frontend/src/shell/TabBar.tsx
git commit -m "feat: add split right/down to tab context menu

Tab right-click menu now offers Split right and Split down options
that create new panes via workspace store.

Co-Authored-By: Claude Opus 4.6 (1M context) <noreply@anthropic.com>"
```

---

### Task 11: Manual Verification

- [ ] **Step 1: Start the dev server**

Run: `cargo tauri dev`

- [ ] **Step 2: Verify single-pane behavior (regression)**

1. App opens with single pane — looks identical to before
2. Open a note — renders in the pane
3. Open multiple tabs — tab bar shows them
4. Switch tabs — content changes
5. Close tabs — works, respects pinned

- [ ] **Step 3: Verify split panes**

1. Right-click a tab → "Split right" → two panes appear side by side
2. Right-click in new pane → "Split down" → three panes
3. Click between panes — active pane highlighted
4. Drag resize handle — panes resize
5. Close a pane — remaining panes fill space

- [ ] **Step 4: Verify view mode switching**

1. Open a .mdx note
2. Status bar shows "LIVE" button
3. Click it → switches to "SOURCE" (EditorPane)
4. Click again → "SPLIT" (LiveEditor with preview)
5. Click again → back to "LIVE" (MdxWysiwyg)
6. Press Ctrl+E → cycles modes
7. Open graph view → mode button hidden

- [ ] **Step 5: Verify persistence**

1. Create a split layout
2. Reload the page
3. Layout persists (same splits, same tabs)

- [ ] **Step 6: Verify no regressions**

1. Command palette works
2. Left sidebar navigation opens notes
3. Right sidebar panels show data
4. Terminal toggle works
5. Zoom in/out works
