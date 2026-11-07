---
category: Changed
changelog: The mux and rusty_term decision logs record Nano's go on C46 block ownership as built in PR #1556 and on C69 slice (b)'s command line as built in PR #1558.
---
## 2026-11-07 - mux C46 and C69 slice (b) approved

- **Changed:** `docs/projects/mux/DECISIONS.md` adds two dated rows: C46 accepted as built in #1556 (bit 35, fields 52–54, owner-only requests, orphaning, hang-up on stop, the `rusty-mux-ctl own` protocol), and C69 slice (b)'s command line accepted as built in #1558 (`--idle-exit`, auto-start on the default socket). C69 slices (c) and (d) and C77 stay Proposed.
- **Changed:** `docs/projects/rusty_term/handoff/DECISIONS.md` adds D138 [stated] (C46 as built in #1556) and D139 [stated] (C69 slice (b)'s command line as built in #1558, answering D137's gate).
- **Known limitations:** docs only; the code landed with PRs #1556 and #1558.
