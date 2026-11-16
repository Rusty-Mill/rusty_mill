---
category: Changed
changelog: rusty_mux will not write utmp/wtmp entries per block; C69 slice (d) is declined (wontdo), not deferred.
---
## 2026-11-16 - rusty_mux: utmp entries declined (C69 slice d)

- **Changed:** Nano chose "Don't build it (wontdo)" for C69 slice (d) (pack Q29, D276; card #1008). No utmp/wtmp writes, no `libutempter`, no setgid helper. C69 stays done on slices (a)-(c). Docs only: pack DECISIONS and OPEN_QUESTIONS, `C69-LIFECYCLE.md`, `CAPABILITIES.md`.
- **Known limitations:** `who` and `w` do not list mux blocks.
