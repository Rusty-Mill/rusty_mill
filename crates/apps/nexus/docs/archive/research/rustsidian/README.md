# Rustsidian research imports

> **Archived 2026-09-08** — Imported from the private `baileyrd/Rustsidian`
> repo (`docs/`) under [RFC 0009](../../../0.1.2/rfcs/0009-four-repo-consolidation.md)
> row 6. Rustsidian (Rust + SvelteKit/Tauri, April 2026) is retired; Nexus
> is the surviving codebase. Nothing here describes Nexus code. Treat it as
> comparative research and design history only.

## What's here

| File | What it is |
|---|---|
| `Obsidian_Capabilities_Analysis.md` | Feature-by-feature inventory of Obsidian 1.12 (editor, core plugins, sync, publish, CLI), synthesized from Obsidian's public docs, changelogs and community resources. Useful as a parity checklist. |
| `Obsidian_UI_Analysis.md` | Layout, panes, modes, settings surface and keyboard model of Obsidian 1.12.7, written from observing the released app. |
| `Obsidian_UI_Architecture.svg` | Diagram accompanying the UI analysis. |
| `Rustsidian_UI_Design.md` | Rustsidian's own desktop UI design document (Tauri 2 + SvelteKit + CodeMirror 6). |
| `Rustsidian_UI_Architecture.svg` | Diagram accompanying the UI design document. |
| `Rustsidian_Development_Roadmap.md` | Rustsidian's phased roadmap through v1.7 with what shipped. |
| `superpowers/specs/`, `superpowers/plans/` | Four spec/plan pairs (UI modernization, nanoed feature integration, reactive note context, split panes / view modes) produced through the superpowers workflow, same convention as [`../../superpowers/`](../../superpowers/). |

Every markdown file carries an `> **Archived 2026-09-08**` line at the top,
per the [archive convention](../../README.md). Content is otherwise verbatim.

## What was deliberately not imported

The RFC row 6 condition was "check licensing/attribution before moving
from a private to a public repo". The following stayed in the private
repo:

- **`docs/obsidian_reverse_engineering/app_asar_main.js` and
  `obsidian_asar_main.js`** — JavaScript extracted from Obsidian's ASAR
  archives. Proprietary Obsidian code; cannot be redistributed.
- **`docs/obsidian_reverse_engineering/0*.md`** (seven static-analysis
  reports on the 1.12.7 installer, Electron setup, network API, security
  and frontend). They quote the extracted source and document sync /
  update internals, so they inherit the same problem as the `.js` files.
  Their observable-behaviour conclusions are already covered by the two
  analysis documents above, which cite only public sources.
- **`docs/screenshots/`** (~45 PNGs). Not referenced by any document, and
  several capture the author's personal Obsidian vault.
- `docs/daily/` and `docs/.rustsidian/` — an empty daily note and a
  default plugin config; app state, not documentation.
