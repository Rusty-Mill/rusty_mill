---
category: Changed
changelog: ts-types moved from crates/apps/rusty_tailscale to crates/libs/net so rusty_mux can use it.
---
## 2026-11-13 - ts-types moves to libs/net

- **Changed:** the `ts-types` crate (wire and API types) moved from `crates/apps/rusty_tailscale/crates/ts-types` to `crates/libs/net/ts-types`, and its layer is now `libs`. Nothing else changed: same name, same code, same dependents.
- **Why:** C27 (rusty_mux, card #276) maps a Tailscale peer to a principal, and ADR-0003 allows a family to split only when an outside crate depends on its reusable part. The rest of `rusty_tailscale` stays in `apps/`. ADR-0003's family table and appendix, the README and DESIGN notes, and the generated workspace map are updated.
