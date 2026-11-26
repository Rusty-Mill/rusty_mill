---
category: Changed
changelog: The workspace tree's parsing and tree model moved into `rusty_mux_client` so the GUI can share it with the terminal client.
---
## 2026-11-26 - Workspace tree model moved to the client crate

- **Changed:** the pure part of the TUI workspace tree (reply parsing, `Tree`, `Explorer` state, `Row`) now lives in `rusty_mux_client::workspace`; the TUI keeps key handling (`explorer::press`), text drawing (`explorer::render`, `render_tree`) and re-exports the model, so `rusty_mux_tui::explorer` still has every name it had, apart from three internal Rust API moves: `Explorer::keys` → `explorer::press`, `Explorer::render` → `explorer::render`, and `Tree::render` → `explorer::render_tree`. Wire, CLI and behaviour are unchanged. Card #1573, the shared model for the GUI panel (Nano: "yes", 2026-11-26T15:33Z).
- **Known limitations:** no behaviour change; the GUI panel itself comes next.
