---
category: Added
changelog: Mill Board closes a card when a merged pull request says `Closes card #N` (off by default, `auto_close` in the GitHub settings file); card #1620, part 2 of 3.
---
## 2026-11-24 - Mill Board closes a card on a marker in a merged PR

- **Added:** with `auto_close` on, the merge watch closes each open card a merged pull request names on a standalone `Closes card #N` line (also Fixes/Resolves) from the card's own repository, merged into its default branch by an owner, member or collaborator: the PR becomes evidence, the card goes to Done as shipped work, its claim is released and a comment names the PR. The actor is `merge-watch` with `Record` only (ADR-0008).
- **Known limitations:** Blocked, Parked, gated and parent cards are listed but never closed. Evidence and mentions never close. `card_unclose` to revert a wrong close follows in the next PR; until then a wrong marker is fixed by a person.
