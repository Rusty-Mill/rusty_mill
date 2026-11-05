---
category: Added
changelog: Plan for roadmap arc 2, the rusty_term GUI as a rusty_mux client (docs only).
---
## 2026-11-05 - rusty_term: plan for the GUI as a mux client

- **Added:** `docs/projects/term/ARC2-MUX-CLIENT.md`: the crate shape (a session-source port in `rusty_term`, new `rusty_mux_client` and `rusty_mux_gui` crates) and slices S1–S5 for design pack frames F1–F7.
- **Known limitations:** docs only. The new crates and the rusty_mux → `rusty_term` edge wait for the owner's go.
