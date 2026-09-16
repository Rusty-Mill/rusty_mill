> **Archived 2026-09-08** — Imported verbatim from the private `baileyrd/Rustsidian` repo (`docs/`) under RFC 0009 row 6. Rustsidian is retired; this is reference material only and describes Rustsidian's SvelteKit/Tauri stack, not Nexus.

# UI Modernization Implementation Plan

> **For agentic workers:** REQUIRED SUB-SKILL: Use superpowers:subagent-driven-development (recommended) or superpowers:executing-plans to implement this plan task-by-task. Steps use checkbox (`- [ ]`) syntax for tracking.

**Goal:** Modernize the Rustsidian frontend with an Obsidian/Notion-inspired design, centralized panel state management, animated collapsible panels, and dark/light theme support.

**Architecture:** Extract scattered panel state from +page.svelte into a centralized `panelState` store. Build reusable `ResizablePanel`, `CollapsibleSection`, and `TopStrip` layout components. Introduce a CSS custom property-based theme system with dark/light modes. All existing panel content components (FileExplorer, GraphView, AiPanel, etc.) remain unchanged — only their wrappers and styling change.

**Tech Stack:** Svelte 5 (runes), SvelteKit 2, Tailwind CSS 4, CSS custom properties, localStorage

---

## File Structure

### New Files

| File | Responsibility |
|------|---------------|
| `src/lib/theme/tokens.css` | CSS custom properties for dark/light themes, radius tokens |
| `src/lib/theme/ThemeProvider.svelte` | Theme detection, localStorage persistence, data-theme attribute |
| `src/lib/layout/panelState.svelte.ts` | Centralized reactive store for all panel open/close, width, content state |
| `src/lib/layout/ResizablePanel.svelte` | Reusable resizable sidebar wrapper with drag handle |
| `src/lib/layout/CollapsibleSection.svelte` | Animated expand/collapse section for sidebar panels |
| `src/lib/layout/TopStrip.svelte` | Full-width top row: collapse icons, sidebar headers, tab bar, panel icons |

### Modified Files

| File | Changes |
|------|---------|
| `src/app.css` | Import tokens.css, replace hardcoded colors with var() tokens |
| `src/app.html` | Add data-theme attribute to html element |
| `src/routes/+page.svelte` | Major simplification: delegate to panelState and new components |
| `src/lib/layout/ActivityBar.svelte` | 44px, rounded icons, accent tint, themed colors |
| `src/lib/layout/StatusBar.svelte` | Themed colors, 26px height |
| `src/lib/layout/SplitPane.svelte` | Themed divider colors |
| `src/lib/editor/TabBar.svelte` | Rounded tabs, themed colors, new-tab button |
| `src/lib/editor/EditorToolbar.svelte` | Rounded hover states, themed colors |
| `src/lib/editor/EditorPane.svelte` | Modernized empty state with quick actions |
| `src/lib/sidebar/RightSidebar.svelte` | Use CollapsibleSection, themed colors |
| `src/lib/explorer/FileExplorer.svelte` | Rounded selection, accent tint, themed colors |

---

### Task 1: Theme Tokens CSS

**Files:**
- Create: `src/lib/theme/tokens.css`

- [ ] **Step 1: Create the tokens file**

```css
/* src/lib/theme/tokens.css */

[data-theme="dark"] {
  --bg-primary: #1a1b1e;
  --bg-secondary: #232428;
  --bg-tertiary: #2c2d32;
  --bg-elevated: #383a40;
  --text-primary: #e4e4e7;
  --text-secondary: #a1a1aa;
  --text-muted: #71717a;
  --accent: #7c3aed;
  --accent-subtle: rgba(124, 58, 237, 0.14);
  --accent-hover: #6d28d9;
  --border-subtle: #2c2d32;
  --border-default: #383a40;
  --radius-sm: 4px;
  --radius-md: 8px;
  --radius-lg: 12px;
  --transition-fast: 0.15s ease;
  --transition-panel: 0.2s ease-out;
}

[data-theme="light"] {
  --bg-primary: #ffffff;
  --bg-secondary: #f3f4f6;
  --bg-tertiary: #e5e7eb;
  --bg-elevated: #d1d5db;
  --text-primary: #1a1b1e;
  --text-secondary: #6b7280;
  --text-muted: #9ca3af;
  --accent: #7c3aed;
  --accent-subtle: rgba(124, 58, 237, 0.14);
  --accent-hover: #6d28d9;
  --border-subtle: #e5e7eb;
  --border-default: #d1d5db;
  --radius-sm: 4px;
  --radius-md: 8px;
  --radius-lg: 12px;
  --transition-fast: 0.15s ease;
  --transition-panel: 0.2s ease-out;
}
```

- [ ] **Step 2: Import tokens in app.css**

Replace the contents of `src/app.css` with:

```css
@import "tailwindcss";
@import "@xterm/xterm/css/xterm.css";
@import "$lib/theme/tokens.css";

/* ─── Global ─── */
* { box-sizing: border-box; }
html, body { margin: 0; padding: 0; height: 100%; overflow: hidden; }
body {
  font-family: -apple-system, BlinkMacSystemFont, 'Segoe UI', Roboto, Oxygen, Ubuntu, Cantarell, sans-serif;
  background: var(--bg-primary);
  color: var(--text-primary);
}

/* ─── CodeMirror ─── */
.cm-editor {
  font-family: 'JetBrains Mono', 'Fira Code', 'Cascadia Code', Consolas, monospace;
  font-size: 14px;
  line-height: 1.7;
  background: var(--bg-primary) !important;
}
.cm-editor .cm-content { padding: 16px 24px; }
.cm-editor .cm-gutters {
  border-right: 1px solid var(--border-default);
  background: var(--bg-primary);
  color: var(--text-muted);
}
.cm-editor .cm-activeLineGutter { background: var(--bg-tertiary); }
.cm-editor .cm-activeLine { background: var(--bg-tertiary); }
.cm-editor .cm-scroller { overflow: auto; }

/* ─── Autocomplete ─── */
.cm-tooltip-autocomplete {
  background: var(--bg-secondary) !important;
  border: 1px solid var(--border-default) !important;
  border-radius: var(--radius-sm);
  box-shadow: 0 4px 16px rgba(0,0,0,0.4);
}
.cm-tooltip-autocomplete ul li[aria-selected] {
  background: var(--accent-subtle) !important;
  color: var(--text-primary) !important;
}
.cm-tooltip-autocomplete ul li {
  padding: 2px 8px;
  font-size: 13px;
  color: var(--text-primary);
}
```

- [ ] **Step 3: Add data-theme to app.html**

Read `src/app.html` and add `data-theme="dark"` to the `<html>` tag as a default (ThemeProvider will manage it dynamically).

- [ ] **Step 4: Verify the app still builds**

Run: `cd frontend && npm run build`
Expected: Build succeeds with no errors.

- [ ] **Step 5: Commit**

```bash
git add frontend/src/lib/theme/tokens.css frontend/src/app.css frontend/src/app.html
git commit -m "feat: add CSS theme tokens and migrate app.css to custom properties"
```

---

### Task 2: ThemeProvider Component

**Files:**
- Create: `src/lib/theme/ThemeProvider.svelte`

- [ ] **Step 1: Create ThemeProvider**

```svelte
<script lang="ts">
  import type { Snippet } from 'svelte';
  import { onMount } from 'svelte';

  interface Props {
    children: Snippet;
  }

  let { children }: Props = $props();

  type Theme = 'dark' | 'light' | 'system';

  let theme = $state<Theme>('system');
  let resolvedTheme = $state<'dark' | 'light'>('dark');

  function getSystemTheme(): 'dark' | 'light' {
    return window.matchMedia('(prefers-color-scheme: dark)').matches ? 'dark' : 'light';
  }

  function applyTheme(t: Theme) {
    theme = t;
    resolvedTheme = t === 'system' ? getSystemTheme() : t;
    document.documentElement.setAttribute('data-theme', resolvedTheme);
    localStorage.setItem('rustsidian-theme', t);
  }

  export function toggleTheme() {
    const order: Theme[] = ['system', 'dark', 'light'];
    const next = order[(order.indexOf(theme) + 1) % order.length];
    applyTheme(next);
  }

  export function getTheme(): Theme {
    return theme;
  }

  export function getResolvedTheme(): 'dark' | 'light' {
    return resolvedTheme;
  }

  onMount(() => {
    const saved = localStorage.getItem('rustsidian-theme') as Theme | null;
    applyTheme(saved ?? 'system');

    const mql = window.matchMedia('(prefers-color-scheme: dark)');
    const handler = () => {
      if (theme === 'system') {
        resolvedTheme = getSystemTheme();
        document.documentElement.setAttribute('data-theme', resolvedTheme);
      }
    };
    mql.addEventListener('change', handler);
    return () => mql.removeEventListener('change', handler);
  });
</script>

{@render children()}
```

- [ ] **Step 2: Verify build**

Run: `cd frontend && npm run build`
Expected: Build succeeds.

- [ ] **Step 3: Commit**

```bash
git add frontend/src/lib/theme/ThemeProvider.svelte
git commit -m "feat: add ThemeProvider with dark/light/system theme support"
```

---

### Task 3: Panel State Store

**Files:**
- Create: `src/lib/layout/panelState.svelte.ts`

- [ ] **Step 1: Create the panel state store**

```typescript
// src/lib/layout/panelState.svelte.ts

export interface PanelSlot {
  open: boolean;
  width: number;
  minWidth: number;
  maxWidth: number;
  content: string;
}

// --- State ---

let left = $state<PanelSlot>({
  open: true,
  width: 260,
  minWidth: 160,
  maxWidth: 500,
  content: 'files',
});

let right = $state<PanelSlot>({
  open: true,
  width: 280,
  minWidth: 160,
  maxWidth: 500,
  content: 'outline',
});

let bottom = $state<PanelSlot>({
  open: false,
  width: 0, // height for bottom
  minWidth: 0,
  maxWidth: 0,
  content: 'terminal',
});

let splitDirection = $state<'horizontal' | 'vertical' | null>(null);

// --- Getters ---

export function getLeft(): PanelSlot { return left; }
export function getRight(): PanelSlot { return right; }
export function getBottom(): PanelSlot { return bottom; }
export function getSplitDirection(): 'horizontal' | 'vertical' | null { return splitDirection; }

// --- Actions ---

export function togglePanel(id: 'left' | 'right' | 'bottom') {
  const panel = id === 'left' ? left : id === 'right' ? right : bottom;
  panel.open = !panel.open;
  saveState();
}

export function openPanel(id: 'left' | 'right' | 'bottom') {
  const panel = id === 'left' ? left : id === 'right' ? right : bottom;
  panel.open = true;
  saveState();
}

export function closePanel(id: 'left' | 'right' | 'bottom') {
  const panel = id === 'left' ? left : id === 'right' ? right : bottom;
  panel.open = false;
  saveState();
}

export function setPanelWidth(id: 'left' | 'right', width: number) {
  const panel = id === 'left' ? left : right;
  panel.width = Math.max(panel.minWidth, Math.min(panel.maxWidth, width));
  saveState();
}

export function setPanelContent(id: 'left' | 'right' | 'bottom', content: string) {
  const panel = id === 'left' ? left : id === 'right' ? right : bottom;
  panel.content = content;
  saveState();
}

export function setSplitDirection(dir: 'horizontal' | 'vertical' | null) {
  splitDirection = dir;
}

// --- Left sidebar ribbon logic ---

export function handleRibbonToggle(panel: string) {
  // Left sidebar content panels
  const leftPanels = ['files', 'search', 'bookmarks', 'tags'];
  if (leftPanels.includes(panel)) {
    if (left.content === panel && left.open) {
      left.open = false;
    } else {
      left.content = panel;
      left.open = true;
    }
    saveState();
    return panel;
  }
  // Other panels handled by caller (graph, ai, terminal, etc.)
  return null;
}

// --- Persistence ---

function saveState() {
  try {
    localStorage.setItem('rustsidian-panels', JSON.stringify({
      left: { open: left.open, width: left.width, content: left.content },
      right: { open: right.open, width: right.width, content: right.content },
      bottom: { open: bottom.open },
    }));
  } catch {}
}

export function loadState() {
  try {
    const raw = localStorage.getItem('rustsidian-panels');
    if (!raw) return;
    const saved = JSON.parse(raw);
    if (saved.left) {
      left.open = saved.left.open ?? true;
      left.width = saved.left.width ?? 260;
      left.content = saved.left.content ?? 'files';
    }
    if (saved.right) {
      right.open = saved.right.open ?? true;
      right.width = saved.right.width ?? 280;
      right.content = saved.right.content ?? 'outline';
    }
    if (saved.bottom) {
      bottom.open = saved.bottom.open ?? false;
    }
  } catch {}
}
```

- [ ] **Step 2: Verify build**

Run: `cd frontend && npm run build`
Expected: Build succeeds.

- [ ] **Step 3: Commit**

```bash
git add frontend/src/lib/layout/panelState.svelte.ts
git commit -m "feat: add centralized panel state store with localStorage persistence"
```

---

### Task 4: ResizablePanel Component

**Files:**
- Create: `src/lib/layout/ResizablePanel.svelte`

- [ ] **Step 1: Create ResizablePanel**

```svelte
<script lang="ts">
  import type { Snippet } from 'svelte';
  import { setPanelWidth } from './panelState.svelte';

  interface Props {
    side: 'left' | 'right';
    width: number;
    open: boolean;
    children: Snippet;
  }

  let { side, width, open, children }: Props = $props();

  let isDragging = $state(false);

  function onPointerDown(e: PointerEvent) {
    isDragging = true;
    (e.target as HTMLElement).setPointerCapture(e.pointerId);
  }

  function onPointerMove(e: PointerEvent) {
    if (!isDragging) return;
    if (side === 'left') {
      // 44px for activity bar
      setPanelWidth('left', e.clientX - 44);
    } else {
      setPanelWidth('right', window.innerWidth - e.clientX);
    }
  }

  function onPointerUp() {
    isDragging = false;
  }
</script>

{#if open}
  <aside
    class="resizable-panel"
    class:panel-left={side === 'left'}
    class:panel-right={side === 'right'}
    style="width:{width}px"
  >
    <div class="panel-content">
      {@render children()}
    </div>
  </aside>
  <div
    class="resize-handle"
    class:handle-left={side === 'left'}
    class:handle-right={side === 'right'}
    onpointerdown={onPointerDown}
    onpointermove={onPointerMove}
    onpointerup={onPointerUp}
    role="separator"
    aria-orientation="vertical"
  ></div>
{/if}

<style>
  .resizable-panel {
    display: flex;
    flex-direction: column;
    overflow: hidden;
    background: var(--bg-secondary);
    flex-shrink: 0;
    transition: width var(--transition-panel), opacity var(--transition-panel);
  }

  .panel-content {
    flex: 1;
    overflow: hidden;
    display: flex;
    flex-direction: column;
  }

  .resize-handle {
    width: 2px;
    cursor: col-resize;
    background: var(--border-subtle);
    flex-shrink: 0;
    z-index: 10;
    transition: background var(--transition-fast);
  }
  .resize-handle:hover {
    background: var(--accent);
  }
</style>
```

- [ ] **Step 2: Verify build**

Run: `cd frontend && npm run build`
Expected: Build succeeds.

- [ ] **Step 3: Commit**

```bash
git add frontend/src/lib/layout/ResizablePanel.svelte
git commit -m "feat: add ResizablePanel component with drag-to-resize"
```

---

### Task 5: CollapsibleSection Component

**Files:**
- Create: `src/lib/layout/CollapsibleSection.svelte`

- [ ] **Step 1: Create CollapsibleSection**

```svelte
<script lang="ts">
  import type { Snippet } from 'svelte';

  interface Props {
    title: string;
    defaultOpen?: boolean;
    badge?: string;
    children: Snippet;
  }

  let { title, defaultOpen = true, badge, children }: Props = $props();

  let isOpen = $state(defaultOpen);
</script>

<div class="collapsible-section">
  <button class="section-header" onclick={() => isOpen = !isOpen}>
    <span class="section-chevron" class:open={isOpen}>
      <svg width="10" height="10" viewBox="0 0 24 24" fill="none" stroke="currentColor" stroke-width="2.5">
        <path d="M9 18l6-6-6-6"/>
      </svg>
    </span>
    <span class="section-title">{title}</span>
    {#if badge}
      <span class="section-badge">{badge}</span>
    {/if}
  </button>
  <div class="section-body" class:open={isOpen}>
    <div class="section-body-inner">
      {@render children()}
    </div>
  </div>
</div>

<style>
  .collapsible-section {
    border-bottom: 1px solid var(--border-subtle);
  }

  .section-header {
    display: flex;
    align-items: center;
    gap: 4px;
    width: 100%;
    padding: 8px 12px;
    background: none;
    border: none;
    color: var(--text-muted);
    font-size: 0.6875rem;
    font-weight: 600;
    text-transform: uppercase;
    letter-spacing: 0.05em;
    cursor: pointer;
    text-align: left;
    transition: color var(--transition-fast);
  }
  .section-header:hover { color: var(--text-primary); }

  .section-chevron {
    display: flex;
    align-items: center;
    justify-content: center;
    width: 14px;
    height: 14px;
    flex-shrink: 0;
    transition: transform var(--transition-fast);
  }
  .section-chevron.open {
    transform: rotate(90deg);
  }

  .section-title { flex: 1; }

  .section-badge {
    font-size: 0.625rem;
    color: var(--text-muted);
    font-weight: 400;
  }

  .section-body {
    max-height: 0;
    overflow: hidden;
    transition: max-height 0.2s ease-out;
  }
  .section-body.open {
    max-height: 500px;
  }

  .section-body-inner {
    padding: 0 12px 8px;
  }
</style>
```

- [ ] **Step 2: Verify build**

Run: `cd frontend && npm run build`
Expected: Build succeeds.

- [ ] **Step 3: Commit**

```bash
git add frontend/src/lib/layout/CollapsibleSection.svelte
git commit -m "feat: add CollapsibleSection with animated expand/collapse"
```

---

### Task 6: Modernize ActivityBar

**Files:**
- Modify: `src/lib/layout/ActivityBar.svelte`

- [ ] **Step 1: Update ActivityBar styling and layout**

Replace the full contents of `src/lib/layout/ActivityBar.svelte` with:

```svelte
<script lang="ts">
  interface Props {
    activePanel: string;
    onToggle: (panel: string) => void;
  }

  let { activePanel, onToggle }: Props = $props();

  const topItems = [
    { id: 'files', label: 'File Explorer' },
    { id: 'search', label: 'Search' },
    { id: 'tags', label: 'Tags' },
    { id: 'graph', label: 'Graph View' },
    { id: 'daily', label: 'Daily Note' },
  ];

  const bottomItems = [
    { id: 'ai', label: 'AI Panel' },
    { id: 'terminal', label: 'Terminal' },
    { id: 'plugins', label: 'Plugins' },
  ];
</script>

<nav class="activity-bar" aria-label="Activity Bar">
  <div class="ab-top">
    {#each topItems as item (item.id)}
      <button
        class="ab-icon"
        class:active={activePanel === item.id}
        onclick={() => onToggle(item.id)}
        title={item.label}
        aria-label={item.label}
      >
        <svg class="ab-svg" viewBox="0 0 24 24" fill="none" stroke="currentColor" stroke-width="1.5">
          {#if item.id === 'files'}
            <path d="M3 7V17a2 2 0 002 2h14a2 2 0 002-2V9a2 2 0 00-2-2h-6l-2-2H5a2 2 0 00-2 2z"/>
          {:else if item.id === 'search'}
            <circle cx="11" cy="11" r="6"/><path d="M21 21l-4.35-4.35"/>
          {:else if item.id === 'tags'}
            <path d="M7 7h.01M3 12l8.5-8.5a1 1 0 011.4 0l7.1 7.1a1 1 0 010 1.4L12.5 20a1 1 0 01-1.4 0L3 12z"/>
          {:else if item.id === 'graph'}
            <circle cx="12" cy="5" r="2"/><circle cx="5" cy="19" r="2"/><circle cx="19" cy="19" r="2"/><path d="M12 7v4M7 17l3-6M17 17l-3-6"/>
          {:else if item.id === 'daily'}
            <rect x="3" y="4" width="18" height="18" rx="2"/><path d="M16 2v4M8 2v4M3 10h18"/><path d="M8 14h.01M12 14h.01M16 14h.01M8 18h.01M12 18h.01"/>
          {/if}
        </svg>
      </button>
    {/each}
  </div>
  <div class="ab-separator"></div>
  <div class="ab-bottom">
    {#each bottomItems as item (item.id)}
      <button
        class="ab-icon"
        class:active={activePanel === item.id}
        onclick={() => onToggle(item.id)}
        title={item.label}
        aria-label={item.label}
      >
        <svg class="ab-svg" viewBox="0 0 24 24" fill="none" stroke="currentColor" stroke-width="1.5">
          {#if item.id === 'ai'}
            <path d="M12 2a4 4 0 014 4v1h1a3 3 0 013 3v2a3 3 0 01-3 3h-1v3a4 4 0 01-8 0v-3H7a3 3 0 01-3-3v-2a3 3 0 013-3h1V6a4 4 0 014-4z"/><circle cx="9" cy="10" r="1" fill="currentColor"/><circle cx="15" cy="10" r="1" fill="currentColor"/>
          {:else if item.id === 'terminal'}
            <rect x="2" y="4" width="20" height="16" rx="2"/><path d="M6 10l4 2-4 2"/><path d="M12 16h4"/>
          {:else if item.id === 'plugins'}
            <path d="M12 2v4M8 4h8M5 8h14v12H5z"/><path d="M9 12v4M15 12v4"/>
          {/if}
        </svg>
      </button>
    {/each}
    <div class="ab-spacer"></div>
    <button
      class="ab-icon"
      class:active={activePanel === 'settings'}
      onclick={() => onToggle('settings')}
      title="Settings"
      aria-label="Settings"
    >
      <svg class="ab-svg" viewBox="0 0 24 24" fill="none" stroke="currentColor" stroke-width="1.5">
        <circle cx="12" cy="12" r="3"/><path d="M19.4 15a1.65 1.65 0 00.33 1.82l.06.06a2 2 0 010 2.83 2 2 0 01-2.83 0l-.06-.06a1.65 1.65 0 00-1.82-.33 1.65 1.65 0 00-1 1.51V21a2 2 0 01-4 0v-.09A1.65 1.65 0 009 19.4a1.65 1.65 0 00-1.82.33l-.06.06a2 2 0 01-2.83-2.83l.06-.06A1.65 1.65 0 004.68 15a1.65 1.65 0 00-1.51-1H3a2 2 0 010-4h.09A1.65 1.65 0 004.6 9a1.65 1.65 0 00-.33-1.82l-.06-.06a2 2 0 012.83-2.83l.06.06A1.65 1.65 0 009 4.68a1.65 1.65 0 001-1.51V3a2 2 0 014 0v.09a1.65 1.65 0 001 1.51 1.65 1.65 0 001.82-.33l.06-.06a2 2 0 012.83 2.83l-.06.06A1.65 1.65 0 0019.4 9a1.65 1.65 0 001.51 1H21a2 2 0 010 4h-.09a1.65 1.65 0 00-1.51 1z"/>
      </svg>
    </button>
  </div>
</nav>

<style>
  .activity-bar {
    width: 44px;
    min-width: 44px;
    background: var(--bg-primary);
    display: flex;
    flex-direction: column;
    align-items: center;
    padding: 6px 0;
    flex-shrink: 0;
    overflow: hidden;
    border-right: 1px solid var(--border-subtle);
  }

  .ab-top, .ab-bottom {
    display: flex;
    flex-direction: column;
    align-items: center;
    gap: 2px;
    width: 100%;
  }

  .ab-bottom { flex: 1; }
  .ab-spacer { flex: 1; }

  .ab-separator {
    width: 24px;
    height: 1px;
    background: var(--border-default);
    margin: 4px 0;
  }

  .ab-icon {
    width: 32px;
    height: 32px;
    display: flex;
    align-items: center;
    justify-content: center;
    background: none;
    border: none;
    border-radius: var(--radius-md);
    color: var(--text-muted);
    cursor: pointer;
    padding: 0;
    transition: color var(--transition-fast), background var(--transition-fast);
  }
  .ab-icon:hover {
    color: var(--text-primary);
    background: var(--bg-tertiary);
  }
  .ab-icon.active {
    color: var(--accent);
    background: var(--accent-subtle);
  }

  .ab-svg {
    width: 18px;
    height: 18px;
  }
</style>
```

- [ ] **Step 2: Verify build**

Run: `cd frontend && npm run build`
Expected: Build succeeds.

- [ ] **Step 3: Commit**

```bash
git add frontend/src/lib/layout/ActivityBar.svelte
git commit -m "feat: modernize ActivityBar — 44px, rounded icons, accent tint"
```

---

### Task 7: Modernize TabBar

**Files:**
- Modify: `src/lib/editor/TabBar.svelte`

- [ ] **Step 1: Update TabBar with themed styles and new-tab button**

Replace the full contents of `src/lib/editor/TabBar.svelte` with:

```svelte
<script lang="ts">
  import {
    getTabs,
    getActiveTabIndex,
    setActiveTabIndex,
    closeTab,
    togglePin,
    moveTab,
  } from './editorState.svelte';

  let tabs = $derived(getTabs());
  let activeIndex = $derived(getActiveTabIndex());

  let dragIndex = $state<number | null>(null);

  function onDragStart(e: DragEvent, index: number) {
    dragIndex = index;
    if (e.dataTransfer) e.dataTransfer.effectAllowed = 'move';
  }

  function onDragOver(e: DragEvent) {
    e.preventDefault();
    if (e.dataTransfer) e.dataTransfer.dropEffect = 'move';
  }

  function onDrop(e: DragEvent, toIndex: number) {
    e.preventDefault();
    if (dragIndex !== null && dragIndex !== toIndex) moveTab(dragIndex, toIndex);
    dragIndex = null;
  }

  function onMiddleClick(e: MouseEvent, index: number) {
    if (e.button === 1) { e.preventDefault(); closeTab(index); }
  }

  function onContextMenu(e: MouseEvent, index: number) {
    e.preventDefault();
    togglePin(index);
  }
</script>

<div class="tab-bar" role="tablist">
  {#each tabs as tab, i (tab.path)}
    <button
      class="tab-item"
      class:active={i === activeIndex}
      class:pinned={tab.isPinned}
      role="tab"
      aria-selected={i === activeIndex}
      draggable="true"
      ondragstart={(e) => onDragStart(e, i)}
      ondragover={onDragOver}
      ondrop={(e) => onDrop(e, i)}
      onclick={() => setActiveTabIndex(i)}
      onmousedown={(e) => onMiddleClick(e, i)}
      oncontextmenu={(e) => onContextMenu(e, i)}
      title={tab.path}
    >
      {#if tab.isPinned}
        <span class="pin-icon">&#128204;</span>
      {/if}
      <span class="tab-title">{tab.title || tab.path.split('/').pop()}</span>
      {#if tab.isDirty}
        <span class="dirty-dot" title="Unsaved changes">&#9679;</span>
      {/if}
      {#if !tab.isPinned}
        <!-- svelte-ignore a11y_no_static_element_interactions -->
        <span
          class="close-btn"
          role="button"
          tabindex="-1"
          onclick={(e) => { e.stopPropagation(); closeTab(i); }}
          onkeydown={(e) => { if (e.key === 'Enter') { e.stopPropagation(); closeTab(i); } }}
          title="Close tab"
        >&times;</span>
      {/if}
    </button>
  {/each}
</div>

<style>
  .tab-bar {
    display: flex;
    align-items: center;
    overflow-x: auto;
    flex-shrink: 0;
    min-height: 0;
    gap: 2px;
    scrollbar-width: none;
  }
  .tab-bar::-webkit-scrollbar { display: none; }

  .tab-item {
    display: flex;
    align-items: center;
    gap: 4px;
    padding: 4px 12px;
    font-size: 0.75rem;
    color: var(--text-muted);
    background: transparent;
    border: none;
    cursor: pointer;
    white-space: nowrap;
    min-width: 0;
    max-width: 180px;
    flex-shrink: 0;
    border-radius: var(--radius-md) var(--radius-md) 0 0;
    transition: color var(--transition-fast), background var(--transition-fast);
  }

  .tab-item:hover {
    color: var(--text-secondary);
    background: var(--bg-tertiary);
  }
  .tab-item.active {
    color: var(--text-primary);
    background: var(--bg-secondary);
    border: 1px solid var(--border-subtle);
    border-bottom: 1px solid var(--bg-secondary);
    margin-bottom: -1px;
  }
  .tab-item.pinned { font-style: italic; }

  .tab-title {
    overflow: hidden;
    text-overflow: ellipsis;
    white-space: nowrap;
  }

  .dirty-dot {
    color: #facc15;
    font-size: 0.5rem;
    flex-shrink: 0;
  }

  .pin-icon {
    font-size: 0.625rem;
    flex-shrink: 0;
  }

  .close-btn {
    background: none;
    border: none;
    color: var(--text-muted);
    font-size: 0.75rem;
    cursor: pointer;
    padding: 0 2px;
    line-height: 1;
    flex-shrink: 0;
    opacity: 0;
    transition: opacity var(--transition-fast), color var(--transition-fast);
  }
  .tab-item:hover .close-btn { opacity: 1; }
  .close-btn:hover { color: #ef4444; }
</style>
```

- [ ] **Step 2: Verify build**

Run: `cd frontend && npm run build`
Expected: Build succeeds.

- [ ] **Step 3: Commit**

```bash
git add frontend/src/lib/editor/TabBar.svelte
git commit -m "feat: modernize TabBar — rounded tabs, themed colors, fade-in close"
```

---

### Task 8: TopStrip Component

**Files:**
- Create: `src/lib/layout/TopStrip.svelte`

- [ ] **Step 1: Create TopStrip**

```svelte
<script lang="ts">
  import TabBar from '$lib/editor/TabBar.svelte';
  import { getLeft, getRight, togglePanel, setPanelContent, openPanel } from './panelState.svelte';
  import { refreshTree } from '$lib/explorer/treeStore.svelte';
  import { commands } from '$lib/bindings';

  interface Props {
    onNewNote?: () => void;
  }

  let { onNewNote }: Props = $props();

  let leftPanel = $derived(getLeft());
  let rightPanel = $derived(getRight());

  const rightPanelItems = [
    { id: 'outline', label: 'Outline', icon: 'outline' },
    { id: 'backlinks', label: 'Backlinks', icon: 'backlinks' },
    { id: 'properties', label: 'Properties', icon: 'properties' },
  ];

  function handleRightPanelClick(id: string) {
    if (rightPanel.content === id && rightPanel.open) {
      // Already showing this panel, do nothing (use collapse button to close)
      return;
    }
    setPanelContent('right', id);
    openPanel('right');
  }

  async function handleNewFolder() {
    // Placeholder — will need a modal/input in the future
  }

  function handleCollapseAll() {
    // Dispatch a custom event that FileExplorer can listen to
    window.dispatchEvent(new CustomEvent('collapse-all-folders'));
  }
</script>

<div class="top-strip">
  <!-- Left collapse icon (above activity bar) -->
  <div class="ts-cell ts-collapse-left">
    <button
      class="ts-icon-btn"
      onclick={() => togglePanel('left')}
      title={leftPanel.open ? 'Collapse left sidebar (Ctrl+B)' : 'Expand left sidebar (Ctrl+B)'}
    >
      <svg width="14" height="14" viewBox="0 0 24 24" fill="none" stroke="currentColor" stroke-width="2">
        <rect x="3" y="3" width="18" height="18" rx="3"/>
        <path d="M9 3v18"/>
        {#if leftPanel.open}
          <path d="M14 9l-3 3 3 3"/>
        {:else}
          <path d="M12 9l3 3-3 3"/>
        {/if}
      </svg>
    </button>
  </div>

  <!-- Left sidebar header (only when open) -->
  {#if leftPanel.open}
    <div class="ts-cell ts-sidebar-header">
      <span class="ts-header-title">
        {leftPanel.content === 'files' ? 'Explorer' : leftPanel.content}
      </span>
      <div class="ts-header-actions">
        {#if leftPanel.content === 'files'}
          <button class="ts-icon-btn-sm" onclick={onNewNote} title="New note">
            <svg width="13" height="13" viewBox="0 0 24 24" fill="none" stroke="currentColor" stroke-width="2"><path d="M14 2H6a2 2 0 00-2 2v16a2 2 0 002 2h12a2 2 0 002-2V8z"/><path d="M14 2v6h6"/><path d="M12 18v-6"/><path d="M9 15h6"/></svg>
          </button>
          <button class="ts-icon-btn-sm" onclick={handleNewFolder} title="New folder">
            <svg width="13" height="13" viewBox="0 0 24 24" fill="none" stroke="currentColor" stroke-width="2"><path d="M22 19a2 2 0 01-2 2H4a2 2 0 01-2-2V5a2 2 0 012-2h5l2 3h9a2 2 0 012 2z"/><path d="M12 11v6"/><path d="M9 14h6"/></svg>
          </button>
          <button class="ts-icon-btn-sm" onclick={handleCollapseAll} title="Collapse all">
            <svg width="13" height="13" viewBox="0 0 24 24" fill="none" stroke="currentColor" stroke-width="2"><path d="M4 7l4 4 4-4"/><path d="M12 13l4 4 4-4"/></svg>
          </button>
        {/if}
      </div>
    </div>
  {/if}

  <!-- Tab bar (flexes to fill) -->
  <div class="ts-cell ts-tabs">
    <TabBar />
    <button class="ts-new-tab" title="New tab">+</button>
  </div>

  <!-- Right sidebar controls -->
  <div class="ts-cell ts-right-controls">
    <!-- Collapse toggle (always visible) -->
    <button
      class="ts-icon-btn"
      onclick={() => togglePanel('right')}
      title={rightPanel.open ? 'Collapse right sidebar (Ctrl+Shift+B)' : 'Expand right sidebar (Ctrl+Shift+B)'}
    >
      <svg width="14" height="14" viewBox="0 0 24 24" fill="none" stroke="currentColor" stroke-width="2">
        <rect x="3" y="3" width="18" height="18" rx="3"/>
        <path d="M15 3v18"/>
        {#if rightPanel.open}
          <path d="M10 9l3 3-3 3"/>
        {:else}
          <path d="M14 9l-3 3 3 3"/>
        {/if}
      </svg>
    </button>

    <!-- Panel switcher icons (only when open) -->
    {#if rightPanel.open}
      <div class="ts-separator-v"></div>
      {#each rightPanelItems as item (item.id)}
        <button
          class="ts-icon-btn-sm"
          class:active={rightPanel.content === item.id}
          onclick={() => handleRightPanelClick(item.id)}
          title={item.label}
        >
          <svg width="13" height="13" viewBox="0 0 24 24" fill="none" stroke="currentColor" stroke-width="2">
            {#if item.icon === 'outline'}
              <path d="M4 6h16"/><path d="M8 12h12"/><path d="M8 18h12"/>
            {:else if item.icon === 'backlinks'}
              <path d="M10 13a5 5 0 007.54.54l3-3a5 5 0 00-7.07-7.07l-1.72 1.71"/><path d="M14 11a5 5 0 00-7.54-.54l-3 3a5 5 0 007.07 7.07l1.71-1.71"/>
            {:else if item.icon === 'properties'}
              <path d="M12 20h9"/><path d="M16.5 3.5a2.12 2.12 0 013 3L7 19l-4 1 1-4z"/>
            {/if}
          </svg>
        </button>
      {/each}
    {/if}
  </div>
</div>

<style>
  .top-strip {
    display: flex;
    align-items: center;
    height: 36px;
    flex-shrink: 0;
    border-bottom: 1px solid var(--border-subtle);
    background: var(--bg-primary);
  }

  .ts-cell {
    display: flex;
    align-items: center;
    height: 100%;
  }

  .ts-collapse-left {
    width: 44px;
    justify-content: center;
    flex-shrink: 0;
    border-right: 1px solid var(--border-subtle);
  }

  .ts-sidebar-header {
    padding: 0 10px;
    gap: 6px;
    flex-shrink: 0;
    border-right: 1px solid var(--border-subtle);
    /* Match the left sidebar width — bound dynamically via style attribute if needed */
  }

  .ts-header-title {
    font-size: 0.6875rem;
    font-weight: 600;
    color: var(--text-muted);
    text-transform: uppercase;
    letter-spacing: 0.05em;
    white-space: nowrap;
  }

  .ts-header-actions {
    display: flex;
    gap: 1px;
    margin-left: auto;
  }

  .ts-tabs {
    flex: 1;
    min-width: 0;
    padding: 0 4px;
    gap: 2px;
    overflow: hidden;
  }

  .ts-new-tab {
    background: none;
    border: none;
    color: var(--text-muted);
    font-size: 1rem;
    cursor: pointer;
    padding: 4px 8px;
    border-radius: var(--radius-sm);
    transition: color var(--transition-fast), background var(--transition-fast);
    flex-shrink: 0;
  }
  .ts-new-tab:hover {
    color: var(--text-primary);
    background: var(--bg-tertiary);
  }

  .ts-right-controls {
    padding: 0 8px;
    gap: 2px;
    flex-shrink: 0;
    border-left: 1px solid var(--border-subtle);
  }

  .ts-icon-btn {
    width: 26px;
    height: 26px;
    border-radius: var(--radius-sm);
    display: flex;
    align-items: center;
    justify-content: center;
    background: none;
    border: none;
    color: var(--text-muted);
    cursor: pointer;
    transition: color var(--transition-fast), background var(--transition-fast);
  }
  .ts-icon-btn:hover {
    color: var(--text-primary);
    background: var(--bg-tertiary);
  }

  .ts-icon-btn-sm {
    width: 22px;
    height: 22px;
    border-radius: var(--radius-sm);
    display: flex;
    align-items: center;
    justify-content: center;
    background: none;
    border: none;
    color: var(--text-muted);
    cursor: pointer;
    transition: color var(--transition-fast), background var(--transition-fast);
  }
  .ts-icon-btn-sm:hover {
    color: var(--text-primary);
    background: var(--bg-tertiary);
  }
  .ts-icon-btn-sm.active {
    color: var(--accent);
    background: var(--accent-subtle);
  }

  .ts-separator-v {
    width: 1px;
    height: 14px;
    background: var(--border-subtle);
    margin: 0 2px;
  }
</style>
```

- [ ] **Step 2: Verify build**

Run: `cd frontend && npm run build`
Expected: Build succeeds.

- [ ] **Step 3: Commit**

```bash
git add frontend/src/lib/layout/TopStrip.svelte
git commit -m "feat: add TopStrip — collapse icons, sidebar headers, tab bar, panel switcher"
```

---

### Task 9: Modernize EditorToolbar

**Files:**
- Modify: `src/lib/editor/EditorToolbar.svelte:47-79` (style block)

- [ ] **Step 1: Update EditorToolbar styles to use theme tokens**

Replace the `<style>` block in `src/lib/editor/EditorToolbar.svelte` with:

```css
<style>
  .editor-toolbar {
    display: flex;
    align-items: center;
    gap: 1px;
    padding: 2px 8px;
    background: var(--bg-primary);
    border-bottom: 1px solid var(--border-subtle);
    flex-shrink: 0;
    height: 30px;
  }

  .tb-btn {
    display: flex;
    align-items: center;
    justify-content: center;
    width: 26px;
    height: 24px;
    background: none;
    border: none;
    color: var(--text-muted);
    font-size: 0.8125rem;
    cursor: pointer;
    border-radius: var(--radius-sm);
    transition: color var(--transition-fast), background var(--transition-fast);
  }
  .tb-btn:hover {
    color: var(--text-primary);
    background: var(--bg-elevated);
  }

  .tb-sep {
    color: var(--border-subtle);
    font-size: 0.75rem;
    margin: 0 2px;
  }
</style>
```

- [ ] **Step 2: Verify build**

Run: `cd frontend && npm run build`
Expected: Build succeeds.

- [ ] **Step 3: Commit**

```bash
git add frontend/src/lib/editor/EditorToolbar.svelte
git commit -m "feat: modernize EditorToolbar — themed colors, rounded hover states"
```

---

### Task 10: Modernize StatusBar

**Files:**
- Modify: `src/lib/layout/StatusBar.svelte:37-62` (style block)

- [ ] **Step 1: Update StatusBar styles**

Replace the `<style>` block in `src/lib/layout/StatusBar.svelte` with:

```css
<style>
  .status-bar {
    height: 26px;
    min-height: 26px;
    background: var(--bg-primary);
    border-top: 1px solid var(--border-subtle);
    display: flex;
    align-items: center;
    justify-content: space-between;
    padding: 0 12px;
    font-size: 0.6875rem;
    color: var(--text-muted);
    flex-shrink: 0;
    overflow: hidden;
  }

  .sb-left, .sb-right {
    display: flex;
    align-items: center;
    gap: 6px;
  }

  .sb-separator { color: var(--border-default); }
  .sb-item { white-space: nowrap; }
  .sb-mode { color: var(--text-secondary); }
</style>
```

- [ ] **Step 2: Verify build**

Run: `cd frontend && npm run build`
Expected: Build succeeds.

- [ ] **Step 3: Commit**

```bash
git add frontend/src/lib/layout/StatusBar.svelte
git commit -m "feat: modernize StatusBar — themed colors, 26px height"
```

---

### Task 11: Modernize SplitPane

**Files:**
- Modify: `src/lib/layout/SplitPane.svelte:60-93` (style block)

- [ ] **Step 1: Update SplitPane styles**

Replace the `<style>` block in `src/lib/layout/SplitPane.svelte` with:

```css
<style>
  .split-container {
    display: flex;
    width: 100%;
    height: 100%;
    overflow: hidden;
  }
  .split-container.horizontal { flex-direction: row; }
  .split-container.vertical { flex-direction: column; }

  .split-pane {
    overflow: hidden;
    display: flex;
    flex-direction: column;
    min-width: 0;
    min-height: 0;
  }

  .split-divider {
    flex-shrink: 0;
    background: var(--border-subtle);
    z-index: 10;
    transition: background var(--transition-fast);
  }
  .split-divider:hover { background: var(--accent); }

  .h-divider {
    width: 2px;
    cursor: col-resize;
  }
  .v-divider {
    height: 2px;
    cursor: row-resize;
  }
</style>
```

- [ ] **Step 2: Verify build**

Run: `cd frontend && npm run build`
Expected: Build succeeds.

- [ ] **Step 3: Commit**

```bash
git add frontend/src/lib/layout/SplitPane.svelte
git commit -m "feat: modernize SplitPane — themed divider, accent hover"
```

---

### Task 12: Modernize RightSidebar with CollapsibleSection

**Files:**
- Modify: `src/lib/sidebar/RightSidebar.svelte`

- [ ] **Step 1: Rewrite RightSidebar to use CollapsibleSection and themed styles**

Replace the full contents of `src/lib/sidebar/RightSidebar.svelte` with:

```svelte
<script lang="ts">
  import { commands, type HeadingItem } from '$lib/bindings';
  import { getCurrentFilePath, getActiveTab } from '$lib/editor/editorState.svelte';
  import CollapsibleSection from '$lib/layout/CollapsibleSection.svelte';

  interface Props {
    onOpenFile: (path: string) => void;
    onJumpToLine?: (line: number) => void;
  }

  let { onOpenFile, onJumpToLine }: Props = $props();

  let backlinks = $state<string[]>([]);
  let headings = $state<HeadingItem[]>([]);

  let filePath = $derived(getCurrentFilePath());
  let tab = $derived(getActiveTab());

  $effect(() => {
    const path = filePath;
    if (!path) {
      backlinks = [];
      headings = [];
      return;
    }

    commands.getHeadings(path).then(res => {
      if (res.status === 'ok') headings = res.data;
    });

    const filename = path.split('/').pop()?.replace(/\.mdx?$/, '') ?? '';
    if (filename) {
      commands.searchNotes(`[[${filename}]]`, 20).then(res => {
        if (res.status === 'ok') {
          backlinks = res.data
            .filter(r => r.path !== path)
            .map(r => r.path);
        }
      });
    }
  });
</script>

<aside class="right-sidebar">
  {#if filePath}
    <CollapsibleSection title="Properties">
      {#if tab}
        {@const fm = extractFrontmatter(tab.content)}
        {#if fm.length > 0}
          {#each fm as [key, value]}
            <div class="rs-prop-row">
              <span class="rs-prop-key">{key}</span>
              <span class="rs-prop-val">{value}</span>
            </div>
          {/each}
        {:else}
          <div class="rs-empty">No properties</div>
        {/if}
      {/if}
    </CollapsibleSection>

    <CollapsibleSection title="Backlinks" badge="{backlinks.length}">
      {#if backlinks.length > 0}
        {#each backlinks as bl}
          <button class="rs-link-item" onclick={() => onOpenFile(bl)}>
            {bl.split('/').pop()?.replace(/\.mdx?$/, '') ?? bl}
          </button>
        {/each}
      {:else}
        <div class="rs-empty">No backlinks</div>
      {/if}
    </CollapsibleSection>

    <CollapsibleSection title="Outline">
      {#if headings.length > 0}
        {#each headings as h}
          <button
            class="rs-outline-item"
            style="padding-left: {8 + (h.level - 1) * 12}px"
            onclick={() => onJumpToLine?.(h.line)}
          >
            {h.text}
          </button>
        {/each}
      {:else}
        <div class="rs-empty">No headings</div>
      {/if}
    </CollapsibleSection>
  {:else}
    <div class="rs-empty-state">
      <span>Open a note to see panels</span>
    </div>
  {/if}
</aside>

<script context="module">
  function extractFrontmatter(content: string): [string, string][] {
    if (!content.startsWith('---')) return [];
    const end = content.indexOf('\n---', 3);
    if (end === -1) return [];
    const yaml = content.slice(4, end);
    const pairs: [string, string][] = [];
    for (const line of yaml.split('\n')) {
      const colon = line.indexOf(':');
      if (colon > 0) {
        const key = line.slice(0, colon).trim();
        const val = line.slice(colon + 1).trim();
        if (key && val) pairs.push([key, val]);
      }
    }
    return pairs;
  }
</script>

<style>
  .right-sidebar {
    width: 100%;
    height: 100%;
    display: flex;
    flex-direction: column;
    overflow-y: auto;
    background: var(--bg-secondary);
  }

  .rs-prop-row {
    display: flex;
    justify-content: space-between;
    padding: 2px 0;
    font-size: 0.75rem;
  }
  .rs-prop-key { color: var(--text-muted); }
  .rs-prop-val { color: var(--text-secondary); }

  .rs-link-item {
    display: block;
    width: 100%;
    text-align: left;
    padding: 4px 8px;
    background: none;
    border: none;
    color: var(--accent);
    font-size: 0.75rem;
    cursor: pointer;
    border-radius: var(--radius-sm);
    transition: background var(--transition-fast);
  }
  .rs-link-item:hover { background: var(--bg-tertiary); }

  .rs-outline-item {
    display: block;
    width: 100%;
    text-align: left;
    padding: 3px 8px;
    background: none;
    border: none;
    color: var(--text-secondary);
    font-size: 0.75rem;
    cursor: pointer;
    border-radius: var(--radius-sm);
    transition: background var(--transition-fast);
  }
  .rs-outline-item:hover { background: var(--bg-tertiary); }

  .rs-empty {
    color: var(--text-muted);
    font-size: 0.6875rem;
    font-style: italic;
    padding: 4px 0;
  }

  .rs-empty-state {
    display: flex;
    align-items: center;
    justify-content: center;
    height: 100%;
    color: var(--text-muted);
    font-size: 0.75rem;
  }
</style>
```

- [ ] **Step 2: Verify build**

Run: `cd frontend && npm run build`
Expected: Build succeeds.

- [ ] **Step 3: Commit**

```bash
git add frontend/src/lib/sidebar/RightSidebar.svelte
git commit -m "feat: modernize RightSidebar — CollapsibleSection, themed colors"
```

---

### Task 13: Modernize EditorPane Empty State

**Files:**
- Modify: `src/lib/editor/EditorPane.svelte:105-115` (empty state template)

- [ ] **Step 1: Update the empty state in EditorPane**

In `src/lib/editor/EditorPane.svelte`, replace the `{:else}` block (lines 109-114) with:

```svelte
{:else}
  <div class="empty-state">
    <div class="empty-actions">
      <button class="empty-action accent" onclick={() => window.dispatchEvent(new KeyboardEvent('keydown', { key: 'n', ctrlKey: true }))}>
        Create new note (Ctrl+N)
      </button>
      <button class="empty-action accent" onclick={() => window.dispatchEvent(new KeyboardEvent('keydown', { key: 'p', ctrlKey: true }))}>
        Go to file (Ctrl+O)
      </button>
    </div>
  </div>
{/if}
```

- [ ] **Step 2: Add styles for the empty state**

Add to the `<style>` block in `EditorPane.svelte`:

```css
  .empty-state {
    display: flex;
    align-items: center;
    justify-content: center;
    height: 100%;
    background: var(--bg-primary);
  }

  .empty-actions {
    display: flex;
    flex-direction: column;
    align-items: center;
    gap: 12px;
  }

  .empty-action {
    background: none;
    border: none;
    font-size: 0.875rem;
    cursor: pointer;
    padding: 4px 8px;
    border-radius: var(--radius-sm);
    transition: opacity var(--transition-fast);
  }
  .empty-action:hover { opacity: 0.8; }
  .empty-action.accent { color: var(--accent); }
  .empty-action.muted { color: var(--text-muted); }
```

- [ ] **Step 3: Verify build**

Run: `cd frontend && npm run build`
Expected: Build succeeds.

- [ ] **Step 4: Commit**

```bash
git add frontend/src/lib/editor/EditorPane.svelte
git commit -m "feat: modernize EditorPane empty state — quick action links"
```

---

### Task 14: Modernize FileExplorer

**Files:**
- Modify: `src/lib/explorer/FileExplorer.svelte:67-75` (header section)

- [ ] **Step 1: Update FileExplorer header and themed styles**

In `src/lib/explorer/FileExplorer.svelte`, replace the explorer-header div (line 68) with:

```svelte
  <div class="explorer-header flex items-center justify-between px-3 py-2">
  </div>
```

Remove the old `+` button from the header — new note creation is now in the TopStrip.

- [ ] **Step 2: Update the tree container styles**

Replace the Tailwind classes on the tree-container (line 89) — update the background and selection colors by adding a scoped style block. Add to the bottom of the file before the closing tag:

```svelte
<style>
  .file-explorer {
    background: var(--bg-secondary);
  }
  .explorer-header {
    border-bottom: 1px solid var(--border-subtle);
  }
</style>
```

- [ ] **Step 3: Verify build**

Run: `cd frontend && npm run build`
Expected: Build succeeds.

- [ ] **Step 4: Commit**

```bash
git add frontend/src/lib/explorer/FileExplorer.svelte
git commit -m "feat: modernize FileExplorer — themed background, simplified header"
```

---

### Task 15: Rewire +page.svelte

**Files:**
- Modify: `src/routes/+page.svelte`

This is the biggest task — replacing the scattered state with the new component architecture.

- [ ] **Step 1: Rewrite +page.svelte**

Replace the full contents of `src/routes/+page.svelte` with:

```svelte
<script lang="ts">
  import { onMount } from 'svelte';
  import ThemeProvider from '$lib/theme/ThemeProvider.svelte';
  import TopStrip from '$lib/layout/TopStrip.svelte';
  import ActivityBar from '$lib/layout/ActivityBar.svelte';
  import StatusBar from '$lib/layout/StatusBar.svelte';
  import ResizablePanel from '$lib/layout/ResizablePanel.svelte';
  import FileExplorer from '$lib/explorer/FileExplorer.svelte';
  import EditorPane from '$lib/editor/EditorPane.svelte';
  import EditorToolbar from '$lib/editor/EditorToolbar.svelte';
  import CommandPalette from '$lib/palette/CommandPalette.svelte';
  import SplitPane from '$lib/layout/SplitPane.svelte';
  import TerminalPane from '$lib/terminal/TerminalPane.svelte';
  import GraphView from '$lib/graph/GraphView.svelte';
  import AiPanel from '$lib/ai/AiPanel.svelte';
  import PluginManager from '$lib/plugin/PluginManager.svelte';
  import BasesView from '$lib/bases/BasesView.svelte';
  import RightSidebar from '$lib/sidebar/RightSidebar.svelte';
  import {
    openFile,
    refreshNoteList,
    closeActiveTab,
    togglePin,
    getActiveTabIndex,
  } from '$lib/editor/editorState.svelte';
  import {
    getLeft,
    getRight,
    getBottom,
    getSplitDirection,
    togglePanel,
    setPanelContent,
    handleRibbonToggle,
    setSplitDirection,
    loadState,
  } from '$lib/layout/panelState.svelte';
  import { commands } from '$lib/bindings';

  let vaultReady = $state(false);
  let vaultName = $state('');
  let showPalette = $state(false);

  // Overlay panels (not part of resizable sidebar)
  let showGraph = $state(false);
  let showAi = $state(false);
  let showPlugins = $state(false);
  let showBases = $state(false);

  let editorPaneRef: EditorPane | undefined = $state(undefined);

  // Derive panel state
  let leftPanel = $derived(getLeft());
  let rightPanel = $derived(getRight());
  let bottomPanel = $derived(getBottom());
  let splitDirection = $derived(getSplitDirection());

  onMount(async () => {
    loadState();

    const info = await commands.getVaultInfo();
    if (info.status === 'ok') {
      await commands.openVault(info.data.vault_root);
      vaultReady = true;
      vaultName = info.data.name;
      await refreshNoteList();
    } else {
      const picked = await commands.pickVaultFolder();
      if (picked.status === 'ok') {
        vaultReady = true;
        vaultName = picked.data.name;
        await refreshNoteList();
      }
    }
  });

  async function handleOpenFile(path: string) {
    await openFile(path);
  }

  function handleJumpToLine(line: number) {
    editorPaneRef?.jumpToLine(line);
  }

  function handleRibbon(panel: string) {
    const handled = handleRibbonToggle(panel);
    if (handled) return;

    switch (panel) {
      case 'graph': showGraph = !showGraph; break;
      case 'ai': showAi = !showAi; break;
      case 'terminal': togglePanel('bottom'); break;
      case 'plugins': showPlugins = !showPlugins; break;
      case 'daily':
        commands.listNotes().then(res => {
          if (res.status === 'ok') {
            const today = new Date().toISOString().slice(0, 10);
            const daily = res.data.find(n => n.path.includes(today));
            if (daily) handleOpenFile(daily.path);
          }
        });
        break;
      case 'canvas': showBases = !showBases; break;
      case 'settings': showAi = true; break;
    }
  }

  function handleRunCommand(id: string) {
    switch (id) {
      case 'split-horizontal': setSplitDirection('horizontal'); break;
      case 'split-vertical': setSplitDirection('vertical'); break;
      case 'close-split': setSplitDirection(null); break;
      case 'close-tab': closeActiveTab(); break;
      case 'pin-tab': togglePin(getActiveTabIndex()); break;
      case 'toggle-terminal': togglePanel('bottom'); break;
      case 'toggle-graph': showGraph = !showGraph; break;
      case 'toggle-ai': showAi = !showAi; break;
      case 'toggle-plugins': showPlugins = !showPlugins; break;
      case 'toggle-bases': showBases = !showBases; break;
      case 'toggle-left-sidebar': togglePanel('left'); break;
      case 'toggle-right-sidebar': togglePanel('right'); break;
    }
  }

  function handleFormat(action: string) {
    const formatMap: Record<string, string> = {
      bold: '**', italic: '*', strikethrough: '~~',
      h1: '# ', h2: '## ', h3: '### ',
      quote: '> ', ul: '- ', ol: '1. ',
      task: '- [ ] ', code: '```\n', link: '[]()',
      wikilink: '[[]]',
    };
    const prefix = formatMap[action];
    if (prefix && editorPaneRef) {
      const sel = editorPaneRef.getSelectedText();
      if (sel) {
        if (['**', '*', '~~'].includes(prefix)) {
          editorPaneRef.replaceSelection(`${prefix}${sel}${prefix}`);
        } else {
          editorPaneRef.replaceSelection(`${prefix}${sel}`);
        }
      }
    }
  }

  function onKeyDown(e: KeyboardEvent) {
    if ((e.ctrlKey || e.metaKey) && e.key === 'p') {
      e.preventDefault();
      showPalette = !showPalette;
    }
    if ((e.ctrlKey || e.metaKey) && e.key === 'w') {
      e.preventDefault();
      closeActiveTab();
    }
    if (e.ctrlKey && e.key === '`') {
      e.preventDefault();
      togglePanel('bottom');
    }
    if ((e.ctrlKey || e.metaKey) && e.key === 'b' && !e.shiftKey) {
      e.preventDefault();
      togglePanel('left');
    }
    if ((e.ctrlKey || e.metaKey) && e.shiftKey && e.key === 'B') {
      e.preventDefault();
      togglePanel('right');
    }
  }

  let hasRightContent = $derived(showGraph || showAi || showPlugins || showBases || rightPanel.open);
</script>

<svelte:window onkeydown={onKeyDown} />

{#if showPalette}
  <CommandPalette
    onOpenFile={handleOpenFile}
    onJumpToLine={handleJumpToLine}
    onRunCommand={handleRunCommand}
    onClose={() => { showPalette = false; }}
  />
{/if}

{#if vaultReady}
  <ThemeProvider>
    <div class="app-shell">
      <!-- Top Strip -->
      <TopStrip />

      <div class="app-body">
        <!-- Activity Bar -->
        <ActivityBar activePanel={leftPanel.content} onToggle={handleRibbon} />

        <!-- Left Sidebar -->
        <ResizablePanel side="left" width={leftPanel.width} open={leftPanel.open}>
          {#if leftPanel.content === 'files'}
            <FileExplorer onOpenFile={handleOpenFile} />
          {:else if leftPanel.content === 'search'}
            <div class="sidebar-placeholder">
              <span class="sp-title">Search</span>
              <span class="sp-hint">Use Ctrl+P then / for full-text search</span>
            </div>
          {:else if leftPanel.content === 'tags'}
            <div class="sidebar-placeholder">
              <span class="sp-title">Tags</span>
              <span class="sp-hint">Use Ctrl+P then # for tag search</span>
            </div>
          {:else}
            <div class="sidebar-placeholder">
              <span class="sp-title">{leftPanel.content}</span>
            </div>
          {/if}
        </ResizablePanel>

        <!-- Main Content Area -->
        <main class="editor-area">
          <EditorToolbar onFormat={handleFormat} />
          <div class="editor-content-area">
            {#if bottomPanel.open}
              <SplitPane direction="vertical">
                {#snippet first()}
                  {#if splitDirection}
                    <SplitPane direction={splitDirection}>
                      {#snippet first()}
                        <EditorPane bind:this={editorPaneRef} />
                      {/snippet}
                      {#snippet second()}
                        <EditorPane />
                      {/snippet}
                    </SplitPane>
                  {:else}
                    <EditorPane bind:this={editorPaneRef} />
                  {/if}
                {/snippet}
                {#snippet second()}
                  <div class="terminal-wrapper">
                    <div class="terminal-header">
                      <span>Terminal</span>
                      <button class="terminal-close" onclick={() => togglePanel('bottom')}>&times;</button>
                    </div>
                    <div class="terminal-body">
                      <TerminalPane onOpenFile={handleOpenFile} />
                    </div>
                  </div>
                {/snippet}
              </SplitPane>
            {:else if splitDirection}
              <SplitPane direction={splitDirection}>
                {#snippet first()}
                  <EditorPane bind:this={editorPaneRef} />
                {/snippet}
                {#snippet second()}
                  <EditorPane />
                {/snippet}
              </SplitPane>
            {:else}
              <EditorPane bind:this={editorPaneRef} />
            {/if}
          </div>
        </main>

        <!-- Right Sidebar -->
        {#if hasRightContent}
          <ResizablePanel side="right" width={rightPanel.width} open={rightPanel.open || showGraph || showAi || showPlugins || showBases}>
            {#if showGraph}
              <GraphView onOpenFile={handleOpenFile} />
            {:else if showAi}
              <AiPanel
                selectedText={editorPaneRef?.getSelectedText() ?? ''}
                onInsert={(text) => editorPaneRef?.replaceSelection(text)}
                onOpenFile={handleOpenFile}
              />
            {:else if showPlugins}
              <PluginManager />
            {:else if showBases}
              <BasesView onOpenFile={handleOpenFile} />
            {:else}
              <RightSidebar
                onOpenFile={handleOpenFile}
                onJumpToLine={handleJumpToLine}
              />
            {/if}
          </ResizablePanel>
        {/if}
      </div>

      <!-- Status Bar -->
      <StatusBar />
    </div>
  </ThemeProvider>
{:else}
  <div class="loading-screen">
    <div class="loading-content">
      <div class="loading-title">Rustsidian</div>
      <div class="loading-hint">Opening vault...</div>
    </div>
  </div>
{/if}

<style>
  .app-shell {
    display: flex;
    flex-direction: column;
    height: 100vh;
    width: 100%;
    background: var(--bg-primary);
    color: var(--text-primary);
    overflow: hidden;
  }

  .app-body {
    display: flex;
    flex-direction: row;
    flex: 1;
    min-height: 0;
    overflow: hidden;
  }

  .editor-area {
    flex: 1;
    min-width: 200px;
    display: flex;
    flex-direction: column;
    overflow: hidden;
    background: var(--bg-primary);
  }

  .editor-content-area {
    flex: 1;
    overflow: hidden;
    display: flex;
    flex-direction: column;
  }

  .terminal-wrapper {
    height: 100%;
    display: flex;
    flex-direction: column;
    border-top: 1px solid var(--border-subtle);
  }
  .terminal-header {
    display: flex;
    align-items: center;
    justify-content: space-between;
    padding: 2px 8px;
    background: var(--bg-secondary);
    flex-shrink: 0;
    font-size: 0.6875rem;
    color: var(--text-muted);
    text-transform: uppercase;
    letter-spacing: 0.05em;
  }
  .terminal-close {
    background: none;
    border: none;
    color: var(--text-muted);
    cursor: pointer;
    font-size: 0.875rem;
    transition: color var(--transition-fast);
  }
  .terminal-close:hover { color: var(--text-primary); }
  .terminal-body { flex: 1; overflow: hidden; }

  .sidebar-placeholder {
    display: flex;
    flex-direction: column;
    align-items: center;
    justify-content: center;
    height: 100%;
    gap: 8px;
    color: var(--text-muted);
  }
  .sp-title {
    text-transform: capitalize;
    font-size: 0.875rem;
    color: var(--text-secondary);
  }
  .sp-hint { font-size: 0.6875rem; }

  .loading-screen {
    display: flex;
    align-items: center;
    justify-content: center;
    height: 100vh;
    background: var(--bg-primary);
    color: var(--text-muted);
  }
  .loading-content { text-align: center; }
  .loading-title { font-size: 1.25rem; color: var(--text-primary); font-weight: 600; margin-bottom: 8px; }
  .loading-hint { font-size: 0.875rem; }
</style>
```

- [ ] **Step 2: Verify build**

Run: `cd frontend && npm run build`
Expected: Build succeeds.

- [ ] **Step 3: Verify the app runs**

Run: `cd frontend && npm run dev`
Expected: Dev server starts on port 5173. Open in browser and verify:
- Activity bar shows with rounded icons
- Left sidebar opens/closes with Ctrl+B
- Right sidebar opens/closes with Ctrl+Shift+B
- TopStrip shows with collapse icons and tab bar
- Theme colors are applied (dark mode by default)

- [ ] **Step 4: Commit**

```bash
git add frontend/src/routes/+page.svelte
git commit -m "feat: rewire +page.svelte — panelState, TopStrip, ResizablePanel, ThemeProvider"
```

---

### Task 16: Final Verification and Polish

**Files:**
- No new files

- [ ] **Step 1: Full build check**

Run: `cd frontend && npm run build`
Expected: Build succeeds with no errors.

- [ ] **Step 2: Type check**

Run: `cd frontend && npm run check`
Expected: No type errors.

- [ ] **Step 3: Test the complete UI flow**

Manually verify in the browser:
1. Left sidebar collapse/expand with animation
2. Right sidebar collapse/expand with animation
3. TopStrip collapse icons change chevron direction
4. Right panel switcher icons appear/disappear
5. Left sidebar action icons (new note, new folder, collapse all)
6. Tab bar works with drag-to-reorder, close, pin
7. Editor toolbar formatting buttons work
8. Terminal toggle works (Ctrl+`)
9. Command palette works (Ctrl+P)
10. Empty tab state shows quick actions
11. All colors use theme tokens (no hardcoded hex remaining)

- [ ] **Step 4: Commit any fixes**

```bash
git add -A
git commit -m "fix: UI modernization polish and fixes"
```
