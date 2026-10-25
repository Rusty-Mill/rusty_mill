---
category: Changed
changelog: The rusty_term handoff docs record that rusty_term dropped its direct parking_lot dependency (#956).
---
## 2026-10-24 - rusty_term handoff docs: parking_lot removed

- **Changed:** `docs/projects/mux/RUSTY-TERM-NOTES.md` and `RUSTY-TERM-HANDOFF.md` record that #956 replaced `rusty_term`'s direct `parking_lot` dependency with `rusty_term::sync::Mutex` over `std::sync::Mutex`. With default features `rusty_term` has no `parking_lot`; the `gui` and `gui-gpu` features still pull it in transitively through `arboard` and `wgpu`. The frame lock is still not fair.
- **Known limitations:** documentation only.
