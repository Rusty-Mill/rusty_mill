---
category: Changed
changelog: The rusty_term and mux decision logs record Nano's approval of C69 slice (a)'s command-line changes from PR #1555 (D136), and that mux command-line changes go to him before merge (D137).
---
## 2026-11-06 - mux C69 slice (a): command-line changes approved (D136, D137)

- **Changed:** `docs/projects/rusty_term/handoff/DECISIONS.md` adds D136 [stated], Nano's approval of the optional socket argument, `$RUSTY_MUX_SOCKET` and the default socket as built in PR #1555. It also adds D137 [default], which replaces D132's "not a public-API gate" clause: any mux command-line change, even an additive one, goes to Nano before merge.
- **Changed:** `docs/projects/mux/DECISIONS.md` adds a dated row accepting slice (a)'s command line. The C69 row itself stays Proposed.
- **Known limitations:** docs only; the code landed with PR #1555.
