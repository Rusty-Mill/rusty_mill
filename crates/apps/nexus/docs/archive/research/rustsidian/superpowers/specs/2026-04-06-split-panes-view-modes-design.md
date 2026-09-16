> **Archived 2026-09-08** — Imported verbatim from the private `baileyrd/Rustsidian` repo (`docs/`) under RFC 0009 row 6. Rustsidian is retired; this is reference material only and describes Rustsidian's SvelteKit/Tauri stack, not Nexus.

# Split Panes & View Mode Switching

**Date:** 2026-04-06
**Scope:** Backlog items 4 (split panes wiring) and 5 (per-tab view mode switching), treated as one spec since they share the same rendering path.

## Problem

The editor area is a single-pane `ViewContainer` backed by a flat `appState.tabs` array. A fully-implemented `WorkspaceStore` with split/close/move/resize operations exists but is not wired into the UI. Three editor components exist (MdxWysiwyg, EditorPane, LiveEditor) but the editor plugin always returns MdxWysiwyg regardless of the tab's `viewMode`. There is no user-facing UI to switch view modes.

Two tab systems coexist: `app-store.ts` (active, flat) and `workspace-store.ts` (unused, tree-based). `WorkspaceAPI` is a facade that currently delegates to app-store.

## Design

### 1. WorkspaceStore Becomes Sole Tab Owner

**WorkspaceStore's `TabState` gains two fields:**
- `editorState: EditorState | null` — CodeMirror state for preserving cursor/undo within a session
- `viewMode: ViewMode` — per-tab mode (`"live" | "editor" | "split"`)

`serialize()` excludes `editorState` (non-serializable CodeMirror object). On `restore()`, `editorState` is `null` for every tab — editors create fresh state on mount. This matches current behavior.

**WorkspaceAPI rewired as facade:**
The `WorkspaceAPI` interface stays identical. Its implementation in `createWorkspaceAPI()` switches from importing app-store functions to calling workspace-store methods. Consumers using `app.workspace.openTab()`, `app.workspace.closeTab()`, etc. don't change at all.

Key mappings:
- `openTab(path)` → `workspace.openTabInLeaf(activeLeafId, tab)`
- `closeTab(index)` → `workspace.closeTabInLeaf(activeLeafId, index)`
- `activateTab(index)` → `workspace.setActiveTabInLeaf(activeLeafId, index)`
- `getTabs()` → active leaf's `tabs`
- `getActiveTabIndex()` → active leaf's `activeTabIndex`
- `setViewMode(mode)` → set active tab's `viewMode` in workspace store
- `getViewMode()` → read active tab's `viewMode` from workspace store
- `splitLeaf(leafId, direction)`, `closeLeaf(leafId)` — new methods exposed

**App-store loses tab state:**
Remove from `AppState`: `tabs`, `activeTabIndex`, `viewMode`.
Remove all tab management functions: `openTab`, `closeTab`, `activateTab`, `newTab`, `pinTab`, `moveTab`, `closeOtherTabs`, `closeTabsToRight`, `setTabDirty`, `saveTabEditorState`, `setTabViewMode`, `activeTab`, `activeNotePath`, `nextTab`, `prevTab`.
App-store retains: modal flags (`settingsOpen`, `paletteOpen`, `terminalOpen`), `zoomLevel`, `saveVersion`.

**Consumers directly importing from app-store** (TabBar, EditorPane, tab-navigation plugin, etc.) get redirected to use `app.workspace.*` or import from workspace-store.

### 2. App.tsx Renders WorkspaceRenderer

**Current:** `App.tsx` renders `ViewContainer` (single pane).
**After:** `App.tsx` renders `WorkspaceRenderer` backed by the workspace store's tree.

`WorkspaceRenderer` recursively renders:
- `SplitNode` — flex container (row or column) with `ResizeHandle` between children
- `LeafPane` — tab bar + view content for a single leaf

On first load with no persisted layout, the workspace store initializes with a single leaf (identical to current single-pane behavior).

**ViewContainer is deleted.** Its view resolution logic moves into `LeafPane`.

### 3. View Resolution in LeafPane

LeafPane resolves which component to render for the active tab:

```
if viewMode in ["graph", "table", "canvas"]:
    → app.views.resolveByType(viewMode)
else:
    → app.views.resolve(tab.path)  // returns editor registration
    → registration.factory()       // returns mode-aware wrapper
```

The editor plugin's factory returns a wrapper component that reads the tab's `viewMode` and renders:

| viewMode | Component | Description |
|----------|-----------|-------------|
| `live` | MdxWysiwyg | WYSIWYG with hidden syntax |
| `editor` | EditorPane | Raw source with CodeMirror |
| `split` | LiveEditor | Editor + live preview side by side |

Mode switching saves content (if dirty), destroys the current editor, and creates a fresh one. The new editor loads content from the backend. Undo history is not preserved across mode switches — editors always create fresh `EditorState`.

### 4. View Mode Toggle UI

**Keyboard shortcut:** `Ctrl+E` cycles through `live → editor → split → live`. Registered as a command in the command registry. Only active when the current tab is a markdown/mdx file.

**Status bar button:** A clickable label in the bottom status bar showing the current mode (`LIVE`, `SOURCE`, `SPLIT`). Click cycles to the next mode. Hidden when the active tab is not a markdown/mdx file (graph, table, canvas views don't have modes).

### 5. Split Pane Operations

**Tab context menu additions:**
- "Split Right" — `workspace.splitLeaf(leafId, "horizontal")`
- "Split Down" — `workspace.splitLeaf(leafId, "vertical")`

**Closing a leaf:** When all tabs in a leaf are closed, the leaf is removed and its parent split collapses. If the last leaf is closed, a new empty leaf is created (app always has at least one leaf).

### 6. Resize Handles

`WorkspaceRenderer` wires the existing `ResizeHandle` callbacks to `workspace.setSplitSizes(splitId, sizes)`:
- Convert pixel drag delta to percentage of container size
- Update the store's `sizes` array for the parent split node
- Children render with `flex: sizes[i]` styling
- New splits default to 50/50
- Minimum pane size: 100px enforced by clamping

### 7. What Gets Deleted

- `ViewContainer.tsx` — replaced by WorkspaceRenderer + LeafPane
- Tab state from `app-store.ts` — ~200 lines of tab management
- Tab-related imports from app-store in consumer files

### 8. What Stays Unchanged

- `WorkspaceRenderer.tsx` structure (already exists, needs callback wiring)
- `LeafPane.tsx` structure (already exists, needs view resolution logic)
- `workspace-store.ts` core operations (already implemented)
- All three editor components (MdxWysiwyg, EditorPane, LiveEditor)
- View registry and plugin system
- Right sidebar, left sidebar, ribbon, title bar

### 9. Deferred

- **Reading/preview view mode** — no component exists yet, add to backlog
- **Cross-mode undo preservation** — future optimization, requires editors to support loading from saved EditorState
- **Pop-out windows** — depends on split panes working (backlog item 6)

## Testing

- Single leaf renders identically to current single-pane behavior (regression check)
- Split right/down creates two panes with the same note
- Tab operations (open, close, move, pin) work within a leaf
- Drag tab between leaves
- Ctrl+E cycles view modes — editor component swaps
- Status bar shows current mode and cycles on click
- Resize handle drag adjusts pane proportions
- Layout persists across page reload (localStorage)
- Closing all tabs in a leaf removes the leaf
- Closing the last leaf creates a new empty one
