---
category: Changed
changelog: Renamed `rusty_acp` to `rusty_ibm_acp` (package `rusty-ibm-acp`) so it is not mistaken for Zed's Agent Client Protocol.
---
## 2026-11-04 - Rename rusty_acp to rusty_ibm_acp

- **Changed:** `crates/libs/protocol/rusty_acp` is now `crates/libs/protocol/rusty_ibm_acp`; the package is `rusty-ibm-acp` and the library path `rusty_ibm_acp`. The crate implements IBM/BeeAI's REST Agent Communication Protocol, and the old name read as Zed's JSON-RPC Agent Client Protocol (Mill Board card #727). No workspace crate depends on it, so nothing else changes behaviour; comments and the workspace map are updated.
- **Known limitations:** historical logs (CHANGELOG, RELEASE_NOTES, older change fragments, reviews, ADRs) keep the old name. Links to the upstream `baileyrd/rusty_acp` repository are unchanged.
