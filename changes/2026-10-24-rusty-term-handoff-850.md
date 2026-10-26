---
category: Changed
changelog: The rusty_term handoff docs record that #850 is fixed, so rusty_term_core's l13 feature is tokio-free.
---
## 2026-10-24 - rusty_term handoff docs: #850 fixed

- **Changed:** `docs/projects/mux/RUSTY-TERM-NOTES.md` and `RUSTY-TERM-HANDOFF.md` no longer list #850 as open. #949 made `rusty_lsp`'s runtime optional, so `rusty_term_core --features l13` links no tokio, and a mux may enable `l13`. The handoff also records that #950 merged.
- **Known limitations:** documentation only.
