---
category: Fixed
changelog: The `rusty-mcp` changelog no longer tells consumers to install it by git tag; it is consumed as a workspace dependency (#876).
---
## 2026-10-24 - rusty-mcp changelog install note matches the workspace

- **Fixed:** `crates/libs/protocol/rusty_mcp/CHANGELOG.md` told consumers to depend on `rusty-mcp` by git tag from the old standalone repository. Every consumer uses the root `[workspace.dependencies]` path entry, so the note now shows that, and marks the older tag links as history (#876).
- **Known limitations:** the rename-or-fold decision in #876 stays open until `rusty_mcp_axum` is retired (#875).
