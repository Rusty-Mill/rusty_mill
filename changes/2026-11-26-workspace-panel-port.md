---
category: Added
changelog: The GUI source port gained the workspace explorer panel's view state and calls, ready for the panel itself.
---
## 2026-11-26 - Workspace explorer panel: view state and source port

- **Added:** `rusty_term::gui::explorer` (`Snapshot`, `Row`, `State`, `Intent`, `Key`, `Out`, `Panel`: pure view state, scroll and key/click mapping, tested), the `Source::explores` and `Source::explore` methods with `Explore` and `Explored`, and `Remote::explorer`. Public API, additive; every method has a default, so existing sources are unchanged. The window stores the snapshot and redraws; nothing opens the panel yet. Card #1573, gui-design D99 (layering: the source owns the tree and pushes row snapshots).
- **Known limitations:** no user-visible change; drawing, keys and the `rusty_mux_gui` adapter come next.
