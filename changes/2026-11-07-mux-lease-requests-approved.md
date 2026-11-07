---
category: Changed
changelog: The mux and rusty_term decision logs record Nano's go on lease requests (bit 37, RequestLease, LeaseRequested) as built in PR #1561.
---
## 2026-11-07 - mux lease requests approved

- **Changed:** `docs/projects/mux/DECISIONS.md` adds a dated row accepting lease requests as built in #1561 (bit 37, `RequestLease` 0x23, `LeaseRequested` 0x8f, `rusty-mux-ctl own`'s `lease-requested` line).
- **Changed:** `docs/projects/rusty_term/handoff/DECISIONS.md` adds D141 recording the same go; `OPEN_QUESTIONS.md` Q26 points at it.
- **Known limitations:** docs only; the code landed with PR #1561.
