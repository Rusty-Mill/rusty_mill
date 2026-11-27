---
category: Changed
changelog: Design note for opening a workspace tree file in an editor from the block's own workspace root.
---
## 2026-11-27 - Workspace explorer: open in editor needs a server-side root

- **Changed:** docs only. `WORKSPACE-EXPLORER.md` scopes how the tree can open a file in an editor against the selected block's authoritative workspace root (a launch field behind a new Feature bit; D377). The earlier size-and-time-check attempt (PR #1929) is reverted: no code, wire or Feature bit changes here.
- **Known limitations:** nothing is built; the wire and the program choice wait for Nano's go.
