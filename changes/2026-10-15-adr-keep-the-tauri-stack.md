---
category: Changed
changelog: ADR-0011 records the decision to keep tauri, tauri-build and the in-use Tauri plugins external, with revisit conditions; documentation only.
---
## 2026-10-15 - ADR-0011: keep the Tauri stack external

- **Changed (docs):** `docs/adr/0011-keep-the-tauri-stack.md` records keep-or-replace for `tauri`, `tauri-build`, `tauri-plugin-global-shortcut`, `tauri-plugin-clipboard-manager` and `tauri-plugin-dialog`, with the reason for each and the conditions that would reopen it. Closes #704.
- **Known limitations:** the usage counts are a grep of workspace manifests, not a vetted audit. The frozen Nexus shell's other plugins are not decided. No code changed.
