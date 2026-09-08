> **Archived 2026-09-08** — Imported verbatim from the private `baileyrd/Rustsidian` repo (`docs/`) under RFC 0009 row 6. Rustsidian is retired; this is reference material only and describes Rustsidian's SvelteKit/Tauri stack, not Nexus.

# UI Modernization Design Spec

**Date:** 2026-04-04
**Approach:** Component Refactor + Visual Reskin
**Aesthetic:** Obsidian/Notion — softer colors, rounded corners, subtle borders, web-app feel

## Design Decisions

| Decision | Choice |
|----------|--------|
| Visual direction | Obsidian/Notion modern (softer colors, rounded corners, subtle shadows) |
| Panel collapse animation | Smooth 200ms transitions (opacity then width) |
| Color scheme | Dark + Light with system preference detection as default |
| Activity bar | Keep vertical, modernize (narrower, rounded icons, accent tint active state) |
| Editor toolbar | Keep, modernize (cleaner icons, better grouping, rounded hover states) |
| Resize handles | Visible 1-2px border, highlights on hover |
| Approach | Component refactor + reskin (not CSS-only, not UI library migration) |
| Sidebar collapse icons | Left: above activity bar. Right: left side of panel switcher group |
| Right panel icons when collapsed | Hidden (only collapse toggle visible) |
| Right panel icons when open | Collapse toggle, separator, Search, Outline, Backlinks |

## 1. Theme System

### Architecture

A `ThemeProvider.svelte` component wraps the app, manages the current theme (dark/light/system), and sets CSS custom properties on the root element via `data-theme` attribute. All components reference tokens instead of hardcoded hex values.

A `tokens.css` file defines all CSS custom properties under `[data-theme="dark"]` and `[data-theme="light"]` selectors.

### Dark Theme Palette

| Token | Value | Usage |
|-------|-------|-------|
| `--bg-primary` | `#1a1b1e` | Main background, editor |
| `--bg-secondary` | `#232428` | Sidebars, panels |
| `--bg-tertiary` | `#2c2d32` | Activity bar, inputs |
| `--bg-elevated` | `#383a40` | Hover states, tooltips |
| `--text-primary` | `#e4e4e7` | Primary text |
| `--text-secondary` | `#a1a1aa` | Secondary text |
| `--text-muted` | `#71717a` | Muted text, labels |
| `--accent` | `#7c3aed` | Purple accent (Obsidian-like) |
| `--accent-subtle` | `#7c3aed22` | Accent at 14% opacity for tinted backgrounds |
| `--border-subtle` | `#2c2d32` | Subtle dividers |
| `--border-default` | `#383a40` | Default borders |

### Light Theme Palette

| Token | Value | Usage |
|-------|-------|-------|
| `--bg-primary` | `#ffffff` | Main background |
| `--bg-secondary` | `#f3f4f6` | Sidebars, panels |
| `--bg-tertiary` | `#e5e7eb` | Activity bar, inputs |
| `--bg-elevated` | `#d1d5db` | Hover states, tooltips |
| `--text-primary` | `#1a1b1e` | Primary text |
| `--text-secondary` | `#6b7280` | Secondary text |
| `--text-muted` | `#9ca3af` | Muted text, labels |
| `--accent` | `#7c3aed` | Same accent both themes |
| `--accent-subtle` | `#7c3aed22` | Accent tint |
| `--border-subtle` | `#e5e7eb` | Subtle dividers |
| `--border-default` | `#d1d5db` | Default borders |

### Border Radius Tokens

| Token | Value |
|-------|-------|
| `--radius-sm` | `4px` |
| `--radius-md` | `8px` |
| `--radius-lg` | `12px` |

### Theme Detection

- Reads `prefers-color-scheme` media query on mount
- Stores user override in `localStorage` (`rustsidian-theme`)
- Three modes: `dark`, `light`, `system`
- Toggle accessible from settings gear in activity bar

## 2. Panel System Architecture

### Problem

`+page.svelte` manages 12+ state variables for panel visibility, widths, and resize logic — all inline. Resize handlers are duplicated between left/right sidebars. Panel collapse is instant. Adding a new panel means touching `+page.svelte` in multiple places.

### New Components

#### `panelState.svelte.ts`

Reactive store managing all panel state:

```typescript
interface PanelSlot {
  open: boolean;
  width: number;
  minWidth: number;
  maxWidth: number;
  content: string; // active panel id
}

// Exposed state
panels: { left: PanelSlot, right: PanelSlot, bottom: PanelSlot }

// Methods
toggle(id: 'left' | 'right' | 'bottom'): void
setWidth(id: string, width: number): void
setContent(id: string, content: string): void
```

- Persists widths and open/closed state to `localStorage`
- Default left width: 260px (min 160, max 500)
- Default right width: 280px (min 160, max 500)
- Bottom panel (terminal): tracks open/closed and height only. Toggle via Ctrl+` or activity bar. No resize handle changes — uses existing SplitPane vertical divider.

#### `ResizablePanel.svelte`

Reusable wrapper component:

- Props: `side` (`'left' | 'right'`), `min`, `max`
- Owns its resize handle (1-2px visible border, highlights accent on hover)
- Reads/writes width from `panelState`
- Handles pointer events internally (no global document listeners)
- Renders slot content

#### `CollapsibleSection.svelte`

For sections within sidebars (Properties, Backlinks, Outline):

- Click header to toggle open/closed
- Smooth height animation via CSS `max-height` transition + `overflow: hidden`
- Chevron rotates on toggle (0 → 90 degrees)
- Props: `title`, `defaultOpen`

#### `TopStrip.svelte`

Full-width horizontal row at the top of the app, divided into cells:

1. **Left collapse icon** (44px) — above activity bar, panel-layout SVG with directional chevron
2. **Left sidebar header** — "Explorer" title + action icons (new note, new folder, sort, collapse all). Disappears when left sidebar is collapsed.
3. **Tab bar** — flexes to fill available space. Includes new-tab (+) button.
4. **Right sidebar controls** — collapse toggle (leftmost), separator, panel switcher icons (Search, Outline, Backlinks). When collapsed, only the toggle icon is visible.

### Sidebar Collapse Animation

**Closing (200ms total):**
1. Content opacity 1 → 0 (100ms)
2. Width animates to 0 (100ms ease-out)

**Opening (200ms total):**
1. Width 0 → saved width (100ms)
2. Content opacity 0 → 1 (100ms)

Editor area flexes smoothly to fill/release space via CSS transitions.

### Collapse Icon Behavior

**Left collapse icon:**
- Sits in 44px cell above activity bar in TopStrip
- Panel-layout SVG icon with embedded chevron
- Chevron points left when sidebar is open (collapse direction)
- Chevron points right when sidebar is closed (expand direction)
- Same action as Ctrl+B
- Always visible

**Right collapse icon:**
- Sits at the left side of the right panel controls in TopStrip
- Separated from panel switcher icons by thin vertical line
- Chevron points right when sidebar is open (collapse direction)
- Chevron points left when sidebar is closed (expand direction)
- Same action as Ctrl+Shift+B
- Always visible

**Right panel switcher icons (Search, Outline, Backlinks):**
- Only visible when right sidebar is open
- Active panel has accent-tinted background
- Click switches panel content
- Hidden when sidebar is collapsed

## 3. Visual Modernization

### Activity Bar

- Width: 44px (narrowed from 48px)
- Background: `--bg-primary` (blends with editor, no separate bar color)
- Separated from sidebar by 1px `--border-subtle` border
- Icon buttons: 32x32px, `--radius-md` (8px) rounded
- Active state: `--accent-subtle` tinted background (replaces left border indicator)
- Hover: icon brightens to `--text-primary`
- Icons: same SVGs, consistent 16px size

### Tab Bar (inside TopStrip)

- Active tab: `--bg-secondary` background, 1px `--border-subtle` border, rounded top corners (`--radius-md`)
- Active tab bottom border matches background (tab appears connected to editor)
- Inactive tabs: text-only, `--text-muted` color
- Close button: smaller (10px), fades in on hover
- New tab (+) button after last tab
- Middle-click to close, right-click to pin (unchanged)

### File Explorer

- Rounded hover/selection states (`--radius-md`)
- Active file: `--accent-subtle` tinted background
- Expanded folder chevron: `--accent` color
- More generous padding between items
- Action icons header: new note, new folder, sort, collapse all

### Editor Content

- Blockquotes: left border in `--accent`, `--bg-secondary` tinted background, `--radius-md` rounded
- Code blocks: `--bg-secondary` background, `--radius-md` rounded container
- Wikilinks: `--accent` color
- Line-height: 1.7 for readability

### Editor Toolbar

- Button groups separated by 1px `--border-subtle` vertical lines
- Buttons: `--radius-sm` rounded hover states
- Hover: `--bg-elevated` background
- Icons: `--text-muted` default, `--text-primary` on hover

### Right Sidebar

- Sections (Properties, Backlinks, Outline) use `CollapsibleSection` component
- Section headers: uppercase, `--text-muted`, with rotating chevron
- Tags rendered as accent-colored pills (`--accent-subtle` background, `--accent` text, `--radius-sm`)
- Backlink entries in `--accent` color

### Status Bar

- Height: 26px (slightly reduced from 28px)
- Background: `--bg-primary`
- Top border: 1px `--border-subtle`
- Text: `--text-muted`
- Small accent dot indicator on right side

### Empty Tab State

When no file is open, the editor area shows centered quick actions:
- "Create new note (Ctrl+N)" — `--accent` colored, clickable
- "Go to file (Ctrl+O)" — `--accent` colored, opens command palette
- "Close" — `--text-muted` colored, only shown when other tabs exist

## 4. File Changes

### New Files

| File | Purpose |
|------|---------|
| `$lib/theme/ThemeProvider.svelte` | Theme management, system preference detection, localStorage persistence |
| `$lib/theme/tokens.css` | CSS custom properties for dark/light themes |
| `$lib/layout/panelState.svelte.ts` | Centralized panel state store |
| `$lib/layout/ResizablePanel.svelte` | Reusable resizable panel wrapper |
| `$lib/layout/CollapsibleSection.svelte` | Animated expand/collapse for sidebar sections |
| `$lib/layout/TopStrip.svelte` | Full-width top row with collapse icons, headers, tabs, panel icons |

### Modified Files

| File | Changes |
|------|---------|
| `+page.svelte` | Remove 12+ state vars, inline resize/keyboard handlers. Delegate to panelState and new components. Becomes layout-only. |
| `app.css` | Replace hardcoded colors with `var()` tokens. Add transition utilities. Import `tokens.css`. |
| `ActivityBar.svelte` | 44px width, rounded icon buttons, accent tint active state, `--bg-primary` background |
| `TabBar.svelte` | Moves into TopStrip, rounded tab styling, add new-tab (+) button |
| `EditorToolbar.svelte` | Rounded hover states, themed colors |
| `StatusBar.svelte` | Themed colors, 26px height, subtle top border |
| `SplitPane.svelte` | Themed divider colors only |
| `RightSidebar.svelte` | Sections use CollapsibleSection, panel content switched by panelState |
| `FileExplorer.svelte` | Rounded selection states, accent-tinted active file |
| `EditorPane.svelte` | Add empty tab welcome state |

### Unchanged Files

- `GraphView.svelte`, `AiPanel.svelte`, `PluginManager.svelte`, `BasesView.svelte` — content unchanged, wrapped differently
- `TerminalPane.svelte`, `CommandPalette.svelte` — themed colors only (minimal)
- `editorState.svelte.ts`, `extensions.ts`, `livePreview.ts`, `mdxWidgets.ts`, `smartLinks.ts`, `wikilinkComplete.ts` — no changes
- All Rust backend code — no changes

## 5. Keyboard Shortcuts

All existing shortcuts preserved, same bindings:

| Shortcut | Action |
|----------|--------|
| Ctrl+B | Toggle left sidebar |
| Ctrl+Shift+B | Toggle right sidebar |
| Ctrl+` | Toggle terminal |
| Ctrl+P | Command palette |
| Ctrl+W | Close active tab |
| Ctrl+N | Create new note (new) |
| Ctrl+O | Go to file / open command palette (new) |
