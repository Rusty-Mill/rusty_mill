> **Archived 2026-09-08** — Imported verbatim from the private `baileyrd/Rustsidian` repo (`docs/`) under RFC 0009 row 6. Rustsidian is retired; this is reference material only and describes Rustsidian's SvelteKit/Tauri stack, not Nexus.

# Obsidian 1.12.7 — Detailed UI & Feature Analysis

**Binary analyzed:** `Obsidian.exe`
**Version:** 1.12.7 (ProductVersion: 1.12.7.0)
**Released:** March 23, 2026
**Vendor:** Dynalist Inc.
**Runtime:** Electron (Chromium 127), x86-64 PE32+ (Squirrel installer)
**Installer type:** Squirrel self-extracting, embeds `app.asar` + `obsidian.asar`

---

## 1. Binary / Technical Characteristics

| Property | Value |
|---|---|
| File size | ~201 MB |
| Format | PE32+ GUI, x86-64, Squirrel v1 installer |
| Electron version | Electron 39.6.0 (rolled back from 39.7.0 in 1.12.4 due to window-close bug) |
| Chromium base | 127.0.0.x |
| PE sections | 14 (`.text` 172 MB, `.rdata` 31 MB, `.rsrc` 112 KB, `LZMADEC` stub) |
| Embedded payloads | `resources\app.asar` (SHA-256: `2815a6…`), `resources\obsidian.asar` (SHA-256: `13510e…`) |
| JS compilation | V8 bytecode — application logic is not plaintext in the binary |
| Installer | Squirrel.Windows; post-install layout: `<AppData>\Obsidian\app-1.12.7\` |
| Signing note | Installer re-signed in 1.12.4 to reduce antivirus false-positives |

The separation of `app.asar` (Electron shell + shared libs) and `obsidian.asar` (application logic) is an Obsidian-specific packaging choice that lets them update the vault logic without shipping a new Electron binary.

---

## 2. Top-Level UI Architecture

Obsidian's desktop UI is a single-window Electron app organized into a fixed chrome layer (title bar, ribbon) and a resizable workspace in the center.

```
┌─────────────────────────────────────────────────────────────────┐
│  Title Bar  [← →]  [Vault Name — Obsidian v1.12.7]  [─ □ ✕]  │
├───┬─────────────────────────────────────────────────────────────┤
│ R │  Left Sidebar                 │  Editor / Content Area      │  Right Sidebar
│ i │  [Tab strip]                  │  [Tab bar]                  │  [Tab strip]
│ b │  ┌──────────────────────────┐ │  ┌─────────────────────┐   │  ┌──────────────┐
│ b │  │ Active pane              │ │  │  Tab 1 │ Tab 2 │ + │   │  │ Backlinks    │
│ o │  │ (File Explorer / Search  │ │  ├─────────────────────┤   │  │ Outgoing lnk │
│ n │  │  / Bookmarks / Tags /    │ │  │                     │   │  │ Outline      │
│   │  │  Bases / etc.)           │ │  │  Note content       │   │  │ Tags         │
│   │  └──────────────────────────┘ │  │  (Live Preview /    │   │  └──────────────┘
│   │                               │  │   Source / Reading) │   │
├───┴───────────────────────────────┴──┴─────────────────────┴───┴─────────────────┤
│  Status Bar                                                    [word count] [mode]│
└───────────────────────────────────────────────────────────────────────────────────┘
```

---

## 3. Title Bar

The title bar is custom-rendered (not native OS chrome) and contains:

- **Back / Forward arrows** — navigate session history across open notes (like browser forward/back)
- **Vault name** — centered text showing the active vault name
- **Version badge** — displays the running Obsidian version
- **Window controls** — minimize, maximize/restore, close (Windows-style; positioned right)

The non-native title bar means it cannot be disabled or replaced from within the app. It participates in CSS theming (the `--titlebar-*` CSS variables). In 1.11+, corner smoothing is controlled by the `corner-shape` CSS property (replacing `-electron-corner-smoothing`).

---

## 4. Ribbon

The ribbon is a narrow vertical strip pinned to the far left of the window, outside and independent of the left sidebar. It remains visible when the left sidebar is collapsed.

**Default ribbon items (top to bottom):**
- Open another vault (vault switcher icon)
- Open command palette (pill/spark icon)
- Open a new note
- Open file explorer (toggle left sidebar)
- Open search
- Open graph view
- Open daily note
- Open canvas (new blank canvas)

**Customization:** Right-clicking any ribbon item reveals options to remove it or move it. Items can be re-ordered via drag. Plugins register their own ribbon icons through the API. The ribbon itself can be hidden from `Settings → Appearance → Advanced → Show ribbon`.

---

## 5. Left Sidebar

The left sidebar contains a **tab strip** at the top and a **pane area** below. Multiple plugin tabs can coexist and be switched between.

**Default tabs (core plugins):**
| Tab | Plugin | Function |
|---|---|---|
| Files | File Explorer | Tree view of vault folders and files |
| Search | Global Search | Full-text search with query operators |
| Bookmarks | Bookmarks | Saved notes, headings, blocks, searches |
| Tags | Tag Pane | Hierarchical tag browser |

Additional tabs are added by enabled core or community plugins (e.g., Bases, Calendar, Kanban).

**Sidebar behavior:**
- Collapses/expands via a toggle button or keyboard shortcut (`Ctrl+Shift+L` on Windows)
- Width is resizable by dragging the splitter
- Maintains state across sessions
- On mobile: hidden by default, revealed by swipe-right gesture

### 5.1 File Explorer

A hierarchical file/folder tree for the active vault. Features:
- Create new file or folder (toolbar buttons)
- Right-click context menu: New note, New folder, Rename, Move, Delete (with optional attachment deletion as of 1.12.x), Copy path, Copy Obsidian URI, Reveal in system explorer
- Drag-and-drop to move files or folders
- Collapse all / expand all
- Sort by: File name (A–Z, Z–A), Modified time (new/old first), Created time
- Pinned notes appear at the top when using Bookmarks
- In 1.12.x: when deleting a file, a new prompt asks whether to also delete its attachments (configurable in `Settings → Files & Links`)

### 5.2 Global Search

Vault-wide full-text search with real-time results. Features:
- Query operators: `file:`, `path:`, `content:`, `tag:`, `line:`, `section:`, `block:`, `task:`, `task-todo:`, `task-done:`
- Boolean operators: `AND`, `OR`, `NOT`
- Regular expression support with `regex:` prefix
- Fuzzy matching toggle
- Context snippets for each match
- Collapse/expand file matches
- Copy search results
- Filters: match case, match whole word, explain search terms
- Search history

### 5.3 Bookmarks

Replaces the older Starred plugin (renamed in Obsidian 1.2). Allows saving:
- Notes
- Headings within notes
- Individual blocks (paragraphs, list items)
- Saved searches (with a snapshot of the query)
- Folders
- Graph views

Bookmarks can be organized into named groups and reordered via drag-and-drop.

### 5.4 Tag Pane

Lists all unique tags used across the vault in a collapsible tree (supports nested tags via `#category/subcategory` convention). Each tag shows a count of uses. Clicking a tag opens a search for that tag.

---

## 6. Editor / Content Area (Main Workspace)

The central area is the primary interaction surface and contains one or more **tab groups**, each holding multiple **tabs**.

### 6.1 Tab Bar

Tabs behave like browser tabs:
- `Ctrl+T` opens a new tab; `Ctrl+W` closes the active tab
- Middle-click closes a tab
- Tabs can be pinned (right-click → Pin) so they don't get replaced on navigation
- Tabs can be moved between tab groups via drag
- Long filenames are truncated with ellipsis; hover reveals full name
- A `+` button at the right of the tab bar opens a new empty tab
- Context menu on any tab: Close, Close others, Close to the right, Split right, Split down, Move to new window, Pin/Unpin

### 6.2 Tab Groups and Split Panes

The editor area supports arbitrary splits:
- Split horizontally (side-by-side) or vertically (stacked)
- Multiple tab groups can be visible simultaneously
- Each tab group has its own tab bar
- Linked panes: two panes can be linked so scrolling/following is synchronized
- **Stacked tabs mode:** tabs slide over each other horizontally (useful for navigating many notes in a narrow column)

### 6.3 Pop-out Windows (Desktop only)

Any note or pane can be opened in a separate OS window:
- Right-click tab → "Move to new window"
- The pop-out window is a full Obsidian instance scoped to that note
- Useful for multi-monitor setups
- Pop-out windows persist across sessions if the Workspaces plugin remembers them

### 6.4 Editing Modes

Every note tab in the editor area has three display modes, toggled from the status bar or via `Ctrl+E`:

**Source Mode**
- Raw Markdown text, no rendering
- Syntax markers (asterisks, brackets, etc.) are fully visible
- Ideal for precise editing, debugging wikilinks, or working with frontmatter

**Live Preview** (default)
- Renders Markdown in place while you type
- Syntax markers collapse/hide when the cursor moves away from them
- Links, images, tables, callouts, MathJax, and code blocks all render inline
- The cursor position determines what is "raw" vs "rendered"
- As of 1.12.x: images can be resized by dragging from corners; double-click corner resets size

**Reading View**
- Fully rendered, non-editable output
- Maximum fidelity to the final rendered appearance
- Useful for proofreading and sharing screenshots
- Accessible via the book icon in the tab title bar

---

## 7. Editor Features (Content-Level)

### 7.1 Markdown & Obsidian Flavored Markdown (OFM)

Obsidian supports standard CommonMark plus its own extensions:

| Feature | Syntax |
|---|---|
| Wikilinks | `[[Note Name]]` or `[[Note Name\|Display Text]]` |
| Block references | `[[Note#^blockid]]` |
| Embeds | `![[Note]]`, `![[Note#Heading]]`, `![[image.png\|300]]` |
| Tags | `#tag`, `#nested/tag` |
| Footnotes | `[^1]` / `[^1]: footnote text` |
| Callouts | `> [!NOTE]`, `> [!WARNING]`, `> [!TIP]`, etc. (17+ types) |
| Math | `$inline$` and `$$block$$` (MathJax/KaTeX) |
| Mermaid diagrams | ` ```mermaid ` code blocks |
| Task lists | `- [ ]` and `- [x]` |
| Highlight | `==highlighted text==` |
| Strikethrough | `~~strikethrough~~` |

### 7.2 Properties (Frontmatter)

The Properties panel replaces the raw YAML frontmatter display with a typed UI (introduced in Obsidian 1.4):
- Property types: Text, Number, Date, Date & Time, Checkbox, List, Tags, Aliases
- Click any property value to edit inline
- Add new properties from the `+` button
- Property names are globally registered and type-inferred across the vault
- Properties panel can be toggled from the view menu or `Ctrl+;`
- Source mode still shows raw YAML; Live Preview and Reading mode show the Properties UI

### 7.3 Canvas

A freeform spatial note-taking surface (`.canvas` files, JSON format):
- Nodes: notes (embedded `.md` files), plain text cards, web pages (iframes), images, videos, PDFs
- Edges: directed or undirected arrows with optional labels
- Groups: colored bounding boxes to cluster nodes
- Infinite canvas with pan (middle-drag or Space+drag) and zoom (scroll or Ctrl+scroll)
- Minimap in the bottom-right corner
- **1.12.x addition:** Canvas files now appear in Backlinks view; links within a canvas count in Graph view

### 7.4 Slash Commands

Type `/` in the editor to open an inline command palette for inserting:
- Tables
- Code blocks
- Callouts
- Horizontal rules
- Embeds
- Templates
- Commands from enabled plugins

### 7.5 Vim Mode

Optional Vim keybinding layer (toggle in `Settings → Editor → Vim key bindings`). Supports:
- Normal, insert, visual modes
- Standard Vim motions (`hjkl`, `w`, `b`, `gg`, `G`, `0`, `$`)
- Operators (`d`, `c`, `y`, `p`)
- Ex commands (`:w` saves, `:q` navigates away)
- Macros via `q`

---

## 8. Right Sidebar

The right sidebar mirrors the left sidebar structurally (tab strip + pane area) but defaults to content-contextual plugins.

**Default / commonly populated tabs:**
| Tab | Plugin | Function |
|---|---|---|
| Backlinks | Backlinks | Notes linking TO the current note |
| Outgoing links | Outgoing links | Links FROM the current note |
| Outline | Outline | Heading hierarchy for the current note |
| Tags | Tag Pane | (if also placed here) |

### 8.1 Backlinks Panel

Shows every note that contains a wikilink, alias link, or unlinked mention of the currently open note. Sub-sections:
- **Linked mentions** — explicit `[[wikilinks]]` to the current note
- **Unlinked mentions** — text occurrences matching the note title or aliases (toggle on/off)
- Each result shows context snippet; click to open
- In 1.12.x: backlinks now include Canvas files that reference the current note

### 8.2 Outgoing Links Panel

Shows all links from the current note:
- Linked (valid, resolved wikilinks)
- Unlinked (suggested links — text in the current note matching other note titles)

### 8.3 Outline Panel

Live table of contents derived from the heading structure (`#`, `##`, `###`, etc.):
- Click any heading to jump to it in the editor
- Updates in real-time as headings are added/removed
- Indentation reflects heading level

---

## 9. Bases (Core Plugin — introduced ~1.7, matured in 1.12)

Bases turns the vault into a database. A `.base` file is a view definition that queries notes by their properties.

### 9.1 View Types

| View | Description |
|---|---|
| **Table** | Notes as rows; columns map to properties |
| **List** | Bulleted/numbered list of note titles with optional metadata |
| **Gallery** | Grid of cards; supports cover images via a configured image property |
| **Map** | Pins on an interactive map using latitude/longitude properties |

### 9.2 Filtering

Two filter stages applied sequentially:
- **All views filter** — applies to all views in the base (scope limiter)
- **This view filter** — applies only to the current view (refinement)

Filter conditions support: property value, tag, folder, file name, created/modified date, file size. Boolean logic: AND, OR, NOT. Advanced mode accepts a structured expression syntax (toggled via the `</>` icon).

In 1.12.x: A search toolbar button was added for filtering query results directly. Right-clicking a single row now reveals file-context menu actions.

### 9.3 Formulas

Custom computed columns defined in the Properties pop-up via **Add formula**. Formula language supports:
- Arithmetic: `+`, `-`, `*`, `/`, `^`, `%`
- Comparison: `=`, `!=`, `<`, `>`, `<=`, `>=`
- Boolean: `and`, `or`, `not`
- String ops: `concat()`, `length()`, `contains()`, `slice()`, `replace()`, `upper()`, `lower()`
- Date ops: `now()`, `today()`, `dateAdd()`, `dateSub()`, `format()`
- Math: `abs()`, `ceil()`, `floor()`, `round()`, `min()`, `max()`
- Conditional: `if(condition, trueVal, falseVal)`
- Property access: `prop("property-name")`

### 9.4 Grouping, Sorting, and Summaries

- **Sort:** click column headers to cycle ascending → descending → unsorted
- **Group by:** any property; creates collapsible section headers per unique value
- **Summaries** (bottom row in Table view): Average, Sum, Min, Max, Median, Earliest, Latest, Checked, Unique — available per column based on property type
- In 1.12.x: drag-and-drop to import files into Bases views

---

## 10. Command Palette

Opened with `Ctrl+P` (or `Cmd+P` on macOS). A modal fuzzy-search over all registered commands from core and community plugins. Features:
- Fuzzy matching — typing abbreviations works (e.g., "dnop" matches "Daily Note: Open today's note")
- Pinned commands — pin frequently used commands to appear at the top
- Hotkey badge shown next to commands that have assigned shortcuts
- Recent commands shown at top on first open
- `Esc` or click outside to close

---

## 11. Quick Switcher

Opened with `Ctrl+O`. Fuzzy filename search for fast note navigation:
- Matches on file name and path
- Recent files shown at top before typing
- `Enter` opens in the current tab; `Ctrl+Enter` opens in a new tab
- Alias support: if a note has an alias, it can be found by that alias
- The Enhanced Quick Switcher community plugin extends this with header/block search

---

## 12. Graph View

A force-directed node-link diagram of the vault's wikilink structure.

**Controls:**
- Pan: drag background
- Zoom: scroll wheel
- Node click: opens note
- Node hover: highlights connections

**Filters panel (left side of graph overlay):**
- Search: filter which nodes are visible by filename match
- Tags: toggle tag nodes on/off
- Attachments: show/hide attachment files
- Existing files only: hide orphan stubs
- Orphans: show/hide notes with no links

**Groups:** Color-code nodes by tag, folder, or path pattern

**Display settings:**
- Node size: scales by number of links (backlinks or local)
- Link thickness
- Text fade threshold (zoom level at which labels appear)
- Center force, repel force, link force, link distance (physics simulation sliders)

**Local Graph:** Available from the view menu in any note — shows only the note's immediate neighbor network up to N hops (configurable depth: 1–5).

---

## 13. Settings Modal

Opened with `Ctrl+,`. A full-screen modal organized into a left sidebar (categories) and a right content panel.

### 13.1 Options Categories

| Category | Key settings |
|---|---|
| **Editor** | Default editing mode (Live Preview / Source), spell check, line width, font size, vim keybindings, smart indent with lists, auto-pair brackets/Markdown syntax, fold indents, fold headings, show line numbers, relative line numbers, strict line breaks, properties position (top/inline), auto-convert HTML |
| **Files & Links** | Default location for new notes (vault root / same folder / specified folder), folder to watch for new attachments, default attachment location, wikilinks vs Markdown links, relative vs shortest path, detect all file extensions, delete behavior (system trash / vault trash / permanent), confirm before deleting attachments (1.12.x), auto-update internal links on rename |
| **Appearance** | Base theme (Light / Dark / Adapt to system), color scheme (Obsidian default + installed themes), interface font, text font, monospace font, font size (px), line height, custom CSS snippets (enable/disable per snippet), zoom level, native menus toggle, window frame style (hidden / native), show inline title, show tab title bar, ribbon visibility, corner-shape (1.11+) |
| **Hotkeys** | Searchable list of all commands; click to assign hotkey; conflict detection |
| **About** | Version info, check for updates, auto-update toggle, opt-in to Insider builds, debug info copy, open sandbox vault |

### 13.2 Account (Obsidian Account)

- Sign in / sign out
- Access Obsidian Sync and Obsidian Publish subscriptions
- View sync vault list and storage usage
- Manage published sites

### 13.3 Core Plugins

Each core plugin has a toggle (on/off) and a settings gear icon:

| Plugin | Function |
|---|---|
| Audio recorder | Record audio directly into a note |
| Backlinks | Backlinks panel (right sidebar) |
| Bases | The `.base` database-view system |
| Bookmarks | Save and organize note/heading/block bookmarks |
| Canvas | Freeform spatial boards |
| Command palette | `Ctrl+P` quick-command modal |
| Daily notes | Create/open a note for today by date template |
| File explorer | Left sidebar file tree |
| File recovery | Snapshot history; restore previous versions of a note |
| Files | Core file list (underlies File Explorer) |
| Format converter | One-time Markdown format conversion on import |
| Graph view | Force-directed link graph |
| Note composer | Merge notes, extract selections to new notes |
| Outgoing links | Right sidebar outgoing links panel |
| Outline | Right sidebar heading TOC |
| Page preview | Hover preview popup when mousing over wikilinks |
| Properties view | Right sidebar all-notes property browser |
| Publish | Publish vault content to Obsidian Publish |
| Quick switcher | `Ctrl+O` filename search modal |
| Random note | Open a random note from the vault |
| Search | Left sidebar global search |
| Slash commands | `/` inline command menu in editor |
| Slides | Render a note as a slide presentation (reveal.js) |
| Sync | Obsidian Sync (requires subscription) |
| Tag pane | Tag browser in sidebar |
| Templates | Insert note templates with date/time variables |
| Unique note creator | Creates notes with a UID prefix (Zettelkasten-style) |
| Web clipper | Clip web content into notes (added in 1.7+) |
| Word count | Word/character counter in status bar |
| Workspaces | Save and restore named workspace layouts |

### 13.4 Community Plugins

- Browse and install plugins from the official community registry
- Enable/disable installed plugins individually
- Each installed plugin gets its own settings panel in the left nav
- Restricted mode toggle (disables all community plugins; on by default for new vaults)
- Plugin updates shown with a badge count

---

## 14. Status Bar

Pinned to the bottom-right of the window. Contents vary by context:

**Standard items:**
- **Word count / Character count** — shows counts for the entire note or selected text
- **Editing mode indicator** — shows current mode (Live Preview / Source / Reading); click to cycle modes
- **Sync status** — spinning icon when syncing; checkmark when up to date (requires Obsidian Sync)
- **Spell check indicator** (when spell check is enabled)

Plugin items: many community plugins (e.g., Dataview, Tasks, Calendar) add their own status bar indicators. The order and visibility of status bar items can be customized via the Status Bar Organizer community plugin.

---

## 15. Vault Switcher / Vault Management

Accessible from the ribbon (vault icon) or `Ctrl+Shift+V`:
- Lists all known vaults with icons and last-opened time
- Create new vault (local)
- Open folder as vault
- Open vault from Obsidian Sync (requires account)
- The switcher opens as a modal over the current window

---

## 16. Window Management

### 16.1 Pop-out Windows

Any tab can be detached into a floating OS window (right-click tab → "Move to new window"). These windows behave as independent Obsidian instances with access to the same vault.

### 16.2 Workspaces Plugin

Save and restore named layouts:
- Saves: tab groups, open files per tab, sidebar visibility, sidebar widths, sidebar tab states, pop-out window positions
- Restore via command palette: "Manage workspaces"
- Useful for context-switching between different projects or workflows

### 16.3 Window Frame Options

`Settings → Appearance → Window frame style`:
- **Hidden** (default on Windows) — uses Obsidian's custom title bar
- **Native** — defers to the OS window chrome; title bar is provided by the OS

---

## 17. Keyboard Accessibility & Hotkeys

Obsidian is heavily keyboard-centric. All commands are reachable from the command palette. Hotkeys are configurable in `Settings → Hotkeys`.

**Key default bindings:**

| Action | Shortcut (Windows/Linux) |
|---|---|
| Command palette | `Ctrl+P` |
| Quick switcher | `Ctrl+O` |
| Search | `Ctrl+Shift+F` |
| New note | `Ctrl+N` |
| Toggle sidebar | `Ctrl+Shift+L` / `Ctrl+Shift+R` |
| Toggle reading view | `Ctrl+E` |
| Open graph view | `Ctrl+G` |
| Toggle properties | `Ctrl+;` |
| Fold/unfold heading | `Ctrl+Shift+[` / `]` |
| Close tab | `Ctrl+W` |
| Navigate history back | `Alt+←` |
| Navigate history forward | `Alt+→` |
| Open settings | `Ctrl+,` |
| Open daily note | set in plugin settings |
| Toggle checkbox | `Ctrl+L` |

---

## 18. Obsidian CLI (New in 1.12)

The headline feature of the 1.12 release cycle is the official command-line interface, making Obsidian scriptable from terminals and automated pipelines.

**Activation:** `Settings → General → Command line interface` → follow instructions to register the `obsidian` binary in PATH.

**Usage pattern:**
```
obsidian <command> [param=value] [flag]
```

Parameters use `key=value` format; flags are bare booleans; `file=<name>` resolves like a wikilink; `path=<path>` uses exact vault-relative paths.

**Command categories introduced across 1.12.x:**

| Category | Commands | Notes |
|---|---|---|
| File ops | `read`, `create`, `append`, `prepend`, `move`, `delete` | Full CRUD on vault files |
| Search | `search`, `search:context` | `search` = path-only fast search; `search:context` = full text + snippets (split in 1.12.2) |
| Daily notes | `daily:read`, `daily:append`, `daily:open` | Integrates with Daily Notes core plugin config |
| Tasks | `tasks`, `tasks toggle` | Read and toggle tasks by line number |
| Properties | `property:set`, `property:get` | Read/write frontmatter properties |
| Tags | `tags` | List tags with counts |
| Backlinks | `backlinks` | Get backlink list for a file |
| Vaults | `vault` | Target a specific vault via `vault=<name>` |
| TUI mode | (no args) | Launches a full-screen keyboard-driven file browser |
| Dev tools | `eval`, `dev:screenshot`, `dev:dom`, `dev:css`, `dev:console`, `dev:errors` | Plugin/theme development workflow |
| Plugin dev | `plugin:reload` | Hot-reload a plugin by ID |
| Help | `help` (added 1.12.2) | Shows all available commands |
| Misc | `open` | Open a specific note in the GUI |

**Output modifiers (global flags):**
- `--copy` — copies output to clipboard
- `silent` — suppresses GUI focus
- `total` — returns count on list commands
- `counts` — returns usage counts (e.g., on tags)
- `overwrite` — overwrites on create

**Bug fixes in 1.12.7 CLI:**
- Autocompletion for Obsidian commands when using `id=` parameter
- Fixed incorrect Linux-specific directory check on macOS
- Changed socket file to a hidden dotfile on macOS and Linux

---

## 19. Notable UI Changes in the 1.12.x Series

| Version | Change |
|---|---|
| 1.12.0 | CLI introduced; `search` / `search:context` split |
| 1.12.2 | `help` command added; file rename via CLI; search split |
| 1.12.4 | System language default + onboarding language picker; image resize by corner drag in Live Preview; delete-with-attachments prompt; iOS Share extension; Bases drag-and-drop import; Bases row context menu; Bases search toolbar |
| 1.12.4 | Canvas backlinks now appear in Backlinks view; Canvas links count in Graph view |
| 1.12.7 | CLI socket file moved to dotfile; macOS CLI path bug fix; CLI autocomplete for `id=`; copy/paste line behavior fixes; editor regression fixes from 1.12.4; callout image scrollbar fix in Reading mode; image double-click reset fix |

---

## 20. Theming & Customization System

Obsidian exposes a deep CSS variable system for theming. The entire UI is overridable.

**Theme layers:**
1. **Base theme** — Light or Dark (sets the CSS `data-theme` attribute)
2. **Color scheme** — Community or first-party themes installed from Settings → Appearance
3. **CSS snippets** — Individual `.css` files in `<vault>/.obsidian/snippets/`, toggled individually

**Key CSS variables:** `--background-primary`, `--background-secondary`, `--text-normal`, `--text-muted`, `--text-accent`, `--interactive-accent`, `--font-text-size`, `--line-height-normal`, `--font-monospace`, `--corner-radius` (via `corner-shape` property since 1.11).

**As of 1.12.x:** `corner-shape` CSS property replaces `-electron-corner-smoothing` for rounded UI elements; requires Chromium 139+ (available from Obsidian 1.11+).

---

## 21. Observations Relevant to Rustsidian

This analysis was performed in the context of building **Rustsidian**, a Rust/Tauri/SvelteKit-based PKM application. Key observations:

1. **The two-ASAR split** (`app.asar` vs `obsidian.asar`) is worth noting architecturally — Obsidian separates Electron plumbing from application logic for update granularity. Rustsidian can achieve similar separation via Tauri plugin crates.

2. **Bases is still table-first** — Map view and Gallery view exist but are secondary. Rustsidian's MDX-first approach could offer richer views (e.g., live JSX components as view renderers instead of opaque Electron subframes).

3. **The CLI is a socket-based IPC to the running GUI process** — The `obsidian` binary communicates with the Electron app over a Unix socket (now a dotfile). Rustsidian's MCP server design is architecturally superior: it doesn't require the GUI to be running for read/write operations.

4. **Live Preview is implemented in CodeMirror 6** (as confirmed by Obsidian's public acknowledgment and consistent with Rustsidian's own CodeMirror 6 choice). The cursor-position-aware rendering is CM6's `DecorationSet` mechanism.

5. **Properties panel is a separate view component** overlaid on the editor, not part of the CM6 editor itself — worth replicating this separation in Rustsidian's SvelteKit UI to keep editor and metadata concerns decoupled.

6. **Community plugin ecosystem is Obsidian's strongest moat** — with 1,800+ plugins. Rustsidian's WASM plugin system needs to be ergonomic enough to attract developer adoption, but the WIT-based API approach is more robust than Obsidian's raw `app` object exposure.

7. **The Settings modal is a full-screen overlay**, not a side panel — this creates a clear mental model break (you're "in settings" or "in the vault"). Worth preserving this pattern in Rustsidian.

8. **Pop-out windows use additional Electron renderer processes** — Tauri 2.0's multi-window support maps directly to this; each window can host an independent SvelteKit view referencing the shared Rust core.

---

*Analysis performed April 6, 2026. Binary: Obsidian.exe (1.12.7, March 23 2026). Sources: PE/binary analysis, web research via GitHub, search engine results, Obsidian official changelog and help documentation.*
