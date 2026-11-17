---
category: Changed
changelog: Mill Board's scope picker lists projects only; repositories no longer sort or filter the board.
---
## 2026-11-17 - Mill Board picker lists projects only

- **Changed:** the board's top dropdown now lists "All projects" and each project (the Inbox included) with its card count, instead of repositories. Repositories no longer filter the board, the roadmap or metrics scope, or group the card dialog's project list. New cards go to the chosen project, else the Inbox.
- **Unchanged:** repository linkage in Settings (Repositories and Projects tabs) and the server API, which still accepts a `repo` argument.
- **Known limitations:** a saved repository choice from an earlier visit is ignored.
