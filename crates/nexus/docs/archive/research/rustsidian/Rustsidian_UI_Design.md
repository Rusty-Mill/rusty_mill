> **Archived 2026-09-08** — Imported verbatim from the private `baileyrd/Rustsidian` repo (`docs/`) under RFC 0009 row 6. Rustsidian is retired; this is reference material only and describes Rustsidian's SvelteKit/Tauri stack, not Nexus.

# Rustsidian UI Design Document

> **Version:** 0.1 — April 4, 2026
> **Status:** Living document — evolves with each delivery phase
> **Scope:** Desktop application (Tauri 2 + SvelteKit + CodeMirror 6). CLI/TUI and mobile are addressed separately.

---

## Table of Contents

1. [Design Philosophy](#1-design-philosophy)
2. [Global Layout](#2-global-layout)
3. [Window Chrome & Title Bar](#3-window-chrome--title-bar)
4. [Activity Bar (Ribbon)](#4-activity-bar-ribbon)
5. [Left Sidebar](#5-left-sidebar)
6. [Editor Area](#6-editor-area)
7. [Right Sidebar](#7-right-sidebar)
8. [Status Bar](#8-status-bar)
9. [Editor Component](#9-editor-component)
10. [Command Palette](#10-command-palette)
11. [Quick Switcher](#11-quick-switcher)
12. [Graph View](#12-graph-view)
13. [Canvas](#13-canvas)
14. [Terminal Panel](#14-terminal-panel)
15. [AI Panel](#15-ai-panel)
16. [Bases (Database Views)](#16-bases-database-views)
17. [Settings](#17-settings)
18. [Modals & Overlays](#18-modals--overlays)
19. [Context Menus](#19-context-menus)
20. [Theming System](#20-theming-system)
21. [Keyboard Shortcuts](#21-keyboard-shortcuts)
22. [Rustsidian-Specific UI Additions](#22-rustsidian-specific-ui-additions)
23. [Responsive & Accessibility Constraints](#23-responsive--accessibility-constraints)

---

## 1. Design Philosophy

Rustsidian's UI is built on five core principles that distinguish it from Obsidian while preserving the mental model users already have:

**1. Familiar to Obsidian users, native to engineers.** The macro layout — ribbon, dual sidebars, tabbed editor, status bar — mirrors Obsidian precisely so migration has zero relearning cost. Divergences are additive: the terminal panel, MDX toolbar, and AI panel are new surfaces that slot into familiar positions.

**2. Density-respecting.** The default theme targets a medium information density — denser than Notion, lighter than a raw IDE. Every panel is independently resizable and collapsible. Users should be able to get down to a single-pane distraction-free writing environment or a four-pane engineering workbench from the same application.

**3. Text is primary.** UI chrome exists to serve the document, not the other way around. All decorative elements use muted tones; interactive controls come forward only on hover or focus. The editor surface is visually dominant at every layout.

**4. Composable surfaces.** Panes are the unit of composition. A pane can contain: a note (source/preview), a canvas, a graph view, a terminal, a base view, or a browser (web viewer). All pane types participate in the same split/tab/drag system.

**5. AI as a first-class participant.** The AI panel is not a modal or a floating chatbot — it is a persistent sidebar surface with awareness of the open document, the vault, and the user's current selection. It communicates with the Rust MCP server directly over Tauri IPC, not through a screen-scraping layer.

---

## 2. Global Layout

```
┌─────────────────────────────────────────────────────────────────────────────┐
│  TITLE BAR                                           [vault name]  ─ □ ✕   │
├──┬────────────────────┬──────────────────────────────────┬────────────────┤
│  │                    │                                  │                │
│  │   LEFT SIDEBAR     │        EDITOR AREA               │  RIGHT SIDEBAR │
│  │   (collapsible)    │        (tabbed panes)            │  (collapsible) │
│R │                    │                                  │                │
│I │  ┌──────────────┐  │  ┌──────────────────────────┐   │ ┌────────────┐ │
│B │  │ File Explorer│  │  │ [Tab] [Tab] [Tab ×]  + │ ▾ │  │ │ Backlinks  │ │
│B │  │              │  │  ├──────────────────────────┤   │ ├────────────┤ │
│O │  │ (tree view)  │  │  │                          │   │ │ Outgoing   │ │
│N │  │              │  │  │   EDITOR / PANE CONTENT  │   │ │ Links      │ │
│  │  ├──────────────┤  │  │                          │   │ ├────────────┤ │
│  │  │ Search       │  │  │                          │   │ │ Properties │ │
│  │  ├──────────────┤  │  │                          │   │ ├────────────┤ │
│  │  │ Bookmarks    │  │  │                          │   │ │ Outline    │ │
│  │  ├──────────────┤  │  │                          │   │ ├────────────┤ │
│  │  │ Tags         │  │  │                          │   │ │ AI Panel   │ │
│  │  └──────────────┘  │  └──────────────────────────┘   │ └────────────┘ │
│  │                    ├──────────────────────────────────┤                │
│  │                    │   TERMINAL PANEL (collapsible)   │                │
│  │                    │   [bash ×] [zsh ×]  +           │                │
│  │                    │   $ _                            │                │
│  │                    └──────────────────────────────────┘                │
├──┴────────────────────┴──────────────────────────────────┴────────────────┤
│  STATUS BAR   word count · line:col · content-type · sync status · mode   │
└─────────────────────────────────────────────────────────────────────────────┘
```

### Panel Sizing Defaults

| Surface | Default Width/Height | Min | Collapsible |
|---|---|---|---|
| Activity Bar (Ribbon) | 44px wide | 44px (icon-only) | No |
| Left Sidebar | 260px wide | 160px | Yes (Ctrl+B) |
| Right Sidebar | 280px wide | 160px | Yes (Ctrl+Shift+B) |
| Terminal Panel | 240px tall | 120px | Yes (Ctrl+`) |
| Editor Area | fills remaining space | 320px wide | No |
| Status Bar | 28px tall | fixed | No |

All resize handles are 4px drag targets between adjacent surfaces. Double-clicking a resize handle resets that division to its default.

---

## 3. Window Chrome & Title Bar

On macOS, Rustsidian uses the native traffic light controls positioned at the far left of the title bar (standard macOS convention). On Windows and Linux it renders custom minimize/maximize/close buttons on the right.

The center of the title bar displays the current vault name. Clicking the vault name opens a vault switcher dropdown (same as the vault picker in Obsidian). This is the only vault management entry point in the title bar.

On macOS the title bar is draggable for window movement. On Windows/Linux only the title-bar area that is not occupied by a control is draggable.

The title bar does not contain navigation breadcrumbs — those live in the editor pane header instead.

---

## 4. Activity Bar (Ribbon)

The Activity Bar is a narrow vertical strip (44px) on the far left edge of the window. It is always visible and cannot be hidden (unlike Obsidian's ribbon, which can be toggled). Icons are 20px, with 12px padding, rendered in a muted foreground color. On hover they reach full-contrast foreground. The active panel icon receives an accent-colored left border (2px).

### Default Icon Order (top to bottom)

```
  ┌────┐
  │ 📂 │  File Explorer        (toggles left sidebar to Files)
  │    │
  │ 🔍 │  Search               (toggles left sidebar to Search)
  │    │
  │ 🔖 │  Bookmarks            (toggles left sidebar to Bookmarks)
  │    │
  │ 🏷 │  Tags                 (toggles left sidebar to Tags)
  │    │
  │ 🕸 │  Graph View           (opens Graph in editor area)
  │    │
  │ ⬜ │  Canvas               (creates new Canvas or opens last)
  │    │
  │ 📅 │  Daily Note           (opens or creates today's note)
  │    │
  │ ≡  │  ── separator ──
  │    │
  │ 🤖 │  AI Panel             (toggles right sidebar to AI)
  │    │
  │ 💻 │  Terminal             (toggles terminal panel)
  │    │
  │    │
  │ ⚙  │  Settings             (opens Settings modal)   [pinned to bottom]
  └────┘
```

Plugins can register additional ribbon icons. User-pinned commands appear between the separator and the pinned-bottom Settings icon. The icon order above the separator is configurable by drag-and-drop within the ribbon.

---

## 5. Left Sidebar

The left sidebar is a resizable panel that can display one of several **views** at a time, selected by clicking the corresponding ribbon icon. Each view is a full-height panel with its own header and content area.

### 5.1 File Explorer

```
┌─────────────────────────────────────┐
│ Files                     [⊕] [⋮]  │   ← panel header; ⊕ = new note, ⋮ = menu
├─────────────────────────────────────┤
│ 🗂 Projects                          │   ← folder, collapsed
│ ▾ 📁 Daily Notes                    │   ← folder, expanded
│    📄 2026-04-03                     │   ← note file
│    📄 2026-04-04                     │
│ ▾ 📁 Reference                      │
│    📄 Rust Async                     │
│    🖼 diagram.png                    │
│    📊 data.canvas                    │   ← canvas file
│ ▾ 📁 Projects                       │
│    📄 Rustsidian Roadmap             │
│    📄 UI Design                      │
│                                     │
│ [+ New Note]  [+ New Folder]        │   ← bottom action strip
└─────────────────────────────────────┘
```

**Interaction rules:**
- Single-click a note: opens it in the current editor pane (replaces current tab)
- Ctrl+click / middle-click: opens in a new tab
- Right-click: context menu (New note here, New folder here, Rename, Move to, Delete, Copy path, Copy wikilink, Reveal in OS file manager)
- Drag & drop: move notes and folders; dragging onto a folder expands it
- Drag to editor: opens in a new pane at the drop position
- File icons reflect type: `📄` for `.md`, `📄+` for `.mdx`, `📊` for `.canvas`, `🗃` for `.base`, `🖼` for images, `📋` for PDFs
- Inline rename: press `F2` or double-click the file name

**Sort / filter options (⋮ menu):**
- Sort by: Name (A→Z, Z→A), Created (newest/oldest), Modified (newest/oldest)
- Show/hide: Attachments, Hidden files (dot files)
- Collapse all folders

### 5.2 Search Panel

```
┌─────────────────────────────────────┐
│ Search                              │
├─────────────────────────────────────┤
│ ┌───────────────────────────────┐   │
│ │ 🔍 Search…                    │   │   ← full-text input, auto-focus
│ └───────────────────────────────┘   │
│ [Aa] [.*] [\""] [path:] [tag:]    │   ← toggles: case, regex, exact, operators
├─────────────────────────────────────┤
│ 3 results                           │
│                                     │
│ ▾ Rustsidian Roadmap                │   ← result group (note title)
│   …the **MDX pipeline** supports…  │   ← matched excerpt, query highlighted
│   …an **AI-native** architecture…  │
│                                     │
│ ▾ UI Design                         │
│   …**MDX** toolbar appears…        │
│                                     │
└─────────────────────────────────────┘
```

**Search operators (typed inline):**
- `path:folder/` — restrict to a folder
- `file:name` — match by filename
- `tag:#tagname` — filter by tag
- `line:text` — match must appear on a single line
- `section:text` — match within a heading section
- `property:key:value` — filter by frontmatter property

Results update live as the user types (debounced 150ms). Clicking a result opens the note with the first match highlighted. Results retain their source context (2 lines before/after). The search history of the last 20 queries is accessible via the ↑ key in the search box.

### 5.3 Bookmarks Panel

```
┌─────────────────────────────────────┐
│ Bookmarks                  [⊕] [⋮] │
├─────────────────────────────────────┤
│ ▾ 📁 Research                       │
│    📄 Rust Async                     │
│    🔍 "Tauri 2 examples" (search)   │
│ ▾ 📁 Daily                          │
│    📅 Today's Note                   │
│    📌 Rustsidian Roadmap#Phase 3    │   ← heading bookmark
│                                     │
│ Uncategorized                       │
│    📄 UI Design                      │
└─────────────────────────────────────┘
```

Groups are collapsible. Items can be drag-reordered within and between groups. Bookmark types include: notes, headings within notes, blocks, searches, folders, and graph filters.

### 5.4 Tags Panel

```
┌─────────────────────────────────────┐
│ Tags                       [⋮]     │
├─────────────────────────────────────┤
│ ┌───────────────────────────────┐   │
│ │ Filter tags…                  │   │
│ └───────────────────────────────┘   │
├─────────────────────────────────────┤
│ ▾ #project  (14)                    │
│    ▾ #project/rustsidian  (8)       │
│       📄 Roadmap                    │
│       📄 UI Design                  │
│    #project/other  (6)              │
│ #reference  (22)                    │
│ #daily  (30)                        │
└─────────────────────────────────────┘
```

Tags are displayed in a nested tree reflecting `/`-delimited hierarchy. Clicking a tag filters the file list inline. Clicking a note under a tag opens it. The `⋮` menu offers: sort by count/name, flatten hierarchy.

---

## 6. Editor Area

The editor area is the central workspace. It occupies all space between the two sidebars (vertically) and between the title bar and terminal panel / status bar.

### 6.1 Tab Bar

```
┌──────────────────────────────────────────────────────────────────┐
│ 📄 Roadmap   ×  │  📄 UI Design ·×  │  📊 Canvas 1   ×  │ + │ ▾ │
└──────────────────────────────────────────────────────────────────┘
```

- `·` before `×` indicates unsaved changes (same as VS Code's dot indicator)
- `+` creates a new note
- `▾` (tab overflow menu) shows all open tabs when they overflow horizontal width; also shows a "Close all" option
- Tabs can be dragged to reorder within the same pane, or dragged to a split handle to move to another pane
- Middle-click closes a tab
- Ctrl+W closes the active tab
- Ctrl+Tab / Ctrl+Shift+Tab cycles through tabs in MRU order

**Context menu on a tab (right-click):**
- Close, Close Others, Close to the Right, Close All
- Pin tab (pinned tabs don't close on Ctrl+W, show a pin icon instead of ×)
- Split right / Split down
- Open in new window (pop-out)
- Copy file path / Copy wikilink

### 6.2 Pane Splits

The editor area supports arbitrary pane splits. Splits are managed via:
- Dragging a tab onto a split zone (highlighted when dragging)
- Right-click a tab → "Split right" / "Split down"
- Command Palette → "Split editor right/down"

Split zones appear as hover-highlighted edges (top/bottom/left/right quadrants) when a tab is dragged over a pane.

```
  ┌─────────────────┬─────────────────┐
  │                 │                 │
  │   Pane A        │   Pane B        │
  │   (active)      │   (inactive)    │
  │                 │                 │
  └─────────────────┴─────────────────┘
         ↑ drag handle between panes (6px hit target)
```

The inactive pane receives a subtle border/tint. Clicking anywhere in an inactive pane activates it. Both panes maintain their own tab bars independently.

**Linked pane mode:** Two panes can be linked (right-click pane header → "Link with…"). Linked panes scroll to the same position and update in tandem (useful for Live Preview + Source side-by-side).

### 6.3 Pane Header

Each pane (not tab) has a small header above its tab bar:

```
┌──────────────────────────────────────────────────────────────────┐
│  📁 Projects / Rustsidian / UI Design       [👁] [✎] [⋮] [✕⊡]  │
├──────────────────────────────────────────────────────────────────┤
```

- **Breadcrumb**: shows vault-root-relative path. Each segment is clickable to navigate to that folder in the file explorer.
- **👁 / ✎**: Toggle between Reading View and Edit (Live Preview / Source) — cycles through the three editor modes.
- **⋮**: Pane actions menu (close, split, pin, link, open in new window, copy path).
- **✕⊡**: Close pane / maximize pane (fills editor area, hiding all other panes).

### 6.4 Empty State

When no notes are open, the editor area shows:

```
  ┌────────────────────────────────────┐
  │                                    │
  │   🦀                               │
  │                                    │
  │   Rustsidian                       │
  │   Open or create a note to begin  │
  │                                    │
  │   [New Note]  [Open Recent ▾]     │
  │                                    │
  │   Recent:                          │
  │   · Rustsidian Roadmap  2m ago    │
  │   · UI Design            5m ago   │
  │   · 2026-04-03           1d ago   │
  │                                    │
  └────────────────────────────────────┘
```

---

## 7. Right Sidebar

Mirrors the left sidebar structure but defaults to panels that show context for the active note. Multiple panels can be stacked in the right sidebar (unlike the left sidebar which shows one at a time). Each stacked panel has a collapse toggle.

### Default Stack (top to bottom)

```
┌────────────────────────────────┐
│ ▾ Properties              [+] │   ← collapsed or expanded
├────────────────────────────────┤
│ ▾ Backlinks          [⋮] [⊞]  │
├────────────────────────────────┤
│ ▾ Outgoing Links         [⋮]  │
├────────────────────────────────┤
│ ▾ Outline                [⋮]  │
├────────────────────────────────┤
│ ▾ AI Panel                    │   ← Rustsidian addition
└────────────────────────────────┘
```

Panels can be collapsed to just their header bar, dragged to reorder, or dragged to the left sidebar. The `⊞` icon (where present) opens the panel as a full editor pane.

### 7.1 Properties Panel

```
┌────────────────────────────────┐
│ ▾ Properties              [+] │
├────────────────────────────────┤
│ tags       #project #rust      │
│ created    2026-04-03          │
│ status     in-progress         │
│ aliases    (none)              │
│                                │
│ + Add property                 │
└────────────────────────────────┘
```

- Properties render as a structured table, not raw YAML
- Type icons precede each value (📅 for dates, #️⃣ for numbers, ✅ for booleans, 🔗 for links)
- Click any value to edit inline; type inference on commit (e.g., `2026-04-04` → date type)
- `[+]` opens a property type picker to add a new property
- The raw YAML source is always accessible via the editor's source mode

### 7.2 Backlinks Panel

```
┌────────────────────────────────┐
│ ▾ Backlinks (5)          [⋮]  │
├────────────────────────────────┤
│ Linked mentions (3)            │
│  ▾ Rustsidian Roadmap          │
│    …see the [[UI Design]]      │
│    document for…               │
│  ▾ 2026-04-04                  │
│    …reviewing [[UI Design]]…  │
│                                │
│ Unlinked mentions (2)          │
│  ▾ Kickoff Notes               │
│    …the ui design for this…   │
│    [Link]                      │   ← "Link" button creates the wikilink
└────────────────────────────────┘
```

Unlinked mentions are shown collapsed by default. The `[Link]` button rewrites the source note to turn the plain text into a `[[wikilink]]`.

### 7.3 Outline Panel

```
┌────────────────────────────────┐
│ ▾ Outline                [⋮]  │
├────────────────────────────────┤
│ H1 Design Philosophy           │
│  H2 Global Layout              │
│  H2 Window Chrome              │
│   H3 Title Bar                 │
│  H2 Activity Bar               │
│  H2 Left Sidebar               │
│   H3 File Explorer             │
│   H3 Search Panel              │
└────────────────────────────────┘
```

Click a heading to scroll the editor to that position. The current heading (based on cursor position) is highlighted in the outline.

---

## 8. Status Bar

```
┌────────────────────────────────────────────────────────────────────────────┐
│  2,841 words · 14,320 chars  │  Ln 128, Col 4  │  MDX  │  ✓ Synced  │ LP │
└────────────────────────────────────────────────────────────────────────────┘
```

Segments (left to right):
- **Word / char count** — live count of the active document
- **Cursor position** — line and column of the primary cursor
- **Content type** — `MD`, `MDX`, `Canvas`, or `Base`; clicking cycles between showing raw type label and a tooltip with schema info
- **Sync status** — `✓ Synced`, `⟳ Syncing`, `⚠ Conflict`, or `○ Local only` (greyed when no sync configured)
- **Editor mode** — `LP` (Live Preview), `SRC` (Source), `RD` (Reading); clicking cycles through modes
- Plugin-registered segments appear after the editor mode

Status bar segments are individually togglable in Settings → Appearance → Status bar.

---

## 9. Editor Component

The editor is built on **CodeMirror 6** with a custom Rustsidian extension set. It is the heart of the application. All three editor modes (Live Preview, Source, Reading) share the same CodeMirror instance; mode toggling changes the active decoration and rendering extensions, not the underlying document model.

### 9.1 Editor Toolbar

A minimal, context-sensitive toolbar appears above the editor content (below the pane header). It is hidden in Reading View.

```
┌─────────────────────────────────────────────────────────────────────────────┐
│ B  I  S  —  H₁ H₂ H₃  —  " ≡ ⋮≡  —  [ ] ⊞  —  ⌥ {} 🔗 !🔗  —  [MDX+] │
└─────────────────────────────────────────────────────────────────────────────┘
   ↑                                                                    ↑
   Standard Markdown formatting                          MDX-specific actions
```

**Toolbar segments:**
- `B I S` — Bold, Italic, Strikethrough
- `H₁ H₂ H₃` — Heading levels 1–3 (dropdown for 4–6)
- `"` — Blockquote; `≡` — Unordered list; `⋮≡` — Ordered list
- `[ ]` — Task list item; `⊞` — Table insert wizard
- `⌥` — Callout insert picker; `{}` — Code block (with language picker); `🔗` — Insert link; `!🔗` — Insert embed
- `[MDX+]` — Insert MDX component (opens component picker, Rustsidian-specific)

Toolbar buttons act on the current selection or insert at cursor if no selection. All toolbar actions are also available via the Command Palette and default keyboard shortcuts.

The toolbar can be hidden via Settings → Editor → Show toolbar. Even when hidden, all actions remain accessible via keyboard.

### 9.2 Live Preview Mode

This is the default editing mode. CodeMirror renders Markdown decorations for all lines except the one the cursor is on:

- Headings render with enlarged, weighted typography
- Bold/italic text renders its formatted version; the `**` and `_` markers appear only when the cursor enters that span
- `[[wikilinks]]` render as styled link chips; clicking follows the link
- `![[embeds]]` render the embedded content inline (another note, image, PDF first page, audio player)
- `- [ ]` task list items render as actual checkboxes (clicking toggles completion in the source)
- Tables render as HTML tables; cursor inside a table cell reveals the raw Markdown for that cell
- `mermaid` fenced code blocks render the diagram inline below the code (code collapses when cursor leaves)
- `$math$` and `$$math$$` render via MathJax/KaTeX inline
- MDX component syntax (JSX within the document) renders the live component when the cursor is outside the block

### 9.3 Source Mode

Pure Markdown/MDX text editor. All decorations disabled. Only syntax highlighting (via a custom CodeMirror language definition for CommonMark + MDX extensions). Line numbers optional (default off).

### 9.4 Reading View

The document is rendered as HTML in a read-only CodeMirror view with no cursor. MDX components are fully live and interactive. `[[wikilinks]]` are clickable anchor elements. The editor toolbar is hidden.

### 9.5 Vim / Emacs Mode

A Vim keybindings extension (via `@codemirror/vim`) is available as an opt-in setting. When enabled, the status bar rightmost segment shows the current Vim mode (`NORMAL`, `INSERT`, `VISUAL`). Emacs keybindings are also opt-in (via `@codemirror/emacs`).

### 9.6 Autocomplete & Suggestions

The editor provides several layers of autocomplete:

| Trigger | Behavior |
|---|---|
| `[[` | Opens wikilink picker: fuzzy-search all notes, attachments, headings |
| `[[#` | Heading autocomplete for current file |
| `[[note#` | Heading autocomplete for linked file |
| `#` (at word boundary) | Tag autocomplete: shows all tags in vault |
| `/` (slash command) | Inline command picker (insert date, callout, table, component, etc.) |
| `@` (in MDX context) | MDX component picker from the component registry |
| standard word completion | Opt-in: completes words from the current vault's text corpus |

Autocomplete popups are rendered in a floating `cm-tooltip` above or below the cursor. Keyboard: `↑↓` to navigate, `Enter`/`Tab` to accept, `Esc` to dismiss.

### 9.7 Find & Replace

Accessible via `Ctrl+F`. Renders as an overlay bar at the top of the editor pane:

```
┌────────────────────────────────────────────────────────┐
│  Find: [__________________]  [Aa] [.*]  ← 3 of 8 →    │
│  Repl: [__________________]  [Replace]  [Replace All]  │
│                                                  [✕]    │
└────────────────────────────────────────────────────────┘
```

Vault-wide search is in the left sidebar Search panel. Pane-level find is this component.

---

## 10. Command Palette

**Trigger:** `Ctrl+P` (also accessible from any focused surface).

```
┌─────────────────────────────────────────────────────────┐
│  > _                                                    │   ← input with '>' prefix
├─────────────────────────────────────────────────────────┤
│  Recently used:                                         │
│  ▷ New note                              Ctrl+N        │
│  ▷ Open daily note                       Ctrl+Shift+D  │
│  ▷ Split editor right                                  │
├─────────────────────────────────────────────────────────┤
│  All commands:                                          │
│  ▷ Toggle left sidebar                  Ctrl+B         │
│  ▷ Insert callout                                      │
│  ▷ Insert MDX component                               │
│  ▷ Open graph view                                     │
│  ▷ Toggle terminal                      Ctrl+`         │
│  ▷ Open AI panel                                       │
│  …                                                     │
└─────────────────────────────────────────────────────────┘
```

- Fuzzy matching: typing `"split"` shows all split-related commands; typing `"mdx"` shows MDX-specific commands
- Commands contributed by plugins appear alongside core commands
- Recently used commands (last 10) are pinned at the top
- `?` prefix shows help: lists all special prefixes (`>` commands, no prefix = quick switcher, `@` recent files, `#` headings)
- The palette can also accept `:` colon commands for direct navigation: `:123` jumps to line 123

---

## 11. Quick Switcher

**Trigger:** `Ctrl+O`.

```
┌─────────────────────────────────────────────────────────┐
│  Open note: roadmap_                                    │
├─────────────────────────────────────────────────────────┤
│  📄 Rustsidian Roadmap       Projects/Rustsidian/      │
│  📄 API Roadmap              Projects/Other/           │
│     [create "roadmap_"]                                 │
└─────────────────────────────────────────────────────────┘
```

- Fuzzy match across all notes and attachment names in the vault
- Shows the folder path to the right for disambiguation
- If no exact match exists, offers to create a new note with that name
- `Enter` opens in current pane; `Ctrl+Enter` opens in a new pane; `Ctrl+Shift+Enter` opens in a new window
- Recently opened notes appear when the input is empty

---

## 12. Graph View

Graph View opens as a full pane in the editor area (or can be split alongside notes).

### 12.1 Layout

```
┌────────────────────────────────────────────────────────────────┐
│  Graph: All  ·  3,248 nodes  ·  8,916 links          [⋮] [?]  │   ← header
├────────────────────────────────────────────────────────────────┤
│                                                                │
│              ●───●───────●                                     │
│             /│           │\                                    │
│   ●───────●  ●           ● ●──●                                │
│             \             │                                    │
│              ●────────────●                                    │
│                                                                │
├────────────────────────────────────────────────────────────────┤
│  Filters ▾  │  Groups ▾  │  Display ▾  │  [🔍 Filter notes…]  │   ← bottom toolbar
└────────────────────────────────────────────────────────────────┘
```

### 12.2 Node Rendering

- **Default node**: small circle (8px radius), label truncated to 20 chars
- **Node size** scales with the number of connections (configurable: off / by connections / by word count)
- **Orphan nodes** (no links): rendered smaller, greyed, optionally hidden
- **Tags** can appear as their own nodes (larger hexagons) connecting to all notes that carry them
- **Attachments** can appear as square nodes

### 12.3 Interaction

| Action | Effect |
|---|---|
| Click a node | Open linked note in the adjacent pane (or current pane if graph is full-width) |
| Hover a node | Show a tooltip with the note title and link count |
| Drag a node | Re-position it (layout updates around it) |
| Scroll | Zoom in/out |
| Middle-click drag | Pan |
| Right-click node | Context menu: Open, Open in new pane, Pin node, Focus (local graph of just this node), Rename |

### 12.4 Controls

**Filters panel:**
- Search box: filter displayed nodes to those matching the query
- Show: Tags / Attachments / Orphans — toggles
- Hide: Nodes in folder (path filter), nodes by tag

**Groups panel:**
- Assign color rules: e.g., "all notes with `#project` tag → red nodes"
- Up to 6 color groups (matching the Canvas color palette)
- Rules are path, tag, or search-query based

**Display panel:**
- Node size: fixed / by links / by word count
- Link distance (force simulation spring length)
- Repel force (how much nodes push each other apart)
- Show arrows (directed link visualization)
- Animate (toggle force-directed animation vs static layout)

### 12.5 Local Graph

Every note's right-sidebar can include a **Local Graph** panel showing only that note's immediate neighborhood. Accessible via right sidebar panel picker. Depth control: 1–4 degrees of separation.

---

## 13. Canvas

Canvas opens as a full pane in the editor area. The toolbar at the top of the pane provides canvas-specific controls.

### 13.1 Canvas Toolbar

```
┌────────────────────────────────────────────────────────────────┐
│ ◻ Text  │  📄 Note  │  🔗 Link  │  ⬜ Group  │  ─── Edge  │  … │
│                                               ↑ right section  │
│                             [⊞] [Export PNG] [? Keys] [100%] ∓ │
└────────────────────────────────────────────────────────────────┘
```

Left section: **Create tools** (selecting one puts the canvas into creation mode; click on canvas to place).
Right section: minimap toggle `⊞`, export, keyboard reference `?`, zoom percentage, zoom in/out `∓`.

### 13.2 Node Appearance

```
  ┌──────────────────────────────┐
  │  Title (if note node)        │  ← 24px header, file icon + name
  ├──────────────────────────────┤
  │                              │
  │  Content area                │  ← scrollable if tall; rendered Markdown
  │  (note preview / text /      │
  │   embedded URL / image)      │
  │                              │
  └──────────────────────────────┘
```

- **Minimum node size**: 200×120px
- **Default text node size**: 300×200px
- **Default note node size**: 360×280px
- Resize handles on all 8 corners and edge midpoints
- Color chip visible in top-right corner of selected nodes (clicking opens color picker)

### 13.3 Edge Rendering

Edges are bezier curves between nodes. The curve is drawn with a 2px stroke in the edge color. Arrows render as filled triangular arrowheads. Edge labels float at the midpoint of the curve on a small pill background.

Connection handles appear as small circles on the midpoint of each side of a node when the node is hovered. Dragging a handle to another node creates an edge.

### 13.4 Canvas Navigation

- **Zoom to fit all**: `Shift+1`
- **Zoom to selection**: `Shift+2`
- **Reset zoom to 100%**: `Ctrl+Shift+0`
- **Mini-map**: toggleable overlay (bottom-right corner, 200×150px) showing the full canvas with a viewport indicator rectangle; draggable for navigation
- **Mouse**: scroll = pan vertical; Shift+scroll = pan horizontal; Ctrl+scroll = zoom; middle-drag = pan; right-drag = pan

---

## 14. Terminal Panel

The terminal panel is a **Rustsidian-native feature** with no Obsidian equivalent. It is a horizontal panel docked below the editor area, using `xterm.js` on the frontend connected to a `alacritty_terminal`-based PTY backend in Rust via Tauri IPC.

### 14.1 Layout

```
┌────────────────────────────────────────────────────────────────────────────┐
│ TERMINAL  [bash ×]  [zsh ×]  [fish ×]  [+ New]                    [⊡] [✕]│
├────────────────────────────────────────────────────────────────────────────┤
│                                                                            │
│  ~/projects/rustsidian$ cargo build --release                              │
│  Compiling rustsidian-core v0.1.0                                          │
│     Finished release [optimized] target(s) in 12.4s                       │
│  ~/projects/rustsidian$ _                                                  │
│                                                                            │
└────────────────────────────────────────────────────────────────────────────┘
```

- **Tab bar** at the top of the panel (not the main editor tab bar): each terminal session is its own tab
- `[+ New]` opens a new shell session (uses the configured default shell from Settings → Terminal)
- `[⊡]` maximizes the terminal to fill the full editor area (same as popping a pane to full-screen); pressing again restores
- `[✕]` collapses the terminal panel (same as `Ctrl+``)
- `⊞` (panel header icon from the ribbon) re-expands the panel

### 14.2 Terminal Settings

Accessible via Settings → Terminal:
- Default shell (`/bin/zsh`, `/bin/bash`, `/usr/bin/fish`, custom path)
- Working directory on new session: Vault root / OS home / last CWD
- Font: monospace font family, size (default: system mono, 13px)
- Scrollback buffer: 10,000 lines (configurable)
- Theme: follows app theme (auto), or a fixed terminal color scheme

### 14.3 Vault-Aware Shell Integration

When the terminal is opened from within Rustsidian:
- `$RUSTSIDIAN_VAULT` is set to the vault root path
- `$RUSTSIDIAN_NOTE` is set to the path of the currently active note
- A `rustsidian` CLI command (Phase 1 CLI deliverable) is available on PATH when the app is running, allowing shell commands like `rustsidian search "query"` or `rustsidian open "Note Title"` to interact with the running vault

---

## 15. AI Panel

The AI panel is a **Rustsidian-native feature** displayed in the right sidebar. It is a persistent context-aware assistant that communicates with the Rustsidian MCP server directly over Tauri IPC rather than through a third-party API facade.

### 15.1 Panel Layout

```
┌────────────────────────────────┐
│ ▾ AI                  [⚙] [⊞] │
├────────────────────────────────┤
│ Context: UI Design     [×] [+]│   ← active context chips
│          #project       [×]   │
├────────────────────────────────┤
│                                │
│  What relationships does       │
│  this design have with the     │
│  roadmap phases?               │
│                                │
│  ────────────────              │
│                                │
│  The UI Design document        │
│  primarily covers Phase 3      │
│  (Desktop Core). Here are      │
│  the relevant connections:     │
│  …                             │
│                                │
├────────────────────────────────┤
│ ┌────────────────────────────┐ │
│ │ Ask anything…             │ │   ← input textarea (auto-resizes)
│ └────────────────────────────┘ │
│  [📎 Attach]  [🔗 Link note]  ↑ │   ← send button (or Enter)
└────────────────────────────────┘
```

### 15.2 Context Chips

The context row shows what is currently "in scope" for the AI. Default context = the active note. The user can:
- `[×]` remove a context item
- `[+]` add context: pick from open notes, vault search, folders, tag groups, or the full vault

Context is passed to the MCP server as a scoped query; the AI does not receive the entire vault's raw text — it queries via MCP tools (`search`, `read_note`, `list_backlinks`, etc.) as needed.

### 15.3 AI Settings (⚙)

- Model: select from configured providers (local via Ollama, OpenAI, Anthropic, custom endpoint)
- System prompt: override the default vault assistant system prompt
- Max context tokens: cap on how much vault content is injected per request
- Response language: auto / force language
- Show tool calls: toggle visibility of intermediate MCP tool invocations (useful for debugging)

### 15.4 Inline AI Actions

Right-clicking a text selection in the editor opens a context menu that includes AI quick-actions:
- **Summarize selection** → output replaces selection or is appended as a callout
- **Expand selection** → adds more detail inline
- **Ask AI about this** → prefills the AI panel input with the selected text
- **Find related notes** → runs a semantic search and opens results in the backlinks panel
- **Generate tags** → suggests frontmatter tags for the selection

---

## 16. Bases (Database Views)

Bases is the Obsidian-equivalent of Notion's database views. In Rustsidian it is a core feature powered by the `properties_index` and `tasks` SQLite tables.

A `.base` file (JSON schema defining the query and view configuration) opens as a pane in the editor area.

### 16.1 View Types

| View | Description |
|---|---|
| **Table** | Spreadsheet-like grid; columns = properties; rows = notes |
| **Gallery** | Card grid; one card per note showing cover image + key properties |
| **List** | Compact list view; title + selected properties per row |
| **Kanban** | Columns defined by a property value (e.g., `status`); cards are notes |
| **Calendar** | Monthly/weekly calendar; notes placed by a date property |

### 16.2 Table View

```
┌────────────────────────────────────────────────────────────────────────────┐
│ Base: Active Projects                            [+ Add view] [Filter] [⋮]│
├────────────────────────────────────────────────────────────────────────────┤
│ 📋 Table  │  🗂 Kanban  │  📅 Calendar  │  + New view                      │
├───────────────┬──────────────┬────────┬────────────┬────────────────────┤
│ Title       ↕ │ Status     ↕ │ Tags ↕ │ Created  ↕ │ Progress        ↕  │
├───────────────┼──────────────┼────────┼────────────┼────────────────────┤
│ 📄 Roadmap    │ in-progress  │ #rust  │ 2026-04-03 │ ████░░░ 60%        │
│ 📄 UI Design  │ in-progress  │ #rust  │ 2026-04-04 │ ██░░░░░ 30%        │
│ 📄 CLI Spec   │ planned      │ #rust  │ 2026-04-01 │ ░░░░░░░ 0%         │
├───────────────┼──────────────┼────────┼────────────┼────────────────────┤
│ + New row     │              │        │            │                     │
└───────────────┴──────────────┴────────┴────────────┴────────────────────┘
```

- Clicking a row's title opens the note
- Clicking a cell edits that property inline
- Column headers are sortable and draggable
- `[Filter]` opens a filter builder: property → operator → value, with AND/OR logic
- `[⋮]` includes: hide columns, group by property, export to CSV

---

## 17. Settings

**Trigger:** `Ctrl+,` or Settings icon (⚙) in the ribbon.

Settings opens as a modal dialog (not a full pane) covering ~80% of the window with a frosted-glass backdrop. It uses a two-column layout: category nav on the left, settings form on the right.

### 17.1 Navigation Categories

```
  General
  Editor
  ├─ Editing
  ├─ Display
  ├─ MDX
  Appearance
  ├─ Themes
  ├─ CSS Snippets
  ├─ Status Bar
  Keyboard Shortcuts
  Core Features
  ├─ File Explorer
  ├─ Graph View
  ├─ Canvas
  ├─ Bases
  ├─ Daily Notes
  ├─ Templates
  ├─ Bookmarks
  ├─ File Recovery
  Terminal
  AI
  ├─ Providers
  ├─ MCP Server
  Sync
  Plugins
  ├─ Installed
  ├─ Browse
  About
```

### 17.2 Notable Settings

**Editor → Editing:**
- Default editor mode (Live Preview / Source / Reading)
- Vim mode / Emacs mode (toggle)
- Spell check: language, on/off
- Line wrapping: wrap / no wrap
- Tab size
- Auto-pair brackets/quotes
- Smart indent on Enter

**Editor → MDX:**
- Enable MDX (toggle — disables JSX parsing if off)
- Component registry path (folder containing Svelte/JSX components available for `@Component` syntax)
- MDX compilation: in-process (swc) / external dev server

**Appearance → Themes:**
- Base theme: Light / Dark / System
- Accent color: color picker
- Font overrides: UI font, editor font, monospace font
- Zoom level: 80%–200%

**Terminal:**
- Default shell
- Shell environment: inherit from OS / custom env vars
- Font family / size
- Scrollback lines

**AI → Providers:**
- Add/remove model providers (Anthropic, OpenAI, Ollama, custom)
- Per-provider API key (stored in OS keychain, never in vault files)
- Default model for AI panel
- Default model for inline actions (can differ)

**AI → MCP Server:**
- Port (default: 34521)
- Authentication token (auto-generated; used when external agents connect)
- Expose over network: local only / LAN / custom

---

## 18. Modals & Overlays

### 18.1 New Note Modal

Triggered by `Ctrl+N`:

```
┌─────────────────────────────────────────────┐
│  New note                              [✕]  │
├─────────────────────────────────────────────┤
│  Title: [________________________________]  │
│                                             │
│  Location:  📁 Current folder        [▾]   │
│  Template:  (none)                   [▾]   │
│  Type:      ● Markdown   ○ MDX              │
│                                             │
│                      [Cancel]  [Create →]  │
└─────────────────────────────────────────────┘
```

If the title matches a note that already exists, a warning appears inline: "A note with this title exists — open it?"

### 18.2 Link Picker (wikilink autocomplete)

This is not a modal but an inline floating tooltip:

```
  ┌──────────────────────────────┐
  │ 🔍 rust                      │
  ├──────────────────────────────┤
  │ 📄 Rust Async                │  ← highlighted
  │ 📄 Rust Error Handling       │
  │ 📄 Rustsidian Roadmap        │
  │ [Create "rust"]              │
  └──────────────────────────────┘
```

### 18.3 Property Type Picker

When adding a new property via the Properties panel `[+]`:

```
┌────────────────────────┐
│ Property name:         │
│ [________________]     │
│                        │
│ Type:                  │
│  Aa  Text              │
│  #   Number            │
│  📅  Date              │
│  🕐  DateTime          │
│  ✅  Checkbox          │
│  ≡   List              │
│  🔗  Link              │
└────────────────────────┘
```

### 18.4 Confirmation Dialogs

Destructive actions (delete note, clear terminal, remove sync data) require confirmation:

```
┌────────────────────────────────────────┐
│  Delete "2026-04-03.md"?               │
│                                        │
│  This action cannot be undone.         │
│  The file will be moved to the OS      │
│  trash (not permanently deleted yet).  │
│                                        │
│           [Cancel]  [Move to Trash]   │
└────────────────────────────────────────┘
```

The confirm button is always the rightmost, and is styled in a destructive color (red-tinted). Keyboard: `Enter` does **not** confirm destructive dialogs — the user must click or press a labeled key (`D` for "Delete", etc.) to reduce accidents.

---

## 19. Context Menus

Context menus use the OS-native context menu primitives exposed by Tauri (they look and feel native on each platform). All context menus share consistent item ordering: primary actions first, secondary/dangerous actions last, with separators grouping related items.

### 19.1 File Explorer Item

```
  Open
  Open in new tab
  Open in new pane
  ──────────────────
  New note here
  New folder here
  ──────────────────
  Rename             F2
  Move to…
  Duplicate
  ──────────────────
  Copy path
  Copy wikilink
  Reveal in Finder / Explorer
  ──────────────────
  Delete             Del
```

### 19.2 Editor Text Selection

```
  Cut                Ctrl+X
  Copy               Ctrl+C
  Paste              Ctrl+V
  ──────────────────
  Bold               Ctrl+B
  Italic             Ctrl+I
  Link               Ctrl+K
  ──────────────────
  Summarize (AI)
  Expand (AI)
  Ask AI about this
  Find related notes (AI)
```

### 19.3 Canvas Node

```
  Edit
  Open as note
  ──────────────────
  Color ▶  [● ● ● ● ● ●] [Custom…]
  ──────────────────
  Copy
  Duplicate
  Group with selection
  ──────────────────
  Delete
```

---

## 20. Theming System

### 20.1 CSS Custom Properties

The entire UI is defined via a hierarchy of CSS custom properties. Themes override only the design token layer; all structural styles reference tokens, never hard-coded values.

**Token layers:**

```
Layer 0: Base scales (spacing-1 = 4px, spacing-2 = 8px, …)
Layer 1: Semantic colors (--color-bg-primary, --color-text-primary, …)
Layer 2: Component tokens (--tab-bar-height, --sidebar-width-default, …)
Layer 3: Component overrides (set by themes or CSS snippets)
```

**Core semantic color tokens:**

```css
/* Backgrounds */
--color-bg-primary        /* Main editor background */
--color-bg-secondary      /* Sidebar, panel backgrounds */
--color-bg-tertiary       /* Hover states, selected items */
--color-bg-overlay        /* Modals, tooltips */

/* Text */
--color-text-primary      /* Main body text */
--color-text-secondary    /* Muted text (file paths, dates) */
--color-text-muted        /* Placeholder, disabled text */
--color-text-accent       /* Links, active icons */

/* Borders */
--color-border-primary    /* Panel borders, dividers */
--color-border-subtle     /* Input outlines */

/* Accents */
--color-accent            /* User-selected accent color */
--color-accent-hover      /* Hover variant */
--color-accent-active     /* Active/pressed variant */

/* Status */
--color-success           /* Sync ok, task complete */
--color-warning           /* Conflicts, outdated */
--color-error             /* Errors, destructive actions */

/* Syntax highlighting (editor) */
--color-syntax-heading
--color-syntax-link
--color-syntax-code
--color-syntax-comment
--color-syntax-string
--color-syntax-keyword
```

### 20.2 Built-in Themes

Two base themes ship with Rustsidian:

**Default Dark** — inspired by Obsidian's default dark theme: near-black background (`#1e1e2e`), off-white body text, purple accent (#7c6af7). Low-contrast chrome, high-contrast editor content.

**Default Light** — off-white background (`#f8f8f2`), near-black text, same purple accent. Sidebar and panels receive a very slight grey tint to distinguish them from the editor surface.

### 20.3 CSS Snippets

Users can place `.css` files in the vault's `.rustsidian/snippets/` directory. These are loaded after the theme and can override any custom property or structural style. Snippet files are listed in Settings → Appearance → CSS Snippets with individual enable/disable toggles.

### 20.4 Theme Compatibility

The token naming convention is designed to be source-compatible with Obsidian CSS themes. Any Obsidian theme that uses standard Obsidian CSS variables (`--background-primary`, `--text-normal`, etc.) should be usable in Rustsidian with a thin compatibility shim mapping Obsidian variable names to Rustsidian token names.

---

## 21. Keyboard Shortcuts

All shortcuts are configurable in Settings → Keyboard Shortcuts. Conflicts are highlighted with a warning icon.

### Navigation

| Action | Default |
|---|---|
| Open Quick Switcher | `Ctrl+O` |
| Open Command Palette | `Ctrl+P` |
| New note | `Ctrl+N` |
| Open daily note | `Ctrl+Shift+D` |
| Close current tab | `Ctrl+W` |
| Next tab | `Ctrl+Tab` |
| Previous tab | `Ctrl+Shift+Tab` |
| Navigate back | `Ctrl+Alt+←` |
| Navigate forward | `Ctrl+Alt+→` |
| Toggle left sidebar | `Ctrl+B` |
| Toggle right sidebar | `Ctrl+Shift+B` |
| Toggle terminal | `Ctrl+`` ` |
| Split editor right | `Ctrl+\` |
| Focus next pane | `Ctrl+K → Ctrl+→` |

### Editing

| Action | Default |
|---|---|
| Bold | `Ctrl+B` |
| Italic | `Ctrl+I` |
| Strikethrough | `Ctrl+Shift+S` |
| Insert link | `Ctrl+K` |
| Insert wikilink | `Ctrl+Shift+K` |
| Insert code block | `Ctrl+Shift+\`` |
| Toggle task done | `Ctrl+Enter` (on task line) |
| Indent list item | `Tab` |
| Outdent list item | `Shift+Tab` |
| Move line up | `Alt+↑` |
| Move line down | `Alt+↓` |
| Duplicate line | `Ctrl+Shift+D` |
| Find in file | `Ctrl+F` |
| Find in vault | `Ctrl+Shift+F` |

### View

| Action | Default |
|---|---|
| Toggle editor mode | `Ctrl+E` (cycles LP → Source → Reading) |
| Toggle reading view | `Ctrl+Shift+E` |
| Zoom in | `Ctrl+=` |
| Zoom out | `Ctrl+-` |
| Reset zoom | `Ctrl+0` |
| Focus file explorer | `Ctrl+Shift+E` |
| Open settings | `Ctrl+,` |
| Open graph view | no default (assign in settings) |

---

## 22. Rustsidian-Specific UI Additions

These surfaces and behaviors have no Obsidian equivalent. They are the UX expression of Rustsidian's three differentiating features.

### 22.1 MDX Component Picker

In an MDX note, typing `@` or clicking `[MDX+]` in the toolbar opens the component picker:

```
┌──────────────────────────────────────────────┐
│  Insert component                       [✕]  │
├──────────────────────────────────────────────┤
│  🔍 [chart__________________]                │
├──────────────────────────────────────────────┤
│  ▾ Data Visualization                        │
│     📊 LineChart                             │   ← selected
│     📊 BarChart                              │
│     🗺 WorldMap                              │
│  ▾ Layout                                    │
│     ⬜ Callout                               │
│     📑 Tabs                                  │
│     📐 Grid                                  │
│  ▾ Vault                                     │
│     🔗 BacklinkList                          │
│     📋 TaskList                              │
├──────────────────────────────────────────────┤
│  Preview:                                    │
│  ┌────────────────────────────┐              │
│  │  [sample chart render]     │              │
│  └────────────────────────────┘              │
│  Props: data (required), title, color        │
└──────────────────────────────────────────────┘
```

Selecting a component inserts the JSX skeleton with props scaffolded and cursor positioned at the first required prop.

### 22.2 MDX Component Error States

If a component fails to render (missing props, compilation error, import not found):
- The component area renders a red-bordered error card with the error message
- A `[Fix with AI]` button offers to diagnose the error using the AI panel
- The raw JSX source is shown beneath the error for manual editing

### 22.3 Terminal ↔ Vault Integration

The terminal knows about the vault. Special behaviors:
- `cd`-ing into the vault root updates the File Explorer's visual focus (optional, toggle in Terminal settings)
- Output containing vault-relative file paths renders them as clickable links that open the note in the editor
- The `rustsidian` CLI (when running) can open notes: `rustsidian open "Note Title"` — note opens in the active pane

### 22.4 MCP Server Status Indicator

When the embedded MCP server is running (which it always is during normal operation), the status bar shows an `MCP` chip. Clicking it opens a popover:

```
  ┌─────────────────────────────────────┐
  │  MCP Server                         │
  │  Status: ✓ Running                  │
  │  Port: 34521                        │
  │  Connections: 2 (Claude Code, ...)  │
  │  Tools registered: 24               │
  │  [Copy token]  [Stop server]        │
  └─────────────────────────────────────┘
```

This allows the user to hand the MCP connection string to external agents (Claude Code, Cursor, etc.) without leaving Rustsidian.

### 22.5 AI Inline Generation

In any editor pane, pressing `Ctrl+Shift+G` opens an inline generation bar at the cursor position:

```
  …existing note text…
  ────────────────────
  ✨ [Generate: _________________________________]  [→]
  ────────────────────
  …existing note text continues…
```

The user types a natural language instruction. On submit, the generated text is streamed inline as a ghost diff (grey text) that the user can accept (`Tab`) or reject (`Esc`). The generation is scoped to the AI panel's current context.

---

## 23. Responsive & Accessibility Constraints

### 23.1 Minimum Window Size

The application enforces a minimum window size of **800 × 600px**. Below this, the user sees a "window too small" overlay. This is a hard minimum — Rustsidian is a desktop-first application and does not attempt to reflow for phone-sized windows.

### 23.2 Sidebar Collapse Cascade

When the window is resized below certain thresholds, sidebars auto-collapse to their icon-only state:

| Window Width | Auto-collapse |
|---|---|
| < 900px | Right sidebar collapses |
| < 700px | Left sidebar collapses |
| < 500px | Terminal panel collapses |

These are soft collapses — the user can still manually expand them.

### 23.3 Keyboard Navigation

Every interactive element in the UI must be reachable by keyboard:
- Tab order within a panel follows reading order (top → bottom, left → right)
- Modal dialogs trap focus until closed
- All context menus are navigable by arrow keys and closable by Escape
- The Command Palette is the universal escape hatch — if you can't find a UI control, Ctrl+P finds the command

### 23.4 Screen Reader Support

- All icon buttons have `aria-label` attributes
- The file tree uses `role="tree"` and `role="treeitem"` with proper `aria-expanded` state
- The editor is a CodeMirror instance with its own accessibility layer (CodeMirror 6 includes ARIA roles for the editing surface)
- Status bar segments have `role="status"` and update are announced via `aria-live`

### 23.5 Font Scaling

All layout dimensions are defined in `rem` units (not `px`) where they should scale with the user's OS font size preference. The zoom setting in Appearance scales the root font size from 80%–200%.

---

## Appendix A: Pane Type Summary

| Pane Type | Created By | Content Model |
|---|---|---|
| Note (Live Preview) | open any .md/.mdx | CodeMirror + Markdown/MDX decorations |
| Note (Source) | toggle mode | CodeMirror plain text |
| Note (Reading) | toggle mode | HTML render of document |
| Canvas | open .canvas file | xterm-like 2D surface |
| Graph View | ribbon / command | D3/WebGL force graph |
| Base View | open .base file | Table/Kanban/Calendar widget |
| Terminal | ribbon / Ctrl+` | xterm.js + PTY |
| Web Viewer | open external URL | Tauri webview iframe |
| AI Panel | right sidebar | Conversation UI |

---

## Appendix B: File Type to Icon Mapping

| Extension | Icon | Notes |
|---|---|---|
| `.md` | 📄 | Standard markdown note |
| `.mdx` | 📄+ | MDX note (small + badge) |
| `.canvas` | 📊 | Canvas / whiteboard |
| `.base` | 🗃 | Database view definition |
| `.png`, `.jpg`, `.gif`, `.svg`, `.webp` | 🖼 | Image |
| `.pdf` | 📋 | PDF |
| `.mp3`, `.wav`, `.flac`, `.ogg` | 🎵 | Audio |
| `.mp4`, `.mov`, `.webm` | 🎬 | Video |
| `.zip`, `.tar.gz` | 📦 | Archive |
| other | 📎 | Generic attachment |

---

*End of document — v0.1, April 4, 2026*
