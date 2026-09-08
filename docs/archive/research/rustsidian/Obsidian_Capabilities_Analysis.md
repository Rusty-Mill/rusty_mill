> **Archived 2026-09-08** — Imported verbatim from the private `baileyrd/Rustsidian` repo (`docs/`) under RFC 0009 row 6. Rustsidian is retired; this is reference material only and describes Rustsidian's SvelteKit/Tauri stack, not Nexus.

# Obsidian: Comprehensive Capabilities & UI Analysis

> **Research Date:** April 3, 2026
> **Sources:** Obsidian official documentation, changelogs, community resources

---

## Table of Contents

1. [What Is Obsidian?](#what-is-obsidian)
2. [Core Philosophy & Architecture](#core-philosophy--architecture)
3. [User Interface Overview](#user-interface-overview)
4. [Editor Modes](#editor-modes)
5. [Markdown & Formatting Capabilities](#markdown--formatting-capabilities)
6. [Linking & Knowledge Graph](#linking--knowledge-graph)
7. [Organization Systems](#organization-systems)
8. [Core Plugins (30 total)](#core-plugins)
9. [Canvas](#canvas)
10. [Bases (Database Views)](#bases-database-views)
11. [Community Plugins](#community-plugins)
12. [Themes & Appearance](#themes--appearance)
13. [Search & Navigation](#search--navigation)
14. [Hotkeys & Customization](#hotkeys--customization)
15. [Obsidian Sync](#obsidian-sync)
16. [Obsidian Publish](#obsidian-publish)
17. [Mobile Apps (iOS & Android)](#mobile-apps-ios--android)
18. [New in 2026: AI & Collaboration](#new-in-2026-ai--collaboration)
19. [Pricing & Licensing](#pricing--licensing)
20. [Strengths & Limitations](#strengths--limitations)
21. [Official Roadmap: Shipped, In Progress & Planned](#official-roadmap)
22. [Web Clipper](#web-clipper)
23. [Command Line Interface (CLI)](#command-line-interface-cli)
24. [Obsidian Headless Sync Client](#obsidian-headless-sync-client)

---

## What Is Obsidian?

Obsidian is a **local-first, privacy-focused knowledge management and note-taking application** built on top of plain Markdown files. It transforms a folder of `.md` files — called a **vault** — into an interconnected, navigable web of ideas. Unlike cloud-native tools, all notes live on your device as readable, portable plain text, meaning they are fully accessible even if the Obsidian app ceases to exist.

Obsidian runs on **Windows, macOS, Linux, iOS, and Android**, and is available for free for personal use.

---

## Core Philosophy & Architecture

Obsidian is built around several foundational ideas:

- **Local-first**: All data is stored on your device as plain `.md` files. No cloud dependency for core functionality.
- **Plain text permanence**: Notes are standard Markdown, readable in any text editor. Future-proof by design.
- **Non-linear thinking**: Instead of hierarchical folder structures alone, ideas are linked into a knowledge network.
- **Extensibility**: A rich plugin ecosystem (both core and community) lets users tailor Obsidian to their exact workflow.
- **Vault model**: A vault is simply a folder on your computer. You can have multiple vaults for different projects or areas of life, and switch between them.

---

## User Interface Overview

### Main Window Layout

The Obsidian window is divided into several primary regions:

| Region | Description |
|--------|-------------|
| **Left Sidebar** | Contains the Ribbon, File Explorer, Search, Bookmarks, and other left-panel plugins |
| **Main Editor Area** | The central workspace where notes are opened in tabs or split panes |
| **Right Sidebar** | Houses Backlinks, Outgoing Links, Tags, Properties, Outline, and right-panel plugins |
| **Status Bar** | Bottom bar showing word count, cursor position, and plugin status indicators |
| **Title Bar** | At the very top; includes window controls and the vault name |

### Ribbon

The **Ribbon** is a vertical strip running along the far left edge of the interface. It provides icon shortcuts to common commands and is always visible even when the left sidebar is collapsed. Users can pin plugin commands and actions to the ribbon.

### Sidebars

Both the left and right sidebars are:
- Collapsible/expandable with a click
- Splittable into multiple stacked panels
- Draggable — panels can be moved between sidebars or pinned
- Persistent across sessions

### Tabs and Panes

Obsidian uses a **tab and pane system** similar to an IDE:
- Multiple notes can be opened as **tabs** in the main editor
- The window can be **split horizontally or vertically** into multiple panes (called a "pane layout")
- Panes can be **linked**, so they scroll together — useful for keeping an edit pane and preview pane in sync
- **Pop-out windows**: Notes can be detached into their own floating windows (on desktop)

### Workspaces

The **Workspaces core plugin** lets you save entire UI layouts — which notes are open, which panes are visible, sidebar states — and switch between them instantly. Useful for switching between a "research mode" and a "writing mode" layout.

---

## Editor Modes

Obsidian offers three distinct ways to view and edit a note:

### 1. Live Preview (Default)
The primary editing experience as of Obsidian v0.13+. Live Preview is a **WYSIWYG-adjacent** mode:
- The line where your cursor sits shows raw Markdown syntax
- Every other line is rendered (headers look like headers, bold looks bold, links appear as links)
- You see your note "as it will look" while still editing Markdown
- This is the mode most users work in daily

### 2. Source Mode
A **pure Markdown editor** view showing all raw syntax at all times. No rendering. Good for:
- Debugging complex formatting
- Copying raw Markdown to use elsewhere
- Users who prefer a traditional code-editor feel

### 3. Reading View
A **fully rendered, read-only** view of the note. No cursor, no editing. Useful for:
- Reviewing a finished note without distraction
- Sharing a screenshot
- Checking how a note renders before publishing

You can switch between modes using the toggle button in the top-right of each pane, or via the Command Palette.

---

## Markdown & Formatting Capabilities

Obsidian's Markdown implementation combines **CommonMark**, **GitHub Flavored Markdown (GFM)**, and its own extensions.

### Standard Markdown
- Headings (H1–H6)
- Bold, italic, strikethrough, highlight (`==text==`)
- Ordered and unordered lists, nested lists
- Task lists (`- [ ] item` / `- [x] done`)
- Blockquotes
- Horizontal rules
- Tables (GFM table syntax)
- Code blocks (fenced with ` ``` `) with syntax highlighting
- Inline code
- Images (`![alt](path)`)
- Links

### Obsidian-Specific Formatting Extensions

**Callouts**: One of Obsidian's most visually distinctive features. Callouts transform blockquotes into styled, color-coded alert boxes. Syntax:

```
> [!info] Title
> Body text here
```

Callout types include: `note`, `tip`, `important`, `warning`, `caution`, `abstract`, `summary`, `tldr`, `info`, `todo`, `success`, `check`, `done`, `question`, `help`, `faq`, `failure`, `fail`, `missing`, `danger`, `error`, `bug`, `example`, `quote`, `cite`. Callouts can be made **collapsible** with `+` (expanded by default) or `-` (collapsed by default).

**Math (LaTeX)**: Inline math with `$formula$`, block math with `$$formula$$`. Powered by MathJax.

**Diagrams (Mermaid)**: Code blocks marked with `mermaid` render flowcharts, sequence diagrams, Gantt charts, class diagrams, and more.

**HTML**: Limited inline HTML is supported.

---

## Linking & Knowledge Graph

### Internal Linking (Wikilinks)

The cornerstone of Obsidian. Using double-bracket syntax (`[[Note Title]]`), you can:
- Link to any note in your vault
- Link to a specific **heading** within a note: `[[Note Title#Heading]]`
- Link to a specific **block**: `[[Note Title#^blockID]]`
- Provide **display text**: `[[Note Title|Shown Text]]`
- Create a link to a note that doesn't exist yet (creates it on click)

### Embeds (Transclusion)

Prefixing a link with `!` embeds the referenced content **inline** within your note:
- `![[Note Title]]` — embed entire note
- `![[Note Title#Heading]]` — embed a specific section
- `![[Note Title#^blockID]]` — embed a specific block
- `![[image.png]]` — embed an image
- `![[file.pdf]]` — embed a PDF
- `![[audio.mp3]]` — embed audio

Embedded content updates live when the source changes.

### Backlinks

The **Backlinks panel** (right sidebar) automatically shows all notes that link to the currently open note. It can show:
- Linked mentions (explicit `[[links]]`)
- Unlinked mentions (any text that matches the note title, not yet linked)

This allows you to discover unexpected connections you didn't intentionally create.

### Graph View

The **Graph View** is one of Obsidian's most distinctive features — a live, interactive, force-directed network visualization of your entire vault:

- **Nodes** represent notes, attachments, and tags
- **Edges** represent links between them
- **Interactive**: click nodes to navigate to them, drag to rearrange
- **Filterable**: show/hide by tags, folder path, or search query
- **Colorizable**: color-code nodes by group rules (e.g., all notes tagged `#project` show red)
- **Local Graph**: Each note has its own local graph view showing only its direct neighbors (links and backlinks), not the entire vault
- **Depth control**: Expand the local graph to N degrees of separation
- **Display options**: Show/hide orphans, tags, attachments; adjust node size and link distance

---

## Organization Systems

### Folders

Standard file system folders, visible in the **File Explorer** panel. Notes can be organized into folder hierarchies. Folders can be created, renamed, moved, and deleted directly from within Obsidian.

### Tags

Tags are written inline in notes as `#tagname`. They support:
- Nested hierarchies using slashes: `#project/work/active`
- Multi-word tags: `#my-tag`
- Filtering in search, graph, and Bases
- The **Tags View** panel lists all tags and their note counts

Tags can also be defined in **Properties (frontmatter)** as a list.

### Properties (Frontmatter / YAML Metadata)

Properties are structured metadata stored at the top of each note in a YAML block. As of Obsidian 1.4+, properties have a dedicated visual editor. Supported property types:

| Type | Description |
|------|-------------|
| **Text** | Free-form strings (title, author, status) |
| **Number** | Numeric values for sorting or computation |
| **Date** | ISO date format `YYYY-MM-DD` |
| **DateTime** | ISO datetime `YYYY-MM-DDTHH:MM` |
| **Boolean** | `true` / `false` checkboxes |
| **List** | Multiple values (e.g., tags, aliases) |
| **Link** | Reference to another note |

Reserved properties with special handling:
- `tags` — Adds tags to the note visible in tag search
- `aliases` — Alternative names by which the note can be found/linked
- `cssclasses` — Apply custom CSS classes to the note's rendered view
- `publish` — Controls Obsidian Publish visibility

### Bookmarks

The **Bookmarks** panel (left sidebar) works like a browser bookmark manager for your vault. You can bookmark:
- Individual notes
- Specific headings within notes
- Specific blocks
- Searches (saved queries)
- Folders
- Graph views

Bookmarks can be organized into named groups.

---

## Core Plugins

Obsidian ships with approximately **30 core plugins**, all officially maintained by the Obsidian team. They can be individually enabled or disabled via Settings → Core plugins.

### Always-On / Essential
| Plugin | Function |
|--------|----------|
| **File Explorer** | Browse and manage the vault's file/folder tree in the left sidebar |
| **Search** | Full-text search across the vault with operators (`path:`, `file:`, `tag:`, `line:`, `section:`) |
| **Quick Switcher** | Fuzzy-search any note by name and open it instantly (Ctrl/Cmd+O) |
| **Command Palette** | Access any command by name (Ctrl/Cmd+P) — the universal launcher |
| **Graph View** | Interactive network visualization of all notes and their connections |

### Navigation & Discovery
| Plugin | Function |
|--------|----------|
| **Backlinks** | Shows which notes link to the current note; also shows unlinked mentions |
| **Outgoing Links** | Shows links from the current note; suggests potential links for matching text |
| **Outline** | Displays the heading hierarchy of the current note in a sidebar panel |
| **Tags View** | Lists all tags in the vault with note counts; click to filter |
| **Bookmarks** | Save and organize shortcuts to notes, headings, searches, and more |
| **Page Preview** | Hover over a link to see a popover preview of the linked note (no clicking needed) |

### Editing & Writing Tools
| Plugin | Function |
|--------|----------|
| **Templates** | Insert reusable note templates; supports date/time tokens |
| **Daily Notes** | Create or open today's note from a template, automatically dated |
| **Note Composer** | Merge notes together or extract selected content into a new note |
| **Slash Commands** | Type `/` in the editor to trigger an inline command picker |
| **Properties** | Visual GUI editor for YAML frontmatter properties |
| **Properties View** | Browse all properties used across the vault in a sidebar panel |

### Visual & Organization Tools
| Plugin | Function |
|--------|----------|
| **Canvas** | Infinite 2D whiteboard for visual note arrangement (separate section below) |
| **Bases** | Database-style views of notes using properties (separate section below) |
| **Workspaces** | Save and restore full UI layouts |
| **Web Viewer** | Opens external web links inside Obsidian as a browser tab |

### Utility & Maintenance
| Plugin | Function |
|--------|----------|
| **File Recovery** | Automatic periodic snapshots for note recovery; searchable by date/time |
| **Sync** | The official Sync service integration (paid add-on) |
| **Publish** | The official Publish service integration (paid add-on) |
| **Unique Note Creator** | Creates notes with UID-based filenames (Zettelkasten-style) |
| **Random Note** | Opens a random note from the vault — useful for serendipitous discovery |
| **Word Count** | Live word and character count in the status bar |
| **Starred** (deprecated → Bookmarks) | Legacy starred notes system |
| **Slides** | Render a note as a simple presentation using `---` slide separators |
| **Audio Recorder** | Record audio directly into the vault |
| **Zettelkasten Prefixer** | Prefix new notes with a timestamp UID |

---

## Canvas

**Canvas** is a core plugin introduced in late 2022 that provides an **infinite, pannable, zoomable 2D whiteboard** for visual thinking and knowledge layout. It uses the open `.canvas` file format (JSON Canvas spec) and integrates fully with the rest of the vault. Canvas is free and included with Obsidian.

---

### Node Types

Canvas has four distinct node types, each stored as a `node` object in the `.canvas` JSON file:

#### 1. Text Nodes
- Standalone cards containing arbitrary **Markdown-formatted text** typed directly on the canvas
- Support the full Markdown syntax available in regular notes (headings, bold, links, etc.)
- Created by double-clicking on blank canvas space or via the toolbar
- Useful for quick annotations, labels, or self-contained ideas that don't warrant a full vault note

#### 2. File Nodes (Note Nodes)
- Cards linked to an **existing file in the vault** — most commonly `.md` Markdown notes
- Also support images (PNG, JPG, GIF, SVG), PDFs, audio files, and video files
- Support an optional **subpath** parameter to link directly to a specific heading (`#Heading`) or block (`#^blockID`) within a note
- The linked note can be **edited in-place** on the canvas by double-clicking — no need to navigate away
- Changes made on canvas are reflected in the actual note file
- Deletions or renames of the source file are reflected in the canvas

#### 3. Link Nodes (Web Nodes)
- Cards embedding an **external URL** as a live, interactive browser window within the canvas
- URLs are converted into cards that can be expanded to render the full web page
- Fully interactive — you can scroll, click, and interact with the embedded site
- Useful for referencing online sources alongside related notes without context-switching

#### 4. Group Nodes
- Transparent, labeled **bounding boxes** that visually contain and organize other nodes
- Can carry an optional **label** (text displayed at the top of the group)
- Can display an optional **background image** (vault file)
- Groups do not enforce containment programmatically — they're a visual/organizational tool
- Dragging a group moves all nodes inside it together

---

### Edges (Connections)

Edges define relationships between nodes. Every edge is stored in the `.canvas` file's `edges` array with the following configurable properties:

| Property | Options | Default | Description |
|----------|---------|---------|-------------|
| `fromNode` | Node ID | *(required)* | The source node |
| `toNode` | Node ID | *(required)* | The target node |
| `fromSide` | `top`, `right`, `bottom`, `left` | auto | Which side of the source node the edge originates from |
| `toSide` | `top`, `right`, `bottom`, `left` | auto | Which side of the target node the edge connects to |
| `fromEnd` | `none`, `arrow` | `none` | Whether there is an arrowhead at the source end |
| `toEnd` | `none`, `arrow` | `arrow` | Whether there is an arrowhead at the target end |
| `color` | hex string or preset `"1"`–`"6"` | *(theme default)* | Color of the edge line |
| `label` | string | *(none)* | Optional text label displayed on the edge midpoint |

By default, edges are **directed** (arrow at the target end). Setting both `fromEnd` and `toEnd` to `none` creates an undirected line. Setting `fromEnd: arrow` creates a bidirectional arrow. Edge labels float centered on the connection line.

If connected nodes are far apart, you can **navigate to the source or target** by right-clicking the edge and selecting "Go to source" or "Go to target."

---

### Color System

Both nodes and edges support a flexible color encoding with two formats:

- **Preset palette positions**: Strings `"1"` through `"6"` map to six theme-defined colors. The exact colors are intentionally left to the theme/application to define, preserving visual consistency across themes.
- **Custom hex colors**: Any hex string in `#RRGGBB` format (e.g., `"#FF5733"`) for arbitrary custom color.

This applies to card backgrounds, card borders, and edge lines — giving full control over visual coding of relationships and categories.

---

### Navigation & Interaction

#### Panning
| Method | Action |
|--------|--------|
| Scroll wheel | Pan vertically |
| `Shift` + scroll | Pan horizontally |
| `Space` + left-click drag | Pan freely |
| Right-click drag | Pan freely |
| Middle-click drag | Pan freely |

#### Zooming
| Method | Action |
|--------|--------|
| `Ctrl`/`Cmd` + scroll | Zoom in/out |
| `Space` + scroll | Zoom in/out |
| Zoom controls (top-right) | Manual zoom in/out buttons |
| `Shift+1` | **Zoom to fit** — show all nodes |
| `Shift+2` | **Zoom to selection** — fit selected nodes |
| Right-click → "Zoom to selection" | Same as above via context menu |

#### Selection
- **Click** a node to select it
- **Drag** across blank canvas space to rubber-band select multiple nodes
- **Shift+click** to add/remove from selection
- With multiple nodes selected: move together, apply bulk color, delete all, or align to each other

#### Node Operations
- **Resize**: Drag any edge or corner of a card
- **Connect**: Hover a node's edge to reveal connection handles; drag to another node to draw an edge
- **Edit**: Double-click a text or note card to enter edit mode in-place
- **Color**: Right-click → assign one of 6 preset colors or a custom hex
- **Copy/paste**: Works across different canvas files within the same vault

#### Shortcuts Reference
A **`?` button** in the top-right corner of the canvas surface shows all available keyboard shortcuts in a quick reference overlay.

---

### Embedding & Nesting

Canvas is deeply integrated with the rest of the vault:

- **Canvas in notes**: A `.canvas` file can be embedded inside a Markdown note using standard embed syntax (`![[filename.canvas]]`), displaying a live preview of the canvas inline
- **Canvas within canvas**: A canvas file can itself be added as a card on another canvas — enabling nested hierarchical visual structures
- **Notes in canvas**: Any note in the vault can be opened as a card on any canvas

---

### Vault Integration

| Feature | Details |
|---------|---------|
| **File Explorer** | `.canvas` files appear like regular notes and can be organized in folders |
| **Graph View** | Notes referenced on a canvas generate actual graph links; canvas participates in the knowledge graph |
| **Backlinks** | Any note embedded on a canvas generates a backlink in the referenced note's backlinks panel |
| **Search** | Canvas files are searchable via the vault's full-text search |
| **Quick Switcher** | Canvas files appear in the Quick Switcher and can be opened like any note |
| **Bookmarks** | Canvas files can be bookmarked |
| **Version History** | Canvas changes are tracked in Obsidian Sync's version history |

---

### Export

- **Export to PNG**: Right-click the canvas background → "Export as image" produces a PNG snapshot of the entire canvas or a selected region

---

### JSON Canvas: Open File Format

The `.canvas` file format was published by Obsidian as an **open specification** called [JSON Canvas](https://jsoncanvas.org/), released under the MIT license. Key facts:

- Stored as standard `.json` files with a `.canvas` extension — readable in any text editor
- Top-level structure: `{ "nodes": [...], "edges": [...] }`
- **Any application or tool can freely implement the format** for import, export, or storage
- Designed for longevity, readability, interoperability, and extensibility
- Source on GitHub: [obsidianmd/jsoncanvas](https://github.com/obsidianmd/jsoncanvas)

This means `.canvas` files are not proprietary — your visual layouts are as portable as your Markdown notes.

---

### Use Cases

Canvas excels in the following scenarios:

- **Brainstorming & mind mapping**: Lay out ideas spatially without linear structure
- **Research synthesis**: Arrange source notes, quotes, and web references to find connections
- **Project planning**: Visual overview of phases, tasks, and dependencies
- **Storyboarding & narrative structure**: Lay out scenes, chapters, or story arcs spatially
- **System diagramming**: Map out architectures, workflows, or processes with labeled arrows
- **Zettelkasten MOC (Map of Content)**: Build a visual index of related notes as an alternative to text-based MOCs
- **Presentation storyboarding**: Plan a presentation's visual flow before building slides
- **Personal knowledge dashboards**: A single canvas that links out to key notes and reference materials

---

### Community Extensions

The [Advanced Canvas](https://github.com/Developer-Mike/obsidian-advanced-canvas) community plugin significantly extends Canvas capabilities:

- **Graph view integration**: Canvas nodes appear in and interact with the vault's graph view
- **Unlimited styling options**: Custom node shapes, borders, and backgrounds beyond the 6 built-in colors
- **Flowchart mode**: Snap-to-grid and flow-optimized layouts
- **Dynamic presentations**: Step through canvas cards as presentation slides
- **Interconnected knowledge features**: Enhanced navigation and relationship modeling between canvas nodes

---

## Bases (Database Views)

**Bases** is a newer core plugin (introduced 2024–2025) that adds a `.base` file format for **database-style views of your notes**. All data is backed by the local Markdown files and their YAML properties.

### View Types
Each Base file can have multiple views:

| View | Description |
|------|-------------|
| **Table** | Spreadsheet-like grid; files as rows, properties as columns |
| **List** | Bulleted or numbered list of notes matching the filter |
| **Cards** | Gallery-style grid; useful for image-rich notes or kanban-like boards |

### Key Capabilities
- **Filtering**: Apply multi-condition filters using property comparisons (`is`, `is not`, `contains`, `greater than`, etc.). Filters can be stacked (AND/OR logic). Each view has its own filter, plus a global filter applied before view-level filters.
- **Sorting**: Multi-column sorting; drag to reorder sort priority.
- **Property editing**: Edit note properties directly from the table view — no need to open individual notes.
- **Formula fields**: Computed columns based on property values.
- **Multiple views per Base**: Switch between table, list, and card views of the same filtered dataset.
- **Inline note creation**: Add new notes directly from a Base that automatically inherit the base's properties.
- **Search within Base**: Live filtering by text search within the Base.

Bases is often compared to Notion's database feature, but all underlying data remains as plain Markdown with YAML frontmatter.

---

## Community Plugins

Obsidian has a thriving ecosystem of **thousands of community plugins** accessible directly from Settings → Community plugins → Browse. They are built by independent developers and are open-source (typically on GitHub).

Plugin installation is done within Obsidian, and plugins can be enabled/disabled/updated without restarting. Obsidian reviews community plugins for basic security before listing them.

### Notable Plugin Categories

**Data & Queries**
- **Dataview** — SQL-like query language for notes and properties; the predecessor to Bases
- **Tasks** — Advanced task management with due dates, priorities, recurrence
- **Kanban** — Drag-and-drop Kanban boards

**Writing & Editing**
- **Templater** — Advanced templating with JavaScript-powered logic
- **QuickAdd** — Fast note capture, macros, and automation
- **Linter** — Auto-format notes on save (consistent YAML, spacing, etc.)
- **Text Transporter** — Move text between notes

**Navigation & Organization**
- **Recent Files** — List of recently opened notes
- **Folder Notes** — Attach notes to folders
- **Waypoints** — MOC (Map of Content) generation

**Visuals & UI**
- **Excalidraw** — Full Excalidraw whiteboard embedded in Obsidian
- **Style Settings** — GUI settings panel for theme customization
- **Minimal Theme Settings** — Extended controls for the Minimal theme

**Productivity & Integrations**
- **Calendar** — Monthly calendar view integrated with Daily Notes
- **Periodic Notes** — Weekly, monthly, quarterly, yearly notes
- **Git** — Auto-commit vault to a Git repository for versioning/backup
- **Obsidian URI** — Deep-linking from external apps into specific notes

---

## Themes & Appearance

### Built-in Themes
- Dark and Light base themes selectable in Settings → Appearance
- Multiple built-in accent color options

### Community Themes
- Hundreds of community-created themes installable from Settings → Appearance → Manage
- Popular themes include: **Minimal**, **Things**, **Obsidian Nord**, **AnuPpuccin**, **Catppuccin**, **Blue Topaz**

### CSS Customization
- Custom **CSS snippets** can be dropped into the `.obsidian/snippets/` folder and toggled on/off
- Obsidian uses CSS custom properties extensively, making theme overrides surgical and clean
- Full control over fonts, colors, spacing, callout styling, etc.

### Typography Settings
- Custom fonts for the UI and for the editor/note body (separately)
- Font size slider
- Line width control (narrow, medium, wide, maximum)
- Readable line length toggle
- Interface zoom level

### Accent Color
One global accent color affects highlights, links, buttons, and interactive elements throughout the UI.

---

## Search & Navigation

### Full-Text Search
The Search panel supports:
- Case-sensitive/insensitive toggling
- **Regex** patterns
- Boolean operators: `AND`, `OR`, `NOT`
- Scoped operators:
  - `file:` — search by filename
  - `path:` — search within a folder path
  - `tag:` — filter by tag
  - `line:` — match must occur on the same line
  - `section:` — match must occur under the same heading
  - `block:` — match within a block
  - `task:`, `task-todo:`, `task-done:` — filter task items

Search results show matched lines with context and can be expanded.

### Quick Switcher
Fuzzy-search notes by title, open them instantly. Supports:
- Creating a new note from the search string
- Opening in a new pane
- Jumping to headings within a note

### Command Palette
The Command Palette (Ctrl/Cmd+P) is a universal keyboard-driven interface for:
- Running any command from any plugin
- Opening settings
- Changing editor modes
- Inserting callouts, templates, etc.

### Slash Commands (Inline)
Type `/` in the editor to open an inline mini-palette for inserting common elements (tables, callouts, templates, etc.) without leaving the keyboard.

---

## Hotkeys & Customization

### Hotkeys
Every command in Obsidian — including plugin commands — can be assigned a keyboard shortcut:
- Settings → Hotkeys provides a searchable list of all commands
- Each command can have one or more hotkeys
- Conflicts are flagged
- Hotkeys are vault-specific

### Settings Categories
Obsidian's Settings panel is organized into:
- **Editor** — Default editing mode, spell check, line numbers, fold behavior, smart indent
- **Files & Links** — Default attachment folder, wikilinks vs markdown links, new note location
- **Appearance** — Theme, font, zoom, CSS snippets
- **Hotkeys** — All keyboard shortcut mappings
- **Core plugins** — Enable/disable built-in plugins
- **Community plugins** — Browse, install, manage third-party plugins
- Individual plugin settings panels

---

## Obsidian Sync

**Obsidian Sync** is an optional paid service for syncing your vault across multiple devices.

### Plans
- **Standard**: $4/month — 1 synced vault, 1 GB storage
- **Plus**: Higher limits on vault count and storage

### Features
- **End-to-end encryption (E2EE)**: Data is encrypted on-device before upload using AES-256. Obsidian servers never see your plaintext notes. Your encryption password is never transmitted.
- **Standard encryption mode**: Alternatively, Obsidian manages keys for convenience (less private but easier key recovery).
- **Version history**: Every note version is saved approximately every 10 seconds; versions are kept for up to **1 year**.
- **Selective sync**: Choose which file types to sync (images, audio, video, PDFs can all be toggled independently). Exclude specific folders.
- **Conflict resolution**: Handles simultaneous edits from multiple devices.
- **Shared vaults** (newer feature): Share a vault with other Obsidian users for **real-time collaboration**, with **Presence Indicators** showing who is editing.
- **Custom device names** for identifying sync sources.

---

## Obsidian Publish

**Obsidian Publish** is an optional paid service for publishing vault notes as a public or private website.

### Features
- **One-click publishing**: Mark notes with `publish: true` in properties; they are pushed to your Publish site
- **Custom domains**: Point your own domain at your Publish site
- **Custom CSS/JS**: Full styling control via `publish.css` and `publish.js` files
- **Navigation structure**: Customizable nav sidebar
- **Graph view**: Interactive graph embedded in the published site
- **Hover previews**: Wikipedia-style hover preview on links
- **Backlinks panel**: Visible on published pages
- **Stacked pages**: Links open as horizontally stacked panes (like in-app experience)
- **Search**: Full-text search on the published site
- **Access control**: Password-protect private sites
- **SEO controls**: Properties for meta descriptions, social media previews
- **Frontmatter flags** for per-note publish settings

---

## Mobile Apps (iOS & Android)

Obsidian has native apps for both platforms, available for free.

### Feature Parity
The mobile apps include virtually all desktop features:
- Full editor with Live Preview, Source Mode, and Reading View
- Graph View (interactive)
- All core plugins
- Community plugins (install and manage from mobile)
- Custom themes
- Hotkeys (customizable on-screen toolbar)

### Mobile-Specific UI
- **Customizable toolbar**: A configurable row of action buttons above the keyboard
- **Pull-down quick actions**: Swipe gesture to access common commands
- **Sidebar navigation via swipe**: Both sidebars accessible by swiping from edges
- **Bottom toolbar**: Optional bottom bar for navigation actions
- **Tablet-optimized**: Sidebar pinning for larger screens
- **Siri and iOS Shortcuts integration** (iOS, added January 2026)

### Vault Storage on Mobile
- iOS: Uses iCloud or local device storage
- Android: Local device storage, accessible via file manager
- Obsidian Sync is the recommended cross-device solution

---

## New in 2026: AI & Collaboration

Obsidian has begun integrating AI and collaboration features, in line with broader industry trends while maintaining its privacy-first ethos.

### AI Features (2026)
- **Privacy-first AI**: Uses local, small language models for speed and privacy by default
- **Optional API integration**: Connect to larger external AI models via API key for more intensive tasks
- **Note Synthesis**: Select multiple notes and ask the AI to create a summary, identify key themes, or draft a new note based on the selected content
- **In-editor AI assistance**: Context-aware suggestions within the editor

### Real-Time Collaboration (2026)
Built on top of Obsidian Sync:
- Share specific folders or entire vaults with other Obsidian users
- **Simultaneous editing** of the same notes
- **Presence indicators**: See who is currently active in a shared note
- **Version history** for collaboration conflict resolution and rollback

---

## Pricing & Licensing

| Tier | Cost | Use Case |
|------|------|----------|
| **Free (Personal)** | $0 | Personal use; all core features included |
| **Commercial License** | $50/user/year | For use in a for-profit company |
| **Obsidian Sync Standard** | $4/month | 1 vault, 1 GB, cross-device sync |
| **Obsidian Sync Plus** | Higher tier | More vaults and storage |
| **Obsidian Publish** | $8/month | Host notes as a public website |
| **Catalyst** | One-time payment | Access to insider/beta builds early |

Community plugins and themes are free and open-source.

---

## Strengths & Limitations

### Strengths
- **Data ownership**: Notes are plain text files on your device — no vendor lock-in
- **Performance**: Extremely fast even with tens of thousands of notes
- **Flexibility**: Adaptable to nearly any workflow through plugins and settings
- **Offline-first**: Full functionality without internet
- **Privacy**: Optional E2E encryption for Sync; local AI option for 2026 features
- **Rich linking model**: Wikilinks, backlinks, embeds, transclusion, block references
- **Active development**: Frequent updates, responsive team
- **Ecosystem**: Thousands of community plugins covering nearly every use case
- **Graph View**: Unique visual tool for exploring note relationships
- **Canvas**: Powerful visual thinking space not found in most note apps
- **Bases**: Growing into a Notion-like database alternative with local-first data

### Limitations
- **Learning curve**: The flexibility can be overwhelming for new users
- **Markdown-only**: Not suitable for rich document formats natively (no native tables with merged cells, no complex layouts without plugins)
- **Collaboration**: Real-time collaboration is newer and requires Sync subscription
- **Mobile**: While feature-complete, the mobile experience can feel less polished than dedicated mobile apps
- **Publishing**: Obsidian Publish is powerful but requires a subscription; not self-hostable natively
- **No built-in AI (until 2026)**: Prior to recent versions, AI required community plugins or external tools
- **Community plugins**: Quality varies; some are unmaintained; can break on Obsidian updates

---

---

## Official Roadmap

> **Source:** [obsidian.md/roadmap](https://obsidian.md/roadmap/) — as of April 2026
> The official roadmap is organized by release milestone. Items are promoted from *Planned* → *In Progress* → *Shipped* as development progresses.

---

### ✅ Shipped

These capabilities have been released in stable public builds.

#### v1.9 (August 2025)
- **Bases core plugin** — Turn any set of notes into a powerful database. Create Table, List, and Card views of notes filtered and sorted by their YAML properties. Edit properties inline without opening individual notes. Supports formula fields for computed columns.
- **Footnotes View** — A new sidebar tab for managing footnotes in the current file without losing your scroll position in the note.
- **Mobile keyboard improvements** — The on-screen keyboard opens and closes significantly more smoothly on iOS and Android.
- **Mobile navigation update** — Separate buttons for opening a new tab and for the Quick Switcher, improving one-handed mobile navigation.

#### v1.10 (late 2025)
- **Bases: Map view** — Display notes as pins on an interactive geographic map, drawing from location/coordinates properties stored in frontmatter.
- **Bases: List view** — Display filtered notes as a bulleted or numbered list, in addition to the existing Table and Card views.
- **Bases: Group by property** — Group rows or cards within a view by a chosen property (e.g., group tasks by `status`).
- **Bases: Aggregation row** — Calculate totals, averages, counts, min/max, and other aggregate values for the rows currently visible in a Table view.
- **Bases: Plugin API** — Developer API allowing community plugins to register entirely new view types for Bases, unlocking extensibility on par with the core view types.
- **CSV import to Bases** — Convert CSV records directly into individual Markdown notes and populate a Base from imported data.

#### v1.11 (December 2025)
- **Keychain / Secret Storage** — A new "Keychain" section in Settings for securely storing plugin secrets: API keys, tokens, and other credentials. Replaces ad-hoc approaches where plugin secrets were stored in plain-text config files.
- **Full-screen reading mode** — Automatically hides UI chrome (sidebars, toolbars) when entering Reading View, for a distraction-free reading experience.
- **Floating navigation buttons** — Navigation back/forward buttons float contextually in the content area on mobile.
- **Sidebar slide behavior** — New option for sidebars to slide *beside* the content rather than floating over it on narrower screens.
- **Settings section icons** — Icons added to each section in the Settings panel for faster visual scanning.
- **Markdown links in properties** — Text and List property types now support Markdown-formatted links directly within property values.
- **Auto-update of internal links on file rename/move** — When a file is renamed or moved, all internal `[[wikilinks]]` referencing it are automatically updated across the vault.
- **Daily note format picker** — Daily Notes setup now provides a dropdown of predefined date-format options instead of requiring manual format strings.

#### v1.12 (February–March 2026)
- **Obsidian CLI** — A command-line interface bundled with the desktop app. Enables scripting and automation directly from the terminal: open daily notes, search vaults, add tasks, create notes from templates, manage plugins, move files, and more. Integrates with shell scripts and external tools.
- **Bases: Search** — A search/filter toolbar within any Base view to interactively narrow results by text query, complementing the existing property-based filters.
- **Bases: Drag-and-drop import** — Drag files into a Base view to import them.
- **Image resizing in Live Preview** — Drag the corner handle of any embedded image in Live Preview to resize it inline. Double-click the corner to reset to original size.
- **Attachment cleanup on delete** — When deleting a note, Obsidian now prompts to also delete its associated attachments. Configurable via Files & Links settings (Always / Ask / Never).
- **HTML-preserving copy/paste** — Copying text from the editor now produces HTML-formatted clipboard content, fixing paste fidelity when pasting into Google Docs, Notion, and other rich-text applications.
- **iOS Share Sheet extension** — A native iOS Share extension so content from Safari, social apps, and other sources can be saved directly to a vault without opening Obsidian first.
- **System language detection** — Obsidian automatically detects the device's system language and translates the onboarding experience accordingly.

---

### 🔄 In Progress / Near-Term Planned

These are features confirmed by the official roadmap as actively in development or firmly queued for upcoming releases.

| Feature | Details |
|---------|---------|
| **Bases: Calendar view** | Display notes as events on an interactive calendar, using date properties for placement |
| **Bases: Kanban view** | Side-by-side kanban-style column layout within Bases, powered by a property that defines column/status |
| **Bases: Publish support** | Ability to include Bases views on Obsidian Publish sites |
| **Visual Markdown table editor** | A GUI table editor for creating and editing Markdown tables without manually typing pipe syntax |
| **Mobile: Lock screen / home screen quick actions** | iOS and Android widgets and quick actions accessible from the lock screen and home screen without opening the app |
| **Accessible Sync plan** | A lower-cost or free Sync tier for users with a single vault and basic sync needs |

---

### 📋 Longer-Term Planned

These items appear on the roadmap without a specific assigned release but are confirmed as intended future capabilities.

- **Bases: More view types** — Additional layout options beyond Table, List, Card, Map, Calendar, and Kanban (specific types not yet disclosed).
- **Open canvas file format** — *(Now shipped — see Canvas section above)* The `.canvas` format was released as [JSON Canvas](https://jsoncanvas.org/) under MIT license, enabling third-party tool interoperability.
- **Context menus for Markdown formatting** — Right-click context menus in the editor to apply common Markdown formatting (bold, italic, heading, link, etc.) via mouse without keyboard shortcuts.
- **Property renaming and searching at vault level** — Tools to find, rename, and manage property keys across all notes in a vault from a single interface.

---

### Roadmap Observations

The roadmap reveals several clear strategic directions for Obsidian:

**Bases is the primary growth surface.** Nearly every roadmap milestone through 2026 adds new view types (Map, Calendar, Kanban), aggregation, search, API extensibility, and publishing support to Bases. This positions Bases as Obsidian's answer to Notion's database feature — with all data remaining local Markdown.

**CLI signals a developer/power-user push.** The v1.12 CLI is a meaningful shift toward treating Obsidian as a platform component in broader automation workflows, not just a standalone app.

**Mobile is closing the feature gap.** Recent milestones (v1.11–v1.12) have focused heavily on mobile UX polish, iOS-native integrations (Share Sheet, Siri/Shortcuts, lock screen actions), and touchscreen ergonomics.

**Infrastructure for plugins is maturing.** Keychain/Secret Storage and the Bases Plugin API both point toward a more capable plugin ecosystem with better security primitives.

---

---

## Web Clipper

Obsidian Web Clipper is the **official browser extension** for capturing and saving web content directly into your Obsidian vault. It is free, open-source (MIT license), and available across all major browsers and platforms. The extension converts web content into durable Markdown files stored locally in your vault — never in the cloud.

> **Repository:** [github.com/obsidianmd/obsidian-clipper](https://github.com/obsidianmd/obsidian-clipper)
> **Latest version:** v1.3.0+ (59 releases as of April 2026)
> **Built with:** TypeScript (80.8%), SCSS (12.2%), HTML (5.2%)

---

### Platform & Browser Support

| Platform | Supported Browsers / Stores |
|---|---|
| **Chromium-based (desktop)** | Chrome, Brave, Edge, Arc, Orion, and all Chromium-based browsers via Chrome Web Store |
| **Firefox (desktop)** | Firefox via Firefox Add-Ons marketplace |
| **Firefox (mobile)** | Firefox for Android |
| **Safari (macOS)** | Safari via App Store (macOS) |
| **Safari (iOS / iPadOS)** | Safari via App Store — includes iPhone and iPad |

The codebase ships three distinct distribution builds: `dist/` (Chromium), `dist_firefox/` (Firefox), and `dist_safari/` (Safari), with cross-browser compatibility handled via the `webextension-polyfill` library.

---

### Core Clipping Capabilities

**What you can clip:**

- **Full page** — saves the entire page content as Markdown
- **Article/main content** — extracts the primary article body, stripping navigation, ads, and boilerplate using the `defuddle` content extraction library
- **Selected text** — clip only highlighted text on a page; selection takes priority as the `{{content}}` variable
- **Highlights** — save specific passages you've highlighted; these persist visually when you revisit the page
- **Any page type** — news articles, blog posts, recipes, product pages, research papers, GitHub issues, YouTube pages, etc.

**Output format:**

All clipped content is saved as standard Markdown (`.md`) files. This means clips are:
- Readable in any text editor without Obsidian
- Portable and future-proof
- Available offline immediately after saving
- Compatible with Obsidian's full linking, search, and graph ecosystem

---

### Highlighting & Annotation

One of the Clipper's standout features is its persistent highlighting system:

- Highlight any passage on a web page using the extension
- Highlights are **saved to your vault** alongside the clipped content
- When you **revisit the same URL**, your previous highlights remain visible on the page — the extension re-applies them automatically
- All highlights are **exportable to JSON** for backup or migration
- Highlights integrate with the `{{highlights}}` template variable so they can be placed in a dedicated section of your note

---

### Reader Mode

The Clipper includes a built-in **Reader Mode** that displays a cleaned-up, distraction-free version of web pages before or instead of clipping. Reader Mode is fully customizable:

- **Typography**: font family, font size, line height, line width
- **Appearance**: light mode, dark mode
- **Themes**: multiple built-in themes
- **Custom CSS**: inject your own styles for complete visual control

This allows reviewing and reading content in a comfortable format before deciding what to save.

---

### Template System

Templates are the Clipper's most powerful feature for structuring captured content. A template defines the exact format, location, and metadata of every note the Clipper creates.

**Template capabilities:**

- **Multiple templates**: create as many templates as needed for different content types (articles, recipes, videos, research, etc.)
- **Template triggers / rules**: automatically select a template based on the current page URL pattern or schema.org data type — so visiting a YouTube page always uses your "Video" template, for example
- **Note name**: define the filename of the saved note using variables
- **Note location**: define which folder within your vault to save to, using variables or static paths
- **Frontmatter / properties**: define YAML properties to populate automatically
- **Note content**: the Markdown body of the note, assembled from variables and static text
- **Conditional logic**: use `if`/`else`/`endif` blocks in template content
- **Loops**: use `for`/`endfor` blocks to iterate over arrays (e.g., render each highlight as a list item)
- **Template compression**: templates are stored using `lz-string` compression
- **Import/export**: templates can be exported as JSON or imported by pasting JSON — shareable with others
- **Copy to clipboard action**: copy any template's JSON directly to clipboard from settings

**Community templates:**
A community-maintained template repository exists at [github.com/obsidian-community/web-clipper-templates](https://github.com/obsidian-community/web-clipper-templates) with ready-made templates for common sites and use cases.

---

### Variable System

Templates are built around a rich variable system. Variables use the syntax `{{variableName}}` and are resolved at clip time.

#### Preset Variables

Automatically extracted from the page — no configuration needed:

| Variable | Description |
|---|---|
| `{{title}}` | Page title |
| `{{content}}` | Article body, highlights, or selection (whichever is active) |
| `{{url}}` | Full URL of the page |
| `{{author}}` | Author name extracted from page metadata |
| `{{description}}` | Page meta description |
| `{{date}}` | Current date (uses dayjs formatting) |
| `{{time}}` | Current time |
| `{{selection}}` | Any text currently selected on the page |
| `{{highlights}}` | All saved highlights for the page |
| `{{fullHtml}}` | The full raw HTML of the page |

#### Meta Variables

Extract data from HTML `<meta>` tags:

- `{{meta:name:description}}` — returns the `<meta name="description">` content
- `{{meta:property:og:title}}` — returns the Open Graph title
- `{{meta:property:og:image}}` — returns the OG image URL
- Any other `name` or `property` meta tag is accessible via this pattern

#### Selector Variables

Extract text from any element on the page using CSS selectors:

- Syntax: `{{selector:cssSelector}}` or `{{selector:cssSelector?attribute}}`
- The optional `?attribute` extracts a specific HTML attribute (e.g., `?href`, `?src`) rather than text content
- Example: `{{selector:.article-date?datetime}}` extracts the `datetime` attribute from an element with class `article-date`

#### Schema Variables

Extract structured data from schema.org JSON-LD embedded in pages:

- Syntax: `{{schema:propertyPath}}`
- Supports dot notation and array indexing: `{{schema:author[0].name}}`
- Wildcard: `{{schema:author[*].name}}` returns an array of all author names
- Works with any schema.org type: Article, Recipe, Product, Event, etc.

#### Prompt Variables (AI / Interpreter)

Leverage a connected language model to extract or generate content using natural language:

- Syntax: `{{"your natural language prompt"}}` — double quotes distinguish prompts from preset variables
- Requires the Interpreter feature to be enabled and configured
- Example: `{{"a one-paragraph summary of this article"}}` generates a summary on clip
- Example: `{{"extract all people mentioned in this article as a comma-separated list"}}`
- Compatible with any model provider (see Interpreter section below)

---

### Filter System

Filters transform variable values before they are inserted into the note. They use pipe notation and can be chained:

```
{{title | lower | trim}}
{{date | date:"YYYY-MM-DD"}}
{{content | blockquote}}
```

The system includes **50+ built-in filters** spanning several categories:

**String manipulation:**
- `lower` — convert to lowercase
- `upper` — convert to uppercase
- `trim` — remove leading/trailing whitespace
- `replace` — find-and-replace (supports regex)
- `slice` — extract a substring or array slice
- `split` — split a string into an array by delimiter
- `join` — join an array into a string with a separator
- `first` / `last` — return first or last item of an array
- `length` — return character or item count

**Date formatting (via dayjs):**
- `date:"FORMAT"` — format a date value, e.g. `date:"YYYY-MM-DD"`, `date:"MMM D, YYYY"`
- `date:("outputFormat", "inputFormat")` — parse and reformat dates with explicit input format, e.g. `"12/01/2024"|date:("YYYY-MM-DD","MM/DD/YYYY")`

**Markdown processing:**
- `blockquote` — adds `> ` prefix to each line, converting content to a Markdown blockquote
- `markdown` — processes HTML and converts it to Markdown
- `strip_tags` — removes HTML tags, leaving plain text
- `strip_md` — removes Markdown formatting, leaving plain text

**URL / text utilities:**
- `encode_uri` — URL-encodes a string
- `decode_uri` — URL-decodes a string
- `capitalize` — capitalizes the first letter of each word

Filters work with **all variable types** — preset, meta, selector, schema, and prompt variables.

---

### Interpreter (AI Integration)

The **Interpreter** is an optional, configurable AI layer that powers prompt variables in templates. It sends page content to a language model and returns results inline during the clip process.

**Compatible model providers (any that support the OpenAI-compatible API format):**

- OpenAI / ChatGPT
- Anthropic / Claude
- Google / Gemini
- Ollama (local models — fully private, no data leaves your machine)
- OpenRouter (aggregator for 100+ models)
- Hugging Face
- Any other OpenAI-compatible endpoint

**Configuration:**
- Set provider, model, and API key in extension settings
- Interpreter must be enabled before prompt variables in templates will resolve
- Multiple prompt variables can be used in a single template, each calling the model separately

**Use cases for prompt variables:**
- Generate a one-paragraph summary
- Extract key takeaways as a bullet list
- Translate content to another language
- Perform sentiment analysis
- Extract named entities (people, places, organizations)
- Classify content by topic or type
- Rewrite content in a different tone
- Answer a specific question about the page

---

### Properties & Types System

The Clipper has a dedicated system for managing Obsidian properties (YAML frontmatter) across templates:

- **Settings → Properties**: a centralized view where all properties used across all templates can be seen and modified in one place
- **Property types**: properties can be typed (text, number, date, list, checkbox, etc.) to match your Obsidian vault's property schema
- **Import from Obsidian**: import property definitions from your vault's `types.json` file so the Clipper matches your existing property types exactly
- **Export to Obsidian**: export Web Clipper property definitions back to `types.json` format for use in Obsidian itself
- **Per-template properties**: each template defines its own set of properties to populate, using variables for dynamic values

---

### Settings Management

The Clipper provides robust import/export of configuration:

- **Export all settings**: save the complete extension configuration (templates, properties, interpreter settings) to a JSON file
- **Import all settings**: restore from a JSON backup
- **Import templates via JSON paste**: paste a template JSON string directly into the settings UI to add a community or shared template
- **Import types via JSON paste**: same for property type definitions
- **Copy template JSON**: one-click action to copy any template's JSON to the clipboard for sharing

---

### Productivity Features

- **Hotkeys**: configure keyboard shortcuts to save pages with a single keystroke, without opening the extension popup
- **Default vault selection**: set which Obsidian vault receives clips when you have multiple vaults open
- **Automatic template selection**: template trigger rules mean the right template is selected automatically based on the URL — no manual selection needed for known sites

---

### Privacy & Data Model

- **100% local storage**: all clipped content goes directly into your local Obsidian vault as Markdown files — nothing is sent to Obsidian's servers
- **No cloud dependency**: the extension communicates only with your local Obsidian app (via the Obsidian URI protocol) and, optionally, with your chosen AI provider if Interpreter is configured
- **Durable format**: Markdown files are readable by any text editor and will remain accessible regardless of whether Obsidian or the Clipper extension continues to exist
- **HTML sanitization**: all clipped HTML is sanitized via `dompurify` before conversion to Markdown

---

### Technical Dependencies

| Library | Role |
|---|---|
| `webextension-polyfill` | Cross-browser API compatibility (Chrome/Firefox/Safari) |
| `defuddle` | Content extraction and HTML-to-Markdown conversion |
| `dayjs` | Date and time parsing and formatting |
| `lz-string` | Template compression for storage |
| `lucide` | Icon library |
| `dompurify` | HTML sanitization before conversion |

---

---

## Command Line Interface (CLI)

The **Obsidian CLI** ships bundled with **Obsidian 1.12+** (publicly available February 27, 2026 — no paid Catalyst license required). It allows you to interact with a running Obsidian instance directly from the terminal, enabling scripting, automation, and AI agent integration. Obsidian must be open for most CLI commands to function.

### Setup

Enable via **Settings → General → Command line interface**, then follow on-screen instructions to add `obsidian` to your system PATH. The binary is registered via `~/.zprofile` on macOS/zsh; bash and fish users must add the path manually to their shell config. Restart your terminal after setup.

### Syntax

```
obsidian <command> [param=value] [flag]
```

- **Parameters** use `key=value` format; quote values with spaces: `name="My Note"`
- **Flags** are bare boolean switches with no value: `silent`, `overwrite`, `total`, `counts`
- **Exception**: `--copy` uses the `--` prefix (copies output to clipboard)
- **Multiline content**: use `\n` for newlines and `\t` for tabs

### TUI Mode

Running `obsidian` with **no arguments** launches a full-screen **Terminal UI (TUI)** — a keyboard-driven file browser for your vault with tab completion and command history.

### File & Vault Targeting

| Param | Behavior |
|-------|----------|
| `file=<name>` | Resolves like a wikilink — name only, no path or extension needed |
| `path=<path>` | Exact path from vault root, e.g. `folder/note.md` |
| *(neither)* | Targets the currently active file in Obsidian |
| `vault=<name>` | Targets a specific vault (use as first parameter); defaults to most recently focused vault |

**Example:**
```bash
obsidian vault="My Vault" search query="test"
```

### Core File Operations

```bash
obsidian read file="My Note"
obsidian create name="New Note" content="# Hello" template="Template" silent
obsidian append file="My Note" content="New line"
obsidian prepend file="My Note" content="Header line"
obsidian move file="My Note" dest="Archive/My Note"
obsidian delete file="My Note"
obsidian search query="search term" limit=10
```

### Daily Notes

```bash
obsidian daily:read                           # Read today's daily note
obsidian daily:append content="- [ ] Task"   # Append to today's daily note
```

### Task Management

```bash
obsidian tasks daily todo         # Incomplete tasks from today's daily note
obsidian tasks daily done         # Completed tasks from today's daily note
obsidian tasks file="My Note"     # All tasks in a specific note
obsidian tasks toggle line=5 file="My Note"  # Toggle task at a specific line
```

### Properties (Frontmatter)

```bash
obsidian property:set name="status" value="done" file="My Note"
```

### Tags

```bash
obsidian tags sort=count counts   # List tags sorted by count with totals
```

### Backlinks

```bash
obsidian backlinks file="My Note"   # Find all notes that link to this note
```

### Global Output Modifiers

| Modifier | Effect |
|----------|--------|
| `--copy` | Copies command output to clipboard |
| `silent` | Prevents the targeted file from opening in the Obsidian GUI |
| `total` | Returns a count on list commands |
| `counts` | Returns counts (e.g. on tag commands) |
| `overwrite` | Overwrites an existing file on create |

### Plugin & Theme Development Workflow

The CLI is purpose-built to streamline the plugin development iteration loop:

```bash
# 1. Reload plugin after code changes
obsidian plugin:reload id=my-plugin

# 2. Check for runtime errors
obsidian dev:errors

# 3. Verify visually — screenshot or DOM inspection
obsidian dev:screenshot path=screenshot.png
obsidian dev:dom selector=".workspace-leaf" text

# 4. Check console output for warnings or logs
obsidian dev:console level=error
```

### Advanced Developer Commands

```bash
# Execute JavaScript in the live Obsidian app context
obsidian eval code="app.vault.getFiles().length"

# Inspect a CSS property value on a selector
obsidian dev:css selector=".workspace-leaf" prop=background-color

# Toggle mobile device emulation
obsidian dev:mobile on
```

Additional developer commands (Chrome DevTools Protocol, debugger controls) are accessible via `obsidian help`.

### Command Count & Discovery

Over **100 commands** are available across all categories. Run `obsidian help` for the authoritative, always-up-to-date full list.

### Automation Use Cases

- Append tasks or content to daily notes from shell scripts or cron jobs
- Auto-tag notes and aggregate daily notes into weekly summaries
- CI/CD pipeline integration for vault content processing
- Agentic AI tools (Claude Code, etc.) interacting with the vault without full machine access
- Plugin/theme development iteration — reload, screenshot, inspect, debug, all from the terminal

### AI Agent Integration (obsidian-skills)

The **kepano/obsidian-skills** project (by Obsidian CEO Steph Ango; 19k+ stars, MIT license) provides agent skill files that teach AI assistants how to use the Obsidian CLI and file formats. Compatible with Claude Code, Codex CLI, OpenCode, and any skills-compatible agent.

| Skill | Purpose |
|-------|---------|
| **obsidian-cli** | Full CLI integration — all vault commands above |
| **obsidian-markdown** | Create/edit Obsidian Flavored Markdown (wikilinks, embeds, callouts) |
| **obsidian-bases** | Work with `.base` files — views, filters, formulas, summaries |
| **json-canvas** | Create/manipulate JSON Canvas files (nodes, edges, groups) |
| **defuddle** | Extract clean markdown from web content |

**Repository:** [github.com/kepano/obsidian-skills](https://github.com/kepano/obsidian-skills)

---

## Obsidian Headless Sync Client

**obsidian-headless** is a **separate official tool** — distinct from the desktop CLI — that syncs and publishes vaults from the command line *without the Obsidian GUI being open*. Designed for servers, automation pipelines, and headless environments.

- **Repository:** [github.com/obsidianmd/obsidian-headless](https://github.com/obsidianmd/obsidian-headless)
- **Install:** `npm install -g obsidian-headless`
- **Command:** `ob`
- **Requires:** Node.js 22+
- **Platforms:** Windows, macOS, Linux

### Authentication

```bash
ob login    # Interactive; supports email/password/2FA parameters
ob logout   # Clears stored credentials
```

### Sync Capabilities

- List and create remote vaults
- Connect a local directory to a remote vault (standard or end-to-end encryption)
- One-time sync or continuous sync with file watching
- **Sync modes**: bidirectional, pull-only, mirror-remote
- Configurable conflict resolution strategies
- Selective file type inclusion and folder exclusion
- Sync status monitoring and vault unlinking

### Publish Capabilities

- Create and list Obsidian Publish sites
- Connect vaults to publish destinations
- Upload/remove files with dry-run preview
- **Selective publishing** via frontmatter `publish: true/false` flags
- `--all` flag to publish untagged content
- Configurable included/excluded folders, theme, navigation, and site components

### Special Features

- **btime native module**: Preserves original file creation timestamps on Windows/macOS during downloads (Linux functions normally without it)
- **E2E encryption**: Full end-to-end encryption option on vault creation, in addition to standard managed encryption

### Headless Use Cases

- Continuous vault sync on a server, Raspberry Pi, or Docker container
- Automated remote backups with full Sync privacy and encryption
- Scheduled publishing workflows (e.g., static site generation from vault content)
- CI/CD pipelines for vault content
- Give AI agents vault access without exposing the full machine
- Sync a shared team vault to a server that feeds other tools (search indexes, dashboards, etc.)
- Run scheduled automations — aggregate daily notes into weekly summaries, auto-tag, etc.

---

*Analysis compiled from official Obsidian documentation, changelogs, roadmap, and community resources — April 2026.*
