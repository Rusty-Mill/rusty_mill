---
category: Fixed
changelog: The workspace tree's real-server test waits for the listing instead of sleeping, and the GUI's `BLOCK` doc names the feature bits `PANE` now asks for.
---
## 2026-11-25 - Workspace explorer review nits

- **Fixed:** `the_workspace_tree_opens_against_a_real_server` waits for the root row rather than a fixed second; the doc on the GUI's `BLOCK` set names `BLOCK_CALL` and `WORKSPACE` (D349), which the GUI asks for but does not use yet.
- **Known limitations:** none.
