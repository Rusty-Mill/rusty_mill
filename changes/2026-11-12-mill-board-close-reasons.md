---
category: Added
changelog: Mill Board: card_close drops unstarted cards with a required reason; metrics count dropped work apart from shipped.
---
## 2026-11-12 - Mill Board: close reasons and dropped work

- **Added:** `card_close` (needs `triage`) closes a Backlog or queued card without shipping it, for a required reason (`wont_do`, `duplicate` or `superseded`) and an optional note, which becomes a comment. The card is Done with no ship time and a `closed` record, so it never counts as shipped.
- **Added:** `board_metrics` reports dropped work apart from shipped: `weeks[].dropped`, `by_project[].dropped` and `dropped` per reason over the last 30 days. Card summaries carry `closed` when set.
- **Changed:** in rusty_tick a closed card's task is `wontdo` with a reserved `closed:<reason>:<ms>` tag, written in one request; a task marked won't-do in rusty_tick itself reads as `wont_do`, so the owner's earlier closes count as dropped.
- **Changed:** rollups (`card_trace`, `board_milestones`) count dropped children apart from done (`dropped`), and `ready_to_close` needs at least one shipped child.
- **Known limitations:** started, blocked and Done cards can't be closed with a reason. Dependencies on a dropped card still count it as Done. The web UI has no close button or dropped chart yet. The note is added just before the close, so a failed close retried may repeat it.
