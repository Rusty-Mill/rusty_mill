---
category: Added
changelog: In the workspace tree, `[` and `]` move to the previous or next block of the same card.
---
## 2026-11-27 - mux-tui: card-wide workspace tree (#1573, D379, D380)

- **Added:** `]` and `[` in the workspace tree show the workspace of the next or previous block on the same card that is shown in the window. The block takes the focus, so its input lease is taken the usual way and the listing waits for it; while the tree is open other clients cannot type in that block. A lease held by another client reads "leased elsewhere" and is never taken over. Closing the tree gives the first block its focus back. The TUI's control connection now also asks for the agent labels (`AGENT_META`), which is how it learns each block's card.
- **Changed:** a tree on another block is view-only (D380): it lists and opens directories and copies paths, but `c` (cd) and `o` (open in editor) are refused with a note, so the lease taken for listing grants no input or launch. The block the tree was opened over keeps both.
- **Known limitations:** only blocks shown in the current window are reached; the card is the launcher's own label, not proof of any path. No per-agent badge, no new wire.
