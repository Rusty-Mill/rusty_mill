---
category: Added
changelog: Mill Board cards can name their repository when a project spans several, and board_projects reports each project's repositories.
---
## 2026-11-14 - Mill Board: card repository and project view (repos apart from projects, PR 2 of 5)

- **Added:** an optional `repo` on cards (`card_add`, `card_assign`), validated against the project's repositories and cleared when a card is re-filed without one; kept in the card's rusty_tick board document (left out while unset, so older documents are unchanged and no tick change is needed). `board_projects` gains `projects` (`{project: {repos: {repo: [paths]}, default_repo}}`) next to the unchanged `repos`. `board_owns` takes an optional `repo`.
- **Known limitations:** the settings UI still cannot save a spanning project (next PR); roadmap and metrics do not filter by repository yet.
